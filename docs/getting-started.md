# Getting started

This guide starts the built-in virtual composition. It does not access serial
hardware.

## Prerequisites

- a current stable Rust toolchain with Cargo;
- Windows 10/11 or another platform supported by the virtual-only build;
- PowerShell for the examples below.

From the repository root:

```powershell
cargo build --workspace
cargo test --workspace
```

## Start a virtual Runtime

Use port `0` to let the OS select a free loopback port:

```powershell
cargo run -p lab-runtime -- --serve --profile virtual-demo --port 0
```

After startup the process writes one readiness line to stdout:

```json
{"boot_id":"<32 lowercase hex characters>","port":7420,"state":"ready"}
```

The actual port may differ. The listener is loopback-only. With no command-line
arguments the binary runs a finite demonstration and exits; it does not start the
Application server.

For a declarative composition, copy and edit
[`examples/runtime.virtual.toml`](../examples/runtime.virtual.toml), then run:

```powershell
cargo run -p lab-runtime -- --serve --config D:\absolute\path\runtime.virtual.toml
```

Configuration startup accepts exactly one TOML path. The candidate is read,
bounded, parsed, cross-validated, frozen with its referenced artifacts, and only
then activated. The example config includes a thermal model, Reference, safe
profile, PID controller, loopback port, and SQLite recording path. Do not use the
physical Metakon example unless you intentionally want to open its configured COM
resource.

## Connect over NDJSON

TCP is the current local transport, JSON is the serialization, and the Application
API defines semantics. Each request and response is one UTF-8 JSON object followed
by LF (`\n`; CRLF input is also accepted).

This PowerShell snippet connects and sends `hello`, discovery, current measurements,
and a clean shutdown. Replace `$port` with the readiness value:

```powershell
$port = 7420
$client = [Net.Sockets.TcpClient]::new('127.0.0.1', $port)
$stream = $client.GetStream()
$reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false))
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$writer.NewLine = "`n"
$writer.AutoFlush = $true

$writer.WriteLine('{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}')
$hello = $reader.ReadLine() | ConvertFrom-Json
$hello.result | ConvertTo-Json -Depth 8

$writer.WriteLine('{"v":1,"msg_id":"discover-1","op":"discover","args":{}}')
$reader.ReadLine() | ConvertFrom-Json | ConvertTo-Json -Depth 8

$writer.WriteLine('{"v":1,"msg_id":"current-1","op":"measurements_current","args":{}}')
$reader.ReadLine() | ConvertFrom-Json | ConvertTo-Json -Depth 8

$scope = $hello.result.scope
$shutdown = @{v=1; msg_id='shutdown-1'; op='runtime_shutdown';
  request_id=@{scope=$scope; seq='1'}; args=@{}} | ConvertTo-Json -Compress -Depth 4
$writer.WriteLine($shutdown)
$reader.ReadLine() | ConvertFrom-Json | ConvertTo-Json -Depth 8 # accepted
$reader.ReadLine() | ConvertFrom-Json | ConvertTo-Json -Depth 8 # terminal
$client.Dispose()
```

`hello` returns the boot ID, scope, next mutation sequence, supported operations,
capabilities, and exact bounds. The `discover` and `measurements_current` results are
frozen first pages. If `complete` is false, request subsequent pages with the
returned `projection` and `next_index` using `discovery_page` or
`measurements_page`.

## Record an experiment

Recording must be configured before startup. The compiled virtual profile accepts
an absolute local SQLite path:

```powershell
$db = (Join-Path $PWD 'preview.sqlite')
cargo run -p lab-runtime -- --serve --profile virtual-demo --port 0 `
  --record-db $db --record-policy required
```

After `hello`, start recording with the current scope and next sequence:

```json
{"v":1,"msg_id":"record-1","op":"recording_start","request_id":{"scope":"<scope>","seq":"1"},"args":{"label":"preview run"}}
```

The service first returns `state:"accepted"`, then `state:"completed"` only after
the durable start/provenance transaction commits. Save the completed result's
`run_id`, then stop that exact run with the next sequence:

```json
{"v":1,"msg_id":"record-2","op":"recording_stop","request_id":{"scope":"<scope>","seq":"2"},"args":{"run_id":{"boot_id":"<boot_id>","run_no":"<run_no>"}}}
```

A successful stop means accepted facts were drained and the interval/run seal
transaction committed. It does not mean the storage worker or boot is already
closed. Use `runtime_shutdown` for the process lifecycle.

## Configuration and resources

`configuration_status`, `configuration_properties`, and `resource` expose the
active validated deployment. Property mutation uses the same revision-fenced
stage/apply path as file configuration; it is not direct live-field mutation.
`reconnect_resource` is explicit and generation-fenced. A reconnect never rearms a
failed controller.

## Diagnostics

The default diagnostic level is `INFO`. On Windows logs are written to:

```text
%LOCALAPPDATA%\lab-runtime\logs
```

`LAB_RUNTIME_LOG_LEVEL` may be `ERROR`, `WARN`, `INFO`, `DEBUG`, or `TRACE`.
`LAB_RUNTIME_LOG_DIRECTORY` overrides the directory. Logs are bounded and
best-effort; they are not experiment history. See
[Safety and failure behavior](safety-and-failures.md#diagnostic-logging).

## Next reading

- [Architecture and concepts](architecture.md)
- [Application API](application-api.md)
- [Recorder and SQLite](recorder-sqlite.md)
- [Extending the Runtime](extending-runtime.md)
