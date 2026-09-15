(ns lab.repl
  "Small interactive helpers for one explicit client connection."
  (:require [lab.client :as client]))

(defonce session (atom nil))

(defn open! [port]
  (reset! session (client/connect! port)))

(defn show! []
  (when-let [connection @session]
    (client/discover! connection)))

(defn close! []
  (when-let [connection @session] (client/close! connection))
  (reset! session nil))
