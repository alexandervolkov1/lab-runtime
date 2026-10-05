# Configuration

This guide describes the configuration mechanisms implemented by the current
`lab-runtime` and `lab-workbench` executables. Runtime remains the authoritative
experiment owner. Workbench configuration controls only its client connection,
presentation, recovery evidence, and optional local client endpoint.

## Mental model

There is no universal configuration overlay or precedence chain. The product has
separate configuration domains:

```text
Runtime process
  |-- CLI .............. startup mode and listeners
  |-- deployment TOML .. topology, safety, Recorder
  `-- live API ......... validated Runtime mutations

Workbench process
  |-- CLI .............. connection and local endpoint
  `-- workspace ........ presentation and recovery files
```

CLI profile startup and deployment-file startup are mutually exclusive. Environment
variables configure only bounded diagnostics or operating-system data locations;
they do not overlay deployment TOML. A Workbench workspace cannot override Runtime
topology, safety, Recorder, or listener configuration.

## Minimal safe deployment

[`examples/runtime.minimal.toml`](../examples/runtime.minimal.toml) is the smallest
public deployment example. It opens no serial hardware, asks the OS for a free TCP
port, creates a Runtime-owned SQLite archive next to the copied configuration, and
publishes one virtual measurement plus one fixed Reference.

Use a user-writable directory so the relative Recorder path is unambiguous:

```powershell
$run = [IO.Path]::GetFullPath((Join-Path $PWD "minimal-run"))
New-Item -ItemType Directory -Force $run | Out-Null
Copy-Item ./examples/runtime.minimal.toml (Join-Path $run "runtime.toml")

# Source tree
cargo run -p lab-runtime --locked -- --serve --config (Join-Path $run "runtime.toml")
```

From an extracted preview package, replace the last command with:

```powershell
./lab-runtime.exe --serve --config (Join-Path $run "runtime.toml")
```

Successful startup prints one JSON readiness line. Because the example uses
`port = 0`, read the selected port from that line:

```json
{"boot_id":"<32 lowercase hex characters>","port":<nonzero port>,"state":"ready"}
```

The archive is `$run\history.sqlite`. Stop Runtime with Ctrl+C so its bounded
controller/output/Recorder shutdown completes.

## Runtime startup modes

`lab-runtime` has three behaviors:

1. With no arguments it runs a finite demonstration and exits. It does not start an
   Application listener.
2. `--serve --profile virtual-demo ...` starts the compiled safe virtual profile.
3. `--serve --config PATH` reads one strict deployment document and starts that
   validated composition.

The two serving forms cannot be combined. Configuration-file values cannot be
overridden with profile CLI flags.

### Runtime command-line options

| Option | Value | Default | Scope | Restart? | Meaning |
|---|---|---|---|---|---|
| `--serve` | none | absent | process | yes | Required first token for either server mode. |
| `--config` | nonempty path | none | deployment startup | yes | Selects strict deployment TOML. It is exclusive with all profile options. |
| `--profile` | exactly `virtual-demo` | none | compiled profile | yes | Selects the built-in virtual composition. |
| `--bind` | numeric IPv4 address | `127.0.0.1` | profile TCP listener | yes | Selects the interface address for the Application TCP listener. |
| `--port` | integer `0..=65535` | none | profile TCP listener | yes | Required with `virtual-demo`; `0` asks the OS for a free port. |
| `--record-db` | local absolute path | Recorder disabled | profile Recorder | yes | Enables SQLite recording. Relative and UNC/network paths are rejected. |
| `--record-policy` | `required` or `best-effort` | `required` when `--record-db` is present | profile Recorder | yes | Selects the storage-failure policy. It is valid only after `--record-db`. |
| `--ws-port` | integer `0..=65535` | WebSocket disabled | profile WebSocket listener | yes | Enables the loopback WebSocket endpoint; `0` asks the OS for a port. |
| `--ws-origin` | exact origin; repeatable | none | profile WebSocket policy | yes | At least one is required with `--ws-port`; at most 16 are accepted. |

The profile parser is deliberately strict. Its accepted order is:

```text
--serve --profile virtual-demo [--bind IPv4] --port PORT
  [--record-db ABSOLUTE_PATH [--record-policy required|best-effort]]
  [--ws-port PORT --ws-origin ORIGIN [--ws-origin ORIGIN ...]]
```

`--bind` affects only the profile TCP/NDJSON Application listener. It accepts a
numeric IPv4 address, including a configured LAN/VPN address or the explicit wildcard
`0.0.0.0`; it never discovers an interface automatically. The optional WebSocket
listener remains IPv4 loopback-only. There is no `--listen`, hostname,
config-overlay, or hot-reload CLI option.

In deployment mode, `[server]`, `[server.websocket]`, and `[recording]` supply the
corresponding settings. The only accepted CLI form is exactly:

```text
--serve --config PATH
```

Runtime validates and composes the candidate before printing readiness. A listener
bind failure, Recorder open failure, unsafe composition, or invalid deployment
prevents readiness and exits with an error.

## Deployment TOML

Deployment documents are UTF-8 without a BOM and use `schema_version = 1`. TOML
syntax errors, duplicate TOML keys, and unknown fields are rejected. All schema
tables use strict field decoding.

The required top-level fields and tables are:

| Item | Required | Default | Meaning |
|---|---:|---|---|
| `schema_version` | yes | none | Must be integer `1`. |
| `[runtime]` | yes | none | Stable deployment identity and display name. |
| `[server]` | yes | none | Runtime TCP listener and optional WebSocket table. |
| `[recording]` | yes | none | Recorder enablement, path, and policy. |
| `[[resources]]` | no | empty | Physical serial resources; maximum 8. |
| `[[instruments]]` | no | empty | Virtual or physical instruments; maximum 64. |
| `[[managed_components]]` | no | empty | Trusted native components; maximum 8. |
| `[[references]]` | no | empty | Runtime-owned setpoints; maximum 8. |
| `[[safe_profiles]]` | no | empty | Output safety profiles; maximum 8. |
| `[[controllers]]` | no | empty | Native controllers; maximum 8. |

Array-of-table order is not identity or scheduling order. Runtime sorts candidates by
explicit numeric identity before freezing the effective graph. IDs must be nonzero
and unique within their domain. Logical keys must be unique within their domain,
start with an ASCII letter, contain only ASCII letters, digits, `_`, or `-`, and be
at most 64 bytes. Display names must be nonblank and at most 128 UTF-8 bytes.

### Runtime identity

```toml
[runtime]
key = "virtual-control"
display_name = "Virtual temperature control"
```

Both fields are required. `key` is the stable logical deployment key;
`display_name` is operator-facing metadata.

### TCP and WebSocket listeners

```toml
[server]
host = "127.0.0.1"
port = 7420

[server.websocket]
enabled = true
port = 7421
allowed_origins = ["http://127.0.0.1:3000"]
```

| Field | Required/default | Validation |
|---|---|---|
| `server.host` | required | Must be exactly `127.0.0.1`. |
| `server.port` | required `u16` | `0` selects an OS-assigned loopback TCP port. |
| `websocket` table | omitted by default | Omission means disabled, port `0`, no origins. |
| `websocket.enabled` | required if table exists | Boolean. |
| `websocket.port` | default `0` | `u16`; `0` selects an OS-assigned port. |
| `websocket.allowed_origins` | default empty | Required nonempty when enabled; 1 through 16 unique exact origins. |

An enabled origin is ASCII, at most 256 bytes, uses `http` or `https`, has a
lowercase hostname and explicit nonzero port, and has no wildcard, path, query,
fragment, or user information. A disabled WebSocket table must retain neither a
nonzero port nor any origins. The Upgrade path is `/application/v1` and the required
subprotocol is `lab-runtime.application.v1`.

WebSocket origins are browser-origin checks, not remote-access authentication. The
listener remains IPv4 loopback-only.

### Recorder configuration

```toml
[recording]
enabled = true
path = "history.sqlite"
policy = "required"
```

| Field | Required/default | Validation and behavior |
|---|---|---|
| `enabled` | required | Boolean. |
| `path` | default empty | Required and nonempty when enabled. Relative paths use the deployment file's parent. Parent traversal is rejected. SQLite ultimately requires a local absolute resolved path. |
| `policy` | required | `required` or `best_effort`. `required` is invalid when recording is disabled. |

If recording is disabled, use:

```toml
[recording]
enabled = false
policy = "best_effort"
```

Both policies require the database to open successfully at startup. After startup,
`required` makes Recorder failure fail closed for experiment-critical control;
`best_effort` exposes the storage failure without stopping unrelated valid native
acquisition/control solely because of that failure. Neither policy fabricates a
durable acknowledgement. See [Recorder and SQLite](recorder-sqlite.md).

### Serial resources

```toml
[[resources]]
id = 1
key = "temperature-bus"
kind = "windows_com_read_only"
port = "COM3"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
read_timeout_ms = 50
write_timeout_ms = 50
open_timeout_ms = 2000
recovery_timeout_ms = 2000
```

All fields are required.

| Field | Accepted values |
|---|---|
| `kind` | `windows_com_read_only` or `windows_com` |
| `port` | `COM` followed by a nonzero `u16`; comparison is normalized, so aliases cannot bind the same port twice |
| `baud_rate` | `1..=4_000_000` |
| `data_bits` | `5..=8` |
| `parity` | `none`, `odd`, or `even` |
| `stop_bits` | `1` or `2` |
| `flow_control` | `none`, `software`, or `hardware` |
| `read_timeout_ms`, `write_timeout_ms` | `1..=250` |
| `open_timeout_ms`, `recovery_timeout_ms` | `1..=2000` |

A resource cannot be shared between a SimpleDevice adapter and the native Metakon
adapter. Declaring a physical resource is an instruction to open real hardware during
startup after validation; do not copy physical examples without reviewing the port
and safety configuration.

### Virtual instruments

`virtual_measurement` publishes a virtual read-only temperature signal:

```toml
[[instruments]]
id = 1
key = "temperature"
kind = "virtual_measurement"
display_name = "Virtual temperature"
history_capacity = 64
base_temperature = 20.0
poll_period_ms = 100
```

| Field | Required/default | Validation |
|---|---|---|
| `history_capacity` | required | `1..=1024` samples |
| `base_temperature` | required | finite `-100.0..=100.0` |
| `measurement_enabled` | default `true` | Boolean; deployment-only after startup |
| `external_publication` | default `false` | Boolean; read-only configuration property |
| `poll_period_ms` | required | `1..=60000` |

`thermal_plant` is the virtual control plant used by the full example:

| Field | Required/default | Validation |
|---|---|---|
| `id`, `key`, `display_name` | required | Common identity rules |
| `history_capacity` | required | `1..=1024` |
| `ambient_temperature`, `initial_temperature` | required | Finite numbers |
| `gain_per_percent` | required | Finite and greater than zero |
| `time_constant_ms` | required | Nonzero `u64` |
| `poll_period_ms` | required | `1..=60000` |

The thermal plant exposes the accepted temperature/input and heater/output model used
by the example's controller. It remains explicitly virtual; clients must not present
its values as physical evidence.

### Native Metakon instrument

```toml
[[instruments]]
id = 11
key = "furnace-temperature"
kind = "metakon"
definition = "definitions/metakon-513-thermocouple.json"
resource_id = 1
address = 1
poll_period_ms = 1000
queue_timeout_ms = 1000
transaction_timeout_ms = 1000
```

`definition`, `resource_id`, and `address` are required. `address` is `1..=247`;
`poll_period_ms` is `1..=60000`. The queue and transaction timeouts each default to
`1000` ms and accept `1..=60000`. The referenced strict JSON definition is limited to
16 KiB, resolved relative to the deployment file, frozen for provenance, and checked
against the configured instrument and controller bindings.

Metakon is the current source-integrated native physical implementation, not a
dynamic driver plugin. See the
[native Rust instrument guide](developer/full-driver-tutorial.md) for its complete
developer boundary and [Extending the Runtime](extending-runtime.md) for the
extension overview.

### SimpleDevice instance

A persistent deployment can bind a compiled declarative definition with:

```toml
[[instruments]]
id = 21
key = "simple-temperature"
kind = "simple_device"
display_name = "Simple temperature"
definition = "definitions/simple-temperature.json"
resource_id = 1
address = 1
channel = 1
poll_period_ms = 1000
queue_timeout_ms = 1000
transaction_timeout_ms = 1000
history_capacity = 64
```

All fields are required. The resource must be an eligible COM resource.
`poll_period_ms` is `10..=60000`; queue and transaction timeouts are `1..=2000`;
history capacity is `1..=1024`; `address` and `channel` are `u16`. A deployment admits
at most 32 SimpleDevice instances and 16 distinct definition files. Each raw
definition is at most 8,192 bytes and is resolved relative to the deployment file.

The [SimpleDevice reference](simple-device.md) defines the exact JSON grammar,
READ, WRITE, ACK, READBACK, bounds, mapping, and output-safety semantics. Follow the
[SimpleDevice tutorial](developer/simple-device-tutorial.md) for validated read-only
and writable examples. The lifecycle boundary is also summarized in
[Extending the Runtime](extending-runtime.md#add-a-declarative-simpledevice).

### Managed components

The only production registration currently accepted is
`native.moving_mean.v1`:

```toml
[[managed_components]]
id = 2
instrument_id = 2
key = "temperature-mean"
display_name = "Moving mean"
implementation = "native.moving_mean.v1"
input_instrument_id = 1
period_ms = 100
config = { window = 8 }
```

All identity, implementation, cadence, and display fields are required;
`input_instrument_id` is syntactically optional but required by this implementation.
`period_ms` is `1..=60000`. Its configuration must contain exactly integer `window`
in `2..=64`. Component instrument identities cannot collide with ordinary instrument
identities, inputs must resolve, and managed dependency cycles are rejected. There is
no dynamic plugin or script lookup.

### References

References require `id`, `key`, `kind`, finite `value`, `unit_id`, and `unit_symbol`.

For a fixed Reference, omit `target` and `rate`:

```toml
[[references]]
id = 1
key = "temperature-setpoint"
kind = "fixed"
value = 20.0
unit_id = "degC"
unit_symbol = "°C"
```

For `kind = "ramp"`, both finite `target` and finite positive `rate` are required.
Unit identities contain 1 through 32 printable non-whitespace ASCII bytes. Unit
symbols are nonblank UTF-8 up to 16 bytes. Controller input and Reference unit
identities must match exactly.

### Safe profiles and controllers

Every controller output must have one unique safe profile for the same instrument and
parameter:

```toml
[[safe_profiles]]
instrument_id = 1
parameter_id = 2
min = 0.0
max = 100.0
safe_value = 0.0
max_lease_ms = 2000
max_proposal_ttl_ms = 200
required_evidence = "readback"
```

All values are required. Bounds and safe value must be finite, `min < max`, and
`safe_value` must be inside the inclusive range. Both time limits must be nonzero.
`required_evidence` is `ack` or `readback`; ACK and readback are distinct evidence.
The target must be an eligible output. Non-SimpleDevice built-in output ranges must
remain within `0.0..=100.0`; SimpleDevice definitions are cross-validated against
their typed output semantics.

A controller declares all of these required fields:

| Group | Fields and constraints |
|---|---|
| identity | nonzero unique `id`; unique `key` |
| input | existing `input_instrument_id` and signal `input_parameter_id` |
| output | `output_instrument_id` and `output_parameter_id` with a safe profile |
| Reference | existing `reference_id` with an exactly compatible unit |
| cadence | `period_ms` in `1..=60000` |
| EMA | `ema_time_constant_ms` in `1..=60000`; `ema_warmup_samples` in `1..=1024` |
| PID | finite `kp`, `ki`, `kd`, `output_min`, `output_max`; minimum less than maximum |
| freshness | `max_input_age_ms`, `max_tick_gap_ms` in `1..=60000` |
| authority | `lease_lifetime_ms`, `proposal_ttl_ms` in `1..=60000` |

The output range must stay inside the safe profile. Lease and proposal lifetimes may
not exceed their safe-profile maxima, and proposal TTL may not exceed lease lifetime.
Configuration never bypasses central `OutputAuthority`; controller startup remains
safe and disarmed until its explicit lifecycle permits output.

### Global bounds and cross-references

| Bound | Value |
|---|---:|
| main TOML bytes | 65,536 |
| inexpensive TOML nesting pre-scan | 8 |
| TOML value assignments pre-scan | 4,096 |
| frozen referenced artifacts | 128 |
| TOML plus retained artifact bytes | 1,048,576 |
| resources / instruments | 8 / 64 |
| managed components / References / controllers / safe profiles | 8 each |

Configuration rejects duplicate IDs or keys, duplicate COM bindings, unresolved
resource/input/Reference links, managed cycles, incompatible units, missing safe
profiles, unsafe controller limits, incompatible definitions, and nonfinite numeric
values. Physical definition files and SimpleDevice definitions undergo their own
strict bounded validation before startup can touch a device.

## Full annotated virtual deployment

[`examples/runtime.virtual.toml`](../examples/runtime.virtual.toml) is the fuller safe
example. It shows:

- a fixed loopback TCP port (`7420`);
- a required Recorder archive relative to the deployment file;
- a virtual thermal plant with temperature and heater parameters;
- a ramp Reference;
- a readback-required safe profile;
- one PID controller whose range and lifetimes fit that profile.

Copy it into a user-writable directory before running it:

```powershell
$run = [IO.Path]::GetFullPath((Join-Path $PWD "virtual-control-run"))
New-Item -ItemType Directory -Force $run | Out-Null
Copy-Item ./examples/runtime.virtual.toml (Join-Path $run "runtime.toml")

# Source tree
cargo run -p lab-runtime --locked -- --serve --config (Join-Path $run "runtime.toml")

# Extracted package alternative
# ./lab-runtime.exe --serve --config (Join-Path $run "runtime.toml")
```

The command creates `$run\history.sqlite` and listens on `127.0.0.1:7420`.
If that port is occupied, edit the copied file; there is no CLI port override for
deployment mode.

## Paths and working directories

Path bases are intentionally not universal:

| Path | Relative base | Notes |
|---|---|---|
| `--config PATH` | process current working directory | Runtime lexically makes it absolute before reading. |
| deployment `recording.path` | parent of the resolved deployment file | Must resolve to a local absolute SQLite path; `..` traversal is rejected. |
| instrument `definition` | parent of the resolved deployment file | `..` traversal is rejected; exact bytes are frozen for provenance. |
| profile `--record-db` | none | Must already be an absolute local path; UNC/network forms are rejected. |
| diagnostics directory | value as supplied | Prefer absolute `LAB_RUNTIME_LOG_DIRECTORY`; a relative value follows process filesystem semantics. |
| Workbench `--workspace` | process current working directory | Resolved lexically and through its deepest existing ancestor before ownership. |
| default Workbench workspace | Windows user-data directory | `%LOCALAPPDATA%\lab-runtime\workbench`, falling back to `%APPDATA%`. |

The deployment path is lexically absolutized, not canonicalized. Relative referenced
paths are owned by the deployment file even if Runtime's current directory later
differs. Absolute referenced paths are accepted subject to the same no-parent-
traversal rule; Recorder storage separately rejects nonlocal/UNC paths.

Use `[IO.Path]::GetFullPath(...)` for CLI paths on Windows when the intended base
should be obvious in a log or shell history.

## Workbench startup options

| Option | Value | Default | Scope | Restart? | Meaning |
|---|---|---|---|---|---|
| `--connect` | numeric socket address | none; required | Runtime client | yes | Address used by the one Workbench Application client worker. |
| `--scope` | retained Application scope string | journal scope if usable, otherwise a new scope | Runtime client recovery | yes | Explicitly requests reattachment to an existing retained scope. |
| `--workspace` | path | Windows user-data workspace | Workbench persistence | yes | Directory containing presentation and recovery files and protected by one process guard. |
| `--workbench-listen` | numeric IPv4-loopback socket | disabled | external Workbench API | yes | Enables the opt-in bounded TCP/NDJSON client endpoint; port `0` asks the OS for a port. |

Arguments may be ordered freely, but each option may appear only once. Unknown,
duplicate, or missing options fail startup and print usage. `--connect` accepts a
numeric socket address, not a hostname. `--workbench-listen` additionally rejects
wildcard, non-loopback, hostname, and IPv6 forms.

When enabled, the Workbench endpoint prints its actual address:

```json
{"workbench_endpoint":"127.0.0.1:<port>"}
```

This endpoint is unauthenticated and intentionally loopback-only. It does not create
another Runtime client, sequencer, recovery engine, or presentation owner. See the
[Workbench API](workbench-api.md).

## Workbench workspace and presentation persistence

The resolved workspace contains two independent formats:

- `presentation-v1.json`: an optional, strict, validated client-owned presentation
  document, bounded to 1,048,576 serialized bytes;
- `recovery-v1.json`: the worker-owned exact mutation recovery journal, bounded to
  65,536 bytes and at most eight records.

Workbench acquires an OS-owned Windows process guard before opening either file. A
second Workbench using the same resolved workspace fails. The guard is released by
normal exit or process death.

If `--workspace` is omitted, `%LOCALAPPDATA%` is preferred and `%APPDATA%` is the
fallback. If neither exists, default-workspace selection fails; passing an explicit
workspace avoids that dependency.

At startup, a valid presentation file becomes the process-local presentation. An
invalid or unreadable file is not replaced: Workbench reports a local problem and
uses an empty in-memory document. The current GUI has no Save Layout command or
general persistence manager, so do not assume every in-memory UI change is written
back automatically. Recovery journal writes are separate, validated, and atomic;
loading a journal never retries or resubmits a mutation.

Neither workspace file is Runtime experiment state. Deleting or changing
presentation cannot alter instruments, controllers, Recorder, or output authority.
Recovery files are evidence for manual reconciliation, not permission to infer or
fabricate an operation result.

## Environment and diagnostics

The supported Runtime diagnostic variables are:

| Variable | Default | Classification | Behavior |
|---|---|---|---|
| `LAB_RUNTIME_LOG_LEVEL` | `INFO` | operator diagnostics | Accepts `ERROR`, `WARN`, `INFO`, `DEBUG`, or `TRACE`, case-insensitively. An invalid value warns and falls back to `INFO`. |
| `LAB_RUNTIME_LOG_DIRECTORY` | platform default | operator diagnostics | A nonempty value selects the bounded diagnostic-log directory. Failure to install diagnostics warns; it does not change experiment semantics. |

On Windows the default log directory is
`%LOCALAPPDATA%\lab-runtime\logs`. On other supported Runtime builds,
`$XDG_STATE_HOME/lab-runtime/logs` is used when present, otherwise a temporary
directory fallback is used. Diagnostic files are bounded best-effort troubleshooting,
not Recorder history.

`LOCALAPPDATA` and `APPDATA` also participate in Workbench's default workspace
selection as described above. They are operating-system location inputs, not
deployment overlays.

The source tree contains narrow environment variables for GUI/process acceptance
harnesses. They are test-only seams and are intentionally not supported operator
configuration.

## Live Runtime operations versus deployment configuration

Deployment startup creates Runtime-authoritative state. The Application API then
offers explicit configuration operations described in the
[operation registry](api/operations.md):

- `configuration_status`, `configuration_properties`, and `configuration_page`
  inspect the committed deployment and typed properties;
- `stage_configuration` rereads and validates the same path selected by `--config`;
- `apply_configuration` applies the retained candidate under its candidate and
  revision fences;
- `reload_configuration` performs the accepted combined reload lifecycle;
- `property_configure` changes only properties advertised as writable through the
  same validated candidate lifecycle;
- `stage_simple_device_candidate` and `apply_configuration` handle a bounded
  process-local SimpleDevice overlay.

These operations are unavailable when Runtime was started from `virtual-demo` where
the deployment lifecycle is disabled. They do not select a new arbitrary file, alter
listener addresses, or write changes back to TOML. Process-local property overlays
and API-provisioned SimpleDevice overlays are not restart persistence.

A live mutation is not “higher precedence” than TOML. It is a revision-fenced
Runtime mutation of the active authoritative configuration. A later explicit reload
constructs another candidate from the source file and enters the same lifecycle.

## Precedence and ownership

| Setting domain | Source | Owner | Persistent? | Runtime mutable? | Restart? |
|---|---|---|---:|---:|---:|
| Runtime mode and profile listeners | Runtime CLI | Runtime process | no | no | yes |
| deployment listeners | deployment TOML | Runtime process | source file | no | yes |
| deployment topology and safety | deployment TOML | Runtime | source file plus frozen active revision | only through explicit validated operations | not always; operation effect decides |
| Recorder path and policy | CLI profile or deployment TOML | Runtime | path/policy source plus SQLite archive | path/policy not live-switchable | yes |
| Runtime experiment mutations | Application operations | Runtime | according to Runtime/Recorder contract, not TOML write-back | yes, where advertised | no |
| Workbench connection and endpoint | Workbench CLI | Workbench | no | no | yes |
| Workbench presentation | workspace file and current UI/API actions | Workbench | file only when explicitly persisted by implemented behavior | never Runtime-mutable | Workbench-local |
| Workbench recovery evidence | recovery journal | Workbench client worker | yes, bounded | reconciliation only; never experiment authority | survives Workbench restart when valid |
| diagnostic verbosity/path | environment | diagnostic subsystem | process-local | no | yes |

There is no field-wise merge between profile CLI and deployment TOML. Workbench state
does not override Runtime state. Environment diagnostics do not override either.

The one narrow selection rule inside Workbench is retained-scope choice: an explicit
`--scope` is used first; otherwise a valid journal scope may be requested; otherwise
hello requests a new scope. Runtime still decides whether a requested scope can be
reattached.

## Validation and failure behavior

The deployment lifecycle is:

```text
read bytes
    |
    v
parse + bounds
    |
    v
cross-reference + safety validation
    |
    v
freeze source artifacts
    |
    v
stage candidate
    |
    v
Runtime-owned apply
```

Parsing and validation open no transport, listener, or SQLite database. Referenced
definition files are read only after structural validation, then their exact bytes
and hashes are retained for provenance. Startup proceeds to resource preparation,
safe composition, Recorder opening, and listener binding only after validation.
Readiness is printed only after accepted startup conditions are established.

At startup, any read, UTF-8, TOML, bound, cross-reference, safety, artifact,
transport, Recorder, or listener failure aborts startup without a readiness line.
Invalid deployment validation occurs before listener binding.

For live changes, validation and staging happen before publication. Candidate and
configuration revision fences prevent stale apply. Safety-sensitive effects require
the Runtime-owned safe barrier. A rejected candidate does not partially replace the
active deployment. Prepared SimpleDevice topology is physically inert and hidden
until its accepted atomic publication boundary; failed pre-durable apply leaves no
hidden topology or pending transport work.

## Security implications

- Profile startup defaults the Runtime TCP listener to `127.0.0.1`. An explicit
  non-loopback `--bind` is intended only for a trusted LAN or VPN. It adds no TLS,
  authentication, VPN, firewall rule, or other transport security; `0.0.0.0` exposes
  the listener on every IPv4 interface permitted by the host network and firewall.
- Runtime WebSocket and deployment-configured listeners remain IPv4 loopback-only.
  Deployment `server.host` cannot widen that boundary.
- The Workbench external endpoint is disabled by default and accepts only numeric
  IPv4 loopback. It has no authentication and must not be forwarded or exposed as a
  remote service.
- WebSocket origins are exact browser-origin admission, not user authentication.
- Deployment files are trusted operator input with the ability to select serial
  hardware, Recorder storage, controllers, and outputs. Review physical files before
  starting them.
- Referenced definition and Recorder paths cannot use `..` traversal. Prefer a
  dedicated, access-controlled deployment directory.
- Diagnostic logs can contain bounded operational details. They are not scientific
  history and should be protected as local troubleshooting data.

## Troubleshooting

| Symptom/category | Likely cause | Corrective action |
|---|---|---|
| usage plus unknown/duplicate argument | Unsupported CLI option, duplicate option, or mixed profile/config forms | Use one exact startup grammar from this guide. |
| option needs a value | Missing path, port, origin, scope, or address | Supply the value immediately after its option. |
| invalid port/address | Invalid numeric IPv4 bind, invalid `u16`, hostname where numeric address is required, or non-loopback Workbench endpoint | Use a numeric IPv4 value for Runtime `--bind`, use `127.0.0.1:PORT` for Workbench endpoints, and use port `0` only where documented. |
| no readiness; address already in use | Requested TCP or WebSocket port is occupied | Stop the conflicting process, select another configured port, or use `0`. |
| configuration artifact/read failure | Missing file, wrong current directory, permissions, oversized file, or rejected path traversal | Resolve `--config` with `GetFullPath`, check referenced paths relative to its parent, and use a readable local directory. |
| invalid TOML | Syntax error, duplicate key, unknown field, wrong type, BOM, or unsupported schema version | Compare with the strict field tables and validate UTF-8 without BOM. |
| duplicate identity/binding | Reused ID, logical key, safe-profile target, or normalized COM port | Assign stable unique identities and one owner per COM binding. |
| unresolved reference | Instrument points to a missing resource, component input is absent, controller input/Reference is missing, or output lacks a safe profile | Add the referenced object and verify IDs and units exactly. |
| invalid bound/value | Zero identity, invalid duration/history/serial range, nonfinite number, or unsafe output range | Use the documented inclusive ranges and finite values. |
| Recorder open/start failure | Database path is empty, nonlocal, unwritable, locked incompatibly, or storage initialization failed | Use a writable local absolute result; for deployment files, put a relative filename beside a copied config in a user-writable directory. |
| Workbench workspace failure | No default Windows data environment, inaccessible path, invalid ancestor, or another Workbench owns it | Pass an explicit writable workspace and close the other owner. |
| Workbench endpoint rejected | `--workbench-listen` used wildcard, hostname, IPv6, or non-loopback IPv4 | Use numeric `127.0.0.1:PORT`. |
| physical startup unexpectedly touches hardware | A copied deployment contains `[[resources]]` and physical instruments | Stop Runtime safely and use `runtime.minimal.toml` or `runtime.virtual.toml` until the hardware configuration is intentionally reviewed. |
| diagnostic level warning | Unsupported `LAB_RUNTIME_LOG_LEVEL` | Use one of the five documented level names; Runtime falls back to `INFO`. |

Error text is bounded and may include operating-system details. Diagnose by category;
do not build automation around incidental wording unless the Application API defines
that error code.

## Next steps

- [Getting started](getting-started.md) walks through Runtime, Workbench, a query,
  and Recorder operation.
- [Runtime architecture](architecture.md) explains authority and process boundaries.
- [Application API](api/README.md) defines live query/mutation semantics.
- [Workbench guide](workbench.md) describes workspace, recovery, and operator flows.
- [Recorder and SQLite](recorder-sqlite.md) defines storage and durability.
- [SimpleDevice](simple-device.md) is the declarative device reference, with a
  [step-by-step tutorial](developer/simple-device-tutorial.md).
- [Extending the Runtime](extending-runtime.md) describes current source-level
  extension seams; the
  [native Rust instrument guide](developer/full-driver-tutorial.md) gives the full
  trusted physical-integration workflow.
