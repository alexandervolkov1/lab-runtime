# lab-runtime

`lab-runtime` is a laboratory automation system with two separate executables:

- `lab-runtime` owns authoritative experiment state and behavior;
- `lab-workbench` is the native operator and presentation client.

The Runtime coordinates devices and resources, measurements, References, controllers,
output authority, configuration, Recorder/SQLite, client sessions, mutation
deduplication, and retained operation outcomes. Workbench connects through the same
language-neutral Application API available to other clients and provides live plots,
typed operator workflows, and manual recovery/reconciliation.

Current published release: [v0.1.0-preview.4](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.4)
(Windows Runtime + Workbench; Linux headless Runtime). See
[project status and development map](docs/developer/project-status.md) for the
verified baseline, remaining qualification limits and repository layout.

External clients can use the direct Runtime API or the separate opt-in
[Workbench presentation/client API](docs/workbench-api.md). Workbench's bounded
IPv4-loopback TCP/NDJSON adapter shares its existing single Runtime client worker;
it does not introduce another experiment owner. The [Babashka smoke](clients/babashka-smoke/README.md)
demonstrates both boundaries without a client SDK or a production language dependency.

## Why two processes?

```text
lab-runtime.exe
  owns experiment state and behavior
  owns devices, control, and Recorder
          |
   Application API
          |
lab-workbench.exe
  owns presentation and client state
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
- one Application API over TCP/NDJSON with explicit trusted-LAN opt-in, optional
  loopback WebSocket/JSON, and authenticated WSS through a TLS proxy such as Tuna;
- a Windows-native Workbench with discovery, measurements, live plots, typed
  Reference/controller/PID/resource/property/Recorder workflows, manual recovery,
  TCP/WS/WSS connections through one worker, and an explicit observation-only mode.

The full Application API is broader than the Workbench GUI. Workbench deliberately
exposes a typed safe operator subset rather than a raw button for every API operation.
See the [Application API reference](docs/api/README.md) for the complete surface.
Declarative serial instruments are covered by the
[SimpleDevice reference](docs/simple-device.md) and
[step-by-step tutorial](docs/developer/simple-device-tutorial.md).
Contributors adding a trusted source-integrated physical implementation should use
the [native Rust instrument guide](docs/developer/full-driver-tutorial.md).

## Run the Linux Runtime package

The headless Linux x86_64 package contains `lab-runtime` alone. See the
[Linux Runtime guide](docs/linux-runtime.md) for extraction, GNU/Linux requirements,
serial paths, systemd deployment and reproducible packaging commands.

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

The TCP listener defaults to `127.0.0.1`. Both profile and configuration startup
accept `--bind IPv4 --allow-remote-tcp` for a selected trusted LAN interface; in
profile mode these options precede `--port`. Remote Workbench TCP also requires
`--allow-remote-tcp`. This adds no TLS, authentication, VPN, or firewall configuration.
Runtime WebSocket remains loopback-only; Tuna can expose it through WSS with
`X-Token` authentication and certificate/hostname verification. See
[distributed deployment](docs/distributed-workbench.md) for all-local Windows,
remote Clojure, and Linux Runtime with Windows clients.

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

## Choose what you want to do

| Goal | Start here |
|---|---|
| Run the safe virtual system | [Getting started](docs/getting-started.md) |
| Configure Runtime or Workbench | [Configuration and deployment](docs/configuration.md) |
| Use the native operator GUI | [Workbench user guide](docs/workbench.md) |
| Automate Runtime directly | [Runtime Application API](docs/api/README.md) |
| Automate Workbench presentation | [Workbench external API](docs/workbench-api.md) |
| Connect across machines or through Tuna | [Distributed Workbench deployment](docs/distributed-workbench.md) |
| Record or query history | [Recorder and SQLite](docs/recorder-sqlite.md) |
| Recover after faults | [Recovery and fault handling](docs/recovery-and-faults.md) |
| Understand authority and safety | [Architecture](docs/architecture.md) and [safety](docs/safety-and-failures.md) |
| Add a simple serial instrument | [SimpleDevice reference](docs/simple-device.md) and [tutorial](docs/developer/simple-device-tutorial.md) |
| Add a trusted native instrument | [Extension overview](docs/extending-runtime.md) and [native driver guide](docs/developer/full-driver-tutorial.md) |
| Run optional language examples | [Babashka](clients/babashka-smoke/README.md) and [ClojureScript](clients/clojurescript-smoke/README.md) |
| Contribute or locate current project status | [Development map](docs/developer/project-status.md) |

## Current environment and limitations

- Runtime TCP is loopback by default and may be explicitly bound to a trusted LAN/VPN
  IPv4 address. This project has no remote-network transport-security qualification;
  `--bind` adds no TLS or authentication. Runtime WebSocket remains loopback-only.
- Native Workbench runs on Windows and connects over TCP, WS, or verified WSS.
  `--observe` blocks its Runtime mutations and Exact Retry while allowing queries
  and presentation changes; it is client policy, not Runtime authorization.
- The separate Workbench presentation/client API remains IPv4 loopback-only. A
  remote Clojure client can reach Runtime over LAN TCP or WSS, but cannot directly
  control presentation on another computer through that Workbench API.
- The system is not hard real-time and has not completed exhaustive physical,
  multi-day, disk-full, power-loss, USB/driver, or hardware fault-injection
  qualification.
- ACK or register readback does not prove physical effect.
- Not every Application operation is a Workbench GUI workflow.
- No embedded scripting runtime or production client SDK is included in v0.1.
  External automation uses the language-neutral Application API boundary.
- Recovery evidence has no Discard/Forget action; invalidated evidence remains visible
  and fail-closed.

The published preview.4 contains a portable Windows Runtime + Workbench archive
and a headless Linux x86_64 Runtime archive with a mandatory license companion.
Creating a package does not itself publish or tag a release. Earlier previews and
their exact source/evidence remain available in Git history and GitHub Releases.

## Safety posture

Physical output deliberately distinguishes:

```text
requested != authorized != send_started
send_started != ACK != readback
readback != physical_effect
```

A started write with an ambiguous outcome is not blindly retried. Reconnect and a
fresh measurement do not rearm a failed controller. Recorder/SQLite is durable
scientific and audit history; bounded diagnostic logs are not experiment authority.
Required Recorder failures fail closed. Admission and reserved capacity are not
durability: only validated SQL receipts advance the committed prefix and release
submitted credits. The bounded capacity and its limits are described in
[Recorder and SQLite](docs/recorder-sqlite.md#ingress-and-storage-settings).

## License

`lab-runtime` is licensed under the [MIT License](LICENSE).
