# lab-runtime

`lab-runtime` is a local laboratory automation system with two separate executables:

- `lab-runtime` owns authoritative experiment state and behavior;
- `lab-workbench` is the native operator and presentation client.

The Runtime coordinates devices and resources, measurements, References, controllers,
output authority, configuration, Recorder/SQLite, client sessions, mutation
deduplication, and retained operation outcomes. Workbench connects through the same
language-neutral Application API available to other clients and provides live plots,
typed operator workflows, and manual recovery/reconciliation.

External clients can use the direct Runtime API or the separate opt-in
[Workbench presentation/client API](docs/workbench-api.md). Workbench's bounded
IPv4-loopback TCP/NDJSON adapter shares its existing single Runtime client worker;
it does not introduce another experiment owner. The [Babashka smoke](clients/babashka-smoke/README.md)
demonstrates both boundaries without a client SDK or a production language dependency.

## Why two processes?

```text
lab-runtime.exe                         lab-workbench.exe
----------------                         -----------------
authoritative experiment state    <->   private Rust Application client
devices, References, controllers        WorkbenchModel
resources and output authority           PresentationDocument
Recorder and configuration               native egui GUI
sessions and operation outcomes          plots and operator/recovery workflows

                     Application API
```

Runtime owns experiment semantics. Workbench owns presentation semantics.

Workbench lifetime is not Runtime lifetime, and GUI lifetime is not experiment
lifetime. Closing, crashing, or disconnecting Workbench does not shut down Runtime,
stop its Recorder, or roll back an admitted operation.

## Current capabilities

- virtual instruments and declaratively configured physical resources;
- bounded declarative SimpleDevice onboarding through the existing Application API;
- current measurements, bounded recent history, live subscriptions, and durable
  Recorder history;
- References, native PID controllers, finite output leases, and central output
  authority;
- resource reconnect and typed configuration-property updates;
- Runtime-owned SQLite recording with provenance, gaps, and lifecycle sealing;
- one local Application API over TCP/NDJSON and optional loopback WebSocket/JSON;
- a Windows-native Workbench with discovery, measurements, live plots, typed
  Reference/controller/PID/resource/property/Recorder workflows, and manual recovery.

The full Application API is broader than the Workbench GUI. Workbench deliberately
exposes a typed safe operator subset rather than a raw button for every API operation.
See the [Application API reference](docs/api/README.md) for the complete surface.
Declarative serial instruments are covered by the
[SimpleDevice reference](docs/simple-device.md) and
[step-by-step tutorial](docs/developer/simple-device-tutorial.md).

## Run the portable Windows package

The portable preview archive contains both release executables. After extraction,
start Runtime from the package root:

```powershell
$db = [IO.Path]::GetFullPath((Join-Path $PWD "demo.sqlite"))
./lab-runtime.exe --serve --profile virtual-demo --port 7420 `
  --record-db $db --record-policy required
```

Then start Workbench in another terminal:

```powershell
$workspace = [IO.Path]::GetFullPath((Join-Path $PWD ".workbench-demo"))
./lab-workbench.exe --connect 127.0.0.1:7420 --workspace $workspace
```

Neither command needs the source repository. The package also includes the public
documentation, the safe virtual configuration, and optional language-boundary
examples described below.

## Build and run from source

Building the complete workspace, including Workbench, requires Rust 1.95 and Cargo.
Workbench is native to Windows today; the commands below use PowerShell.

```powershell
cargo build --workspace --locked
```

Start a safe virtual Runtime with a local Recorder database. Port `7420` must be free.

```powershell
$db = [IO.Path]::GetFullPath((Join-Path $PWD "demo.sqlite"))

cargo run -p lab-runtime --locked -- `
  --serve `
  --profile virtual-demo `
  --port 7420 `
  --record-db $db `
  --record-policy required
```

In a second terminal, start Workbench with its own workspace:

```powershell
$workspace = [IO.Path]::GetFullPath((Join-Path $PWD ".workbench-demo"))

cargo run -p lab-workbench --locked -- `
  --connect 127.0.0.1:7420 `
  --workspace $workspace
```

Workbench connects immediately. Wait until its status is **Fresh**, then select a
signal in Discovery to see its authoritative observation and live plot.

Connected is not the same as Fresh. Fresh means Workbench has rebuilt the required
authoritative Runtime projections, established its aggregate subscription, and caught
up to the rebuild barrier. Likewise, Accepted or Completed operation evidence is not
by itself current physical or projection state.

Follow [Getting started](docs/getting-started.md) for the complete virtual-demo
workflow, including a Reference retune, Recorder start/stop, reconnect behavior, port
selection, and a small API-client example.

For exact startup flags, deployment TOML fields, path bases, listener policy,
Workbench workspace behavior, and validation failures, use the
[Configuration Guide](docs/configuration.md).

## Process lifetime and recovery

- Explicit Workbench **Disconnect** performs no automatic reconnect. A later Connect
  is a new user action.
- After unexpected transport loss, Workbench may perform one bounded retained-scope
  reattach episode while cached observations are visibly stale.
- Closing or crashing Workbench leaves Runtime and its experiment-owned work alive.
- Runtime shutdown is a separate explicit lifecycle action; close Workbench by closing
  its window, not by shutting down Runtime.
- Mutation recovery is manual. Workbench provides Check Status and confirmed Exact
  Retry where the retained evidence permits it; mutations are never retried
  automatically.

## Documentation

- [Getting started with Runtime and Workbench](docs/getting-started.md)
- [Configuration and deployment](docs/configuration.md)
- [SimpleDevice reference](docs/simple-device.md)
- [SimpleDevice developer tutorial](docs/developer/simple-device-tutorial.md)
- [Workbench user guide](docs/workbench.md)
- [Runtime architecture and concepts](docs/architecture.md)
- [Application API reference](docs/api/README.md)
- [Recorder and SQLite archive](docs/recorder-sqlite.md)
- [Safety and failure behavior](docs/safety-and-failures.md)
- [Extending the Runtime](docs/extending-runtime.md)

## Current environment and limitations

- Runtime listeners are local/loopback. This project has no remote-network security
  qualification.
- Native Workbench currently runs on Windows and currently connects over TCP. Runtime
  may also expose its optional loopback WebSocket transport to other clients.
- The system is not hard real-time and has not completed exhaustive physical,
  multi-day, disk-full, power-loss, USB/driver, or hardware fault-injection
  qualification.
- ACK or register readback does not prove physical effect.
- Not every Application operation is a Workbench GUI workflow.
- No scripting language or embedded automation runtime is selected for v0.1.
  Future automation is planned around the language-neutral Application API boundary.
- Recovery evidence has no Discard/Forget action; invalidated evidence remains visible
  and fail-closed.

The published [v0.1.0-preview.1](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.1)
Windows package is the earlier Runtime-only preview. Current packaging produces a
portable Windows archive containing both Runtime and Workbench; creating a package
does not publish or tag a release.

## Safety posture

Physical output deliberately distinguishes:

```text
requested != authorized != send_started != ACK != readback != physical_effect
```

A started write with an ambiguous outcome is not blindly retried. Reconnect and a
fresh measurement do not rearm a failed controller. Recorder/SQLite is durable
scientific and audit history; bounded diagnostic logs are not experiment authority.

## License

`lab-runtime` is licensed under the [MIT License](LICENSE).
