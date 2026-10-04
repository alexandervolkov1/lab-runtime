# Minimal Babashka boundary acceptance

These two standalone scripts are wire examples and acceptance evidence, not a client
SDK or the future Clojure client architecture. Babashka is optional tooling; Cargo
builds and production executables do not depend on it. Tested with Babashka 1.13.220.
The scripts use its included Cheshire JSON parser and Java TCP streams, with no Rust
crate access, dependencies to install, recovery, mutation retry, or subscriptions.

From an extracted preview package, with `bb` on PATH, the runner automatically uses
the release executables at the package root:

```powershell
./clients/babashka-smoke/run-smoke.ps1 -Repeat 3
```

From the repository root, with `bb` on PATH and PowerShell 7 on Windows:

```powershell
cargo build -p lab-runtime -p lab-workbench --locked
./clients/babashka-smoke/run-smoke.ps1 -Repeat 3
```

For another layout, pass `-RuntimeExe` and `-WorkbenchExe` explicitly. The runner
never downloads or builds either product executable.

The runner starts the actual `lab-runtime.exe --serve --profile virtual-demo --port 0`,
reads its bound port, and starts the native Workbench with a temporary workspace and
`--workbench-listen 127.0.0.1:0`. It reads Workbench's readiness address rather than
guessing a port. A desktop capable of running the native Glow Workbench is required.
Only the scripts exchange Application/Workbench protocol messages; PowerShell manages
processes and reads readiness/PASS output. Each script has a 15-second process deadline;
connect/read deadlines also apply. Cleanup terminates only fixture processes, waits
at most five seconds each, and never sends a Runtime shutdown operation. Temporary
workspace/logs remain under the printed system-temp path for diagnosis.

To exercise already running **virtual-demo/test** instances separately:

```powershell
bb clients/babashka-smoke/runtime.clj 7420
bb clients/babashka-smoke/workbench.clj 7421
```

`runtime.clj` sends exactly:

```json
{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}
{"v":1,"msg_id":"reference-1","op":"reference","args":{"reference":"1"}}
```

Babashka parses both responses and checks version, correlation, result type,
protocol identity, Runtime-issued boot/scope/next sequence, advertised `reference`,
and the Reference ID, decimal revision, and numeric target. It prints the observed
values, closes the socket, and exits nonzero on errors. No Runtime mutation is sent;
hello plus a read-only query is the approved safe subset of the M17.1 matrix.

`workbench.clj` separately exercises Workbench hello, bounded `client_status` readiness
observation, a mediated `reference` query and its correlated `lab_update`, presentation
read, one UI-only add-plot, a deliberately stale remove-plot (`revision_conflict`),
the exact final document/revision, and `presentation_changed`. It then disconnects.
It leaves its plot in the temporary presentation; it does not alter Runtime experiment
state. Unique call IDs are used throughout each connection. Readiness observation is
not mutation retry or recovery-status polling.

The surfaces remain distinct:

```text
runtime.clj   -> Runtime Application TCP API
workbench.clj -> Workbench TCP API -> existing single ClientHandle -> Runtime
```

The separate [browser smoke](../clojurescript-smoke/README.md) remains the real-browser
Runtime WebSocket regression. See the canonical
[Runtime Application API](../../docs/api/README.md) and
[Workbench API](../../docs/workbench-api.md) for the two surfaces; this README and
the scripts are not competing protocol specifications. Full Arduino + Clojure +
Clay integration is post-preview practical work, outside this smoke.
