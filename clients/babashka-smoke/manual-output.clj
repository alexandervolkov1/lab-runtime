;; Optional external Application v1 check for the built-in virtual furnace.
;; No reconnect or mutation retry. Runtime does not require this script or bb.
(require '[cheshire.core :as json]
         '[babashka.cli :as cli]
         '[babashka.http-client.websocket :as ws])
(import '[java.net Socket InetSocketAddress URI]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter]
        '[java.util.concurrent ArrayBlockingQueue TimeUnit])

(defn check [ok message] (when-not ok (throw (ex-info message {}))))
(defn emit [kind value] (println (json/generate-string {:kind kind :data value})))
(defn now [] (System/nanoTime))
(defn remaining-ms [deadline]
  (let [ns (- deadline (now))]
    (check (pos? ns) "Timed out; command outcome may be unknown. Do not repeat it automatically.")
    (max 1 (quot ns 1000000))))

(defn connect [{:keys [url origin timeout-ms]}]
  (let [uri (URI. url) scheme (.getScheme uri)]
    (check (and (.getHost uri) (nil? (.getUserInfo uri)) (nil? (.getQuery uri))
                (nil? (.getFragment uri))) "URL must have a host and no credentials, query or fragment")
    (case scheme
      "tcp"
      (let [socket (Socket.)]
        (.connect socket (InetSocketAddress. (.getHost uri) (.getPort uri)) timeout-ms)
        (let [reader (BufferedReader. (InputStreamReader. (.getInputStream socket) "UTF-8"))
              writer (OutputStreamWriter. (.getOutputStream socket) "UTF-8")]
          {:send (fn [text] (.write writer (str text "\n")) (.flush writer))
           :receive (fn [deadline]
                      (.setSoTimeout socket (int (remaining-ms deadline)))
                      (let [line (.readLine reader)]
                        (check line "Runtime disconnected; preserve the last request identity") line))
           :close #(.close socket)}))
      ("ws" "wss")
      (let [inbox (ArrayBlockingQueue. 128) failure (atom nil) fragments (atom "")
            token (System/getenv "LAB_RUNTIME_ACCESS_KEY")
            _ (check (or (not token) (= scheme "wss")
                         (#{"localhost" "127.0.0.1" "::1"} (.getHost uri)))
                     "Send an access key only over WSS or loopback WS")
            socket (ws/websocket
                    {:uri url :connect-timeout timeout-ms
                     :headers (cond-> {"Origin" origin} token (assoc "X-Token" token))
                     :subprotocols ["lab-runtime.application.v1"]
                     :on-message
                     (fn [socket data last?]
                       (let [text (swap! fragments str data)]
                         (if (> (count text) 65536)
                           (do (reset! failure "WebSocket message exceeds client bound") (ws/abort! socket))
                           (when last?
                             (reset! fragments "")
                             (when-not (.offer inbox text)
                               (reset! failure "Client event queue overflow; rebuild state")
                               (ws/abort! socket))))))
                     :on-error (fn [_ _] (reset! failure "WebSocket failed; do not repeat commands"))
                     :on-close (fn [_ _ _] (.offer inbox ::closed))})]
        {:send (fn [text] (.get (.sendText socket text true) timeout-ms TimeUnit/MILLISECONDS))
         :receive (fn [deadline]
                    (check (nil? @failure) @failure)
                    (let [text (.poll inbox (remaining-ms deadline) TimeUnit/MILLISECONDS)]
                      (check text "Timed out waiting for WebSocket response or event")
                      (check (not= text ::closed) "Runtime closed WebSocket; preserve request identity")
                      text))
         :close #(ws/abort! socket)})
      (throw (ex-info "URL scheme must be tcp, ws or wss" {})))))

(defn run-check [options]
  (let [{send :send receive :receive close :close} (connect options)
        timeout-ns (* 1000000 (:timeout-ms options))
        msg (atom 0) seqno (atom 0) scope (atom nil)
        signal {:instrument "1" :parameter "1"} actuator {:instrument "1" :parameter "2"}
        points (atom 0) latest-point (atom nil)]
    (try
      (letfn [(event! [reply]
                (when (and (= "event" (:type reply)) (= "signal" (:kind reply))
                           (= signal (:target reply)))
                  (let [point (:data reply)]
                    (check (= "good" (:quality point)) "Furnace measurement unavailable")
                    (when @latest-point
                      (check (> (parse-long (:observed_at_ns point))
                                (parse-long (:observed_at_ns @latest-point)))
                             "Subscription did not deliver a new measurement"))
                    (reset! latest-point point)
                    (swap! points inc)))
                (emit "event" reply))
              (read! [deadline] (json/parse-string (receive deadline) true))
              (exchange [op args mutation? expected-failure?]
                (let [id (str "check-" (swap! msg inc))
                      request (cond-> {:v 1 :msg_id id :op op :args args}
                                mutation? (assoc :request_id {:scope @scope :seq (str (swap! seqno inc))}))
                      deadline (+ (now) timeout-ns)]
                  ;; Preserve exact mutation identity/payload, never HTTP secrets.
                  (emit "request" request)
                  (send (json/generate-string request))
                  (loop []
                    (let [reply (read! deadline)]
                      (if (#{"event" "subscription_progress"} (:type reply))
                        (do (event! reply) (recur))
                        (do
                          (emit "response" reply)
                          (check (= id (:msg_id reply)) "Unexpected response correlation")
                          (if (= "accepted" (:state reply))
                            (recur)
                            (do
                              (check (if expected-failure? (= "failed" (:state reply))
                                         (if mutation? (= "completed" (:state reply)) (= "result" (:type reply))))
                                     (str op " failed or returned an unexpected outcome; no automatic retry"))
                              (if expected-failure? reply (:result reply))))))))))
              (query [op args] (exchange op args false false))
              (mutate [op args] (exchange op args true false))
              (state [] (query "output" {:actuator actuator}))
              (fences [] (let [s (state)] {:actuator actuator :expected_instance (:authority_instance s)
                                         :expected_epoch (:epoch s)}))
              (wait-query [query-fn predicate]
                (let [deadline (+ (now) timeout-ns)]
                  (loop []
                    (remaining-ms deadline)
                    (let [value (query-fn)]
                      (if (predicate value) value
                          (do (Thread/sleep 50) (recur)))))))
              (safe [] (wait-query state #(and (:safe_confirmed %) (nil? (:owner %)))))
              (new-points [n]
                ;; No latest/measurement_window, or any request, in this loop.
                (let [target (+ @points n) deadline (+ (now) timeout-ns)]
                  (while (< @points target)
                    (let [reply (read! deadline)]
                      (check (#{"event" "subscription_progress"} (:type reply))
                             "Unexpected response while waiting for subscription")
                      (event! reply)))))
              (controller [] (query "controller" {:controller "1"}))
              (wait-controller [expected] (wait-query controller #(= expected (:state %))))
              (history [args]
                (let [job (mutate "history_read" args)
                      page (query "history_page" {:page_token (:page_token job)})]
                  (check (:released (query "history_release" {:page_token (:page_token job)}))
                         "History page not released") page))]
        (let [hello (query "hello" {:scope nil})
              _ (reset! scope (:scope hello))
              _ (reset! seqno (dec (parse-long (:next_seq hello))))
              records (loop [page (query "discover" {}) found []]
                        (let [found (into found (:records page))]
                          (if (:complete page) found
                              (recur (query "discovery_page" {:projection (:projection page) :index (:next_index page)}) found))))
              device (first (filter #(and (= "instrument" (:kind %)) (= "1" (:id %))) records))
              _ (check (= "virtual" (:identity_class device)) "This check requires the built-in virtual furnace")
              _ (emit "descriptor" (query "describe" {:instrument "1"}))
              before (query "latest" {:signal signal})
              _ (check (= "good" (:quality before)) "Initial temperature unavailable")
              _ (check (some #{"recording_start"} (:operations hello)) "Start Runtime with --record-db")
              cursor {:boot_id (:boot_id hello)
                      :seq (:event_seq (:revision (query "measurements_current" {})))}
              subscription (query "subscribe" {:after cursor :filter {:kinds ["signal"] :targets []}})
              _ (new-points 4)
              run (mutate "recording_start" {:label "Furnace API check"})
              initial (safe)
              lease "2000000000" ttl "200000000"
              _ (check (>= (parse-long (get-in initial [:profile :max_lease_ns])) (parse-long lease))
                       "Furnace lease profile differs from built-in profile")]
          (mutate "output_acquire" (assoc (fences) :lease_lifetime_ns lease))
          (doseq [power [10.0 20.0 15.0]]
            (let [receipt (mutate "output_write" (assoc (fences) :value power :proposal_ttl_ns ttl))]
              (check (and (:terminal receipt) (= "virtual_simulation" (:evidence_source receipt))
                          (#{"acknowledged" "readback_verified"} (:outcome receipt)))
                     "Write lacks terminal simulated delivery evidence")
              (new-points 5)
              (let [expires (:lease_expires_at (state))]
                (mutate "output_renew" (assoc (fences) :lease_lifetime_ns lease))
                (check (> (parse-long (:lease_expires_at (state))) (parse-long expires))
                       "Lease did not advance"))))
          (mutate "output_safe" (fences))
          (safe)
          (check (> (:value @latest-point) (:value before)) "Virtual furnace temperature did not rise")
          ;; Observe finite expiry without renewal, then exercise PID lifecycle.
          (mutate "output_acquire" (assoc (fences) :lease_lifetime_ns "500000000"))
          (safe)
          (let [reference (query "reference" {:reference "1"})]
            (mutate "reference_configure" {:reference "1" :expected_revision (:revision reference)
                                           :kind "fixed" :value 40.0}))
          (mutate "controller_start" {:controller "1"})
          (wait-controller "running")
          (let [conflict (exchange "output_acquire" (assoc (fences) :lease_lifetime_ns lease) true true)]
            (check (and (= "busy" (:code conflict)) (= "automatic" (get-in (state) [:owner :kind])))
                   "Expected PID ownership conflict without changing authority")
            (emit "expected_conflict" conflict))
          (new-points 5)
          (mutate "controller_pause" {:controller "1"})
          (wait-controller "paused") (safe)
          (mutate "controller_resume" {:controller "1"})
          (wait-controller "running") (new-points 5)
          (mutate "controller_pause" {:controller "1"})
          (wait-controller "paused") (safe)
          (let [stopped (mutate "recording_stop" {:run_id (:run_id run)})
                completion (:completion stopped)
                status (query "recording_status" {})
                database (:database_id status)
                runs (history {:mode "runs" :database_id database :max_records 32 :cursor nil})
                recorded (history {:mode "measurements" :database_id database :boot_id (:boot_id hello)
                                   :run_id (:run_id run) :signal signal :from_ns "0"
                                   :to_ns (str (inc (parse-long (:observed_at_ns @latest-point))))
                                   :max_records 128 :cursor nil})]
            (check (every? true? (map completion [:accepted_facts_drained :interval_sealed :transaction_committed]))
                   "Recorder stop lacks durable completion evidence")
            (check (some #(= (:run_id run) (:run_id %)) (:runs runs)) "Recorded run missing from history")
            (emit "history" recorded)
            (check (seq (:rows recorded)) "Recorded measurements missing from history")
            (query "unsubscribe" {:subscription (:subscription subscription)})
            (when (:shutdown options)
              (check (:recorder_flushed (mutate "runtime_shutdown" {}))
                     "Shutdown did not confirm Recorder flush"))
            (emit "PASS" {:transport (.getScheme (URI. (:url options))) :new_measurements @points
                          :run_id (:run_id run) :recording stopped}))))
      (finally (close)))))

(try
  (let [options (cli/parse-opts *command-line-args*
                              {:restrict true
                               :spec {:url {:default "tcp://127.0.0.1:7420"}
                                      :origin {:default "http://localhost:3000"}
                                      :timeout-ms {:coerce :long :default 10000}
                                      :shutdown {:coerce :boolean} :help {:coerce :boolean}}})]
    (if (:help options)
      (println "bb manual-output.clj [--url tcp://HOST:PORT | ws://HOST:PORT/application/v1 | wss://HOST/application/v1] [--origin ORIGIN] [--timeout-ms 10000] [--shutdown]\nWSS access key: LAB_RUNTIME_ACCESS_KEY environment variable. Requires virtual-demo with Recorder. No automatic command retries.")
      (do (check (<= 1000 (:timeout-ms options) 60000) "timeout-ms must be 1000..60000")
          (run-check options))))
  (catch Exception error
    (binding [*out* *err*] (println "FAIL:" (.getMessage error)))
    (System/exit 1)))
