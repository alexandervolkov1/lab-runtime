;; Direct Runtime example. No Workbench, internal Rust access, retry or reconnect.
;; Default target is the built-in virtual thermal plant. Physical use requires an
;; operator-identified Arduino emulator with an independently verified definition.
(require '[cheshire.core :as json])
(import '[java.net Socket InetSocketAddress]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter])

(defn check [ok message] (when-not ok (throw (ex-info message {}))))
(defn emit [kind value] (println (json/generate-string {:kind kind :data value})))

(let [[port option instrument temperature output & extra] *command-line-args*
      physical (= option "--identified-emulator")
      shutdown (= option "--shutdown-fixture")
      _ (check (and port (empty? extra)
                    (or (nil? option) (and shutdown (nil? instrument)) (and physical instrument temperature output)))
               "usage: bb manual-output.clj PORT [--shutdown-fixture | --identified-emulator INSTRUMENT TEMPERATURE_PARAMETER OUTPUT_PARAMETER]")
      instrument (or instrument "1") temperature (or temperature "1") output (or output "2")
      actuator {:instrument instrument :parameter output}
      signal {:instrument instrument :parameter temperature}
      message (atom 0) sequence (atom 0) scope (atom nil)]
  (with-open [socket (Socket.)]
    (.connect socket (InetSocketAddress. "127.0.0.1" (Integer/parseInt port)) 2000)
    (.setSoTimeout socket 35000)
    (with-open [reader (BufferedReader. (InputStreamReader. (.getInputStream socket) "UTF-8"))
                writer (OutputStreamWriter. (.getOutputStream socket) "UTF-8")]
      (letfn [(exchange [op args mutate?]
                (let [id (str "manual-" (swap! message inc))
                      request (cond-> {:v 1 :msg_id id :op op :args args}
                                mutate? (assoc :request_id {:scope @scope :seq (str (swap! sequence inc))}))]
                  (emit "request" request)
                  (.write writer (str (json/generate-string request) "\n")) (.flush writer)
                  (loop []
                    (let [line (.readLine reader)]
                      (check line "Runtime disconnected; preserve request identity; do not repeat mutation")
                      (let [reply (json/parse-string line true)]
                        (emit "response" reply)
                        (check (= id (:msg_id reply)) "unexpected response correlation")
                        (if (= "accepted" (:state reply))
                          (recur)
                          (do (check (if mutate? (= "completed" (:state reply)) (= "result" (:type reply)))
                                     "operation not successful; do not retry or automatically rearm")
                              (:result reply))))))))
              (query [op args] (exchange op args false))
              (mutate [op args] (exchange op args true))
              (state [] (query "output" {:actuator actuator}))
              (fences [] (let [s (state)] {:actuator actuator :expected_instance (:authority_instance s) :expected_epoch (:epoch s)}))
              (wait-safe []
                (loop [remaining 100]
                  (let [s (state)]
                    (if (:safe_confirmed s) s
                        (do (check (pos? remaining) "safe evidence not obtained; no automatic retry")
                            (Thread/sleep 100) (recur (dec remaining)))))))]
        (let [hello (query "hello" {:scope nil})
              _ (reset! scope (:scope hello))
              _ (reset! sequence (dec (Long/parseLong (:next_seq hello))))
              _ (doseq [op ["output_acquire" "output_renew" "output_write" "output_safe" "recording_start" "recording_stop"]]
                  (check (some #{op} (:operations hello)) (str "missing operation " op)))
              records (loop [page (query "discover" {}) records []]
                        (let [records (into records (:records page))]
                          (if (:complete page) records
                              (recur (query "discovery_page" {:projection (:projection page) :index (:next_index page)}) records))))
              device (first (filter #(and (= "instrument" (:kind %)) (= instrument (:id %))) records))
              _ (check device "instrument not discovered")
              _ (check (= (if physical "physical" "virtual") (:identity_class device))
                       "target identity class mismatch; never guess a physical device")
              descriptor (query "describe" {:instrument instrument})
              initial (state)
              _ (check (and (:manual_supported initial) (:safe_confirmed initial) (nil? (:owner initial)))
                       "output not available for explicit manual acquisition")
              lease (str (min 2000000000 (Long/parseLong (get-in initial [:profile :max_lease_ns]))))
              ttl (str (min 200000000 (Long/parseLong (get-in initial [:profile :max_proposal_ttl_ns]))))
              _ (check (>= (Long/parseLong lease) 1000000000) "example cadence requires lease of at least one second")
              values [10.0 20.0 15.0]
              _ (check (every? #(<= (get-in initial [:profile :min]) % (get-in initial [:profile :max])) values)
                       "example powers outside installed operating policy")
              before (query "latest" {:signal signal})
              run (mutate "recording_start" {:label "direct Clojure manual heater power"})]
          (emit "descriptor" descriptor)
          (mutate "output_acquire" (assoc (fences) :lease_lifetime_ns lease))
          ;; An external algorithm may choose each next value from the observed
          ;; temperature. These three fixed finite steps make this example reproducible.
          (doseq [power values]
            (let [receipt (mutate "output_write" (assoc (fences) :value power :proposal_ttl_ns ttl))]
              (check (:terminal receipt) "write result was only queued")
              (check (#{"acknowledged" "readback_verified"} (:outcome receipt)) "delivery evidence absent")
              (Thread/sleep 500)
              (emit "observed" {:temperature (query "latest" {:signal signal}) :output (state)})
              (mutate "output_renew" (assoc (fences) :lease_lifetime_ns lease))))
          (mutate "output_safe" (fences))
          (let [safe (wait-safe)
                after (query "latest" {:signal signal})
                stopped (mutate "recording_stop" {:run_id (:run_id run)})]
            (check (and (:safe_confirmed safe) (nil? (:owner safe))) "safe procedure not confirmed")
            (check (and (= "good" (:quality before)) (= "good" (:quality after))) "temperature unavailable")
            (when-not physical (check (> (:value after) (:value before)) "virtual temperature did not respond"))
            (check (and (get-in stopped [:completion :accepted_facts_drained]) (get-in stopped [:completion :interval_sealed]) (get-in stopped [:completion :transaction_committed]))
                   "recording not durably sealed")
            (when shutdown (mutate "runtime_shutdown" {}))
            (emit "PASS" {:surface "direct-runtime-manual-output" :boot_id (:boot_id hello) :scope @scope
                          :physical_emulator physical :before before :after after :safe safe :recording stopped})))))))
