;; Explicit virtual-demo acceptance: direct WS/WSS Runtime authority plus local
;; Workbench presentation. No ambiguous mutation replay and no measurement relay.
(require '[cheshire.core :as json])
(import '[java.net URI Socket InetSocketAddress]
        '[java.net.http HttpClient WebSocket$Listener]
        '[java.time Duration]
        '[java.util.concurrent ArrayBlockingQueue TimeUnit]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter])

(defn check [condition message]
  (when-not condition (throw (ex-info message {}))))

(defn connection-options [arguments]
  (loop [remaining arguments options {}]
    (if (empty? remaining)
      options
      (let [[flag value & rest] remaining]
        (case flag
          "--allow-insecure-ws"
          (do (check (not (:allow-insecure options)) "duplicate plaintext opt-in")
              (recur (next remaining) (assoc options :allow-insecure true)))
          "--token-env"
          (do (check (and value (not (.startsWith value "--")) (not (:token-env options)))
                     "--token-env requires one variable name")
              (recur rest (assoc options :token-env value)))
          (throw (ex-info "unknown connection option" {})))))))

(let [[endpoint port & options] *command-line-args*
      parsed (connection-options options)
      allow? (:allow-insecure parsed)
      token-name (:token-env parsed)
      _ (check (and endpoint (<= (count endpoint) 1024)) "missing or oversized endpoint")
      uri (try (URI. endpoint) (catch Exception _ (throw (ex-info "invalid endpoint" {}))))
      queue (ArrayBlockingQueue. 32)
      partial (StringBuilder.)
      partial-bytes (atom 0)
      ids (atom 0)
      next-id #(str "distributed-" (swap! ids inc))]
  (check (and endpoint port (#{"ws" "wss"} (.getScheme uri))
              (= "/application/v1" (.getPath uri))
              (nil? (.getUserInfo uri)) (nil? (.getQuery uri)) (nil? (.getFragment uri)))
         "usage: bb distributed.clj WS_OR_WSS_ENDPOINT WORKBENCH_PORT|--runtime-only [--allow-insecure-ws] [--token-env NAME]")
  (check (or (= "wss" (.getScheme uri)) (#{"127.0.0.1" "[::1]"} (.getHost uri)) allow?)
         "remote plaintext WS requires explicit --allow-insecure-ws")
  (when (and (= "ws" (.getScheme uri)) allow?)
    (binding [*out* *err*] (println "WARNING: plaintext WS on an explicitly trusted network only")))
  (let [listener (reify WebSocket$Listener
                   (onOpen [_ ws] (.request ws 1))
                   (onText [_ ws text last]
                     (swap! partial-bytes + (alength (.getBytes (str text) "UTF-8")))
                     (if (> @partial-bytes 16383)
                       (do (.offer queue {:transport-error true}) (.abort ws))
                       (do (.append partial (str text))
                           (when last
                             (let [frame (try (json/parse-string (.toString partial) true)
                                              (catch Exception _ {:transport-error true}))]
                               (.setLength partial 0)
                               (reset! partial-bytes 0)
                               (when-not (.offer queue frame) (.abort ws))))
                           (.request ws 1)))
                     nil)
                   (onBinary [_ ws _ _] (.offer queue {:transport-error true}) (.abort ws) nil)
                   (onClose [_ _ _ _] (.offer queue {:transport-error true}) nil)
                   (onError [_ _ _] (.offer queue {:transport-error true})))
        client (.build (.connectTimeout (HttpClient/newBuilder) (Duration/ofSeconds 2)))
        builder (doto (.newWebSocketBuilder client)
                  (.connectTimeout (Duration/ofSeconds 2))
                  (.subprotocols "lab-runtime.application.v1" (into-array String []))
                  (.header "Origin" "http://127.0.0.1:3000"))
        _ (when token-name
            (let [token (System/getenv token-name)]
              (check (and token (<= 1 (count token) 512) (re-matches #"[\x20-\x7e]+" token))
                     "tunnel token environment variable is unavailable or invalid")
              (.header builder "X-Token" token)))
        ;; Do not print Java handshake exceptions: a proxy may reflect credentials.
        ws (try (.get (.buildAsync builder uri listener) 3 TimeUnit/SECONDS)
                (catch Exception _ (throw (ex-info "TLS/authorized WebSocket handshake failed" {}))))]
    (try
      (letfn [(send! [message]
                (check (<= (alength (.getBytes (json/generate-string message) "UTF-8")) 16383) "Application message bound")
                (.get (.sendText ws (json/generate-string message) true) 2 TimeUnit/SECONDS))
              (receive [id terminal?]
                (let [deadline (+ (System/nanoTime) 5000000000)]
                  (loop [remaining 128]
                    (check (and (pos? remaining) (< (System/nanoTime) deadline)) "Runtime response deadline; mutation is not replayed")
                    (let [frame (.poll queue (max 1 (quot (- deadline (System/nanoTime)) 1000000)) TimeUnit/MILLISECONDS)]
                      (check (and frame (not (:transport-error frame))) "Runtime transport lost; mutation is not replayed")
                      (if (and (= id (:msg_id frame)) (or (not terminal?) (#{"completed" "failed"} (:state frame))))
                        frame (recur (dec remaining)))))))
              (query [op args]
                (let [id (next-id)]
                  (send! {:v 1 :msg_id id :op op :args args})
                  (let [frame (receive id false)]
                    (check (= "result" (:type frame)) "Runtime query rejected")
                    (:result frame))))]
        (let [hello (query "hello" {:scope nil})
              discovery (query "discover" {})
              _ (check (some #(and (= "instrument" (:kind %)) (= "1" (:id %)) (= "virtual" (:identity_class %))) (:records discovery))
                       "this acceptance example requires virtual-demo instrument 1")
              before (query "reference" {:reference "1"})
              id (next-id)
              identity {:scope (:scope hello) :seq (:next_seq hello)}
              target (+ 1.0 (:target before))]
          (send! {:v 1 :msg_id id :op "reference_retune" :request_id identity
                  :args {:reference "1" :expected_revision (:revision before) :target target :rate 2.0}})
          (let [accepted (receive id false)]
            (check (and (= "operation" (:type accepted)) (= "accepted" (:state accepted)) (= identity (:request_id accepted)))
                   "Runtime did not authoritatively accept the mutation"))
          (let [completed (receive id true)]
            (check (and (= "completed" (:state completed)) (= identity (:request_id completed))) "Runtime mutation did not complete"))
          (let [after (query "reference" {:reference "1"})]
            (check (= target (:target after)) "Runtime snapshot did not confirm mutation")
            ;; Independent local presentation channel, never forwards observations.
            (if (= port "--runtime-only")
              (println (json/generate-string {:status "PASS" :surface "direct-ws-runtime"
                                              :runtime_revision (:revision after) :runtime_target target
                                              :workbench_required false :mutation_replay false}))
              (with-open [socket (Socket.)]
              (.connect socket (InetSocketAddress. "127.0.0.1" (Integer/parseInt port)) 2000)
              (.setSoTimeout socket 5000)
              (with-open [reader (BufferedReader. (InputStreamReader. (.getInputStream socket) "UTF-8"))
                          writer (OutputStreamWriter. (.getOutputStream socket) "UTF-8")]
                (letfn [(request [call op args]
                          (.write writer (str (json/generate-string {:v 1 :type "request" :call_id call :op op :args args}) "\n"))
                          (.flush writer)
                          (loop [remaining 64]
                            (check (pos? remaining) "Workbench response bound")
                            (let [line (.readLine reader)]
                              (check line "Workbench closed")
                              (let [frame (json/parse-string line true)]
                                (if (= call (:call_id frame))
                                  (do (check (= "result" (:type frame)) "Workbench request rejected") (:result frame))
                                  (recur (dec remaining)))))))]
                  (let [workbench (request "hello" "hello" {})
                        before (request "before" "presentation_get" {})
                        revision (:presentation_revision before)
                        plot {:id (str "distributed-" revision) :title "Distributed virtual-demo"
                              :time_window_seconds 60.0 :axes {:y_min nil :y_max nil}
                              :traces [{:id (str "distributed-temperature-" revision) :source {:kind "signal" :instrument "1" :parameter "1"}
                                        :display_label "Runtime temperature" :visible true
                                        :style {:color "cyan" :width 1.5} :display_unit nil}]}
                        changed (request "add" "ui_add_plot" {:expected {:workbench_id (:workbench_id workbench) :revision revision} :plot plot})
                        presented (request "after" "presentation_get" {})]
                    (check (= (:presentation_revision changed) (:presentation_revision presented) (str (inc (bigint revision))))
                           "presentation revision did not advance exactly once")
                    (check (some #(= (:id plot) (:id %)) (get-in presented [:document :plots])) "plot did not commit")
                    (println (json/generate-string {:status "PASS" :surface "distributed-virtual-demo"
                                                    :runtime_revision (:revision after) :runtime_target target
                                                    :presentation_revision (:presentation_revision presented)
                                                    :mutation_replay false :measurement_relay false}))))))))))
      (finally
        (try (.get (.sendClose ws 1000 "done") 1 TimeUnit/SECONDS) (catch Exception _ (.abort ws)))))))
