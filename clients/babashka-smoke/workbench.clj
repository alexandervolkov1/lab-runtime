;; Separate wire example required by M17.1; no Runtime mutation/recovery logic.
(require '[cheshire.core :as json])
(import '[java.net Socket InetSocketAddress]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter])

(defn check [condition message]
  (when-not condition (throw (ex-info message {}))))

(let [[port & extra] *command-line-args*
      deadline (+ (System/nanoTime) 10000000000)
      changed (atom nil)]
  (check (and port (empty? extra)) "usage: bb workbench.clj PORT")
  (with-open [socket (Socket.)]
    (.connect socket (InetSocketAddress. "127.0.0.1" (Integer/parseInt port)) 2000)
    (.setSoTimeout socket 2000)
    (with-open [reader (BufferedReader. (InputStreamReader. (.getInputStream socket) "UTF-8"))
                writer (OutputStreamWriter. (.getOutputStream socket) "UTF-8")]
      (letfn [(receive-until [predicate]
                (loop [remaining 64]
                  (check (and (pos? remaining) (< (System/nanoTime) deadline)) "Workbench response deadline/budget")
                  (let [line (.readLine reader)]
                    (check (some? line) "Workbench closed before response")
                    (let [frame (json/parse-string line true)]
                      (check (= 1 (:v frame)) "unexpected Workbench version")
                      (when (= "presentation_changed" (:event frame)) (reset! changed frame))
                      (if (predicate frame) frame (recur (dec remaining)))))))
              (request [id op args]
                (.write writer (str (json/generate-string
                                     {:v 1 :type "request" :call_id id :op op :args args}) "\n"))
                (.flush writer)
                (receive-until #(= id (:call_id %))))
              (result [frame]
                (check (= "result" (:type frame)) (str "unexpected Workbench response: " frame))
                (:result frame))]
        (let [hello (result (request "hello" "hello" {}))
              id (:workbench_id hello)]
          (check (= {:id "lab-runtime.workbench" :version 1} (:protocol hello)) "unexpected Workbench protocol")
          ;; Bounded readiness observation only; no lab retry or recovery/status polling.
          (loop [attempt 0]
            (check (and (< attempt 100) (< (System/nanoTime) deadline)) "Runtime client readiness deadline")
            (let [status (result (request (str "status-" attempt) "client_status" {}))]
              (check (= id (:workbench_id status)) "Workbench identity changed")
              (when-not (= "ready" (get-in status [:runtime_client :connection]))
                (Thread/sleep 20)
                (recur (inc attempt)))))
          (let [submitted (result (request "lab" "lab_query" {:op "reference" :args {:reference "1"}}))]
            (check (= "submitted" (:state submitted)) "query was not submitted")
            (let [event (receive-until #(and (= "lab_update" (:event %))
                                            (= "lab" (get-in % [:data :call_id]))))]
              (check (and (= id (:workbench_id event))
                          (= (:command_id submitted) (get-in event [:data :command_id]))
                          (= "result" (get-in event [:data :kind]))
                          (= "result" (get-in event [:data :runtime :type]))
                          (= "1" (get-in event [:data :runtime :result :reference])))
                     "mediated query lost its Runtime result/correlation")))
          (let [before (result (request "before" "presentation_get" {}))
                revision (:presentation_revision before)
                expected {:workbench_id id :revision revision}
                plot {:id (str "bb-smoke-" revision) :title "Babashka smoke"
                      :time_window_seconds 60.0 :axes {:y_min nil :y_max nil} :traces []}
                mutation (result (request "add" "ui_add_plot" {:expected expected :plot plot}))
                next-revision (str (inc (bigint revision)))
                conflict (request "stale" "ui_remove_plot" {:expected expected :plot_id (:id plot)})
                after (result (request "after" "presentation_get" {}))]
            (check (= next-revision (:presentation_revision mutation) (:presentation_revision after))
                   "UI revision did not advance exactly once")
            (check (and (= "error" (:type conflict))
                        (= "revision_conflict" (get-in conflict [:error :code]))) "stale revision was not rejected")
            (check (= (update (:document before) :plots conj plot) (:document after))
                   "UI mutation/conflict did not preserve the exact document")
            (check (and (= id (:workbench_id @changed))
                        (= next-revision (get-in @changed [:data :presentation_revision]))
                        (= "ui_add_plot" (get-in @changed [:data :operation])))
                   "missing presentation_changed event")
            (println (json/generate-string {:status "PASS" :surface "workbench" :workbench_id id
                                            :presentation_revision next-revision :revision_conflict true
                                            :lab_update true :presentation_changed true}))))))))
