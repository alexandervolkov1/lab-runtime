# Browser/ClojureScript Runtime smoke

This deliberately small external client proves that compiled ClojureScript running
in a real browser can use the existing loopback WebSocket Application endpoint. It
is a boundary smoke, not a client SDK or GUI.

The canonical protocol specification is the
[Runtime Application API](../../docs/api/README.md). This smoke exercises that direct
Runtime surface; it does not implement or specify the separate
[Workbench API](../../docs/workbench-api.md).

The source uses only `js/WebSocket`, `js/Promise`, bounded timers, JSON and minimal
DOM text output. It performs hello, query, subscription, mutation lifecycle,
disconnect, retained-scope reattach, operation reconciliation, exact dedup resubmission,
event replay, unsubscribe and final committed-state query.

## Requirements

- Java 21 or newer;
- official Clojure CLI;
- ClojureScript `1.12.145` (resolved by the runner from the pinned Maven
  coordinate);
- Python 3 for a temporary numeric-loopback HTTP server;
- installed Google Chrome or Microsoft Edge;
- a `lab-runtime.exe` built from the repository under test, or a packaged executable
  selected explicitly below.

## Run

Run from the source checkout; preview.5 user packages do not contain `clients/`.
From the repository root:

```powershell
cargo build -p lab-runtime --locked
./clients/clojurescript-smoke/run-smoke.ps1
```

To test an extracted preview executable, stay in the source checkout and select
its path explicitly. Replace the example directory with your extracted folder:

```powershell
./clients/clojurescript-smoke/run-smoke.ps1 -RuntimeExe 'C:\lab-runtime-package\lab-runtime.exe'
```

This optional browser smoke still requires the Java, Clojure, Python, and browser
tools listed above; none are Runtime or Workbench production dependencies.

The runner requires `127.0.0.1:9000` to be free, compiles into a unique directory
under the system temporary directory, starts Runtime with the exact allowed Origin
`http://127.0.0.1:9000`, parses readiness, starts the HTTP server, drives a real
headless Chromium browser, and requires `data-status="pass"` in the captured DOM.

All generated JavaScript, Closure output, browser profile data, logs and the HTTP
root remain outside the repository and are removed on exit. Maven dependencies use
the normal user cache and are not repository artifacts. Child processes are
terminated in `finally` even when the smoke fails.

Optional parameters allow an explicit browser/runtime path or a different free
numeric-loopback HTTP port:

```powershell
./clients/clojurescript-smoke/run-smoke.ps1 `
  -BrowserExe 'C:\Program Files\Google\Chrome\Application\chrome.exe' `
  -RuntimeExe './target/debug/lab-runtime.exe' `
  -OriginPort 9000
```

The browser supplies the Origin header. The ClojureScript source never synthesizes
or sets it.
