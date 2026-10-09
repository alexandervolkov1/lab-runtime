;; Explicit example: bb runtime.clj PORT [--host IPv4 --allow-remote-tcp].
(require '[cheshire.core :as json])
(import '[java.net Socket InetSocketAddress]
        '[java.io BufferedReader InputStreamReader OutputStreamWriter])

(defn check [condition message]
  (when-not condition (throw (ex-info message {}))))

(defn tcp-options [args]
  (loop [args args host nil allow-remote false]
    (if (empty? args)
      {:host (or host "127.0.0.1") :allow-remote allow-remote}
      (case (first args)
        "--host" (do (check (and (nil? host) (second args)) "--host needs one IPv4 address")
                     (recur (nnext args) (second args) allow-remote))
        "--allow-remote-tcp" (do (check (not allow-remote) "duplicate remote TCP option")
                                (recur (next args) host true))
        (throw (ex-info "unknown TCP option" {}))))))

(let [[port & extra] *command-line-args*
      {:keys [host allow-remote]} (tcp-options extra)]
  (check port "usage: bb runtime.clj PORT [--host IPv4 --allow-remote-tcp]")
  (check (re-matches #"[0-9]{1,3}(\.[0-9]{1,3}){3}" host) "host must be a numeric IPv4 address")
  (let [octets (mapv #(Integer/parseInt %) (re-seq #"[0-9]+" host))
        first-byte (first octets)
        loopback (= first-byte 127)]
    (check (every? #(<= 0 % 255) octets) "invalid IPv4 octet")
    (check (and (> first-byte 0) (< first-byte 224)) "host must be a unicast IPv4 address")
    (check (or loopback allow-remote)
           "remote TCP requires --allow-remote-tcp; trusted LAN only, without TLS/authentication")
    (when-not loopback
      (binding [*out* *err*]
        (println "WARNING: plaintext TCP; restrict the Runtime port to trusted LAN clients."))))
  (with-open [socket (Socket.)]
    (.connect socket (InetSocketAddress. host (Integer/parseInt port)) 2000)
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
