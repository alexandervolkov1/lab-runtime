# lab-runtime

`lab-runtime` is a headless Rust runtime for laboratory automation. It combines
periodic physical and virtual acquisition, bounded recent history, native control,
central output safety, durable SQLite recording, and a local language-neutral
Application API.

The v0.1 core is functionally complete and has passed its developer-preview
technical gate. This means the accepted functionality has no known
preview-blocking correctness, safety, or durability defect; it is not production
certification or exhaustive physical qualification.

## Developer Preview

The current Developer Preview is published as a GitHub Pre-release:

- [Download v0.1.0-preview.1](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.1)

The Windows x86_64 package is
`lab-runtime-developer-preview-windows-x86_64.zip`. A matching SHA-256 checksum
file is included with the release assets. This is not a stable production release.

## Design boundary

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The Runtime is the sole authoritative mutable experiment owner. Clients query
committed state and submit bounded operations over local TCP/NDJSON. A client does
not own the experiment, and disconnecting it does not roll back admitted work.
There is no GUI, Presentation API, bundled client SDK, scripting runtime, or remote
security model in the current preview.

## Included

- physical and virtual instruments with generation-fenced acquisition;
- current measurements, bounded recent history, subscriptions, and durable history;
- References, native PID controllers, finite leases, and central `OutputAuthority`;
- SQLite experiment archives with provenance, gaps, and lifecycle sealing;
- declarative deployment configuration and explicit physical-resource reconnect;
- statically registered, bounded native managed components;
- bounded, lossy diagnostic logging separate from experiment history;
- 42 Application operations and 25 composition-dependent capabilities.

## Quick start

Prerequisites are a current stable Rust toolchain and, on Windows, PowerShell.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p lab-runtime -- --serve --profile virtual-demo --port 0
```

The service prints one readiness JSON object containing the selected loopback
`port`. Connect to `127.0.0.1:<port>` and exchange one UTF-8 JSON object per line.
The first request is `hello`:

```json
{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}
```

See [Getting started](docs/getting-started.md) for a complete local example and
configuration-based startup.

### Extracted developer-preview package

From the extracted package root, start the safe virtual composition with:

```powershell
.\lab-runtime.exe --serve --config .\examples\runtime.virtual.toml
```

The Windows x86_64 binary uses the operating-system UCRT and requires the Microsoft
Visual C++ 2015–2022 x64 runtime (`VCRUNTIME140.dll`). No non-system application DLL
is shipped; SQLite is built into the executable.

The starter configuration opens no serial port. It listens on loopback TCP port
`7420`, writes its SQLite archive beside the configuration as
`examples\history.sqlite`, and writes bounded diagnostic logs under
`%LOCALAPPDATA%\lab-runtime\logs`. Stop it through the `runtime_shutdown`
Application operation or `Ctrl+C`; see [Getting started](docs/getting-started.md).

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture and concepts](docs/architecture.md)
- [Application API reference](docs/application-api.md)
- [Recorder and SQLite archive reference](docs/recorder-sqlite.md)
- [Safety and failure behavior](docs/safety-and-failures.md)
- [Extending the Runtime](docs/extending-runtime.md)

## Safety posture

Physical output deliberately distinguishes:

```text
requested != authorized != send_started != ACK != readback != physical_effect
```

A started write with an ambiguous outcome is not blindly retried. Reconnect and a
fresh `Good` measurement do not rearm a failed controller. Output requires fresh
authority, a finite lease, current generation/epoch, and a final check immediately
before the first possible byte.

Recorder/SQLite is durable scientific and experiment-audit history. Diagnostic
logs are bounded, lossy troubleshooting data and never experiment authority.

## Preview limitations

The current preview is not hard real-time and has no remote-network security
qualification. It has not completed multi-day unattended, physical disk-full, real
power-loss, exhaustive USB/driver, or hardware fault-injection qualification.
ACK or register readback does not prove physical heater effect. The current
transport is local TCP/NDJSON; additional transports are not implemented.

## License

`lab-runtime` is licensed under the [MIT License](LICENSE).
