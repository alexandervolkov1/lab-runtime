# SimpleDevice

SimpleDevice is the Runtime's bounded declarative path for a simple serial
instrument. A strict JSON definition describes fixed requests, fixed-length
responses, typed scalar conversion, and optional WRITE/ACK/READBACK behavior. The
Runtime compiles that definition into the same instrument, signal, actuator,
history, output-authority, and Recorder paths used by native instruments.

SimpleDevice is not a dynamic plugin or scripting system. The Runtime remains the
sole experiment owner, and the definition never receives transport or output
authority.

## When to use it

Use SimpleDevice when all of these are true:

- the device uses an existing bounded Windows COM resource;
- each transaction is one request and one fixed-length response, at most 64 bytes;
- request bytes can be built from literals, address/channel fields, one scalar
  output value, and one of the supported checksums;
- response validation needs only fixed-offset literal/address/channel matches and
  an optional supported checksum;
- observations are fixed-width integer or IEEE-754 `f32` scalars;
- at most one output-affecting actuator is needed per definition.

Use a trusted native Rust integration when the protocol needs a state machine,
variable-length or delimiter-driven parsing, arbitrary text-number parsing,
unsupported binary framing/checksums, a custom transport, multiple coupled outputs,
device-specific recovery, or unusual scheduling. There is currently no dynamic
driver plugin API. Use the
[native Rust instrument guide](developer/full-driver-tutorial.md) for the trusted
source-level path, or see [Extending the Runtime](extending-runtime.md) for the
extension overview.

## Mental model

A read becomes an ordinary Runtime signal:

```text
scheduled READ
    -> bounded serial transaction
    -> fixed response validation
    -> scalar decode and scaling
    -> Runtime signal observation
```

A write remains under the central safety owner:

```text
desired value
    -> OutputAuthority
    -> WRITE after final authority check
    -> send-start evidence
    -> ACK
    -> separate READBACK, when configured
    -> authoritative observed state
```

WRITE, ACK, and READBACK are different evidence. An accepted request or authority
lease is not evidence that bytes were sent. Send start is not an ACK. An ACK proves
only that the response matched the declared ACK plan; it does not prove physical
effect. READBACK reports the separately observed encoded value, but even register
readback may not prove the final physical effect outside the device.

## Deployment binding

A persistent definition is bound through one `[[instruments]]` entry:

```toml
[[instruments]]
id = 1001
key = "temperature-device"
kind = "simple_device"
display_name = "Temperature device"
definition = "read-only.json"
resource_id = 7
address = 1
channel = 1
poll_period_ms = 500
queue_timeout_ms = 250
transaction_timeout_ms = 500
history_capacity = 256
```

All fields are required. `definition` is resolved relative to the deployment TOML,
read at validation time, and frozen with the deployment provenance. `resource_id`
must name an eligible COM resource. A read-only device may use
`windows_com_read_only` or `windows_com`; an output device requires `windows_com`.
SimpleDevice and native adapters cannot share one resource.

| Field | Domain |
|---|---|
| `id` | nonzero unique Runtime instrument ID |
| `key` | unique logical key; 1..64 ASCII letters/digits/`_`/`-`, starting with a letter |
| `display_name` | nonblank, at most 128 UTF-8 bytes |
| `definition` | relative or absolute JSON path; parent traversal is rejected |
| `resource_id` | existing eligible resource ID |
| `address`, `channel` | unsigned 16-bit values; must fit every selected field encoding |
| `poll_period_ms` | `10..=60000` |
| `queue_timeout_ms` | `1..=2000` |
| `transaction_timeout_ms` | `1..=2000` |
| `history_capacity` | `1..=1024` |

A deployment admits at most 32 SimpleDevice instances and 16 distinct definition
files. Multiple instances on one resource must have response plans whose instance
matches unambiguously distinguish them.

The Application API also accepts one complete bounded process-local candidate through
`stage_simple_device_candidate`, followed by `apply_configuration`. The strict
candidate has exactly three fields: `schema_version` (exactly `1`), `definition`
(one complete definition object), and `instances` (one bounded array). `definition`
uses the exact schema and compiler below. Every instance contains the
same `instrument_id`, `key`, `display_name`, `resource_id`, `address`, `channel`,
`poll_period_ms`, `queue_timeout_ms`, `transaction_timeout_ms`, and
`history_capacity` values represented by persistent deployment fields. A read-only
candidate has 1..=4 instances and forbids `safe_profile` and `controller`. An
output-capable candidate has exactly one instance and requires `safe_profile`; its
optional `controller` uses the ordinary controller identities, input/output bindings,
Reference ID, PID/EMA values, timing, lease, and proposal-TTL fields documented by
[Configuration](configuration.md#safe-profiles-and-controllers). Unknown fields are
rejected at every candidate level.

The complete compact candidate is limited to 8,192 bytes, 900 JSON values/object
members, and container depth 8. It is cross-referenced against current Runtime
resources and configuration revision. The overlay is authoritative Runtime state
while the process runs, but it is not written back to the deployment file and is not
restored after Runtime restart. See the
[Application operation registry](api/operations.md#complete-operation-registry).

## Definition-file format

The file is strict UTF-8 JSON. Unknown fields, unknown tagged variants, malformed
UTF-8, and trailing JSON are rejected. The raw file is at most 8,192 bytes; its
normalized compact representation is at most 6,144 bytes.

Top-level fields are all required:

| Field | Type | Rule |
|---|---|---|
| `format_version` | integer | exactly `1` |
| `definition_id` | string | logical-key rules above |
| `definition_version` | integer | nonzero `u32` |
| `parameters` | array | 1..=16 entries |

Every parameter object has these fields:

| Field | Type / accepted values | Rule |
|---|---|---|
| `parameter_id` | nonzero `u64` | unique in the definition |
| `key` | logical-key string | unique in the definition |
| `display_name` | string | nonblank, at most 128 UTF-8 bytes |
| `role` | `measurement`, `diagnostic`, `actuator` | see semantic combinations below |
| `access` | `read_only`, `read_write`, `write_only` | see semantic combinations below |
| `unit_id`, `unit_symbol` | strings | ID: 1..32 printable non-whitespace ASCII bytes; symbol: nonblank UTF-8 up to 16 bytes |
| `value_type` | `integer`, `float` | engineering-value type |
| `engineering_min`, `engineering_max` | JSON numbers | inclusive declared range |
| `write_effect` | `none`, `output_affecting` | must match the role/access |
| `encoding` | object | required raw scalar transform |
| `read` | object or omitted | required for observations and read/write actuators |
| `write` | object or omitted | required only for actuators |

Only two semantic parameter shapes are accepted:

- observation: role `measurement` or `diagnostic`, access `read_only`,
  `write_effect:"none"`, a `read` plan, and no `write` plan;
- actuator: role `actuator`, access `read_write` or `write_only`,
  `write_effect:"output_affecting"`, `value_type:"float"`, and a `write` plan.
  A read/write actuator also requires `read`.

At most one actuator is allowed in a definition. Integer ranges must be JSON integer
values fitting `i64`, with minimum less than or equal to maximum. Float ranges must
be finite, must have minimum strictly less than maximum, and reject negative zero.

## Scalar encoding and scaling

`encoding` has exactly three required fields:

```json
{"raw":"i16_be","scale":0.01,"offset":0.0}
```

Supported `raw` values are:

```text
u8, i8,
u16_le, u16_be, i16_le, i16_be,
u32_le, u32_be, i32_le, i32_be,
f32_le, f32_be
```

`scale` must be finite and nonzero. `offset` must be finite. Negative zero is
rejected for both. A read decodes:

```text
engineering value = raw value * scale + offset
```

A write encodes:

```text
raw value = (engineering value - offset) / scale
```

The requested engineering value must be finite, inside the declared range, and
exactly representable by the selected raw encoding and transform. Integer raw
encodings are not rounded or clipped. `f32` encodings must round-trip exactly through
finite `f32`. An `integer` engineering value must remain integral after decoding.

## Request grammar

A request is `{"segments":[...]}` with 1..=8 segments and a compiled length of
1..=64 bytes. It may contain at most four nonliteral inserts. Segment objects are
strict tagged objects:

| `type` | Other fields | Meaning |
|---|---|---|
| `literal` | `hex` | literal bytes; lowercase, nonempty, even-length hex; at most 32 bytes per segment |
| `instance_field` | `field`, `encoding` | insert deployment `address` or `channel` as `u8`, `u16_le`, or `u16_be` |
| `value_field` | none | insert the parameter's encoded raw value |
| `checksum` | `algorithm` | append `sum8`, `xor8`, or `crc16_modbus` over preceding request bytes |

READ and READBACK requests must contain no `value_field`. A WRITE request must contain
exactly one. A request checksum is optional, may occur once, and must be the last
segment. Its input is limited to 62 bytes. Modbus CRC uses the conventional `0xffff`
initial value and polynomial `0xa001`; the two result bytes are little-endian.

Literal segments can express ASCII bytes and line endings, for example `4745540d0a`
is `GET` followed by CRLF. SimpleDevice does not otherwise treat the request as text.

## Response grammar

A response plan has:

| Field | Type | Rule |
|---|---|---|
| `exact_length` | integer | `1..=64` bytes |
| `matches` | array | 0..=8 strict match objects; ACK requires at least one |
| `extract` | object or omitted | required for READ/READBACK; forbidden for ACK |
| `checksum` | object or omitted | optional supported checksum at a fixed offset |

Match variants are:

```json
{"type":"literal_match","offset":0,"hex":"543d"}
{"type":"instance_match","offset":2,"field":"address","encoding":"u8"}
```

A literal match is at most 16 bytes. Instance fields and encodings have the same
meaning as request inserts. The scalar extraction is:

```json
{"type":"scalar_extract","offset":2}
```

Its width comes from the parameter's raw encoding. A response checksum is:

```json
{"type":"checksum","offset":6,"algorithm":"crc16_modbus"}
```

It validates the response prefix before `offset`; checksum input is 1..=62 bytes.
All fixed fields must fit the exact response. Extract, matches, and checksum bytes
may not overlap, except identical literal match evidence may overlap. Invalid length,
content, checksum, extraction, or scalar range fails that transaction rather than
publishing fabricated evidence.

### Supported

- literal binary or ASCII bytes expressed as lowercase hex;
- fixed-width address/channel inserts and matches;
- one fixed-width scalar value in a WRITE;
- fixed-offset scalar extraction;
- signed/unsigned 8/16/32-bit integers and IEEE-754 `f32`;
- little- and big-endian encodings where listed;
- `sum8`, `xor8`, and Modbus CRC16;
- fixed CR/LF bytes as literals.

### Not supported

- arbitrary decimal-text parsing or formatting;
- regex, delimiter-driven, variable-length, or streaming response parsing;
- arbitrary expressions, scripts, templates, or conditional request branches;
- fixed-width decimal text fields;
- arbitrary binary structures, escaping, or packet state machines;
- checksums/CRCs other than the three listed;
- automatic retry of uncertain writes.

## READ and signal lifecycle

For each readable parameter, the Runtime periodically admits the precompiled READ to
the bound resource queue. The resource has one bounded transaction owner. A valid
fixed response is matched, checksum-checked if configured, decoded, scaled, range
checked, and committed as an ordinary typed signal observation. Current measurement,
recent history, subscriptions, controller input, Workbench projections, and Recorder
then consume the generic Runtime signal path.

A malformed, timed-out, stale, or unavailable response never becomes a good
observation. Resource loss makes affected observations unavailable until current
generation evidence is obtained after recovery. The Runtime does not let a client
fabricate freshness.

Prepared topology is deliberately inert: parsing, staging, and preparation do not
start physical scheduler reads. Reads begin only after the Runtime completes the
accepted durable publication boundary.

## WRITE and OutputAuthority

SimpleDevice does not expose a raw write operation. A controller or other trusted
Runtime proposal must pass through central `OutputAuthority`. The proposal and lease
have finite lifetimes, the value must satisfy the safe profile and definition range,
and the command must be exactly encodable.

Immediately before the first possible byte, the resource path rechecks authority,
epoch, binding generation, mapping revision, and deadline. Only then can it record
send start and issue the compiled WRITE. SimpleDevice cannot bypass this check.

```text
proposal
    -> finite authority lease
    -> bounded queue reservation
    -> final authority and identity check
    -> first possible output byte
    -> ACK validation
    -> optional READBACK transaction
```

Safe profiles are deployment-owned. Their range must fit the declared actuator range,
their safe value must be exactly encodable, and `required_evidence="readback"`
requires a configured readback plan. Reconnect, reload, and safe transition do not
silently rearm control.

## ACK semantics

An ACK response has a fixed exact length, at least one semantic match, no scalar
extract, and an optional supported checksum.

An ACK proves that the current correlated transaction received bytes satisfying that
plan. It does not prove that the device applied the value, that a process changed, or
that later readback will agree.

| Situation | Runtime meaning |
|---|---|
| matching ACK, no required readback | acknowledged evidence; not physical-effect proof |
| matching ACK, readback required | retain ACK evidence and issue the separate readback |
| missing/malformed ACK | no successful ACK; output fails or remains uncertain according to send-start evidence |
| timeout/drop before send start | no send-start evidence; never report success |
| timeout/drop after send start | ambiguous physical outcome; fail closed, no blind retry |
| old-generation ACK | fenced; cannot settle the replacement binding |
| ACK then mismatching readback | failed/mismatch evidence, not success |
| ACK then absent readback | ambiguous or unavailable evidence, not verified state |

## READBACK semantics

READBACK is a second compiled READ transaction in `write.readback`. It uses the same
raw encoding, scale, and offset as the actuator. SimpleDevice verifies the returned
raw scalar exactly against the raw value sent by WRITE; it does not apply a tolerance.
On a match, Runtime can record readback-verified observed state. A mismatch, malformed
response, timeout, or transport loss cannot be promoted to verified state.

A write plan may omit readback when the deployment safe profile accepts ACK evidence.
The remaining evidence then stops at ACK: it still is not physical-effect proof. A
safe profile that requires readback is rejected unless the definition provides it.

## Publication lifecycle

```text
definition
    -> parse and compile
    -> complete candidate validation
    -> hidden, physically inert preparation
    -> event-capacity and durable-state boundary
    -> atomic Runtime publication
    -> visible active topology and scheduling
```

The Runtime validates identities, references, resources, output policy, compiled
transactions, and bounds before activation. Prepared instruments are hidden and their
reads do not run. Publication reserves the required EventLog capacity before the
durable activation point, then installs the topology atomically. Participating
resource reconnect is fenced while publication is pending. A failed pre-durable apply
cleans up prepared topology and pending transport work; no half-published instrument
becomes observable.

## Reconnect and stale evidence

```text
old binding                         new binding
WRITE starts -> connection loss     generation advances
        |                                  |
        `-- old ACK/READBACK ---------------X  fenced
                                           |
                                  fresh transactions only
```

Resource binding generation and output mapping revision identify the topology under
which work was admitted. Reconnect/replacement retires the old authority and fences
old WRITE, ACK, READBACK, and read completions. A late old response cannot make the
new binding ready, settle its output, or publish a fresh observation. A pending
publication also fences participating reconnect so preparation cannot race a binding
replacement.

If connection loss occurs after send start, the physical result is ambiguous. The
Runtime preserves that evidence and does not blindly retry either the requested value
or a safe value. Recovery requires new current-generation evidence and explicit
operator/controller lifecycle actions where applicable.

## Recorder and provenance

Recorder is Runtime-owned. When enabled, the ordinary activation and signal/output
paths retain the SimpleDevice definition identity and version, hashes of raw and
canonical definition bytes, instance binding, resource identity, address/channel,
binding generation, mapping revision, and the corresponding observations/output
evidence. Reconnect and replacement therefore remain distinguishable in scientific
and audit history.

Clients and scripts consume these Runtime-owned facts; they do not create or rewrite
provenance. See [Recorder and SQLite](recorder-sqlite.md) for storage and run behavior.

For reconnect incidents, ambiguous post-send outcomes, and the no-blind-retry
decision model, see [Recovery and fault handling](recovery-and-faults.md).

## Limits

| Item | Bound |
|---|---|
| raw definition | 8,192 bytes |
| normalized canonical definition | 6,144 bytes |
| definitions per deployment | 16 |
| instances per deployment | 32 |
| parameters per definition | 16 |
| actuators per definition | 1 |
| request segments | 1..=8 |
| nonliteral request inserts | at most 4 |
| literal request segment | at most 32 bytes |
| response matches | at most 8 |
| literal response match | at most 16 bytes |
| compiled request / exact response | 1..=64 bytes |
| checksum input | at most 62 bytes |
| Application candidate compact JSON | 8,192 bytes |
| Application candidate values/members | 900 |
| Application candidate container depth | 8 |

## Validation and troubleshooting

| Symptom/category | Likely cause | Safe corrective action |
|---|---|---|
| JSON parse or unknown-field rejection | malformed JSON, wrong spelling, extra field | validate UTF-8 JSON and compare with the exact schema; do not ignore fields |
| size/shape rejection | raw/canonical file, candidate, array, or transaction exceeds a bound | simplify the definition; do not raise bounds to hide a protocol mismatch |
| duplicate/zero identity | repeated parameter/instrument ID or key | assign stable nonzero IDs and unique keys |
| resource reference failure | missing/ineligible resource or native adapter already owns it | bind one eligible COM resource exclusively |
| unsupported grammar | text parsing, variable framing, unsupported checksum/state machine | use a native integration instead of approximating safety evidence |
| address/channel failure | value does not fit selected `u8` field or correlations are ambiguous | choose a fitting encoding and add fixed instance matches |
| scalar/range failure | nonfinite value, invalid transform/range, or non-exact encoding | correct range/scale/offset; never depend on silent clipping/rounding |
| bad READ response | wrong length/literal/instance/checksum/extract/range | compare captured bytes with the fixed plan; no good observation is published |
| malformed or missing ACK | response does not meet the strict ACK plan | inspect protocol/resource health; do not infer that the write did not occur |
| failed READBACK | mismatch, timeout, malformed response, or unavailable resource | treat output as unverified/ambiguous; investigate before rearming |
| output policy rejection | missing safe profile, out-of-range safe value, wrong resource/evidence | correct the deployment policy and validate again |
| reconnect or stale completion | binding changed while work was outstanding | wait for fresh current-generation observations; never reuse old evidence |
| publication/reconnect conflict | reconnect requested while publication owns the resource fence | let the atomic apply finish or fail, then act on the resulting state |
| no serial observations | public examples use placeholder `COM256` | configure the real port and test against the actual protocol; parser tests do not open hardware |

## Tutorials and testing

Follow the [step-by-step SimpleDevice tutorial](developer/simple-device-tutorial.md)
for the included read-only and writable definitions. The repository validates every
published definition with the production parser and compiler and validates both
deployment bindings with the production deployment loader. These checks prove schema,
compilation, and binding behavior; they do not claim physical serial qualification.

For a real device, add deterministic captured-byte/codec tests, validate a safe
deployment before connecting hardware, test malformed/timeout/reconnect behavior, and
qualify WRITE/ACK/READBACK against the device manual. Run the relevant Runtime and
Core tests described in the tutorial before proposing a definition for deployment.
