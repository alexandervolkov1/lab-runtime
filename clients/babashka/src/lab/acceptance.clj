(ns lab.acceptance
  "Actual A/B process checkpoints for the M6 virtual loopback slice."
  (:require [cheshire.core :as json]
            [lab.client :as client]))

(defn- require-completed [reply]
  (when (not= (:state reply) "completed")
    (throw (ex-info "operation_failed" {:code (:code reply) :reply reply})))
  (:result reply))

(defn- until! [deadline-ms predicate]
  (let [deadline (+ (System/nanoTime) (* 1000000 deadline-ms))]
    (loop []
      (if-let [value (predicate)] value
        (do (when (>= (System/nanoTime) deadline)
              (throw (ex-info "observation_deadline" {:code "observation_deadline"})))
            (Thread/yield) (recur))))))

(defn- output! [connection actuator]
  (client/query! connection "output" {:actuator actuator}))

(defn- controller! [connection controller]
  (client/query! connection "controller" {:controller controller}))

(defn- reference! [connection reference]
  (client/query! connection "reference" {:reference reference}))

(defn- plant-roles [connection discovery]
  (some (fn [instrument]
          (let [descriptor (client/describe! connection (:id instrument))
                parameters (:parameters descriptor)
                measurement (some #(when (and (= (:role %) "measurement")
                                              (= (get-in % [:unit :id]) "degC")) %) parameters)
                actuator (some #(when (and (= (:role %) "actuator")
                                           (= (:write_effect %) "output_affecting")) %) parameters)]
            (when (and measurement actuator)
              {:signal {:instrument (:id descriptor) :parameter (:id measurement)}
               :actuator {:instrument (:id descriptor) :parameter (:id actuator)}})))
        (:instruments discovery)))

(defn- run-a [port]
  (let [connection (client/connect! port)]
    (try
      (let [discovery (client/discover! connection)
            _ (when (not= (count (:components discovery)) 2)
                (throw (ex-info "missing_real_lua_components" {})))
            {:keys [signal actuator]} (or (plant-roles connection discovery)
                                          (throw (ex-info "plant_capability_missing" {})))
            controller (get-in discovery [:controllers 0 :id])
            reference (get-in discovery [:references 0 :id])
            initially (controller! connection controller)
            _ (when (not= (:state initially) "ready")
                (throw (ex-info "controller_not_ready" {:state (:state initially)})))
            starting-sample (until! 3000 #(let [sample (client/latest! connection signal)]
                                            (when (= (:quality sample) "good") sample)))
            starting-temp (:value starting-sample)
            snapshot (client/snapshot! connection)
            _ (client/subscribe! connection (:cursor snapshot))
            ramp (reference! connection reference)
            retuned (require-completed
                     (client/command! connection "reference_retune"
                                      {:reference reference :expected_revision (:revision ramp)
                                       :target 60.0 :rate 5.0}))
            _ (client/drain-buffered! connection)
            pid (require-completed
                 (client/command! connection "controller_configure_pid"
                                  {:controller controller :expected_revision (:revision initially)
                                   :pid {:kp 4.0 :ki 0.5 :kd 0.0 :output_min 0.0 :output_max 100.0}}))
            _ (client/drain-buffered! connection)
            warming (require-completed
                     (client/command! connection "controller_start" {:controller controller}))
            _ (when (not= (:state warming) "warming")
                (throw (ex-info "start_did_not_commit_warming" {})))
            running (until! 6000 #(do (client/drain-buffered! connection)
                                      (let [state (controller! connection controller)]
                                        (when (= (:state state) "running") state))))
            movement (until! 7000 #(do (client/drain-buffered! connection)
                                       (let [sample (client/latest! connection signal)
                                             output (output! connection actuator)]
                                         (when (and (= (:quality sample) "good")
                                                    (> (:value sample) (+ starting-temp 0.01))
                                                    (= (:state output) "armed_auto")
                                                    (some? (:readback output)))
                                           {:sample sample :output output}))))
            before-retune (reference! connection reference)
            continuous (require-completed
                        (client/command! connection "reference_retune"
                                         {:reference reference :expected_revision (:revision before-retune)
                                          :target 55.0 :rate 2.0}))
            old-elapsed (/ (- (Long/parseLong (:committed_at continuous))
                              (Long/parseLong (:last_at before-retune))) 1.0e9)
            expected-value (min 60.0 (+ (:value before-retune) (* 5.0 old-elapsed)))
            _ (when (or (neg? old-elapsed)
                        (> (Math/abs (- (:value continuous) expected-value)) 0.001))
                (throw (ex-info "ramp_retune_discontinuous" {:before before-retune
                                                              :result continuous})))
            controller-after (controller! connection controller)
            _ (when (or (not= (:state controller-after) "running")
                        (not= (:revision controller-after) (:revision pid))
                        (not (<= 0.0 (:latest_output controller-after) 100.0)))
                (throw (ex-info "pid_changed_or_stopped_on_ramp_retune" {})))
            _ (client/drain-buffered! connection)
            lease (:output movement)
            checkpoint {:checkpoint "a" :boot_id @(:boot-id connection)
                        :scope @(:scope connection) :cursor (or @(:cursor connection) (:cursor snapshot))
                        :controller controller :reference reference :signal signal :actuator actuator
                        :controller_state (:state running) :sample_quality (get-in movement [:sample :quality])
                        :temperature (get-in movement [:sample :value])
                        :sent_value (get-in lease [:sent :value])
                        :instance (:instance lease) :owner (:owner lease) :epoch (:epoch lease)
                        :lease_expires_at (:lease_expires_at lease)
                        :retune_request_id {:scope @(:scope connection) :seq "4"}
                        :next_seq (str @(:next-seq connection))
                        :retune_value (:value continuous) :retune_at (:committed_at continuous)
                        :ramp_revision (:revision retuned) :pid_revision (:revision pid)}]
        (println (json/generate-string checkpoint))
        (flush)
        ;; The Rust harness kills A without pause; this socket stays open until then.
        (loop [] (try (client/read-event! connection)
                      (catch Exception error
                        (when (not= "frame_timeout" (:code (ex-data error))) (throw error))))
               (recur)))
      (finally (client/close! connection)))))

(defn- run-b [port checkpoint-text]
  (let [checkpoint (json/parse-string checkpoint-text true)
        connection (client/connect! port (:scope checkpoint))]
    (try
      (when (not= @(:boot-id connection) (:boot_id checkpoint))
        (throw (ex-info "instance_changed" {})))
      (reset! (:cursor connection) (:cursor checkpoint))
      (let [retune-status (client/operation-status! connection (:retune_request_id checkpoint))
            _ (when (not= (:state retune-status) "completed")
                (throw (ex-info "retune_outcome_not_retained" {:status retune-status})))
            snapshot (client/snapshot! connection)
            old-subscription (client/subscribe! connection (:cursor checkpoint))
            replay (until! 3000 #(try (let [event (client/read-event! connection)]
                                        (when (some? (:seq event)) event))
                                      (catch Exception error
                                        (when (not= "frame_timeout" (:code (ex-data error)))
                                          (throw error)) nil)))
            _ (client/query! connection "unsubscribe"
                             {:subscription (:subscription old-subscription)})]
        ;; The prior cursor is explicitly replayed above. A fresh frozen barrier
        ;; can then be installed if the caller chooses; queries remain pure.
        (let [controller (:controller checkpoint)
              actuator (:actuator checkpoint)
              signal (:signal checkpoint)
              reference (:reference checkpoint)
              before (controller! connection controller)
              output-before (output! connection actuator)
              latest (client/latest! connection signal)
              _ (when (not= (:state before) "running")
                  (throw (ex-info "native_owner_stopped_on_client_disconnect" {})))
              _ (when (not= (:quality latest) "good")
                  (throw (ex-info "plant_observation_missing" {})))
              _ (when (<= (:value latest) (:temperature checkpoint))
                  (throw (ex-info "plant_did_not_move_client_free" {})))
              _ (when (not= (:instance output-before) (:instance checkpoint))
                  (throw (ex-info "authority_instance_changed" {})))
              before-ref (reference! connection reference)
              paused (require-completed
                      (client/command! connection "controller_pause" {:controller controller}))
              _ (when (not= (:state paused) "paused")
                  (throw (ex-info "pause_failed" {})))
              safe (output! connection actuator)
              _ (when (or (not (:safe_confirmed safe)) (some? (:owner safe)))
                  (throw (ex-info "rust_safe_evidence_missing" {})))
              after-ref (until! 1000 #(let [now (reference! connection reference)]
                                        (when (not= (:last_at now) (:last_at before-ref)) now)))
              terminal (require-completed
                        (client/command! connection "runtime_shutdown" {}))
              final {:checkpoint "b" :boot_id @(:boot-id connection)
                     :resumed_scope @(:scope connection)
                     :replay_seq (:seq replay)
                     :state_before_pause (:state before)
                     :instance (:instance output-before) :owner (:owner output-before)
                     :epoch (:epoch output-before) :lease_expires_at (:lease_expires_at output-before)
                     :temperature (:value latest) :safe_confirmed (:safe_confirmed safe)
                     :lease_after_pause (:lease_expires_at safe)
                     :reference_paused_last_at (:last_at after-ref)
                     :shutdown_safe (:safe_confirmed terminal)
                     :shutdown_cleanup (:cleanup_complete terminal)}]
          (println (json/generate-string final)) (flush)))
      (finally (client/close! connection)))))

(defn -main [mode port & [checkpoint]]
  (let [port (Integer/parseInt port)]
    (case mode
      "a" (run-a port)
      "b" (run-b port checkpoint)
      (throw (ex-info "unknown_acceptance_mode" {:mode mode})))))
