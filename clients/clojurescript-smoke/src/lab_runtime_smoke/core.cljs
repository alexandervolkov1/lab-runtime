(ns lab-runtime-smoke.core)

(def config (.-LAB_RUNTIME_SMOKE_CONFIG js/window))
(def pending (atom {}))
(def events (atom []))
(def traces (atom []))
(def finished (atom false))

(defn now-ms [] (.now js/Date))

(defn fail-error [message]
  (js/Error. message))

(defn require! [condition message]
  (when-not condition
    (throw (fail-error message))))

(defn bounded-message [error]
  (let [message (or (.-message error) (str error))]
    (.slice message 0 (min 300 (.-length message)))))

(defn render! [status value]
  (let [root (.getElementById js/document "app")
        result (.getElementById js/document "result")]
    (.setAttribute root "data-status" status)
    (set! (.-textContent result) (.stringify js/JSON (clj->js value)))))

(defn reject-pending! [message]
  (doseq [[_ entry] @pending]
    (js/clearTimeout (:timer entry))
    ((:reject entry) (fail-error message)))
  (reset! pending {}))

(defn remember-event! [message]
  (swap! events
         (fn [current]
           (let [next (conj current message)]
             (if (> (count next) 64)
               (vec (take-last 64 next))
               next)))))

(defn remember-trace! [value]
  (swap! traces
         (fn [current]
           (vec (take-last 16 (conj current value))))))

(defn handle-message! [event]
  (try
    (let [message (.parse js/JSON (.-data event))
          msg-id (aget message "msg_id")]
      (remember-trace! {:kind "message"
                        :msg_id msg-id
                        :type (aget message "type")
                        :state (aget message "state")
                        :code (aget message "code")})
      (if (some? msg-id)
        (when-let [entry (get @pending msg-id)]
          (if (= :operation (:mode entry))
            (if (= "accepted" (aget message "state"))
              (swap! pending assoc-in [msg-id :accepted] true)
              (do
                (swap! pending dissoc msg-id)
                (js/clearTimeout (:timer entry))
                ((:resolve entry)
                 #js {"accepted" (:accepted entry) "terminal" message})))
            (do
              (swap! pending dissoc msg-id)
              (js/clearTimeout (:timer entry))
              ((:resolve entry) message))))
        (remember-event! message)))
    (catch :default error
      (reject-pending! (str "invalid server JSON: " (bounded-message error))))))

(defn connect! []
  (js/Promise.
   (fn [resolve reject]
     (let [socket (js/WebSocket. (.-wsUrl config) (.-subprotocol config))
           timer (js/setTimeout
                  (fn []
                    (.close socket)
                    (reject (fail-error "WebSocket open deadline exceeded")))
                  (.-stepTimeoutMs config))]
       (set! (.-onmessage socket) handle-message!)
       (set! (.-onopen socket)
             (fn [_]
               (js/clearTimeout timer)
               (if (= (.-protocol socket) (.-subprotocol config))
                 (resolve socket)
                 (do
                   (.close socket)
                   (reject (fail-error "required WebSocket subprotocol not selected"))))))
       (set! (.-onerror socket)
             (fn [_]
               (js/clearTimeout timer)
               (reject (fail-error "WebSocket connection failed"))))
       (set! (.-onclose socket)
             (fn [event]
               (remember-trace! {:kind "close"
                                 :code (.-code event)
                                 :reason (.-reason event)})
               (reject-pending! "WebSocket closed with an exchange pending")))))))

(defn exchange! [socket request mode]
  (js/Promise.
   (fn [resolve reject]
     (let [msg-id (aget request "msg_id")
           timer (js/setTimeout
                  (fn []
                    (swap! pending dissoc msg-id)
                    (reject (fail-error
                             (str "Application deadline exceeded: " msg-id
                                  " trace=" (pr-str @traces)))))
                  (.-stepTimeoutMs config))]
       (require! (not (contains? @pending msg-id)) "duplicate smoke msg_id")
       (swap! pending assoc msg-id {:resolve resolve
                                    :reject reject
                                    :timer timer
                                    :mode mode
                                    :accepted false})
       (try
         (.send socket (.stringify js/JSON request))
         (catch :default error
           (swap! pending dissoc msg-id)
           (js/clearTimeout timer)
           (reject error)))))))

(defn request! [socket value]
  (exchange! socket (clj->js value) :single))

(defn operation! [socket value]
  (exchange! socket (clj->js value) :operation))

(defn close! [socket]
  (js/Promise.
   (fn [resolve reject]
     (if (= (.-CLOSED js/WebSocket) (.-readyState socket))
       (resolve true)
       (let [timer (js/setTimeout
                    (fn [] (reject (fail-error "WebSocket close deadline exceeded")))
                    (.-stepTimeoutMs config))]
         (set! (.-onclose socket)
               (fn [_]
                 (js/clearTimeout timer)
                 (resolve true)))
         (.close socket 1000 "smoke complete"))))))

(defn wait-event! [predicate]
  (js/Promise.
   (fn [resolve reject]
     (let [deadline (+ (now-ms) (.-stepTimeoutMs config))]
       (letfn [(check! []
                 (if-let [event (first (filter predicate @events))]
                   (resolve event)
                   (if (< (now-ms) deadline)
                     (js/setTimeout check! 20)
                     (reject (fail-error "Application event deadline exceeded")))))]
         (check!))))))

(defn delay! [milliseconds]
  (js/Promise. (fn [resolve _] (js/setTimeout resolve milliseconds))))

(defn hello-request [scope msg-id]
  {:v 1 :msg_id msg-id :op "hello" :args {:scope scope}})

(defn reattach! [scope deadline]
  (if (>= (now-ms) deadline)
    (js/Promise.reject (fail-error "scope reattach deadline exceeded"))
    (-> (connect!)
        (.then
         (fn [socket]
           (-> (request! socket (hello-request scope "reattach"))
               (.then
                (fn [hello]
                  (cond
                    (= "result" (aget hello "type"))
                    #js {"socket" socket "hello" hello}

                    (= "scope_in_use" (aget hello "code"))
                    (-> (close! socket)
                        (.then (fn [_] (delay! 50)))
                        (.then (fn [_] (reattach! scope deadline))))

                    :else
                    (throw (fail-error
                            (str "reattach failed: " (aget hello "code")))))))))))))

(defn mutation-event? [scope event]
  (let [request-id (aget event "request_id")]
    (and (= "event" (aget event "type"))
         (= "reference" (aget event "kind"))
         (some? request-id)
         (= scope (aget request-id "scope"))
         (= "1" (aget request-id "seq")))))

(defn run-sequence! []
  (let [state (atom {})]
    (-> (connect!)
        (.then
         (fn [socket]
           (swap! state assoc :socket-a socket :subprotocol (.-protocol socket))
           (request! socket (hello-request nil "hello-a"))))
        (.then
         (fn [hello]
           (require! (= "result" (aget hello "type")) "hello A was not a result")
           (let [result (aget hello "result")
                 scope (aget result "scope")
                 boot-id (aget result "boot_id")
                 cursor (aget result "event_latest")]
             (require! (and (string? scope) (pos? (.-length scope))) "hello scope missing")
             (require! (and (string? boot-id) (pos? (.-length boot-id))) "hello boot_id missing")
             (require! (js/Array.isArray (aget result "operations")) "hello operations missing")
             (require! (js/Array.isArray (aget result "capabilities")) "hello capabilities missing")
             (require! (some? cursor) "hello event_latest missing")
             (swap! state assoc :scope scope :boot-id boot-id :cursor cursor)
             (request! (:socket-a @state)
                       {:v 1 :msg_id "reference-a" :op "reference"
                        :args {:reference "1"}}))))
        (.then
         (fn [reference]
           (require! (= "result" (aget reference "type")) "reference query failed")
           (let [revision (aget (aget reference "result") "revision")]
             (swap! state assoc :initial-revision revision)
             (request! (:socket-a @state)
                       {:v 1 :msg_id "subscribe-a" :op "subscribe"
                        :args {:after (:cursor @state)
                               :filter {:kinds [] :targets []}}}))))
        (.then
         (fn [subscription]
           (require! (= "result" (aget subscription "type")) "subscription A failed")
           (swap! state assoc :subscription-a
                  (aget (aget subscription "result") "subscription"))
           (let [mutation {:v 1 :msg_id "mutation" :op "reference_retune"
                           :request_id {:scope (:scope @state) :seq "1"}
                           :args {:reference "1"
                                  :expected_revision (:initial-revision @state)
                                  :target 55.0 :rate 2.0}}]
             (swap! state assoc :mutation mutation)
             (operation! (:socket-a @state) mutation))))
        (.then
         (fn [lifecycle]
           (require! (aget lifecycle "accepted") "mutation accepted state not observed")
           (let [terminal (aget lifecycle "terminal")]
             (require! (= "completed" (aget terminal "state")) "mutation did not complete")
             (swap! state assoc :terminal terminal)
             (wait-event! (partial mutation-event? (:scope @state))))))
        (.then
         (fn [event]
           (swap! state assoc :event-a event)
           (close! (:socket-a @state))))
        (.then
         (fn [_]
           (reset! events [])
           (reattach! (:scope @state) (+ (now-ms) (.-reattachTimeoutMs config)))))
        (.then
         (fn [reattached]
           (let [socket (aget reattached "socket")
                 hello (aget reattached "hello")
                 result (aget hello "result")]
             (require! (= (:scope @state) (aget result "scope")) "reattached scope changed")
             (require! (= "2" (aget result "next_seq")) "reattached next_seq was not 2")
             (swap! state assoc :socket-b socket)
             (request! socket
                       {:v 1 :msg_id "status" :op "operation_status"
                        :args {:request_id {:scope (:scope @state) :seq "1"}}}))))
        (.then
         (fn [status]
           (require! (= "completed" (aget (aget status "result") "state"))
                     "retained operation_status was not completed")
           (swap! state assoc :retained-status (aget status "result"))
           (request! (:socket-b @state) (:mutation @state))))
        (.then
         (fn [retry]
           (require! (= "completed" (aget retry "state")) "exact retry was not retained")
           (require! (= (.stringify js/JSON (aget retry "result"))
                        (.stringify js/JSON (aget (:terminal @state) "result")))
                     "exact retry result changed")
           (swap! state assoc :retry retry)
           (request! (:socket-b @state)
                     {:v 1 :msg_id "subscribe-b" :op "subscribe"
                      :args {:after (:cursor @state)
                             :filter {:kinds [] :targets []}}})))
        (.then
         (fn [subscription]
           (require! (= "result" (aget subscription "type")) "subscription B failed")
           (swap! state assoc :subscription-b
                  (aget (aget subscription "result") "subscription"))
           (wait-event! (partial mutation-event? (:scope @state)))))
        (.then
         (fn [replayed]
           (swap! state assoc :replayed replayed)
           (request! (:socket-b @state)
                     {:v 1 :msg_id "unsubscribe-b" :op "unsubscribe"
                      :args {:subscription (:subscription-b @state)}})))
        (.then
         (fn [unsubscribed]
           (require! (true? (aget (aget unsubscribed "result") "removed"))
                     "subscription B was not removed")
           (request! (:socket-b @state)
                     {:v 1 :msg_id "reference-b" :op "reference"
                      :args {:reference "1"}})))
        (.then
         (fn [reference]
           (require! (= "result" (aget reference "type")) "final reference query failed")
           (let [revision (aget (aget reference "result") "revision")
                 terminal-revision (aget (aget (:terminal @state) "result") "revision")]
             (require! (= revision terminal-revision) "committed reference revision changed")
             (swap! state assoc :final-revision revision)
             (close! (:socket-b @state)))))
        (.then
         (fn [_]
           {:boot_id (:boot-id @state)
            :scope (:scope @state)
            :selected_subprotocol (:subprotocol @state)
            :mutation_accepted true
            :mutation_completed true
            :retained_operation_status (aget (:retained-status @state) "state")
            :exact_retry_retained true
            :event_observed true
            :replay_observed true
            :reattach_succeeded true
            :final_query_succeeded true
            :final_reference_revision (:final-revision @state)
            :clean_browser_close true})))))

(defn start! []
  (render! "running" {:status "running"})
  (let [overall (js/setTimeout
                 (fn []
                   (when (compare-and-set! finished false true)
                     (reject-pending! "overall smoke deadline exceeded")
                     (render! "fail" {:status "fail"
                                      :error "overall smoke deadline exceeded"})))
                 (.-overallTimeoutMs config))]
    (-> (run-sequence!)
        (.then
         (fn [summary]
           (when (compare-and-set! finished false true)
             (js/clearTimeout overall)
             (render! "pass" summary))))
        (.catch
         (fn [error]
           (when (compare-and-set! finished false true)
             (js/clearTimeout overall)
             (render! "fail" {:status "fail"
                              :error (bounded-message error)})))))))

(start!)
