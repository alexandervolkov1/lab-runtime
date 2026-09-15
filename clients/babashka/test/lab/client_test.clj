(ns lab.client-test
  "Actual Babashka socket-level checks for bounded failure and recovery signals."
  (:require [cheshire.core :as json]
            [clojure.test :refer [deftest is run-tests]]
            [lab.client :as client])
  (:import [java.net ServerSocket]
           [java.io BufferedReader InputStreamReader]))

(def boot "0123456789abcdef0123456789abcdef")
(defn- write-json! [socket value]
  (let [output (.getOutputStream socket)]
    (.write output (.getBytes (str (json/generate-string value) "\n") "UTF-8"))
    (.flush output)))

(defn- scripted-server [script]
  (let [listener (ServerSocket. 0)
        port (.getLocalPort listener)
        worker (future
                 (with-open [server (.accept listener)]
                   (let [reader (BufferedReader. (InputStreamReader. (.getInputStream server) "UTF-8"))
                         hello (json/parse-string (.readLine reader) true)]
                     (write-json! server {:v 1 :type "result" :msg_id (:msg_id hello)
                                          :result {:boot_id boot :scope (str boot ":1") :next_seq "1"}})
                     (script server reader))))]
    {:port port :worker worker :listener listener}))

(defn- finish! [server]
  (.close ^ServerSocket (:listener server))
  (deref (:worker server) 2000 ::unfinished))

(deftest malformed_utf8_event_is_rejected_without_replacement_decoding
  (let [server (scripted-server
                (fn [socket _]
                  (let [out (.getOutputStream socket)]
                    (.write out (.getBytes (str "{\"v\":1,\"type\":\"event\",\"boot_id\":\"" boot
                                                "\",\"seq\":\"1\",\"data\":\"") "UTF-8"))
                    (.write out (byte-array [(byte -1)]))
                    (.write out (.getBytes "\"}\n" "UTF-8"))
                    (.flush out))))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (is (= "invalid_utf8" (:code (ex-data (try (client/read-event! connection)
                                                    (catch Exception error error))))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest replay_gap_requires_resync_instead_of_a_generic_stream_error
  (let [server (scripted-server
                (fn [socket _]
                  (write-json! socket {:v 1 :type "error" :code "event_gap"
                                       :resync_required true})))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (is (= "resync_required" (:code (ex-data (try (client/read-event! connection)
                                                        (catch Exception error error))))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest failed_terminal_outcome_advances_sequence_but_never_claims_completion
  (let [server (scripted-server
                (fn [socket reader]
                  (let [command (json/parse-string (.readLine reader) true)
                        correlation (:msg_id command)
                        request-id (:request_id command)]
                    (write-json! socket {:v 1 :type "operation" :msg_id correlation
                                         :request_id request-id :state "accepted"})
                    (write-json! socket {:v 1 :type "operation" :msg_id correlation
                                         :request_id request-id :state "failed" :code "revision_conflict"}))))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (let [outcome (client/command! connection "reference_retune"
                                         {:reference "1" :expected_revision "1" :target 40.0 :rate 2.0})]
            (is (= "failed" (:state outcome)))
            (is (= "revision_conflict" (:code outcome)))
            (is (= 2 @(:next-seq connection))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest overflow_after_acceptance_exposes_original_uncertain_request_id
  (let [server (scripted-server
                (fn [socket reader]
                  (let [command (json/parse-string (.readLine reader) true)
                        request-id (:request_id command)]
                    (write-json! socket {:v 1 :type "operation" :msg_id (:msg_id command)
                                         :request_id request-id :state "accepted"})
                    (dotimes [n 17]
                      (write-json! socket {:v 1 :type "event" :boot_id boot :seq (str (inc n))
                                           :kind "signal" :data {:value n}})))))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (let [error (try (client/command! connection "reference_retune"
                                             {:reference "1" :expected_revision "1" :target 40.0 :rate 2.0})
                           (catch Exception error error))]
            (is (= "resync_required" (:code (ex-data error))))
            (is (= {:scope (str boot ":1") :seq "1"} @(:uncertain connection))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest unknown_outcome_remains_epistemic_and_does_not_issue_a_replacement_command
  (let [server (scripted-server
                (fn [socket reader]
                  (let [query (json/parse-string (.readLine reader) true)]
                    (write-json! socket {:v 1 :type "result" :msg_id (:msg_id query)
                                         :result {:state "outcome_unknown"}}))))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (is (= "outcome_unknown" (:state (client/operation-status! connection
                                                {:scope (str boot ":1") :seq "1"}))))
          (is (= 1 @(:next-seq connection)))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest changed_boot_in_event_requires_instance_reconciliation
  (let [server (scripted-server
                (fn [socket _]
                  (write-json! socket {:v 1 :type "event"
                                       :boot_id "fedcba9876543210fedcba9876543210" :seq "1"})))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (is (= "instance_changed" (:code (ex-data (try (client/read-event! connection)
                                                         (catch Exception error error))))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest frozen_snapshot_pages_cannot_accumulate_above_the_client_budget
  (let [cursor {:boot_id boot :seq "0"}
        record {:kind "signal" :data {:blob (apply str (repeat 8000 "x"))}}
        server (scripted-server
                (fn [socket reader]
                  (loop [page 0]
                    (when-let [request (.readLine reader)]
                      (let [exchange (json/parse-string request true)
                            op (:op exchange)]
                        (cond
                          (= op "snapshot_release")
                          (write-json! socket {:v 1 :type "result" :msg_id (:msg_id exchange)
                                               :result {:released true}})
                          (#{"runtime_snapshot" "snapshot_page"} op)
                          (do (write-json! socket {:v 1 :type "result" :msg_id (:msg_id exchange)
                                                   :result {:snapshot "s1" :cursor cursor
                                                            :records [record] :next_index (when (< page 35) (str (inc page)))}})
                              (recur (inc page)))))))))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (is (= "resync_required" (:code (ex-data (try (client/snapshot! connection)
                                                        (catch Exception error error))))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(deftest reconnect_discards_already_applied_sequence_before_new_replay_event
  (let [server (scripted-server
                (fn [socket _]
                  (write-json! socket {:v 1 :type "event" :boot_id boot :seq "3" :kind "signal"})
                  (write-json! socket {:v 1 :type "event" :boot_id boot :seq "4" :kind "signal"})))]
    (try
      (let [connection (client/connect! (:port server))]
        (try
          (reset! (:cursor connection) {:boot_id boot :seq "3"})
          (is (= "4" (:seq (client/read-event! connection))))
          (is (= "4" (:seq @(:cursor connection))))
          (finally (client/close! connection))))
      (finally (finish! server)))))

(defn -main []
  (let [result (run-tests 'lab.client-test)]
    (when (pos? (+ (:fail result) (:error result)))
      (System/exit 1))))
