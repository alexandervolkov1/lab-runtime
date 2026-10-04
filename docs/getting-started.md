# Getting started with Runtime and Workbench

This guide runs the built-in virtual laboratory, connects the native Workbench,
observes live measurements, retunes a virtual Reference through the typed operator
workflow, and optionally records the run. The virtual profile opens no serial hardware.

## Prerequisites

- Windows for the native Workbench;
- PowerShell;
- a free local TCP port (the primary example uses `7420`).

An extracted preview package already contains both executables and does not require
Rust. Building from source requires Rust 1.95 with Cargo. Run package commands from
the extracted package root or source commands from the repository root. Runtime
listeners are loopback-only; this guide does not configure remote access.

The recommended progression is:

1. start the built-in virtual Runtime;
2. start Workbench and wait for Fresh;
3. perform a hello/query through the direct Runtime API;
4. start the minimal deployment TOML;
5. inspect and operate the Runtime-owned Recorder;
6. continue with the [Configuration Guide](configuration.md).

## Build

`lab-runtime.exe` and `lab-workbench.exe` are already present in an extracted
package, so package users skip this step.

```powershell
cargo build --workspace --locked
```

This builds both product executables:

- `lab-runtime`, the authoritative experiment process;
- `lab-workbench`, the native operator and presentation client.

## Start a safe virtual Runtime

Resolve an absolute local path for the SQLite archive, then start the built-in
`virtual-demo` profile:

```powershell
$db = [IO.Path]::GetFullPath((Join-Path $PWD "demo.sqlite"))

cargo run -p lab-runtime --locked -- `
  --serve `
  --profile virtual-demo `
  --port 7420 `
  --record-db $db `
  --record-policy required
```

From an extracted package, the equivalent command is:

```powershell
./lab-runtime.exe --serve --profile virtual-demo --port 7420 `
  --record-db $db --record-policy required
```

Port `7420` must be free. The profile contains a virtual thermal plant, Reference 1,
a PID controller, and the configured Runtime-owned Recorder. Its measurements are
virtual observations, not evidence from physical hardware.

Running `lab-runtime` without arguments executes a finite demonstration and exits;
it does not start the Application server.

## Understand readiness

Runtime validates and constructs its composition before it announces readiness. It
then writes one JSON line to stdout:

```json
{"boot_id":"<32 lowercase hex characters>","port":7420,"state":"ready"}
```

Do not start a client until this line appears. The `boot_id` identifies this Runtime
process, and `port` is the actual TCP listener.

To let the operating system choose a free port, use `--port 0` instead. Read the
`port` from the readiness line and substitute it in the Workbench command:

```powershell
cargo run -p lab-runtime --locked -- `
  --serve `
  --profile virtual-demo `
  --port 0 `
  --record-db $db `
  --record-policy required
```

The optional WebSocket listener, when configured, is reported separately in the same
readiness object. Runtime supports local TCP/NDJSON and optional loopback
WebSocket/JSON. Native Workbench uses TCP today.

## Start Workbench

In a second PowerShell terminal, create an absolute workspace path and connect:

```powershell
$workspace = [IO.Path]::GetFullPath((Join-Path $PWD ".workbench-demo"))

cargo run -p lab-workbench --locked -- `
  --connect 127.0.0.1:7420 `
  --workspace $workspace
```

From an extracted package, use:

```powershell
./lab-workbench.exe --connect 127.0.0.1:7420 --workspace $workspace
```

If Runtime selected another port, replace `7420`. Workbench acquires exclusive
ownership of the workspace, loads its bounded recovery journal and presentation
document when present, opens one private Application client, and begins connecting
immediately. Do not open a second Workbench on the same workspace.

## Wait for Fresh

The header shows connection and observation state. Startup normally progresses
through Connecting/AwaitingHello and Rebuilding before it shows **Fresh**.

Connected is not the same as Fresh. Fresh means Workbench has rebuilt the required
authoritative Runtime projections, established its aggregate subscription, and caught
up to the rebuild barrier. Keep mutation controls untouched while observations are
Stale or Rebuilding; the GUI disables unavailable controls.

Accepted or Completed operation evidence is also not the same as current physical or
projection state. Operation evidence describes an identified request. The displayed
Runtime observation remains authoritative for current state.

## Inspect measurements and the live plot

1. In **Discovery**, select a signal, for example **Signal 1/1** from the virtual
   thermal plant.
2. Confirm the detail pane says **Fresh observation**.
3. Watch the live plot populate from authoritative signal events.

The plot is Workbench presentation state. It does not become Runtime experiment
authority, and a virtual value must not be interpreted as physical readback.

## Retune Reference 1

The virtual profile's Reference 1 is a ramp, so the typed retune workflow is available
after the corresponding detail is Fresh:

1. Select **Reference 1** in Discovery.
2. In **Reference controls**, choose a finite **Target** and a positive **Rate**. For
   example, change the target slightly and retain the displayed rate.
3. Select **Retune**.
4. Read the **Confirmation required** summary. The draft includes the current
   authoritative revision; if that revision changes, reload and review rather than
   forcing the stale draft.
5. Select **Confirm** once.
6. Observe the operation state. It may pass quickly through **Accepted / in progress**
   before **Operation completed**.
7. Separately observe the refreshed Reference projection and revision.

Confirm expresses operator intent; it is not evidence of completion. Completed is an
authoritative operation outcome, but the later Reference projection/event remains the
authority for the currently displayed Reference state. Workbench never automatically
retries this mutation.

## Optionally record the experiment

Because the Runtime command configured `--record-db`, Workbench shows the Recorder
and its authoritative state:

1. Enter a short run label in the **Recorder** section.
2. Select **Start recording**, review the confirmation, and select **Confirm**.
3. Wait for the operation outcome and the authoritative Recorder state to show an
   active run.
4. When finished, select **Stop recording**, review, confirm, and wait for the
   authoritative state to return to idle.

Recorder is Runtime-owned. Closing Workbench does not stop an active Runtime or imply
Recorder shutdown. See [Recorder and SQLite](recorder-sqlite.md) for durability,
archive, and sealing semantics.

For a copyable direct-API start/annotation/stop/history workflow, continue with the
[virtual Recorder tutorial](recorder-sqlite.md#virtual-recorder-tutorial).

## Disconnect and reconnect

The **Disconnect** button is an explicit client action:

- Workbench closes its socket and marks cached observations stale;
- it preserves exact recovery/journal evidence;
- it performs no automatic reconnect.

After Disconnected is visible, select **Connect** to make one new manual connection.
Workbench rebuilds from Runtime and must pass through the full barrier before becoming
Fresh again.

Unexpected transport loss is different. After an attached retained scope, Workbench
may make one bounded automatic retained-scope reattach episode. During it the GUI
shows Reattaching/reconnecting state, observations remain stale, and mutation controls
are unavailable. If the episode expires, Workbench stays Disconnected until a manual
Connect. Neither path automatically retries a mutation, Check Status, or Exact Retry.

## Close Workbench

Close the Workbench window when the GUI is no longer needed. A clean close or an OS
process failure releases workspace ownership and stops only the Workbench client.
Runtime, controllers, Recorder, and admitted Runtime work have independent lifetimes.
A replacement Workbench can reopen the workspace and rebuild authoritative
observations.

These are distinct actions:

| Action | Effect |
|---|---|
| Workbench **Disconnect** | Disconnects the client; no automatic reconnect |
| close Workbench | Stops the GUI/client process; Runtime continues |
| Workbench crash | Runtime continues; the OS releases workspace ownership |
| Runtime shutdown | Stops the authoritative Runtime through a separate explicit lifecycle action |

## Stop Runtime explicitly

When the experiment process itself should end, return to the Runtime terminal and
press **Ctrl+C**. Runtime performs its bounded shutdown, including its own controller,
output, and Recorder cleanup. Closing Workbench is not a substitute for this action,
and Runtime shutdown is not the normal way to close the GUI.

## Configuration-based startup

The repository and package include a minimal declarative virtual composition. Copy it
to a user-writable directory so its relative Recorder path has an obvious home:

```powershell
$run = [IO.Path]::GetFullPath((Join-Path $PWD "minimal-run"))
New-Item -ItemType Directory -Force $run | Out-Null
Copy-Item ./examples/runtime.minimal.toml (Join-Path $run "runtime.toml")
$config = Join-Path $run "runtime.toml"

cargo run -p lab-runtime --locked -- --serve --config $config
```

The extracted-package equivalent is:

```powershell
./lab-runtime.exe --serve --config $config
```

The configuration is bounded, parsed, cross-validated, and frozen before activation.
It asks the OS for a free port and creates `$run\history.sqlite`. Read the selected
port from the readiness line before starting Workbench. Review every physical
deployment before use; a file containing serial resources may open its configured
hardware after validation.

The fuller [`runtime.virtual.toml`](../examples/runtime.virtual.toml) example adds a
virtual thermal plant, ramp Reference, safe profile, and PID controller. The
[Configuration Guide](configuration.md) is the canonical reference for both examples,
all Runtime and Workbench CLI options, path resolution, deployment fields, limits,
and failure behavior.

## Advanced: a minimal Application API client

Workbench is the primary operator path, but the Application API is also a supported
client boundary. TCP carries one UTF-8 JSON object followed by LF for each NDJSON
frame. The API, not TCP, defines experiment semantics.

With the virtual Runtime still listening on port `7420`, this PowerShell snippet
performs hello, reads one query, and closes only its own socket:

```powershell
$port = 7420
$client = [Net.Sockets.TcpClient]::new("127.0.0.1", $port)
$stream = $client.GetStream()
$reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false))
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$writer.NewLine = "`n"
$writer.AutoFlush = $true

$writer.WriteLine('{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}')
$hello = $reader.ReadLine() | ConvertFrom-Json
$hello.result | ConvertTo-Json -Depth 8

$writer.WriteLine('{"v":1,"msg_id":"recorder-1","op":"recording_status","args":{}}')
$reader.ReadLine() | ConvertFrom-Json | ConvertTo-Json -Depth 8

$client.Dispose()
```

`hello` returns the process boot identity, a server-issued scope, the next mutation
sequence, the operations and capabilities available in this composition, event
cursors, and exact limits. Closing this socket does not shut down Runtime. See the
[Application API reference](api/README.md) before adding subscriptions or
mutations; clients must not blindly retry mutations.

## Optional external language smoke

With Babashka installed, follow the [minimal Babashka acceptance](../clients/babashka-smoke/README.md)
to run hello plus a safe query against a real virtual Runtime. Its separate Workbench
smoke uses the [opt-in Workbench API](workbench-api.md) for client/presentation work.
These examples are not an SDK; full Arduino + Clojure + Clay integration is later
post-preview work. Babashka is not required to build or run the Rust applications.

## Diagnostics

The default diagnostic level is `INFO`. On Windows, bounded best-effort logs are
written under:

```text
%LOCALAPPDATA%\lab-runtime\logs
```

`LAB_RUNTIME_LOG_LEVEL` accepts `ERROR`, `WARN`, `INFO`, `DEBUG`, or
`TRACE`. `LAB_RUNTIME_LOG_DIRECTORY` selects another directory. Diagnostic logs
are not experiment history. See
[Safety and failure behavior](safety-and-failures.md#diagnostic-logging).

## Next steps

- [Configuration and deployment](configuration.md)
- [SimpleDevice reference](simple-device.md)
- [Build a SimpleDevice instrument](developer/simple-device-tutorial.md)
- [Add a trusted native Rust instrument](developer/full-driver-tutorial.md)
- [Workbench user guide](workbench.md)
- [Runtime architecture and concepts](architecture.md)
- [Application API reference](api/README.md)
- [Recorder and SQLite](recorder-sqlite.md)
- [Recovery and fault handling](recovery-and-faults.md)
- [Safety and failure behavior](safety-and-failures.md)
- [Extending the Runtime](extending-runtime.md)
