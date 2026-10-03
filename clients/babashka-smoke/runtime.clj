;; Explicit acceptance/example only: bb runtime.clj PORT (virtual-demo Runtime).
(require '[cheshire.core :as json])
(import '[java.net Socket InetSocketAddress]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter])

(defn check [condition message]
  (when-not condition (throw (ex-info message {}))))

(let [[port & extra] *command-line-args*]
  (check (and port (empty? extra)) "usage: bb runtime.clj PORT")
  (with-open [socket (Socket.)]
    (.connect socket (InetSocketAddress. "127.0.0.1" (Integer/parseInt port)) 2000)
    (.setSoTimeout socket 5000)
    (with-open [reader (BufferedReader. (InputStreamReader. (.getInputStream socket) "UTF-8"))
                writer (OutputStreamWriter. (.getOutputStream socket) "UTF-8")]
      (letfn [(query [id op args]
                (.write writer (str (json/generate-string {:v 1 :msg_id id :op op :args args}) "\n"))
                (.flush writer)
                (let [line (.readLine reader)]
                  (check (some? line) "Runtime closed before response")
                  (let [response (json/parse-string line true)]
                    (check (= [1 id "result"] ((juxt :v :msg_id :type) response))
                           (str "unexpected Runtime response: " line))
                    (:result response))))]
        (let [hello (query "hello-1" "hello" {:scope nil})
              _ (check (= {:id "lab-runtime.application" :version 1} (:protocol hello))
                       "unexpected Runtime protocol")
              _ (check (and (seq (:boot_id hello)) (seq (:scope hello))
                            (string? (:next_seq hello))
                            (re-matches #"[1-9][0-9]*" (:next_seq hello))
                            (some #{"reference"} (:operations hello)))
                       "hello missing Runtime-generated identity or reference query")
              reference (query "reference-1" "reference" {:reference "1"})]
          (check (and (= "1" (:reference reference))
                      (string? (:revision reference))
                      (re-matches #"[1-9][0-9]*" (:revision reference))
                      (number? (:target reference)))
                 "unexpected virtual-demo Reference snapshot")
          (println (json/generate-string
                    {:status "PASS" :surface "direct-runtime" :boot_id (:boot_id hello)
                     :scope (:scope hello) :next_seq (:next_seq hello)
                     :reference (:reference reference) :revision (:revision reference)
                     :target (:target reference)})))))))
