(ns lab.client
  "Bounded synchronous Babashka adapter for the fixed version-one NDJSON API.
   An uncertain mutation keeps its original request ID; no hidden retry occurs."
  (:require [cheshire.core :as json])
  (:import [java.net Socket SocketTimeoutException]
           [java.io ByteArrayOutputStream]))

(def ^:private frame-limit 16384)
(def ^:private event-limit 16)
(def ^:private frame-timeout-ns 2000000000)

(defn close! [client]
  (.close ^Socket (:socket client)))

(defn- read-frame [client]
  (let [start (System/nanoTime)
        bytes (ByteArrayOutputStream.)
        input (:input client)]
    (loop [size 0]
      (let [elapsed (- (System/nanoTime) start)
            remaining (- frame-timeout-ns elapsed)]
        (when (<= remaining 0)
          (throw (ex-info "frame_timeout" {:code "frame_timeout"})))
        (.setSoTimeout ^Socket (:socket client)
                       (int (max 1 (min 100 (quot remaining 1000000)))))
        (let [byte (try (.read input)
                        (catch SocketTimeoutException _ ::timeout))]
          (cond
            (= byte ::timeout) (recur size)
            (= byte -1) (throw (ex-info "connection_closed" {:code "connection_closed"}))
            (>= size frame-limit) (throw (ex-info "frame_too_large" {:code "frame_too_large"}))
            (= byte 10) (json/parse-string (.toString bytes "UTF-8") true)
            :else (do (.write bytes (int byte)) (recur (inc size)))))))))

(defn- write-frame! [client frame]
  (let [bytes (.getBytes (str (json/generate-string frame) "\n") "UTF-8")]
    (when (> (alength bytes) frame-limit)
      (throw (ex-info "frame_too_large" {:code "frame_too_large"})))
    (.write (:output client) bytes)
    (.flush (:output client))))

(defn- buffer-event! [client event]
  (swap! (:events client)
         (fn [buffer]
           (when (>= (count buffer) event-limit)
             (throw (ex-info "event_buffer_full" {:code "resync_required"})))
           (conj buffer event))))

(defn- next-message! [client]
  (str "m" (swap! (:message client) inc)))

(defn- await-reply [client msg-id command? request-id]
  (loop [accepted? false]
    (let [frame (try (read-frame client)
                     (catch Exception error
                       (when (and command? accepted?)
                         (reset! (:uncertain client) request-id))
                       (throw (ex-info "reply_uncertain" {:code "reply_uncertain"
                                                           :request-id request-id
                                                           :accepted accepted?} error))))]
      (cond
        (or (= (:type frame) "event")
            (= (:type frame) "subscription_progress"))
        (do (buffer-event! client frame) (recur accepted?))

        (not= (:msg_id frame) msg-id)
        (throw (ex-info "unexpected_reply" {:code "unexpected_reply" :frame frame}))

        (= (:type frame) "error") frame

        (and command? (= (:state frame) "accepted"))
        (do (swap! (:next-seq client) inc) (recur true))

        command? (assoc frame :accepted accepted?)

        :else frame))))

(defn- exchange! [client op args request-id]
  (let [msg-id (next-message! client)
        command? (some? request-id)
        request (cond-> {:v 1 :msg_id msg-id :op op :args args}
                  command? (assoc :request_id request-id))]
    (write-frame! client request)
    (await-reply client msg-id command? request-id)))

(defn connect!
  "Open loopback and create/resume only the explicitly provided process scope."
  ([port] (connect! port nil))
  ([port scope]
   (let [socket (Socket. "127.0.0.1" (int port))
         client {:socket socket :input (.getInputStream socket)
                 :output (.getOutputStream socket) :message (atom 0)
                 :events (atom []) :cursor (atom nil) :scope (atom nil)
                 :boot-id (atom nil) :next-seq (atom 1) :uncertain (atom nil)}]
     (try
       (let [hello (exchange! client "hello" {:scope scope} nil)]
         (when (= (:type hello) "error")
           (throw (ex-info "hello_failed" {:code (:code hello) :reply hello})))
         (let [result (:result hello)]
           (reset! (:scope client) (:scope result))
           (reset! (:boot-id client) (:boot_id result))
           (reset! (:next-seq client) (Long/parseLong (:next_seq result))))
         client)
       (catch Exception error (close! client) (throw error))))))

(defn query!
  "Return a committed pure/session result. Domain state is never refreshed here."
  [client op args]
  (let [reply (exchange! client op args nil)]
    (if (= (:type reply) "error")
      (throw (ex-info "query_rejected" {:code (:code reply) :reply reply}))
      (:result reply))))

(defn command!
  "Issue one consecutive mutation; failure remains a returned terminal outcome."
  [client op args]
  (let [request-id {:scope @(:scope client) :seq (str @(:next-seq client))}
        reply (exchange! client op args request-id)]
    (if (= (:type reply) "error")
      (throw (ex-info "command_not_accepted" {:code (:code reply) :request-id request-id}))
      reply)))

(defn retry-identical!
  "Caller-controlled replay of the exact old request ID and typed payload."
  [client request-id op args]
  (exchange! client op args request-id))

(defn operation-status! [client request-id]
  (query! client "operation_status" {:request_id request-id}))

(defn discover! [client] (query! client "discover" {}))
(defn describe! [client instrument] (query! client "describe" {:instrument instrument}))
(defn latest! [client signal] (query! client "latest" {:signal signal}))

(defn snapshot!
  "Install every frozen page under one cursor, capped by the host's 256-KiB policy."
  [client]
  (let [first (query! client "runtime_snapshot" {})
        token (:snapshot first)]
    (loop [records (vec (:records first))
           next-index (:next_index first)
           pages 1]
      (when (> pages 64)
        (throw (ex-info "snapshot_pages_exceeded" {:code "resync_required"})))
      (if (nil? next-index)
        (do (query! client "snapshot_release" {:snapshot token})
            {:cursor (:cursor first) :records records})
        (let [page (query! client "snapshot_page" {:snapshot token :index next-index})]
          (when (not= (:cursor page) (:cursor first))
            (throw (ex-info "snapshot_cursor_changed" {:code "resync_required"})))
          (recur (into records (:records page)) (:next_index page) (inc pages)))))))

(defn subscribe! [client cursor]
  (query! client "subscribe" {:after cursor :filter {:kinds [] :targets []}}))

(defn read-event!
  "Apply one buffered/live event or progress frame, then advance the applied cursor."
  [client]
  (let [frame (if (seq @(:events client))
                (let [event (first @(:events client))]
                  (swap! (:events client) #(vec (subvec % 1))) event)
                (read-frame client))]
    (when (not (#{"event" "subscription_progress"} (:type frame)))
      (throw (ex-info "unexpected_stream_frame" {:code "unexpected_stream_frame" :frame frame})))
    (when (not= (:boot_id frame) @(:boot-id client))
      (throw (ex-info "instance_changed" {:code "instance_changed"})))
    (reset! (:cursor client) {:boot_id (:boot_id frame) :seq (:seq frame)})
    frame))

(defn drain-buffered!
  "Apply at most the fixed sixteen already buffered event/progress frames."
  [client]
  (loop [seen []]
    (if (empty? @(:events client)) seen
        (recur (conj seen (read-event! client))))))
