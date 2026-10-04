# Build a SimpleDevice instrument

This tutorial turns two small fixed serial protocols into validated Runtime
instruments. It uses the public files in `examples/simple-device/` and the production
SimpleDevice parser/compiler. No Arduino, scripting runtime, or plugin system is
involved.

The examples use `COM256` deliberately so automated validation cannot open normal
hardware accidentally. To run against a device, copy the example directory, replace
that placeholder with the real port, and confirm the byte protocol against the device
manual first.

## Prerequisites

- Windows and either an extracted preview package or a Rust source checkout;
- a serial device or deterministic serial emulator matching the selected transcript;
- permission to open its COM port;
- for writable work, a reviewed safe profile and an understanding of
  [Runtime output safety](../safety-and-failures.md).

Read the [SimpleDevice reference](../simple-device.md) for the complete schema and
bounds. This tutorial's definitions are binary protocols with ASCII literal wrappers;
they do not parse decimal text.

## Part 1: read-only temperature

### 1. Write down the byte transcript

The example device accepts:

```text
request:   GET<CR><LF>
bytes:     47 45 54 0d 0a

response:  T=<signed i16 big-endian><CR><LF>
21.50 C:   54 3d 08 66 0d 0a
```

`0x0866` is decimal 2150. With scale `0.01` and offset `0.0`, the Runtime publishes
`21.5` degrees Celsius.

### 2. Inspect the definition

Open [`read-only.json`](../../examples/simple-device/read-only.json). Its one
parameter is a read-only measurement. The request is one literal segment. The
response requires exactly six bytes, matches `T=` and CRLF at fixed offsets, extracts
an `i16_be` at offset 2, and applies the declared scale.

The essential mapping is:

```text
READ literal 47 45 54 0d 0a
    -> response match at fixed offsets
    -> i16_be extract at offset 2
    -> raw * 0.01
    -> signal 1001/1
```

Do not weaken the matches merely to accept an unexpected response. A response that
does not satisfy the fixed plan must not become a good observation.

### 3. Bind a serial resource

Copy [`runtime.read-only.toml`](../../examples/simple-device/runtime.read-only.toml)
and `read-only.json` into one directory. Edit only the copied TOML initially:

```toml
port = "COM7" # use the real port
```

The definition path is relative to the TOML's parent directory. The resource is
`windows_com_read_only`, so this deployment cannot issue output writes. Instrument
ID `1001` and parameter ID `1` form signal identity `1001/1`.

### 4. Validate with the production loader

In a source checkout, this focused test loads both public deployment files and invokes
the same parser/compiler used at startup:

```powershell
cargo test -p lab-runtime --test configuration_validation `
  c1_repository_example_deployments_parse_with_the_production_loader

cargo test -p lab-runtime `
  public_simple_device_examples_use_the_production_parser_and_compiler
```

These checks perform no physical I/O. In an extracted package there is no separate
validate-only command; Runtime validates the complete deployment before it announces
readiness.

### 5. Start Runtime

From a source checkout:

```powershell
cargo run -p lab-runtime -- --serve `
  --config .\examples\simple-device\runtime.read-only.toml
```

From an extracted package:

```powershell
.\lab-runtime.exe --serve `
  --config .\examples\simple-device\runtime.read-only.toml
```

Use your edited copy with the real COM port. Successful composition prints one line:

```json
{"boot_id":"<32 lowercase hex characters>","port":<port>,"state":"ready"}
```

Readiness proves that the deployment was validated and activated and that the loopback
API is listening. It does **not** prove that the serial port opened or that a current
measurement exists. SimpleDevice opens and uses the port asynchronously; an unavailable
port or failed transaction leaves the measurement unavailable until the current binding
produces valid evidence. Structural preparation failures can still prevent readiness.

### 6. Discover and describe the signal

Connect to the printed loopback port using the TCP/NDJSON procedure in
[Getting started](../getting-started.md#advanced-a-minimal-application-api-client). The first request
must be hello:

```json
{"v":1,"msg_id":"hello-1","op":"hello","args":{}}
```

Then inspect instrument `1001`:

```json
{"v":1,"msg_id":"describe-1","op":"describe",
 "args":{"instrument":"1001"}}
```

The result's parameter `id:"1"` is a `measurement`, is `read_only`, uses unit ID
`degC`, and identifies the ordinary Runtime signal. Query its latest observation:

```json
{"v":1,"msg_id":"latest-1","op":"latest",
 "args":{"signal":{"instrument":"1001","parameter":"1"}}}
```

After the example response above is received, the value is `21.5` with current
generation/time/quality metadata supplied by Runtime. Before a valid response, expect
explicit not-observed or unavailable state, not an invented zero.

### 7. Recorder behavior

The supplied deployment keeps recording disabled. To record, configure the Runtime
Recorder as described in [Configuration](../configuration.md#recorder-configuration)
and start a run through the normal Application operation. SimpleDevice measurements
then enter the same Runtime-owned Recorder path as native measurements, including
definition and binding provenance. The client does not write SQLite directly.

## Part 2: writable output with ACK and READBACK

### 1. Separate the evidence stages

The writable example uses these transactions:

```text
WRITE 42.0:
  request:  SET <u16 big-endian raw><CR><LF>
  bytes:    53 45 54 20 01 a4 0d 0a

ACK:
  response: OK<CR><LF>
  bytes:    4f 4b 0d 0a

READBACK:
  request:  GET<CR><LF>
  response: V=<u16 big-endian raw><CR><LF>
  bytes:    56 3d 01 a4 0d 0a
```

The scale is `0.1`, so `42.0` encodes to raw decimal 420 (`0x01a4`). The definition
does not send the text `42`; it inserts two binary bytes.

The safety path is:

```text
desired 42.0
    -> central OutputAuthority authorization
    -> final check before first byte
    -> WRITE and send-start evidence
    -> strict OK/CRLF ACK
    -> separate GET READBACK
    -> exact raw 0x01a4 match
```

Requested is not authorized. Authorized is not sent. Send start is not ACK. ACK is
not READBACK. READBACK is authoritative observed register state when configured, but
does not by itself prove every downstream physical effect.

### 2. Inspect and bind the definition

Open [`writable.json`](../../examples/simple-device/writable.json) and
[`runtime.writable.toml`](../../examples/simple-device/runtime.writable.toml). The
actuator is `read_write`, output-affecting, and bounded to `0.0..=100.0`. Its ordinary
READ and its post-WRITE READBACK use the same raw transform.

The deployment uses `windows_com`, not `windows_com_read_only`, and supplies a safe
profile for instrument `1001`, parameter `2`:

```toml
min = 0.0
max = 100.0
safe_value = 0.0
max_lease_ms = 2000
max_proposal_ttl_ms = 200
required_evidence = "readback"
```

The Runtime rejects this deployment if the range is outside the definition, the safe
value is not exactly encodable, or readback evidence is required but absent.

### 3. Validate before connecting hardware

Run the same two production-loader/compiler tests from Part 1. They assert that
`42.0` compiles to the exact request bytes above, and that ACK/readback plans compile
as expected. This is protocol-shape validation, not physical qualification.

Before using real equipment:

1. replace `COM256` only in a copied deployment;
2. independently verify safe value, range, scaling, byte order, ACK, and readback;
3. start with the physical process in a safe condition;
4. exercise malformed ACK, missing readback, timeout, disconnect, and reconnect;
5. confirm the device's own safety mechanisms independently of Runtime.

### 4. Publish and inspect the actuator

Start Runtime as in Part 1 with the writable TOML and query `describe` for instrument
`1001`. Parameter `2` is reported as an `actuator`, `read_write`, and
`output_affecting`. The `output` query can inspect its authority/evidence projection:

```json
{"v":1,"msg_id":"output-1","op":"output",
 "args":{"actuator":{"instrument":"1001","parameter":"2"}}}
```

There is intentionally no raw Application write operation. An accepted Runtime
controller/proposal path must own the output proposal and central authority. Add a
controller only when its input signal, Reference, timing, bounds, and safe lifecycle
have been designed and validated; see the controller and safe-profile sections of
[Configuration](../configuration.md). Do not create a client shortcut around
`OutputAuthority`.

### 5. Interpret failures safely

- Missing ACK does not prove that no bytes reached the device.
- A disconnect or timeout after send start is an ambiguous physical outcome.
- Runtime does not blindly retry an ambiguous requested or safe command.
- ACK followed by missing/mismatching READBACK is not readback-verified success.
- An old binding's delayed WRITE/ACK/READBACK is fenced after reconnect.
- Reconnect does not automatically rearm a controller or lease.

Treat these states as reasons to inspect current Runtime evidence and the physical
system, not as permission to resend.

## Publication and reconnect checklist

When a file deployment starts, or an Application candidate is staged and applied,
the Runtime performs the complete bounded validation before publication. Prepared
SimpleDevice topology is hidden and physically inert. Event capacity and durable
activation are settled before the topology is atomically made visible and scheduled.
Failed pre-durable apply cleans up without hidden reads or pending writes.

After reconnect, wait for fresh observations from the new binding generation. Never
interpret a late old response as evidence for the replacement. Definition identity,
raw/canonical hashes, resource binding, address/channel, generation, and mapping
revision remain Runtime-owned provenance.

## Common errors

| Symptom | Check |
|---|---|
| definition rejected immediately | UTF-8 JSON, exact field names, lowercase even hex, size/count limits |
| deployment rejected | relative definition location, unique IDs/keys, resource kind/ownership, bounds |
| no measurement | actual COM port/settings, exact response length, fixed literals, endian/scale/range |
| instances collide | include address/channel in requests and fixed response matches |
| write cannot be configured | writable resource, one actuator, safe profile, exact safe-value encoding |
| ACK rejected | exact length and semantic match; ACK cannot contain scalar extract |
| readback fails | distinct GET transaction, exact raw equality, current binding generation |
| output becomes ambiguous | investigate send-start/transport evidence; do not blindly retry |

The full failure table and all supported grammar are in the
[SimpleDevice reference](../simple-device.md#validation-and-troubleshooting).

## Test changes before deployment

For a source contribution, run at least:

```powershell
cargo test -p lab-runtime simple_device
cargo test -p lab-core simple_device
cargo test -p lab-runtime --test configuration_validation
cargo fmt --all -- --check
```

Add deterministic assertions for exact encoded request bytes, accepted and rejected
responses, scalar boundaries, timeout/disconnect, stale-generation completion, and
Recorder provenance as applicable. Hardware qualification is separate: record the
device, firmware, adapter, serial settings, fixtures, and observed fault cases. Do
not describe parser/compiler validation as a physical serial test.

If the protocol cannot be expressed without weakening these fixed bounds and evidence
rules, stop and use the trusted native path described at a high level in
[Extending the Runtime](../extending-runtime.md#add-a-physical-instrument). The full
native-driver tutorial is a separate documentation phase.
