# M12.5 browser/ClojureScript smoke acceptance report

## 1. Status and scope

M12.5 demonstrates that real ClojureScript executing in a real browser can use the
accepted loopback WebSocket endpoint as an ordinary external Application client.
The accepted M12.4 implementation is:

```text
9b58e92cffa79087f60d78032bc34b89499ae961
```

The Runtime commit under test, including the M12.4 acceptance coordination, is:

```text
5d04aa9920a86feec12f577649b9dd2881ab4a8c
```

M12.5 changes only external smoke source, its runner, documentation and active
coordination. It makes no production Rust, Application, Runtime, Recorder, schema,
configuration or safety change.

```text
M12.4 transport parity/fault acceptance: ACCEPTED
M12.5 browser/ClojureScript smoke acceptance: READY FOR EXTERNAL REVIEW
M13: NOT AUTHORIZED
```

## 2. Source and test architecture

The deliberately small external client is under:

```text
clients/clojurescript-smoke/
    README.md
    run-smoke.ps1
    src/lab_runtime_smoke/core.cljs
```

`core.cljs` uses only native browser facilities: `js/WebSocket`, `js/Promise`,
bounded timers, JSON encode/decode and minimal DOM text output. It is not a client
SDK, GUI or presentation model. The only rendered states are `running`, `pass` or
`fail` plus one bounded acceptance summary.

The PowerShell runner compiles the source, starts the existing Runtime, reads its
real readiness JSON, serves a temporary page from numeric loopback, and launches an
installed Chromium browser. It observes the result DOM through Chromium's built-in
DevTools protocol. DevTools is used only to observe the page; the Application
connection is the page's native browser `WebSocket`.

No Node.js WebSocket, Rust client, Tungstenite test peer, curl, Selenium,
Playwright, npm package, bundler or third-party JavaScript WebSocket library is
involved.

## 3. Toolchain and exact commands

Observed toolchain:

| Item | Observed value |
|---|---|
| OS | Microsoft Windows NT 10.0.19045.0 |
| Java | Temurin OpenJDK 25.0.4.1, 2026-08-18 LTS |
| Clojure CLI | 1.12.6.1673 |
| ClojureScript | pinned `org.clojure/clojurescript` 1.12.145 |
| Browser | Google Chrome 153.0.8010.53, headless Chromium engine |
| Python | 3.14.6, temporary static HTTP server only |

Commands run from the repository root:

```powershell
cargo build -p lab-runtime --locked
./clients/clojurescript-smoke/run-smoke.ps1
```

The runner invokes the official compiler equivalently to:

```powershell
clojure -Sdeps <pinned-edn> -M -m cljs.main `
  -co '{:browser-repl false}' -O simple `
  -d <temporary-output> -o <temporary-main.js> `
  -c lab-runtime-smoke.core
```

The exact compiler Maven coordinate is pinned; no unbounded latest dependency is
used.

## 4. Browser endpoint and security evidence

The successful run used:

```text
HTTP Origin:          http://127.0.0.1:9000
Runtime TCP port:     65047
Runtime WS port:      65048
WS endpoint:          ws://127.0.0.1:65048/application/v1
requested protocol:   lab-runtime.application.v1
selected protocol:    lab-runtime.application.v1
```

Relevant readiness fields observed from the Runtime process were:

```json
{
  "port": 65047,
  "state": "ready",
  "websocket": {
    "path": "/application/v1",
    "port": 65048,
    "subprotocol": "lab-runtime.application.v1"
  }
}
```

The Runtime was started with `--port 0 --ws-port 0` and the exact allowlisted
Origin `http://127.0.0.1:9000`. The ClojureScript does not set or synthesize an
Origin header. Chrome supplied the browser Origin, and the existing reviewed
pre-Upgrade policy accepted it. The page also asserted
`socket.protocol == "lab-runtime.application.v1"`.

Ports are ephemeral and will differ on another run.

## 5. Bounds and generated-artifact policy

Each WebSocket open, Application exchange, event wait and clean close has a
five-second ClojureScript deadline. Retained-scope reattach has one fixed
five-second overall deadline; `scope_in_use` retries close each attempted socket
and use a bounded 50 ms retry delay. The complete browser sequence has a 30-second
page deadline and the runner has a 60-second process/observation deadline.

Pending exchanges are bounded by the finite scripted sequence. Diagnostic event
and message traces retain at most 64 and 16 entries respectively. There is no
automatic reconnect loop or accumulating event-listener registration.

Compilation output, Closure files, generated JavaScript, HTTP root, Runtime logs,
browser profile and smoke-process logs use one guarded unique directory under the
system temporary directory. The runner terminates child processes in `finally`,
validates that temporary path before recursive removal, and leaves no generated
artifact in the repository. Resolved Maven artifacts remain only in the normal
user dependency cache.

## 6. Real-browser observed sequence

The Chrome run observed all of the following through the browser page itself:

1. WebSocket A selected the required subprotocol.
2. `hello {scope:null}` returned a scope, boot ID, operations, capabilities and
   `event_latest` cursor.
3. `reference {reference:"1"}` returned the current committed revision.
4. A real Application subscription was installed after the retained cursor.
5. `reference_retune` with request sequence 1 produced both `accepted` and
   `completed`; a matching Application reference event was received.
6. WebSocket A closed cleanly without sending Runtime shutdown.
7. A fresh WebSocket B reattached the same retained scope, respecting the accepted
   `scope_in_use` detach race, and reported `next_seq = "2"`.
8. `operation_status(scope, seq=1)` returned the retained `completed` state.
9. Retrying the exact normalized mutation and request ID returned the same terminal
   result; committed revision remained 2, proving no second execution.
10. A new subscription from the original cursor received the retained reference
    event through replay; the connection-local old subscription was not reused.
11. The new subscription was removed, and a fresh `reference` query returned the
    terminal mutation revision from authoritative committed state.
12. WebSocket B closed cleanly and the page set `data-status="pass"`.

The bounded page summary was:

```json
{
  "boot_id": "48f04fa8641446644a7dcc00f08e0062",
  "scope": "48f04fa8641446644a7dcc00f08e0062:1",
  "selected_subprotocol": "lab-runtime.application.v1",
  "mutation_accepted": true,
  "mutation_completed": true,
  "retained_operation_status": "completed",
  "exact_retry_retained": true,
  "event_observed": true,
  "replay_observed": true,
  "reattach_succeeded": true,
  "final_query_succeeded": true,
  "final_reference_revision": "2",
  "clean_browser_close": true
}
```

This is real-browser evidence. It is not inferred from the Rust transport tests.

## 7. Harness correction during execution

The first harness attempt used Chromium's `--virtual-time-budget` with DOM dumping.
Chrome advanced the page's virtual timers while a live network response was still
pending: hello succeeded, then the next query hit its virtual five-second timer.
Runtime remained healthy and no Runtime error was observed.

The runner was corrected to keep ordinary wall-clock browser timers and observe the
DOM through the browser's built-in loopback DevTools endpoint. The client sequence,
Runtime endpoint and Application semantics did not change. This was a smoke-harness
defect, not a production Rust reproduction, so no production correction was made.

## 8. Rust regression evidence

The complete required Rust regression gate is recorded after the final smoke-source
change:

```text
cargo fmt --all -- --check
    PASS

cargo test --workspace --locked
    PASS

cargo test --workspace --release --locked
    PASS

cargo clippy --workspace --all-targets --locked -- -D warnings
    PASS

git diff --check
    PASS
```

Both workspace test modes report only the two unchanged pre-existing opt-in ignored
tests: diagnostic rotation beyond the 16 MiB retention window and the 2,000-turn,
eight-recording-cycle developer-preview soak. Production Rust has no M12.5 diff.

## 9. Limitations and remaining gate

This is one deterministic local browser smoke on the recorded Windows/Chrome
toolchain. It does not claim broad browser compatibility, remote binding, TLS,
authentication, UI behavior, a maintained client SDK or production certification.
It adds no presentation or scripting semantics to Runtime.

M12 consolidated external acceptance remains a separate gate. M13 is not
authorized.

```text
STATUS: M12_5_READY_FOR_EXTERNAL_REVIEW
```
