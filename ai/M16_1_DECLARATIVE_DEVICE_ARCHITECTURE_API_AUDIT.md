# M16.1 declarative simple-device architecture/API/bounds audit

Status: **ACCEPTED**

This is a read-only architecture decision. It describes current source separately from
future M16 requirements. No declarative-device implementation, wire operation, schema,
Runtime behavior, or dependency is added by this report.

## 1. Executive decision

M16 should add one small, compiled, fixed-length simple-device profile, not a template
language and not a second instrument API.

The recommended v1 is:

- serial/COM through an already configured Runtime resource;
- exact fixed-length requests and replies of 1..=64 bytes;
- one scalar per transaction;
- binary signed/unsigned 8/16/32-bit integers and IEEE-754 binary32, with explicit
  endianness, finite scale, and finite offset;
- integer or float observations, but float-only actuator engineering values through
  the unchanged existing OutputAuthority domain;
- exact tagged literal/instance response matches plus at most one selected checksum;
- periodic reads, one output-capable parameter per definition, strict semantic-match
  ACK, and optional independent exact-raw readback;
- no delimiter framing, ASCII-number parser, multipart upload, resource creation,
  arbitrary expressions, callbacks, raw byte operation, or generic state machine;
- one new deduplicated Application mutation, `stage_simple_device_candidate`, followed
  by the existing `apply_configuration` mutation using an incremental Accepted-to-
  terminal lifecycle;
- one complete candidate in one Application message, at most one definition and four
  instances (at most one output-capable instance), with safe profile/controller nested
  only in that instance and no server-side builder or chunk accumulator;
- an 8,192-byte candidate cap, an exact 900-value maximum for the sole closed candidate
  shape, a 512 KiB process-wide retained provisioning-payload credit, and
  collision-safe retention of the full normalized typed mutation for deduplication;
- process-local activation only. It does not rewrite `runtime.toml` and does not survive
  Runtime restart; file stage/reload is rejected while that overlay is active;
- persistent startup uses exactly `[[instruments]] kind="simple_device"` plus a frozen
  deployment-relative definition artifact; M16.2 implements only that read-only path;
- one Runtime-owned pending/prepared apply and, after an ambiguous provisional send,
  one non-discoverable authority quarantine until explicit resource reconnect/restart;
- ordinary Instrument/Signal/Actuator identities after activation. Discovery,
  measurements, events, Workbench, controller input, recent history, Recorder, and
  durable history stay generic.

The transport executor should gain a protocol-neutral 64-byte transaction bound while
Metakon keeps its own 38-byte codec bound. `expected_response: usize` remains the v1
response specification. Delimiter framing is deferred because the current executor and
COM worker do not represent a delimiter, trailing-byte policy, or bounded scan state.

## 2. Labels used in this report

- **CURRENT SOURCE FACT** describes accepted source today.
- **M16.1 DECISION** freezes the proposed M16 contract.
- **FUTURE IMPLEMENTATION REQUIREMENT** is work for a later reviewed slice.
- **DEFERRED / NON-GOAL** is outside M16 v1.

## 3. Source inventory and authority

The audit inspected the requested owners, including:

- Core descriptors and ownership: `crates/lab-core/src/instrument.rs`, `runtime.rs`,
  `runtime/dispatch.rs`, `runtime/physical_io.rs`, `transport.rs`, `metakon.rs`,
  `output.rs`, `output/authority.rs`, `signal.rs`, and `recording.rs`.
- Runtime composition and configuration: `apps/lab-runtime/src/configuration.rs`,
  `definition.rs`, `deployment.rs`, `configuration_api.rs`,
  `service/configuration.rs`, `service/reconnect.rs`, `host.rs`,
  `host/instruments.rs`, `host/scheduler.rs`, `host/configuration.rs`,
  `host/lifecycle.rs`, and `serial.rs`.
- Application and projections: `protocol.rs`, `wire.rs`, `application.rs`,
  `application/**`, `events.rs`, `measurements.rs`, and `sessions.rs`.
- Recorder: `recorder_api.rs`, `recorder/**`, `host/recording.rs`, and Core recording
  facts.
- Generic Workbench consumption: `apps/lab-workbench/src/model/**`,
  `gui/rebuild.rs`, and `gui/app.rs`.
- Existing executable evidence in Core transport/codec/output tests and Runtime
  configuration, configured-physical, reconnect/Recorder, API, event, measurement,
  provenance, and Workbench model/rebuild tests.

Representative accepted tests used as source evidence include
`partial_io_serializes_two_instruments_without_byte_interleaving`,
`queue_capacity_and_exact_deadline_are_explicit`,
`partial_write_is_not_retried_and_remains_unknown_until_recovery`,
`rebind_fences_queued_intent_even_with_equal_logical_ids`,
`bad_crc_and_late_bytes_require_recovery_before_a_new_read`,
`c12_c17_configured_metakon_publishes_normal_signal_with_binding_identity`,
`c14_explicit_rebind_keeps_logical_id_and_fences_old_measurement`,
`c6_configuration_rebind_replaces_address_definition_and_mapping_revision`,
`c5_live_change_commits_once_with_explicit_diff`,
`c8_stale_expired_and_restart_required_candidates_never_touch_runtime`, and
`c5_exact_loaded_runtime_toml_is_durable_after_path_mutation`.

## 4. Current physical path

### 4.1 Deployment and composition

**CURRENT SOURCE FACT**

```text
runtime.toml
    -> InstrumentDto::Metakon
    -> definition path resolved beneath the deployment directory
    -> exact definition bytes frozen and hashed
    -> strict metakon-5x3-v1 JSON parse/validation
    -> DataInstrumentDefinition + MetakonBinding
    -> host/instruments registration and schedules
    -> Runtime MetakonInstrument + ordinary descriptors/signals/output authority
    -> ResourceExecutor
    -> serial ByteTransport worker
```

The deployment loader bounds `runtime.toml` at 64 KiB, one definition at 16 KiB,
all frozen artifacts at 128 entries/1 MiB, resources at 8, instruments at 64, and
Metakon definition parameters at 16. Definition bytes are frozen before composition;
the Recorder does not reread a later pathname.

Metakon-specific pieces are the `InstrumentDto::Metakon` arm, the
`metakon-5x3-v1` definition profile and known operations, address/register selection,
Metakon frame codec/CRC, compatibility probe, scaling rules, and the Metakon-specific
read schedule and pending correlation types.

Already protocol-neutral pieces are stable Core identities and descriptors,
`SignalBuffer`, queries, controller input, `OutputAuthority`, immutable
`OutputIntent`, resource ownership, `ResourceExecutor` queue/deadline/recovery,
`ByteTransport`, serial settings/worker, recording facts, Application projections,
events, Workbench model/rebuild, and Recorder storage/history.

### 4.2 Current READ

**CURRENT SOURCE FACT**

```text
host periodic Metakon schedule
    -> Command::QueueMetakonRead
    -> encode_read
    -> ResourceExecutor::enqueue_read
    -> exact-length serial transaction
    -> decode_read with address/type/flags/CRC checks
    -> generation + mapping revision recheck
    -> ValueSpec validation
    -> ordinary SignalBuffer commit
    -> recording fact / events / generic queries
```

Reads are optionally retried once only after a successful transport recovery and
within the original queue deadline. Protocol decode failure publishes an unavailable
observation, fences queued work, and requires a clean transport recovery boundary.

### 4.3 Current WRITE

**CURRENT SOURCE FACT**

```text
controller OutputProposal
    -> OutputAuthority validation/reservation
    -> immutable OutputIntent
    -> Metakon request bytes prepared
    -> ResourceExecutor queue
    -> final authority + resource binding generation + mapping revision + deadline check
    -> first positive ByteTransport write admission records send_started/DispatchId
    -> strict ACK
    -> optional separately queued readback
    -> exact engineering-value comparison
    -> authority/controller settlement and recording facts
```

The encoder currently runs before the final send fence. That is acceptable because
prepared bytes carry no authority. The hard boundary is that no first possible output
byte is admitted until the current intent, authority epoch/lease, deadline, binding
generation, and mapping revision all validate. A partial write is ambiguous and is
never retried blindly.

## 5. Transport seam decision

### 5.1 Current executor shape

**CURRENT SOURCE FACT**

`ResourceExecutor` owns a bounded queue of 32 transactions plus the reserved safe
slot. A transaction contains owned request bytes, one exact `expected_response`
length, `Read | Output`, queue deadline, execution timeout, read retry flag/count,
binding generation, mapping revision, and (for output) the immutable intent. The
latest result/response replaces prior state; there is no response history.

Both Core executor validation and the serial worker import
`crate::metakon::MAX_FRAME_BYTES` (38). The serial worker has one request and one
completion mailbox slot. Reads request only the remaining exact byte count.

There is no delimiter, escape, trailing-byte, or bounded-scan response model today.
Bytes after the exact requested length are not semantic success evidence and can
contaminate the next transaction until protocol recovery establishes a boundary.

### 5.2 Frozen M16 v1 transport choice

**M16.1 DECISION**

Choose the minimal combination of options A and C:

1. Introduce a protocol-neutral internal maximum byte transaction of **64 bytes** in
   the transport layer and serial adapter.
2. Keep Metakon's codec-specific maximum at 38 bytes.
3. Retain exact `expected_response` length for simple-device v1.
4. Accept only exact fixed-length replies; a valid response consumes exactly that
   length.
5. Reject empty, truncated, and oversized plans before activation. A short reply that
   reaches the finite timeout is transport failure. A malformed exact-length reply is
   protocol failure and forces the existing recovery boundary.
6. Treat detected residual/trailing bytes as protocol contamination: publish no value,
   fence the old queue, and recover the transport boundary before new work.

**DEFERRED / NON-GOAL**

Delimiter-terminated responses are not in v1. Adding them would require a bounded
response-spec enum, delimiter length, maximum scan, retained partial buffer, escape
policy, trailing-byte policy, and deterministic recovery tests in both executor and
serial worker. That is a transport feature, not a harmless schema option.

No universal protocol trait is required. M16 should generalize the smallest current
transaction/completion seam, not create a plugin framework.

## 6. Definition and instance model

### 6.1 Definition identity

**M16.1 DECISION**

One immutable definition has:

- `format_version = 1`;
- stable `definition_id`, a validated logical key of 1..=64 ASCII bytes;
- nonzero `definition_version: u32` selected by its author;
- Runtime-computed SHA-256 of canonical normalized content;
- 1..=16 parameter definitions;
- the bounded transaction plans referenced by those parameters.

The tuple `(definition_id, definition_version, normalized_sha256)` is exact identity.
Reusing the ID/version with different normalized content is invalid. Display names are
not identity.

Normalization is deterministic canonical JSON produced from the fully typed validated
definition, not the caller's spelling: UTF-8; schema field names in fixed schema order;
map keys (where the schema permits a map) in lexicographic byte order; arrays in their
validated semantic order; no insignificant whitespace; integers in minimal decimal;
and finite floating-point values in the serializer's shortest round-trippable decimal
form. Unknown fields, duplicate keys, negative zero in scalar policy fields, and
nonfinite numbers are rejected before hashing. Implementations must test byte-for-byte
canonical vectors rather than independently reproduce a prose hash convention.

Each parameter has a nonzero stable numeric `ParameterId`, key (1..=64 bytes), display
name (1..=128 UTF-8 bytes), unit ID (1..=32 printable non-whitespace ASCII bytes),
unit symbol (1..=16 nonblank UTF-8 bytes), role, access, engineering scalar type and
inclusive range, write effect, and plan references. These unit limits are exactly
`lab_core::model::{MAX_UNIT_ID_BYTES, MAX_UNIT_SYMBOL_BYTES}` and the `Unit::new`
contract; the Application schema creates no larger pre-Core unit domain. V1 has no
separately assignable
signal identity: a readable measurement/diagnostic uses the existing ordinary
`(InstrumentId, ParameterId)` signal identity; an actuator uses the same pair as its
ordinary `ActuatorId`.

V1 permits:

- `measurement` or `diagnostic` with `read_only`, `write_effect = none`, and
  `value_type = integer | float`;
- at most one `actuator`, with `read_write` or `write_only` and
  `write_effect = output_affecting`, and `value_type = float` only.

Engineering ranges use the receiving Core numeric domains exactly:

- `value_type = integer`: `engineering_min` and `engineering_max` are integral JSON
  numbers, both fit `i64`, and `min <= max`;
- `value_type = float`: both bounds are finite `f64` and `min < max`.

Raw device encoding remains independent: a float engineering parameter, including an
actuator, may use any accepted integer or `f32` raw encoding with its validated scale
and offset. Thus a float actuator encoded as `u16_be` with scale `0.1` remains valid.
A structurally valid integer actuator is semantically invalid and fails before staging
with `invalid_configuration` / `invalid_configuration`, retryable false, resync false.
M16 does not add integer `OutputAuthority`, integer `OutputProposal`, another safe-
profile type, or a second controller/output path.

It does not add configuration/action parameters or dynamic operation names.

### 6.2 Instance identity

**M16.1 DECISION**

One instance has a stable nonzero numeric `InstrumentId`, logical key, display name,
the exact definition identity, an existing `ResourceId`, bounded address/channel
values, poll cadence, queue deadline, transaction timeout, and recent-history
capacity. Several instances may share one compiled immutable definition.

Instance address and channel are unsigned 0..=65535. A definition declares whether a
field consumes 1 or 2 bytes, so validation rejects an instance value that does not fit.

An output instance additionally contains the ordinary safe profile for its one
actuator. It may include one optional native PID controller binding to that actuator
and an already existing Reference. The controller's input may be this or another
ordinary Signal. Controller and Reference identities remain ordinary Runtime
identities; no simple-device controller type is introduced.

V1 admits at most four instances per read-only candidate, but at most **one** instance
when the shared definition contains an actuator. This makes the maximum output and
controller preparation cardinality one without weakening the several-read-only-
instances use case.

#### Complete safe-profile sub-schema

**M16.1 DECISION**

An output-capable instance contains exactly one `safe_profile` object for its actuator:

```text
min: finite f64
max: finite f64, strictly greater than min
safe_value: finite f64 in inclusive [min, max]
max_lease_ms: u64 in 1..=60,000
max_proposal_ttl_ms: u64 in 1..=60,000
required_evidence: "ack" | "readback"
```

`min`/`max` must be inside the actuator engineering range and use its unit. If
`required_evidence = readback`, the actuator definition must contain an independent
readback request/response plan. `ack` still requires the strict ACK plan. A safe
profile grants no lease and does not authorize a send.

#### Complete optional controller sub-schema

**M16.1 DECISION**

An output-capable instance may contain exactly zero or one `controller` object. A
candidate therefore contains at most one controller. The fields are:

```text
id: nonzero u64
key: unique validated 1..=64-byte logical key
input_instrument_id: nonzero u64
input_parameter_id: nonzero u64
output_instrument_id: this candidate instance
output_parameter_id: this definition's actuator
reference_id: nonzero u64 identifying an existing Reference
period_ms: 1..=60,000
ema_time_constant_ms: 1..=60,000
ema_warmup_samples: 1..=1,024
kp: finite f64
ki: finite f64
kd: finite f64
output_min: finite f64
output_max: finite f64, strictly greater than output_min
max_input_age_ms: 1..=60,000
max_tick_gap_ms: 1..=60,000
lease_lifetime_ms: 1..=60,000
proposal_ttl_ms: 1..=60,000
```

The ID and key must not collide with any active or same-candidate controller. The
input must resolve to an ordinary readable float parameter with role `measurement`
and a Signal; its unit must equal the existing Reference unit. The output must resolve
to this instance's `read_write` or `write_only`, `output_affecting` actuator. PID
limits must lie inside both the actuator engineering range and safe
profile range. `lease_lifetime_ms <= max_lease_ms`,
`proposal_ttl_ms <= max_proposal_ttl_ms`, and
`proposal_ttl_ms <= lease_lifetime_ms`.

The tighter existing deployment limit remains authoritative: active controllers plus
the candidate controller must be at most 8, and registration also checks the Core
global capacity of 64. The candidate does not create a new controller kind. During
provisional preparation it is `Created`; after safe evidence and atomic publication it
is prepared into ordinary `Ready`, with no lease and no automatic Start. Running is a
later explicit controller lifecycle action.

#### Core/configuration-compatible text bounds

**M16.1 DECISION**

Every candidate string uses an existing domain bound:

| Field | Exact v1 bound and source contract |
|---|---|
| definition, instance, parameter, controller logical keys | 1..=64 ASCII bytes; first ASCII letter, remainder ASCII alphanumeric/`_`/`-`, matching `validate_key` |
| instrument and parameter display names | nonblank, <=128 UTF-8 bytes, matching configuration validation and Core `MAX_NAME_BYTES` |
| unit ID | 1..=32 printable non-whitespace ASCII bytes, matching Core `Unit::new` |
| unit symbol | nonblank, <=16 UTF-8 bytes, matching Core `Unit::new` |
| literal request hex | even lowercase hex, <=64 characters for 32 bytes |
| match hex | even lowercase hex, <=32 characters for 16 bytes |
| operation/schema enum labels | closed Rust-owned enum spellings, never caller-defined strings |

Stable numeric IDs remain nonzero `u64`; versions remain nonzero `u32`. No candidate
text bound exceeds the receiving Core/configuration domain merely because the shared
JSON codec permits strings up to 512 bytes.

## 7. Exact v1 protocol grammar

### 7.1 Request plan

**M16.1 DECISION**

A request is an ordered list of 1..=8 compile-time segments with a final encoded
length of 1..=64 bytes. Its exact JSON shape is `{"segments":[...]}`. Every segment
is one closed tagged object, with exactly these spellings:

```json
{"type":"literal","hex":"10"}
{"type":"instance_field","field":"address","encoding":"u8"}
{"type":"instance_field","field":"channel","encoding":"u16_be"}
{"type":"value_field"}
{"type":"checksum","algorithm":"crc16_modbus"}
```

`literal.hex` represents 1..=32 bytes as even lowercase hex. `instance_field.field` is
exactly `address` or `channel`; its `encoding` is exactly `u8`, `u16_le`, or
`u16_be`. `value_field` uses the enclosing parameter's one encoding from section 7.3
and has no independently selectable transform. A checksum segment is optional, occurs
at most once and last, and its `algorithm` is exactly `sum8`, `xor8`, or
`crc16_modbus`; omission means no checksum.

A read or readback request cannot contain `value_field`. A write request contains
exactly one. There are at most four nonliteral inserts and one checksum.

### 7.2 Response plan

**M16.1 DECISION**

A response is one closed object with `exact_length`, `matches`, and the
role-conditioned optional `extract` and `checksum` members. It declares one exact
length in 1..=64 and these exact nested spellings:

```json
{
  "exact_length": 8,
  "matches": [
    {"type":"literal_match","offset":0,"hex":"10"},
    {"type":"instance_match","offset":1,"field":"address","encoding":"u8"}
  ],
  "extract": {"type":"scalar_extract","offset":3},
  "checksum": {"type":"checksum","offset":6,"algorithm":"crc16_modbus"}
}
```

- `matches` contains 0..=8 objects. `literal_match` has exactly `type`, `offset`, and
  1..=16 bytes of even lowercase `hex`. `instance_match` has exactly `type`, `offset`,
  `field = "address" | "channel"`, and
  `encoding = "u8" | "u16_le" | "u16_be"`.
- `extract`, when present, is exactly
  `{"type":"scalar_extract","offset":<u8>}`. Its width, signedness, endianness,
  scale, and offset come only from the enclosing parameter's `encoding`.
- `checksum`, when present, is exactly
  `{"type":"checksum","offset":<u8>,"algorithm":<algorithm>}` and validates the
  fixed prefix before its offset. Its algorithms use the same three spellings as the
  request checksum.
- there is at most one extract and at most one checksum;
- no overlaps between match, extract, and checksum fields unless bytes are identical
  literal evidence validated at compile time.

A measurement or readback response has exactly one extract. Combining an immutable
definition with an instance compiles every `instance_match` into fixed expected bytes.
When two or more simple instances share a resource, every measurement, ACK, and
readback response that could otherwise be confused must contain sufficient
`instance_match` evidence to distinguish the responding instance. Validation rejects
the shared composition when the bounded grammar cannot prove that distinction; the
instances must then use dedicated resources or a native adapter.

An ACK has exact length, zero extracts, and at least one semantic match. A semantic
match is a `literal_match` or `instance_match` that establishes expected command,
status, or instance identity. A checksum may additionally validate integrity but is
never sufficient ACK evidence by itself. On a shared multi-instance resource, strict
ACK validation must include sufficient `instance_match` evidence for the target
instance.

### 7.3 Scalar and transform grammar

Every parameter contains exactly one parameter-level object:

```json
"encoding": {
  "raw": "i16_be",
  "scale": 0.1,
  "offset": 0.0
}
```

`encoding` has exactly the three members shown. Allowed `raw` values are exactly:

```text
u8, i8,
u16_le, u16_be, i16_le, i16_be,
u32_le, u32_be, i32_le, i32_be,
f32_le, f32_be
```

Decode uses `engineering = raw * scale + offset`. `scale` must be finite and nonzero;
`offset` finite. IEEE binary32 decoded NaN/infinity is invalid. Write encoding applies
the inverse transform and must produce a value representable by `raw`; integer raw
encodings reject a non-integral inverse result. The engineering range is rechecked
after transformation, and an `integer` engineering parameter additionally requires an
integral transformed engineering value. One parameter's read, write, and readback
plans all use this same encoding and transform; transaction objects cannot override
them.

Selected checksum algorithm strings are exactly:

```text
sum8
xor8
crc16_modbus
```

`sum8` is modulo 256. `crc16_modbus` uses polynomial 0xA001, initial 0xFFFF, and a
little-endian field. No checksum is represented only by omission of the checksum
segment/member, not by an additional enum value. Checksum coverage is the fixed prefix
before the checksum field, at most 62 bytes.
There are no caller-selected polynomials, seeds, ranges, reflection flags, or lookup
tables.

**DEFERRED / NON-GOAL**

ASCII numeric fields, binary64, packed bits/BCD, delimiters, escaping, regex, branches,
loops, variables, callbacks, expressions, user functions, dynamic allocation rules,
and state machines are not v1. A device needing them uses a native Rust adapter.

## 8. Parse, validation, compilation, and ownership

**M16.1 DECISION**

```text
bounded untrusted structured candidate
    -> strict Application JSON parse (already bounded)
    -> simple-device schema parse with unknown/duplicate rejection
    -> structural bounds
    -> semantic and cross-reference validation
    -> canonical normalization + SHA-256
    -> compile fixed offsets/lengths/scalars/checksums into immutable plans
    -> stage one complete candidate
    -> explicit safe apply
```

The external schema belongs to `lab-runtime` configuration/Application code.
`lab-core` does not parse JSON/TOML, inspect filesystem paths, or retain untrusted maps.
Only bounded validated domain descriptors, fixed compiled plan enums/arrays, exact
bytes, scalar metadata, and generation/revision identities may cross the Core owner
boundary. Normal transactions do not walk JSON objects or revalidate schema.

The Runtime/Core single owner retains ordinary instrument observations,
`OutputAuthority`, transport correlation, and pending ACK/readback state. The host owns
external candidate bytes, definition registry, instance schedules, configured COM
resource registry/workers, and provenance assembly. The protocol-neutral compiled plan is immutable and shared by
instances; instance values are copied into bounded request preparation.

Codec execution belongs on the trusted Runtime side of the Application boundary. It
may precompute immutable request bytes before queuing. It cannot open a transport,
grant output authority, alter deadlines, or commit an observation without the Core
owner's generation/revision checks. No callback or generic script-engine seam is
introduced.

For an output, the retained correlation is:

```text
ActuatorId + OutputIntent/DispatchId
+ definition hash/version
+ binding_generation + mapping_revision
+ compiled write/ACK/readback plan identities
```

The exact ACK/readback result can settle only that correlation.

### 8.1 Persistent deployment representation

**M16.1 DECISION**

The persistent baseline gains exactly one deployment instrument kind:

```toml
[[instruments]]
kind = "simple_device"
id = 1001
key = "furnace-1"
display_name = "Furnace 1"
definition = "definitions/simple-furnace-v1.json"
resource_id = 7
address = 1
channel = 0
poll_period_ms = 250
queue_timeout_ms = 250
transaction_timeout_ms = 500
history_capacity = 1024
```

`definition` is a deployment-relative immutable JSON artifact using exactly the same
strict definition schema, identities, grammar, compiler, and Core-compatible bounds as
the definition embedded in an Application candidate. Raw artifact bytes are at most
**8,192**; canonical normalized definition bytes are at most **6,144**. Deployment
loading resolves the path under the existing artifact rules, reads it once, validates
and compiles it, freezes the exact raw bytes, and records their hash/provenance exactly
as other trusted definition artifacts do. Runtime never rereads the path after
activation.

Persistent output policy does not duplicate the compact API nesting. A persistent
output-capable `simple_device` instrument is associated through the existing
deployment-level `[[safe_profiles]]` and optional `[[controllers]]` records. Those
records retain their accepted fields, validation, capacities, native controller kind,
and existing Reference requirement. Both the persistent form and the API candidate
form compile into the same bounded internal definition, instance, safe-profile, and
controller representation before either can become active.

When no API simple-device overlay is active, `stage_configuration` and
`reload_configuration` may load persistent `simple_device` entries through the
ordinary deployment lifecycle and its live/restart-required classification. Once an
API overlay is active, section 11.3's pre-slot reload rejection remains absolute.

**FUTURE IMPLEMENTATION REQUIREMENT**

M16.2 implements only:

```text
persistent/file-backed read-only simple_device startup/composition
    -> frozen definition artifact
    -> compiled fixed READ
    -> ordinary Signal
```

M16.2 adds no Application provisioning, output, safe-profile/controller activation,
or client-uploaded candidate.

## 9. Read-only vertical path

For `furnace-1/temperature`, a validated measurement parameter becomes an ordinary
`ParameterDescriptor` and derived ordinary `SignalId`. One scheduler entry retains
the parameter/plan identity and timing.

**FUTURE IMPLEMENTATION REQUIREMENT**

```text
periodic due
    -> construct <=64-byte fixed request from compiled plan + instance fields
    -> enqueue retryable read with binding_generation/mapping_revision
    -> exact-length completion
    -> validate matches/checksum; extract and transform scalar
    -> recheck current instance/binding/mapping
    -> validate ValueSpec and commit Sample
```

Thereafter unchanged generic paths provide:

- Core `Query::Discover`, descriptor, latest, and bounded signal window;
- Application `discover`/`describe`, `measurements_current`, `latest`,
  `measurement_window`, and signal events/subscriptions;
- Workbench discovery/model/rebuild/live plot through ordinary `instrument` and
  `signal` records;
- controller input through ordinary `SignalId`;
- Core recording facts, Recorder admission, SQLite rows, and durable history.

These generic consumers must not match a `SimpleDevice` kind merely to implement
ordinary signal behavior. Only configuration composition, scheduling, codec, and
physical correlation know the implementation kind.

## 10. Writable actuator, ACK, readback, and ambiguity

### 10.1 Preparation and send fence

**M16.1 DECISION**

```text
controller OutputProposal
    -> existing Runtime OutputAuthority
    -> immutable OutputIntent reservation
    -> compile/encode immutable <=64-byte request
    -> ResourceExecutor queue
    -> immediately before byte offset zero:
       current authority/lease/epoch + intent expiry + binding generation
       + mapping revision + Required Recorder gate
    -> first positive transport admission records send_started/DispatchId
```

Encoding before the final check is allowed. No first possible output byte is allowed
before that check. Declarative data never decides authorization.

### 10.2 Evidence states

**M16.1 DECISION** preserves distinct:

```text
requested != authorized != send_started != ACK != readback != physical_effect
```

- ACK is an exact fixed response plan. It carries no engineering value unless it is a
  separately declared read response (v1 keeps those roles separate). It has at least
  one semantic match; checksum-only ACK is invalid.
- At write preparation, Runtime retains the exact encoded raw scalar bytes produced by
  the validated parameter encoding alongside the immutable output correlation.
- Independent readback uses its own request and scalar response plan after ACK, but the
  response extract uses that same parameter encoding. Strict length, semantic matches,
  instance correlation where required, and checksum all validate before extraction.
- Integer readback compares exact decoded raw numeric equality with the retained written
  raw value. For `f32`, decode rejects NaN/infinity, normalizes both signed-zero bit
  patterns to canonical positive zero, and otherwise requires exact IEEE-754 binary32
  bit identity with the retained encoded value.
- Only exact normalized raw equality permits engineering projection and
  `ReadbackVerified`. This rule is identical for normal-output and safe-output
  verification.
- A mismatch is terminal failure evidence and triggers the existing fail-closed/safe
  behavior; it is positive mismatch evidence, not ambiguity or successful physical
  evidence.
- Timeout or rejection before the first possible byte retires pre-wire with no
  ambiguity.
- Partial write, timeout/disconnect after `send_started`, malformed ACK, or ACK timeout
  is ambiguous/transport-uncertain and never auto-retried.
- Malformed/timeout readback after a valid ACK retains ACK evidence but leaves the
  stronger readback result ambiguous and faults the controller as current policy does.
- Stale binding generation or mapping revision cannot settle the replacement. It is
  ignored/fenced for observation authority and the correlated old output remains
  failed/ambiguous according to its already captured send stage.
- A safe-state obligation is not permission to resend an ambiguous write.

Caller-selected or definition-selected readback tolerance is not in v1. Tolerance-
based verification is deferred and would require a separate reviewed safety policy.

## 11. Configuration and topology lifecycle

### 11.1 Current lifecycle

**CURRENT SOURCE FACT**

The current deployment lifecycle retains one candidate for 30 seconds, computes
effects before mutation, checks expected revision, performs an output safe barrier and
binding preparation when required, then commits one immutable configuration. Current
topology identity changes are classified `restart_required`; live apply currently
handles fixed-topology fields, reinitialization, Metakon rebind, controller rewarm,
and safe-profile changes.

Persistent deployment source, process-local property overlays, and ephemeral Runtime
state are already distinct. `stage_configuration` rereads the Runtime-known path; it
does not accept an uploaded manifest.

### 11.2 M16 v1 topology rules

**M16.1 DECISION**

| Change | V1 decision | Required fence |
|---|---|---|
| Add read-only simple instance on existing resource | live additive candidate | prepare compiled plans/resource compatibility without global `configuration_quiesced`; atomic owner commit |
| Add output-capable instance | one live additive instance maximum | prepare incrementally, then pause Running/Warming controllers, revoke leases, establish the active-output safe barrier, enter global `configuration_quiesced`, cross Required Recorder/candidate safe-evidence fences, and publish atomically |
| Change poll cadence | ordinary live property, 10..=60,000 ms | restart schedule from commit time, no catch-up |
| Change definition content/version | `restart_required` | no active mutation |
| Change device address/channel | `restart_required` | no active mutation |
| Change resource binding | `restart_required` | existing explicit resource reconnect is not remapping |
| Change COM settings/add resource | `restart_required`, deployment-source only | process restart |
| Remove an instance | `restart_required` | no active mutation |
| Replace an instance/definition | `restart_required` | no active mutation |
| Add/change controller or Reference outside the optional additive binding | `restart_required` in this API | deployment-source lifecycle |

The additive apply must prepare all definitions, schedules, descriptors, resource
references, provenance, output authorities, and optional controller bindings before
the authoritative commit. Any probe/safe-evidence/Recorder failure preserves the
prior active deployment. No partially discoverable instrument is permitted.

For an output-capable instance, a valid safe profile is mandatory in the same
candidate. A controller is optional: without it the actuator is visible but remains
safe/disarmed and there is no raw public write operation. If a controller is supplied,
its Reference must already exist, its input/output units and identities must validate,
and the controller is published `Ready` without a lease or output authority. Candidate
validation rejects an unrestricted output topology.

### 11.3 File reload and the process-local overlay

**M16.1 DECISION**

While any API-provisioned simple-device topology is active, both
`stage_configuration` and `reload_configuration` from `runtime.toml` are rejected
**before** reading into or occupying the one staged-candidate slot. The exact public
envelope is:

```text
code = invalid_configuration
category = invalid_configuration
retryable = false
resync_required = false
```

The active topology and configuration revision remain unchanged. V1 does not merge
the persistent file authority with the process-local overlay and never silently drops
the overlay. Runtime restart is the explicit operation that discards the overlay and
restores the validated `runtime.toml` baseline.

`property_configure` remains permitted for properties supported by the active model.
It clones the **current active configuration**, including the process-local
simple-device overlay, applies one bounded property override, and stages/applies that
clone. It therefore cannot erase an API-provisioned device. Its ordinary one-slot and
revision rules still apply.

Future deterministic tests must prove:

1. an active API device makes stage/reload fail with the envelope above before the
   slot is occupied;
2. the active device and revision are unchanged and a subsequent valid
   `stage_simple_device_candidate` can still use the slot;
3. a supported `property_configure` retains the overlay; and
4. Runtime restart removes the API device and restores only the file baseline.

### 11.4 Incremental apply lifecycle and ownership

**CURRENT SOURCE FACT**: ordinary configuration mutations are currently dispatched
synchronously by `Application::handle`. That is not an acceptable execution shape for
a simple-device preparation that may take 15 seconds.

**M16.1 DECISION**

Every `apply_configuration` targeting a staged simple-device candidate uses one
incremental lifecycle:

```text
Application admits request_id in SessionStore
    -> Accepted reply becomes deliverable immediately
    -> staged candidate ownership moves into one PendingSimpleConfigurationApply
    -> later serialized ServiceHost/Runtime owner turns advance bounded steps
    -> terminal Completed or Failed is committed to the same SessionStore identity
```

Application owns only request/session/reply correlation. ServiceHost/Runtime owns the
pending apply, `PreparedSimpleTopology`, quiesce phase, safety progress, Recorder fence,
and eventual commit. Each owner turn performs at most one bounded preparation/service
step and returns; the owner loop never waits/yields until the entire lifecycle
finishes.

A read-only additive apply requires no global output safe barrier and never enters
global `configuration_quiesced`: its bounded preparation runs while existing ordinary
acquisition and controller production continue, and its descriptors/schedules become
visible only at one successful atomic commit.

An output-capable apply has two liveness phases. Before crossing its global safety
boundary, ordinary acquisition/control may continue. It then pauses active
Running/Warming controllers, revokes leases, establishes the existing active-output
safe barrier, and sets `configuration_quiesced = true`. From that point until
commit/failure/quarantine transition, HostCore schedules no new acquisition and
produces no new controller work. Safety service, already-admitted transport, Recorder,
bounded Application/network/client scheduling, queries, `operation_status`, and
subscription/event delivery for already-produced state/events continue every owner
turn. This is owner liveness, not continuous measurement acquisition during the
safety-critical transition.

Every terminal commit, failure, or quarantine transition clears
`configuration_quiesced`. Acquisition scheduling resumes from the current monotonic
time with no catch-up burst. Controllers remain safe/Ready-or-Paused/Failed as
applicable and are never automatically rearmed.

There is at most one process-wide pending simple configuration apply. Its one
15-second absolute monotonic deadline starts when ServiceHost admits preparation, not
when a later sub-step happens, and is never restarted. The immutable staged candidate
is consumed when it moves into this pending state; staging another candidate while it
is pending returns the exact `busy` mapping in section 13. Disconnecting the originating
client does not cancel accepted work. `operation_status` observes the ordinary retained
`Accepted`/`Completed`/`Failed` lifecycle, and exact retry of the same retained identity
returns retained evidence without creating a second apply.

Existing configuration applications that do not contain this provisional
simple-device lifecycle may remain on their current synchronous path. M16 does not
generalize all configuration operations into a new job framework.

Deterministic acceptance must prove all four phases separately:

1. a read-only pending apply leaves unrelated acquisition/control production running;
2. an output pending apply before quiesce leaves ordinary progress running;
3. an output apply after quiesce continues safety, already-admitted transport,
   Recorder, query, `operation_status`, network/client, and already-produced
   subscription/event delivery while new acquisition/control production remains
   suspended; and
4. a terminal commit/failure/quarantine clears quiesce, resumes acquisition from the
   current time without catch-up, and does not rearm controllers.

In every phase the operation remains `Accepted` until one later owner turn publishes
the terminal result, and exactly one pending/prepared apply exists.

### 11.5 Single-owner prepared topology and ambiguity retention

**M16.1 DECISION**

The serialized Runtime/ServiceHost owner holds one non-discoverable
`PreparedSimpleTopology` inside the pending lifecycle. It uses the existing
`OutputAuthority` implementation and `ResourceExecutor`; no host-side raw safe probe,
second authority implementation, or second experiment owner exists.

```text
pending immutable candidate
    -> revision/capacity/resource validation
    -> create non-discoverable descriptors/plans/schedules/bindings
    -> bind candidate safe profile inside Runtime-owned OutputAuthority
    -> reserve Required Recorder activation evidence
    -> pause active Running/Warming controllers and revoke leases
    -> establish existing active-output safe barrier
    -> set configuration_quiesced = true
    -> request safe through that authority
    -> ResourceExecutor final authority/deadline/binding/mapping/Recorder fence
    -> first possible byte records send_started and DispatchId
    -> strict ACK and required independent readback
    -> one serialized atomic publication, or a phase-specific failed transition
```

Candidate queries, discovery, events, measurements, controller lookups, and schedules
remain invisible until commit. Completions carry candidate ID, provisional mapping
revision, binding generation, and dispatch identity and may settle only that
provisional correlation. Successful safe evidence permits one atomic move of authority,
descriptors, optional `Created -> Ready` controller, provenance, and schedules into the
active owner; no controller is started and no observer sees a half-installed graph.

M16-specific queue and transaction timeouts are each at most 2 seconds. With one
candidate output, ACK/readback consume at most 8 seconds including queue waits and the
two existing Recorder waits consume at most 4 seconds, inside the one unchanged
15-second absolute deadline. Preparation before the safety boundary does not quiesce
ordinary production. Only after controllers are paused, leases revoked, and the
active-output safe barrier established does the owner enter `configuration_quiesced`;
the exact suspended/continuing services and terminal resume rules are those in section
11.4.

Failure is split at the existing output ambiguity boundary:

1. **Before `send_started`:** there is no physical ambiguity. The prepared topology is
   dropped, the already-consumed staged candidate is not restored, and the apply becomes
   terminal `Failed`. A later attempt requires a new explicit stage and a new apply
   mutation identity.
2. **After `send_started`:** if required ACK/readback safe evidence is not established,
   the provisional authority is not dropped. It moves into one Runtime-owned,
   non-discoverable `QuarantinedSimpleOutput` reconciliation state. The apply still
   becomes terminal `Failed` and its retained identity can never execute again.

The quarantine is process-wide maximum one and retains exactly: candidate ID,
resource ID, the `OutputAuthority` instance, DispatchId/output-intent correlation,
binding generation, mapping revision, send-started/ACK/readback evidence, and the
authority's ambiguous-safe-resend-blocked/FaultLatched semantics. It never becomes an
Instrument or Actuator, never resends automatically, has no time-based ambiguity
expiry, and blocks another output-capable simple-device preparation on that resource.

`configuration_status` exposes a bounded semantic projection containing the state,
candidate/resource IDs, generations/revision, evidence phase, and
`safe_resend_blocked = true`; it exposes no request/response bytes. Recovery is the
existing explicit `reconnect_resource` lifecycle for the current resource generation.
Reconnect retires/fences the old worker and bytes, establishes a fresh binding, and
then permits quarantine retirement; it does not publish the failed candidate or prove
the old physical output safe. The user must stage a new candidate. Runtime restart
also removes process-local provisional software state but makes no old-safe claim: the
new boot's ordinary safe/unverified policy is authoritative.

A positive non-safe readback mismatch may cause a distinct safe action during the same
15-second lifecycle only through the existing OutputAuthority rules. If required safe
evidence is still absent at the absolute deadline, the quarantine above is retained.

## 12. Resource provisioning decision

**M16.1 DECISION**

V1 requires the target configured COM resource to already exist in the active
deployment. The client supplies only stable `resource_id`; it supplies no COM
path/settings and receives no transport handle.

Resource-kind eligibility is exact:

```text
read-only simple-device instance
    -> windows_com_read_only or windows_com

output-capable simple-device instance
    -> windows_com only
```

An output candidate naming `windows_com_read_only` fails during staging, before any
prepared/output state exists, with:

```text
code = invalid_configuration
category = invalid_configuration
retryable = false
resync_required = false
```

An API candidate may bind a resource that is currently unbound or already bound only
to active simple-device instances. Multiple simple-device instances may share one
resource. Mixing simple-device instances with Metakon or another native protocol
adapter on the same resource is deferred from M16 v1; staging rejects that composition
as `invalid_configuration`. This is a composition rule, not a new resource kind.

Complete workflow:

1. An operator configures the Windows COM resource in `runtime.toml` with port, baud,
   data bits, parity, stop bits, flow control, and finite I/O/open/recovery timeouts.
2. Runtime starts and validates/owns that resource.
3. The client stages a bounded simple-device candidate referencing its `resource_id`.
4. Runtime validates resource kind/composition/state and uses the existing serialized
   executor.
5. Explicit apply prepares/probes, crosses safety/Recorder fences where required, and
   atomically publishes ordinary entities.

Atomic new-resource provisioning is deferred. It would combine OS handle acquisition,
port uniqueness, rollback, generation, and deployment persistence with the first
device slice and is not the smallest safe API.

`reconnect_resource` remains one resource-scoped public mutation. The resource query
publishes one authoritative expected binding generation; reconnect checks it and never
selects one instrument as the public target. For all simple instances bound to that
resource, one accepted reconnect:

1. quiesces/fences all old queued reads, writes, ACKs, and readbacks together;
2. replaces the physical transport and advances the binding generation for every
   bound instance;
3. advances each instance's mapping revision as the current Core rebind model requires,
   even when definition bytes did not change;
4. commits an Unavailable baseline so old Good samples are not current until ordinary
   new-generation reacquisition;
5. re-establishes required safe evidence for an output-capable simple instance before
   release; and
6. leaves its controller non-running, with no automatic authority rearm or mutation
   replay.

Old-generation completions cannot settle any new-generation instance. If the resource
owns a `QuarantinedSimpleOutput`, the same explicit reconnect fences its old dispatch
correlation and permits bounded quarantine retirement; it still does not activate the
failed candidate.

Future M16.5 acceptance must include two or more read-only simple instances sharing one
resource, prove all old work is fenced by one reconnect and all signals reacquire
generically, and separately prove that output-resource reconnect restores safe evidence
without restarting its controller.

## 13. Application provisioning surface

### 13.1 Options considered

- A complete candidate mutation fits the product and reuses the existing one-slot
  deployment lifecycle.
- A create/add/validate/apply builder creates bounded but unnecessary server-side edit
  state, abandonment rules, and more operations.
- A generic uploaded deployment manifest exposes too much topology, duplicates the
  64 KiB file schema, and conflicts with the 16,383-byte Application body.

### 13.2 Frozen surface

**M16.1 DECISION**

Add exactly **one** mutation operation:

```text
stage_simple_device_candidate
```

When implemented, the registry therefore changes from 42 to 43 operations (22
queries remain; mutations change from 20 to 21). Adding the one capability below
changes the hello capability catalog from 25 to 26. Those are future additive source
changes, not current M16.1 repository behavior.

Arguments:

```json
{
  "expected_revision": "<u64 decimal>",
  "candidate": {
    "schema_version": 1,
    "definition": {
      "format_version": 1,
      "definition_id": "<logical key>",
      "definition_version": 1,
      "parameters": [ "1..=16 exact parameter objects and transaction plans" ]
    },
    "instances": [{
      "instrument_id": "<nonzero u64 decimal>",
      "key": "<logical key>",
      "display_name": "<bounded name>",
      "resource_id": "<nonzero u64 decimal>",
      "address": 0,
      "channel": 0,
      "poll_period_ms": 250,
      "queue_timeout_ms": 250,
      "transaction_timeout_ms": 500,
      "history_capacity": 1024,
      "safe_profile": {
        "min": 0.0,
        "max": 100.0,
        "safe_value": 0.0,
        "max_lease_ms": 2000,
        "max_proposal_ttl_ms": 200,
        "required_evidence": "readback"
      },
      "controller": {
        "id": "<nonzero u64 decimal>",
        "key": "<logical key>",
        "input_instrument_id": "<nonzero u64 decimal>",
        "input_parameter_id": "<nonzero u64 decimal>",
        "output_instrument_id": "<same candidate instance>",
        "output_parameter_id": "<definition actuator>",
        "reference_id": "<existing Reference>",
        "period_ms": 200,
        "ema_time_constant_ms": 1000,
        "ema_warmup_samples": 4,
        "kp": 3.0,
        "ki": 0.4,
        "kd": 0.2,
        "output_min": 0.0,
        "output_max": 100.0,
        "max_input_age_ms": 750,
        "max_tick_gap_ms": 750,
        "lease_lifetime_ms": 2000,
        "proposal_ttl_ms": 200
      }
    }]
  }
}
```

This is the sole v1 candidate shape. The candidate root has exactly
`schema_version`, `definition`, and `instances`; there is no top-level controller or
safe-profile collection. A read-only instance omits both optional fields. The one
permitted output-capable instance contains exactly one `safe_profile` and zero or one
`controller`, using every field and cross-check frozen in section 6.2. Because the
candidate contains exactly one definition, instances do not repeat a definition
identity; compilation binds them to the embedded immutable definition.

Each parameter object has exactly the twelve fixed fields
`parameter_id`, `key`, `display_name`, `role`, `access`, `unit_id`, `unit_symbol`,
`value_type`, `engineering_min`, `engineering_max`, `write_effect`, and `encoding`,
followed by role-conditioned transaction members. `encoding` has exactly `raw`,
`scale`, and `offset`. A readable parameter has one `read` object with exactly
`request` and `response`. The actuator has one `write` object with exactly `request`,
`ack`, and optional `readback`; `readback`, when present, has exactly `request` and
`response`. There is no tolerance member. A request has only `segments`; a response
has `exact_length`, `matches`, and role-conditioned optional `extract`/`checksum`. The
exact closed segment, match, extract, and checksum tags and field spellings are those
in section 7. Unknown fields, transaction-local encoding or transform fields, a
readback tolerance, or moving any safe/controller field to another level is
`invalid_args`; there is no second accepted spelling of the v1 schema.

The mutation performs strict parse, validation, cross-reference, normalization/hash,
compilation, effect classification, and staging. It does not activate. Its completed
result is:

```json
{
  "candidate_id": "<u64 decimal>",
  "base_revision": "<u64 decimal>",
  "expires_at_ns": "<u64 decimal>",
  "effects": ["..."],
  "definition": {
    "id": "...",
    "version": "<u32 decimal>",
    "normalized_sha256": "<64 lowercase hex>"
  },
  "instances": ["<instrument id decimal>"]
}
```

Activation uses the existing `apply_configuration { candidate_id,
expected_revision }`. `configuration_status` exposes the one staged candidate and
classified effects; for a simple candidate it later exposes pending preparation or
bounded provisional-output reconciliation as defined in section 11. There is no
validate-only duplicate operation: successful staging is the bounded retained
validation result. Expiry cleans an unapplied staged candidate; beginning apply
consumes it permanently.

Candidate identity, cardinality, expiry, and revision conflict use the existing
Runtime-wide one-slot lifecycle: one candidate, checked u64 ID, 30-second monotonic
retention, base revision, and explicit apply. It is not owned by a socket; disconnect
does not mutate it. A simple apply follows section 11.4: `Accepted` is deliverable
immediately and terminal completion occurs on later owner turns. No chunk state exists.

The staging mutation uses normal `request_id` admission/dedup. Exact retained retry
returns the retained outcome and cannot stage twice; conflicting payload returns
`request_conflict`; old unknown identity remains `outcome_unknown`. Apply is a distinct
explicit mutation with its own next request identity. There is no automatic
stage/apply/status/retry.

Add one hello capability:

```text
simple_device_provisioning
```

It is a discovery label, not authority. Actual availability is the authoritative
advertised operation list and requires the normal Configuration feature plus at least
one eligible configured COM resource.

Provisioning uses only existing public codes. This is the exact mapping; code and
category are deliberately separate fields:

| Provisioning failure class | Wire `code` | `category` | `retryable` | `resync_required` | Authoritative result |
|---|---|---|---:|---:|---|
| candidate object malformed, unknown field, wrong schema version, M16 byte/lexical/cardinality bound | `invalid_args` | `invalid_request` | false | false | no stage |
| structurally valid but semantically invalid/cross-reference-unsafe candidate, including active topology capacity | `invalid_configuration` | `invalid_configuration` | false | false | no stage |
| `role = actuator` with `value_type = integer` | `invalid_configuration` | `invalid_configuration` | false | false | no stage, prepared authority, or alternate output domain |
| referenced configured COM resource absent | `unknown_resource` | `not_found` | false | false | no stage |
| output candidate uses `windows_com_read_only`, or simple/native adapter mixing is requested | `invalid_configuration` | `invalid_configuration` | false | false | no stage or prepared state |
| stale `expected_revision` at stage or apply | `revision_conflict` | `revision_conflict` | false | true | no mutation |
| one staged slot already occupied | `busy` | `capacity_exhausted` | true | false | retained candidate unchanged |
| pending simple apply already exists | `busy` | `capacity_exhausted` | true | false | existing pending apply unchanged |
| provisional-output quarantine already occupies the process-wide slot and another output apply is requested | `busy` | `capacity_exhausted` | true | false | quarantine unchanged; its resource remains blocked |
| SessionStore provisioning-payload byte credit exhausted | `busy` | `capacity_exhausted` | true | false | no admission and no stage |
| candidate expired or unknown on `apply_configuration` | `revision_conflict` | `revision_conflict` | false | true | active topology unchanged |
| active API overlay blocks file stage/reload | `invalid_configuration` | `invalid_configuration` | false | false | no slot occupation; active overlay unchanged |
| resource/transport unavailable before provisional `send_started` | `transport_unavailable` | `transport_unavailable` | true | false | consumed candidate/prepared topology retired; active topology unchanged |
| Required Recorder unavailable before candidate send | `recording_unavailable` | `recording_unavailable` | true | false | consumed candidate/prepared topology retired without ambiguity; active topology unchanged |
| Required Recorder commit/durability fails after required safe evidence | `recording_unavailable` | `recording_unavailable` | true | false | no publication; verified-safe provisional state retires, active topology unchanged |
| active safe barrier or candidate pre-publication safe evidence fails | `output_rejected` | `invalid_configuration` | false | false | no commit; after send-start provisional authority enters reconciliation quarantine |
| internal serialized-owner/prepared-state failure | `operation_failed` | `operation_failed` | false | false | no success fabricated; phase-specific retirement/quarantine follows section 11.5 |

Shared JSON/framing failures that occur before operation mapping keep their existing
exact codec codes (for example `frame_too_large`, `json_depth`, or `json_values`); the
table does not alias them to `invalid_args`. Implementations and clients must read the
actual `category`, `retryable`, and `resync_required` fields rather than derive them
from a code name.

No public raw bytes, filesystem path, serial settings, handle, command execution, or
dynamic operation is exposed.

Application envelope protocol remains **v1**. This is an additive hello-advertised
operation/capability using operation version 1 and the existing session model, so no
wire envelope version bump is required. Implementation must update the semantic API
inventory/version documentation when the operation ships; it must not silently alter
existing operation schemas.

### 13.3 Payload-size decision

**CURRENT SOURCE FACT**: Application JSON body is at most 16,383 bytes; a current
file definition may be 16 KiB and therefore cannot fit unchanged inside an envelope.

**M16.1 DECISION**:

- complete serialized `candidate` JSON: at most **8,192 bytes**;
- canonical normalized `definition`: at most **6,144 bytes**;
- no chunks/multipart and no path/artifact reference;
- one definition and 1..=4 read-only instances, or one output-capable instance;
- one M16-specific candidate lexical maximum of **900**, counted exactly as the shared
  codec counts it: `1 + every object member + every array element`;
- standalone candidate JSON depth at most **8**.

The strict external schema represents literal/match bytes as an even-length lowercase
hex string (at most 64/32 characters for the existing 32/16-byte limits), not as an
array of JSON numbers. That representation is decoded during compilation and never
becomes a public raw-write operation.

The two legal instance variants are mutually exclusive. Their exact upper-bound
accounting is:

| Candidate contribution | Four read-only instances | One output-capable instance |
|---|---:|---:|
| candidate root marker and three members (`schema_version`, `definition`, `instances`) | 4 | 4 |
| definition fixed members | 4 | 4 |
| 16 parameter array elements | 16 | 16 |
| twelve fixed fields for each of 16 parameters, including the `encoding` member | 192 | 192 |
| three nested `encoding` members for each of 16 parameters | 48 | 48 |
| instance array elements | 4 | 1 |
| ten base instance members | 40 | 10 |
| nested `safe_profile` member plus its six fields | 0 | 7 |
| nested optional `controller` member plus its 19 fields | 0 | 20 |
| all read/write/ACK/readback plan members, segment/match array elements, and exact tagged segment/match/extract/checksum fields combined | at most 592 | at most 598 |
| **exact admitted structural maximum** | **900** | **900** |

The collective plan budget is a schema rule in addition to every local plan bound:
592 for the four-read-only-instance variant and 598 for the one-output-instance
variant. A candidate that respects each local array capacity but exceeds its variant's
collective budget is not schema-valid and returns `invalid_args`. Both mutually
exclusive variants therefore have the same exact maximum of **900**, below the shared
1,024-value ceiling. The calculation never combines four read-only instances with the
output-only nested objects and therefore does not imply a second candidate shape.

For the maximal structurally legal mutation envelope, the shared upper bound is:

```text
1 root marker
+ 5 root members
+ 2 args members
+ 2 request_id members
+ (900 - 1 candidate root marker)
= 909 values/members <= 1,024
```

The deepest exact candidate path is
`candidate.definition.parameters[].read.response.matches[]`, depth **8**. Embedding
adds the request root and `args`, so whole-message depth is **10 <= 16**. Candidate
keys/strings use their tighter schema
bounds (the largest display string is 128 bytes); outer `msg_id`/scope are 64 bytes,
so every key/string remains <=512 bytes.

With a maximal 8,192-byte candidate, 64-byte `msg_id`, 64-byte scope, 20-byte decimal
sequence and revision, and fixed operation name, the maximum canonical compact
provisioning request produced by the production encoder is **8,496 bytes**, leaving
7,887 bytes below the 16,383-byte Application body cap. Incoming spelling, whitespace,
and escaping need not be canonical; the shared decoder independently enforces the
authoritative 16,383-byte raw Application-body limit before schema admission. The
file-backed Metakon 16 KiB parser bound is not reused as an API promise.

A future executable oracle must construct the jointly maximal legal schema shape,
wrap it in the maximal valid mutation envelope, serialize it through the production
DTO, and run the production lexical pre-scan and decoder. The one test asserts body
`<=8,496`, candidate count `<=900`, whole count `<=909`, candidate depth `<=8`, whole
depth `<=10`, unit ID/symbol `<=32/16`, and every string/key within its receiving-domain
bound (and therefore `<=512`). Boundary tests prove that adding one lexical value,
field, or element past either exact 900/909 maximum or another frozen closed-schema
maximum is rejected.

### 13.4 SessionStore retained provisioning memory

**CURRENT SOURCE FACT**: `SessionStore::Record` retains the complete normalized typed
`Mutation` for exact deduplication. Existing bounds are 64 pending operations, 256
terminal operations, and 600 seconds of terminal retention.

**M16.1 DECISION**: keep full collision-safe typed equality and add one process-wide
**524,288-byte retained provisioning-payload credit** inside `SessionStore`. No
hash-only equality is permitted.

One provisioning record is charged:

```text
align_up(canonical_normalized_candidate_bytes, 64) + 8,192 bytes
```

The second term covers the fixed bounded typed tree, owned strings/vectors, mutation
variant, and per-record bookkeeping; implementation must use bounded containers or
account actual capacities so this charge is a real ceiling, not an estimate. A maximum
candidate therefore costs 16,384 bytes. Without the new credit, the existing maximum
64 pending plus 256 terminal records could retain
`320 * 16,384 = 5,242,880` provisioning bytes including structural allowance. The
selected credit instead permits at most 32 maximum candidates
(`524,288 / 16,384`) to be retained
across pending and terminal records; smaller records may occupy more record slots, but
the existing 64/256 cardinalities and the 524,288-byte credit both apply. The exact
worst-case incremental retained process memory contributed by M16 provisioning
payloads is **524,288 bytes**.

Admission reserves the credit before retaining the record or advancing mutation
sequencing. Insufficient credit returns exact `busy` / `capacity_exhausted`, retryable
true, resync false, and creates no operation/staged candidate. Credit is released when
a rejected pre-admission reservation unwinds, a scope is removed, a pending record is
terminally removed, a terminal record expires/is evicted at the existing 600-second
bound, or Runtime shuts down. Future M16.4/M16.6 tests must cover one-byte-under,
exact-credit, one-byte-over, 32 maximal records, mixed-size records, expiry/scope
cleanup, and exact same/different-payload dedup comparisons.

## 14. Persistence semantics

**M16.1 DECISION**

An API-provisioned candidate is a process-local active configuration overlay. Once
applied, it survives client/Workbench disconnect or crash but **does not survive
Runtime restart**. Runtime never rewrites `runtime.toml`, creates a hidden deployment
file, or treats a client path as authority. Startup returns to the operator-managed
persistent deployment source, including any file-backed `kind="simple_device"`
instruments and their frozen definition artifacts.

An unclean Runtime restart after a process-local output device existed does not
establish that device's physical output as safe. Because the API overlay is not
persistent, that device is outside Runtime semantic authority until it is explicitly
provisioned again; no old safe/readback evidence is restored across boot. This does
not authorize replay of an old mutation or output.

There is no v1 persist/export operation. Durable installation remains an explicit
operator-managed deployment update outside the Application API. A client may
re-provision after a new boot as a new explicit action; no old mutation is replayed.

This preserves:

```text
persistent deployment configuration
    != current Runtime state
    != experiment procedure
```

Recorder provenance must nevertheless describe the exact process-local active
candidate while it is active.

## 15. Provenance model

**FUTURE IMPLEMENTATION REQUIREMENT**

For each activation, Recorder provenance captures before recording admission:

- exact canonical definition bytes and SHA-256;
- definition format version, stable ID, and author version;
- exact canonical instance mapping and its hash;
- stable InstrumentId/ParameterIds/logical keys (not display names as identity);
- ResourceId, binding generation, mapping revision, and active configuration revision;
- compiled grammar/version and selected checksum/scalar algorithms;
- safe profile and optional controller/reference binding for an actuator;
- whether the source was startup deployment or process-local Application candidate;
- existing Runtime package/binary and deployment/startup provenance.

The existing Recorder content-addressed model already bounds entries to 128, each
content entry to 64 KiB, activation objects to 256, object descriptors to 4 KiB, and
aggregate entry memory credit to 1 MiB. M16 canonical definition content is at most
6,144 bytes. One definition produces one content entry, not one copy per measurement.

Each measurement/output fact continues to carry the ordinary signal/actuator identity,
binding generation, and mapping revision. Mutable display labels and raw mutable client
objects never become scientific identity.

## 16. Generation and stale-completion fencing

**M16.1 DECISION**

Existing domains are sufficient:

- ResourceExecutor transport generation identifies recovery/session replacement.
- `binding_generation` identifies physical resource binding replacement.
- `mapping_revision` is the live per-instrument mapping/correlation fence. It covers
  definition/address/codec identity and may also advance during resource rebind even
  when those declarative bytes are unchanged.
- deployment configuration revision fences staging/apply.
- authority epoch/lease/DispatchId fences output ownership and settlement.

No separate declarative-definition generation is added. The exact definition hash is
provenance and validation identity; `mapping_revision` is the live completion fence,
not merely a definition-version counter.

Every queued poll, ACK, and readback retains binding generation and mapping revision.
Commit/settlement compares them with the current instance. Therefore an old response
after reconnect, old response after definition replacement, old ACK after actuator
remap, old readback after mapping change, and old scheduled poll after instance
replacement cannot commit into the replacement. Checked generation/revision exhaustion
rejects the change rather than wrapping.

Although v1 API replacement is restart-required, these fences remain mandatory for
resource reconnect and later deployment-source replacement.

## 17. Bounds

The table freezes proposed M16 limits. Existing shared limits are identified rather
than duplicated into a new pool.

| Retained structure/value | Owner | Capacity | Overflow/rejection | Lifetime and cleanup |
|---|---|---:|---|---|
| active simple definitions | ServiceHost configuration | 16 | `invalid_configuration` / `invalid_configuration` | Runtime process; replaced only by supported config lifecycle |
| persistent raw definition artifact | frozen deployment | 8,192 bytes | invalid deployment/candidate before activation | frozen once at load/stage; deployment candidate/active lifetime |
| definition canonical bytes | frozen deployment or API candidate/active provenance | 6,144 | persistent invalid deployment or API `invalid_args` before compile | deployment candidate/active baseline, or API candidate/overlay until restart |
| unit ID / symbol | Core Unit | 32 / 16 UTF-8 bytes | `invalid_configuration` during semantic validation | descriptor lifetime |
| parameters per definition | compiled definition | 16 | `invalid_args` / `invalid_request` | definition lifetime |
| integer engineering range | compiled parameter/Core Value domain | integral JSON bounds fitting `i64`, `min <= max` | semantic excess/nonintegral bound: `invalid_configuration` / same category | descriptor lifetime |
| float engineering range | compiled parameter/Core Value domain | finite `f64`, `min < max` | invalid/nonfinite range: `invalid_configuration` / same category | descriptor lifetime |
| actuator engineering domain | existing OutputAuthority | `value_type = float` only; raw encoding may be integer or `f32` | integer actuator: `invalid_configuration` / same category before stage | actuator/authority lifetime |
| scalar encoding/transform per parameter | compiled definition | exactly 1 (`raw`, finite nonzero `scale`, finite `offset`) | missing/duplicate/unsupported/invalid: `invalid_args` / `invalid_request` | definition lifetime; shared by read/write/readback plans |
| active simple instances | Runtime/Host | 32, still inside global 64 instruments | `invalid_configuration` / `invalid_configuration` | Runtime process |
| instances per API candidate | staged candidate | 4 read-only or 1 output-capable | `invalid_args` / `invalid_request` | <=30 s staged, then active or freed |
| output-capable instances/safe profiles/controllers per candidate | staged/prepared candidate | 1 / 1 / 1 | excess cardinality: `invalid_args` / `invalid_request` | candidate/prepared/active lifetime |
| safe-profile/controller semantic graph | staged candidate | one fully cross-validated graph | invalid role/unit/range/reference/safety: `invalid_configuration` / same category | candidate/prepared/active lifetime |
| active configured controllers | Runtime configuration/Core | 8 configuration, 64 Core | `invalid_configuration` / `invalid_configuration` before prepare | Runtime process |
| transaction plans per parameter | compiled definition | 3 (`read`, `write+ack`, `readback`) | `invalid_args` / `invalid_request` | definition lifetime |
| total plans per definition | compiled definition | 18 (15 read + 3 for the one actuator) | `invalid_args` / `invalid_request` | definition lifetime |
| request bytes | compiled transaction | 64 | `invalid_args` / `invalid_request` before activation | immutable plan / one queued copy |
| exact response bytes | executor active response | 64 | schema excess: `invalid_args`; runtime short/malformed reply is a transport/protocol failure | one transaction; latest response replaces |
| request segments | compiled request | 8 | `invalid_args` / `invalid_request` | definition lifetime |
| literal bytes per segment | compiled request | 32 | `invalid_args` / `invalid_request` | definition lifetime |
| instance/value field inserts | compiled request | 4 plus one value maximum | `invalid_args` / `invalid_request` | definition lifetime |
| response match fields | compiled response | 8, only `literal_match` or `instance_match` | `invalid_args` / `invalid_request`; insufficient shared-resource correlation is `invalid_configuration` / same category | definition/compiled-instance lifetime |
| bytes per match field | compiled response | 16 | `invalid_args` / `invalid_request` | definition lifetime |
| extract fields | compiled response | 1 | `invalid_args` / `invalid_request` | definition lifetime |
| strict ACK semantic matches | compiled ACK response | at least 1 literal/instance match; checksum does not count | `invalid_configuration` / same category | compiled definition/instance lifetime |
| aggregate plan lexical contribution | candidate schema | 592 values/members for four-read-only variant; 598 for one-output variant | `invalid_args` / `invalid_request` | decode/compile only |
| delimiter bytes/scans | none | 0 | `invalid_args` / `invalid_request` | not retained |
| checksum fields | compiled request/response | 1 each | `invalid_args` / `invalid_request` | definition lifetime |
| checksum input | transaction bytes | <=62 | `invalid_args` / `invalid_request` | transaction only |
| ASCII numeric bytes | none | 0 | `invalid_args` / `invalid_request` | not retained |
| device address/channel | instance | u16 each; encoded width 1 or 2 | `invalid_configuration` / same category on width mismatch | instance lifetime |
| queued physical transactions | existing ResourceExecutor | 32/resource plus one reserved safe slot | local bounded `Busy`; provisioning maps exhaustion to `busy` / `capacity_exhausted` | terminal/fenced/recovery/shutdown |
| COM worker requests/completions | configured COM worker | one slot each | WouldBlock/bounded failure | one device call |
| read retry | executor transaction | one retry maximum | terminal unavailable after recovery/deadline | original queue deadline |
| poll cadence | instance scheduler | 10..=60,000 ms | `invalid_args` / `invalid_request` | active instance; rescheduled from commit |
| recent-history capacity per signal | Core SignalBuffer via instance | 1..=1,024 (inside Core maximum 4,096) | `invalid_args` / `invalid_request` | active instance; oldest sample evicted locally |
| queue timeout | simple instance/transaction | 1..=2,000 ms | `invalid_args`; admitted work expires before send | transaction |
| execution timeout | simple instance/transaction | 1..=2,000 ms | `invalid_args`; finite transport failure | starts at first execution poll |
| pending output intent | existing OutputAuthority | one/actuator | Busy/reject | settled/revoked/expired |
| ACK state | Runtime output correlation | one/in-flight actuator | cannot admit a second dispatch | ACK/failure/recovery |
| readback state | Runtime output correlation | one/in-flight actuator | fail closed | readback/failure/recovery |
| retained written raw scalar | Runtime output correlation | one encoded scalar, <=4 bytes, per in-flight actuator | second dispatch rejected; malformed/nonmatching readback fails closed | dispatch through ACK/readback settlement or fencing |
| pending simple configuration apply | ServiceHost/Runtime owner + SessionStore correlation | 1 process-wide, 15 s absolute | `busy` / `capacity_exhausted`; exact retry returns retained evidence | Accepted until terminal; commit, pre-send retirement, or post-send quarantine transition |
| output-apply `configuration_quiesced` phase | ServiceHost/Runtime owner | one boolean phase inside the one pending output apply | no second apply; no new acquisition/control production while set | entered only after controller pause/lease revoke/active-output safe barrier; cleared on commit/failure/quarantine, then schedules resume from current time |
| prepared simple topology | serialized Runtime/ServiceHost apply owner | 1, non-discoverable, inside same 15 s deadline | second apply `busy`; phase-specific retirement/quarantine | apply only; atomic commit, pre-send drop, or move to quarantine |
| quarantined provisional output | Runtime-owned OutputAuthority reconciliation state | 1 process-wide, non-discoverable | blocks output provisioning on its resource; no auto resend | no time expiry; explicit successful resource reconnect or Runtime restart retires software state |
| complete API candidate JSON | wire request/staged candidate | 8,192 bytes | `invalid_args`; shared codec may reject before operation | request, then one staged candidate |
| candidate/whole-envelope lexical values | shared codec + M16 schema | exact v1 maxima 900 / 909 | `invalid_args` for M16 schema excess; `json_values` before mapping if shared limit is crossed | decode only; typed candidate afterward |
| candidate/whole-envelope JSON depth | shared codec + M16 schema | 8 / 10 | `invalid_args` for schema excess; `json_depth` before mapping if shared limit is crossed | decode only |
| maximal canonical compact provisioning request | production encoder/shared Application codec | 8,496 bytes | raw incoming body still independently capped at 16,383 | one request body |
| retained provisioning-payload credit | SessionStore | 524,288 bytes process-wide; per record `align64(candidate)+8,192` | `busy` / `capacity_exhausted` before admission | pending, then <=600 s terminal; scope/eviction/shutdown cleanup |
| candidate chunks | none | 0 | unsupported | none |
| staged candidates | existing DeploymentLifecycle | 1 Runtime-wide | occupied: `busy` / `capacity_exhausted` | 30 s or expiry; consumed when simple apply preparation begins |
| candidate definition/instances | staged candidate | 1 / 4 read-only or 1 output | `invalid_args` / `invalid_request` | staged/active process lifetime |
| property overlays | existing deployment | 32 | configuration capacity | process lifecycle |
| provenance entries/content | Recorder | 128 / 64 KiB each, 1 MiB aggregate credit | `recording_unavailable` / same category; Required fails closed | durable activation content |
| provenance objects/descriptor | Recorder | 256 / 4 KiB each | `recording_unavailable` / same category | durable activation content |
| diagnostic/public text | config/public error | 256 UTF-8 bytes, character-safe truncation | truncate with visible marker; never raw payload echo | latest/one envelope, no history |

The global existing bounds of 8 resources, 64 instruments, 4,096 recent samples per
signal, 256 Core recording facts/256 KiB, and Application 16,383-byte JSON body remain
authoritative. M16 introduces no unbounded map, list, parser, response buffer, retry
history, or candidate builder.

## 18. Deterministic failure matrix

Every future test must assert the authoritative state after failure, not just an error.

| Case | Expected authoritative state |
|---|---|
| unknown field / bad schema version / malformed payload | no staged candidate, active deployment unchanged |
| candidate/definition/request/response oversize | exact `invalid_args`/`invalid_request`; rejected before compile/activation; no allocation beyond admission bound |
| maximum legal candidate wrapped in maximum mutation envelope | production canonical encoder/codec accepts body <=8,496, candidate/whole lexical counts <=900/909, depths <=8/10, units <=32/16 and all other strings at receiving-domain bounds; the shared 1,024 limit remains unbreached |
| persistent definition raw/canonical exact and one byte over | 8,192/6,144 accepted; excess deployment candidate rejected before active composition; frozen artifact bytes never reread |
| candidate exceeds 900 lexical values while below byte limit | exact `invalid_args`/`invalid_request`; no SessionStore admission or stage |
| too many parameters/plans/instances | no staged candidate |
| duplicate definition/parameter/instrument/key IDs | no staged candidate |
| invalid unit/range/access/role/write-effect combination | no staged candidate |
| integer observation range bound is fractional, outside `i64`, or has `min > max` | exact nonretryable `invalid_configuration`; no staged candidate |
| float observation/actuator range is nonfinite or has `min >= max` | exact nonretryable `invalid_configuration`; no staged candidate |
| actuator declares `value_type = integer` | exact `invalid_configuration`/same category, retryable false, resync false; no stage, prepared topology, or integer output authority |
| unit ID 33 bytes or symbol 17 bytes | semantic rejection before Core descriptor construction; no widened pre-Core unit domain |
| output without valid safe profile | no staged candidate and no actuator published |
| `required_evidence=readback` without independent readback plan | exact `invalid_configuration`; no staged candidate |
| duplicate/colliding controller ID/key or more than one candidate controller | no staged candidate; active controller capacity unchanged |
| controller input/reference unit mismatch, non-measurement input, non-actuator output, or PID/safe timing/range violation | no staged candidate and no controller/output published |
| output candidate bound to `windows_com_read_only` | exact nonretryable `invalid_configuration`; no prepared topology/output authority created |
| candidate mixes simple and native/Metakon adapters on one resource | exact nonretryable `invalid_configuration`; active composition unchanged |
| missing/duplicate parameter `encoding`, unsupported raw scalar, or transaction-local encoding/transform | exact `invalid_args`/`invalid_request`; no staged candidate |
| zero/nonfinite scale, nonfinite offset, or caller-supplied readback tolerance | exact `invalid_args`/`invalid_request`; no staged candidate |
| invalid/overlapping offsets or request construction | no staged candidate |
| malformed literal/instance match tag, instance encoding, extract tag, or checksum tag | exact `invalid_args`/`invalid_request`; no staged candidate |
| shared-resource response lacks sufficient instance correlation | exact nonretryable `invalid_configuration`; no staged candidate or active composition |
| ACK has no semantic match or has checksum only | exact nonretryable `invalid_configuration`; no staged candidate |
| checksum mismatch / wrong literal or instance match / wrong exact length | no sample/ACK; unavailable or ambiguous by send stage; transport recovery required |
| truncated response | finite timeout; no value fabricated |
| residual/oversized response | protocol failure/recovery; no value fabricated |
| invalid float/nonfinite/integer conversion/numeric overflow | unavailable read or pre-wire write rejection |
| decoded engineering value outside range | unavailable observation; no out-of-range good sample |
| write value outside range/unrepresentable | rejected before send; zero bytes; authority fail-closed |
| ResourceExecutor queue saturation | bounded local Busy; provisioning prepare returns exact `busy`/`capacity_exhausted`; no hidden queue/history |
| read timeout before/after request send | unavailable sample; only accepted one read retry after clean recovery and original deadline |
| write timeout before first byte | pre-wire failure, no ambiguous send evidence |
| partial write/timeout/disconnect after send_started | exact Ambiguous/transport-uncertain evidence; no retry |
| bad ACK / ACK timeout | no acknowledged/readback claim; ambiguous after send_started; safe obligation retained |
| malformed/timeout readback | ACK may remain, readback not verified, controller fails closed |
| integer readback raw mismatch | terminal positive mismatch failure, never physical success or ambiguity |
| finite `f32` readback differs in normalized raw bits | terminal positive mismatch failure; `-0.0` and `+0.0` both normalize to positive zero, while NaN/infinity is malformed |
| non-safe readback permits distinct authority-governed safe action but deadline ends without required evidence | no publication; one quarantined provisional authority remains, with no blind resend |
| 2+ read-only simple instances share one resource and reconnect succeeds | one expected resource generation checked; all old work fenced; binding/mapping fences advance for every instance; every signal reacquires generically |
| output-capable simple resource reconnect | safe evidence re-established before release; controller remains non-running and is not automatically rearmed |
| stale old-generation completion | cannot commit observation, ACK, or readback for replacement |
| definition/mapping replacement race | old mapping revision cannot settle new mapping |
| stale configuration revision | exact `revision_conflict`/same category, resync required; no stage/apply mutation |
| occupied candidate slot | exact retryable `busy`/`capacity_exhausted`; existing candidate unchanged |
| SessionStore retained provisioning credit exact/one-byte-over | exact-credit admission succeeds; over-credit returns retryable `busy` before record/high-water/stage mutation |
| terminal provisioning retention expires or scope is removed | exact payload credit released; full typed equality existed for its entire retained lifetime |
| candidate expiry/unknown apply | exact `revision_conflict`, resync required; candidate absent, active deployment unchanged |
| active API device then file stage/reload | exact nonretryable `invalid_configuration` before slot occupation; device/revision unchanged; later simple candidate can take slot |
| property override with active API device | clone contains the overlay; supported property changes without erasing device |
| unclean Runtime restart after API output activation | process-local device absent and `runtime.toml` baseline restored; old physical output is not declared safe, old safe/readback evidence is not restored, and no old mutation/output is replayed |
| simple apply admitted | Accepted reply deliverable immediately; one pending owner correlation exists; no synchronous 15-second handler wait |
| read-only apply pending | unrelated ordinary acquisition/control continues; prepared entities remain invisible until atomic commit |
| output apply pending before global safety boundary | ordinary acquisition/control and all owner services continue under the unchanged absolute deadline |
| output apply pending after `configuration_quiesced = true` | no new acquisition scheduling or controller production; safety, already-admitted transport, Recorder, queries, `operation_status`, client/network scheduling, and delivery of already-produced events continue |
| output apply commit/failure/quarantine transition | quiesce clears; acquisition resumes from current time without catch-up; controllers remain safe/Ready-or-Paused/Failed and are not automatically rearmed |
| originating client disconnects during Accepted apply | pending Runtime-owned work continues to one terminal outcome; disconnect creates no cancellation |
| exact retry of pending/terminal apply identity | retained Accepted/Completed/Failed returned; no second pending/prepared lifecycle |
| second pending/prepared apply | exact retryable `busy`; one bounded pending/prepared owner remains |
| provisional resource/transport unavailable before `send_started` | exact retryable `transport_unavailable`; consumed candidate/prepared topology retired, prior active deployment remains |
| any provisional failure before `send_started` | apply terminal Failed; candidate consumed and not restored; zero ambiguity; a new stage plus new apply identity is required |
| provisional output reaches `send_started`, then ACK/readback/deadline failure | apply terminal Failed; non-discoverable authority/correlation moves to one quarantine; no automatic resend or time expiry |
| `configuration_status` during provisional quarantine | bounded IDs/generations/evidence/safe-resend-blocked projection visible; no raw protocol bytes |
| another output apply while provisional quarantine exists | exact retryable `busy`/`capacity_exhausted`; quarantine unchanged and its resource remains blocked |
| explicit reconnect of quarantined resource | old worker/bytes and dispatch generation fenced; quarantine retired only after fresh binding; failed candidate remains unpublished; new stage required |
| Runtime restart with provisional quarantine | process-local state gone but no old-safe claim; new-boot safe/unverified policy remains authoritative |
| provisional completion with wrong candidate/binding/mapping identity | cannot settle prepared or active authority; no publication |
| successful provisional safe evidence | one serialized atomic commit; descriptors/authority/controller/schedules appear together, controller Ready without lease |
| safe barrier/pre-publication safe-evidence failure | exact nonretryable `output_rejected`/`invalid_configuration`; no commit/rearm; active authority stays safe |
| Required Recorder reservation/commit failure | exact retryable `recording_unavailable`; pre-send retires without ambiguity, post-safe-evidence retires verified provisional state; no activation publication |
| serialized owner/prepared-state internal failure | exact nonretryable `operation_failed`; before send state retires, after send state quarantines; no success or partial topology |
| Workbench/client death during staging/apply | Runtime-owned accepted lifecycle continues independently; no Runtime/controller/Recorder shutdown |

Executable acceptance must include exact zero-byte/partial-byte/ACK boundaries, queue
and deadline boundary values, generation exhaustion, Recorder memory/disk provenance,
and prior deployment equality after failed apply.

## 19. Worked conceptual furnace example

The following is data-shape evidence, not a shipping schema example.

```text
definition
  format_version: 1
  definition_id: simple-furnace-v1
  definition_version: 1

  parameters:
    1 temperature
      role: measurement; access: read_only; unit: degC
      engineering: float -50..500
      encoding { raw: i16_be, scale: 0.1, offset: 0.0 }
      READ request.segments:
        { type: literal, hex: "10" }
        { type: instance_field, field: address, encoding: u8 }
        { type: instance_field, field: channel, encoding: u8 }
        { type: checksum, algorithm: crc16_modbus }
      response:
        exact_length: 8
        matches:
          { type: literal_match, offset: 0, hex: "10" }
          { type: instance_match, offset: 1, field: address, encoding: u8 }
          { type: instance_match, offset: 2, field: channel, encoding: u8 }
          { type: literal_match, offset: 5, hex: "00" }
        extract: { type: scalar_extract, offset: 3 }
        checksum: { type: checksum, offset: 6, algorithm: crc16_modbus }

    2 heater_power
      role: actuator; access: read_write; unit: percent
      engineering: float 0..100; write_effect: output_affecting
      encoding { raw: u16_be, scale: 0.1, offset: 0.0 }
      WRITE request.segments:
        { type: literal, hex: "20" }
        { type: instance_field, field: address, encoding: u8 }
        { type: instance_field, field: channel, encoding: u8 }
        { type: value_field }
        { type: checksum, algorithm: crc16_modbus }
      ACK:
        exact_length: 6
        matches:
          { type: literal_match, offset: 0, hex: "20" }
          { type: instance_match, offset: 1, field: address, encoding: u8 }
          { type: instance_match, offset: 2, field: channel, encoding: u8 }
          { type: literal_match, offset: 3, hex: "00" }
        checksum: { type: checksum, offset: 4, algorithm: crc16_modbus }
        extract: absent
      READBACK request.segments:
        { type: literal, hex: "21" }
        { type: instance_field, field: address, encoding: u8 }
        { type: instance_field, field: channel, encoding: u8 }
        { type: checksum, algorithm: crc16_modbus }
      READBACK response:
        exact_length: 8
        matches:
          { type: literal_match, offset: 0, hex: "21" }
          { type: instance_match, offset: 1, field: address, encoding: u8 }
          { type: instance_match, offset: 2, field: channel, encoding: u8 }
          { type: literal_match, offset: 5, hex: "00" }
        extract: { type: scalar_extract, offset: 3 }
        checksum: { type: checksum, offset: 6, algorithm: crc16_modbus }
      verification: exact normalized raw u16 equality with retained written raw value

instance
  instrument_id: 1001
  key/display: furnace-1 / Furnace 1
  exact definition identity above
  resource_id: pre-existing COM resource 7
  address/channel: 1 / 0
  poll: 250 ms; queue timeout: 250 ms; transaction timeout: 500 ms
  history capacity: 1024
  safe profile for actuator 2:
    min 0; max 100; safe_value 0
    max_lease_ms 2000; max_proposal_ttl_ms 200
    required_evidence readback
  optional native PID controller:
    id 2001; key furnace-1-pid
    input 1001:1; output 1001:2; existing Reference 10
    period_ms 200
    ema_time_constant_ms 1000; ema_warmup_samples 4
    kp 3.0; ki 0.4; kd 0.2; output_min 0; output_max 100
    max_input_age_ms 750; max_tick_gap_ms 750
    lease_lifetime_ms 2000; proposal_ttl_ms 200
    provisional Created -> published Ready, never armed by apply
```

Flow:

```text
stage_simple_device_candidate
  -> validate/compile/hash/classify
  -> apply_configuration accepted immediately under retained request identity
  -> later owner turns advance non-discoverable Runtime-owned prepared topology
  -> existing OutputAuthority safe dispatch -> ACK -> required readback
  -> atomic publication, pre-send retirement, or post-send reconciliation quarantine
  -> ordinary discover/describe records
  -> periodic temperature Sample
  -> measurements/event -> Workbench plot
  -> recent SignalBuffer + Recorder/durable history
  -> ordinary controller input
  -> OutputAuthority proposal and final send fence
  -> declarative bytes -> strict ACK -> separate readback
  -> resource reconnect advances binding/mapping fences
```

No Arduino, furnace, simple-device, or declarative branch appears in generic
Application projections, Workbench, controller algorithms, signal history, or Recorder
measurement storage.

## 20. Future external client boundary

A future external client uses semantic Application operations and identities:

```text
future Clojure client -> Application API -> Reference/controller/Recorder/signals
```

It does not know COM port, baud, address bytes, framing, CRC, register offsets, scaling,
SQLite path, leases, or OutputAuthority internals. Provisioning is an explicit setup
action; experiment procedure is not a serial driver. No Clojure or scripting code is
implemented or authorized here. M13.2 Steel remains unrelated and blocked.

## 21. Required architecture answers

1. **Protocol subset:** exact fixed binary request/reply, 64 bytes, one scalar,
   bounded literal/instance/value/checksum request segments, literal/instance response
   matches, and one parameter-level binary integer/f32 encoding/transform shared by
   read, write, and readback. Measurements/diagnostics may be integer or float;
   actuators remain float-only in the existing OutputAuthority domain even when their
   raw encoding is integer.
2. **Framing:** fixed-length only; delimiter deferred.
3. **Format/schema owner:** one strict definition JSON schema owned by `lab-runtime`,
   used both as a frozen deployment-relative artifact and embedded in the sole
   Application candidate shape; format version 1; no Core file parsing.
4. **Identity:** definition ID + author version + normalized SHA-256; numeric stable
   InstrumentId/ParameterId for instances/entities; names are labels.
5. **Normalized representation:** immutable bounded plan enums with resolved offsets,
   lengths, compiled instance comparisons, raw scalar/checksum algorithms, finite
   parameter transforms, and validated cross references.
6. **Codec execution:** trusted Runtime side, with no callback/script/transport owner;
   schema parsing stays in application configuration code.
7. **Transport bound:** protocol-neutral executor/COM maximum becomes 64; Metakon
   retains 38.
8. **Read path:** scheduled compiled request -> executor -> strict decode -> generation
   recheck -> ordinary Signal commit.
9. **Write path:** ordinary OutputAuthority -> prepared request -> final fence -> send
   -> strict semantic-match ACK -> optional independent readback -> exact normalized
   raw equality with the retained written raw scalar.
10. **Final send fence:** immediately before the first positive byte admission, validate
    current intent, lease/epoch, deadline, Recorder gate, binding generation, and mapping
    revision; record DispatchId when the first write is admitted.
11. **Resource provisioning:** active pre-existing configured COM resource only;
    read-only accepts `windows_com_read_only`/`windows_com`, output requires
    `windows_com`; v1 forbids sharing with Metakon/native adapters.
12. **Output prerequisite:** the complete six-field same-candidate safe profile is
    mandatory; its evidence mode cross-validates ACK/readback. At most one complete
    native PID controller may bind an existing Reference; no controller means a
    safe/disarmed float actuator only. No integer or alternate output authority is
    introduced.
13. **Lifecycle:** simple apply becomes Accepted immediately, then one incremental
    Runtime-owned pending/prepared lifecycle advances over owner turns under a
    15-second absolute deadline. Pre-send failure consumes/drops; post-send ambiguity
    retains one non-discoverable authority quarantine. Read-only preparation never
    globally quiesces acquisition/control. Output preparation allows ordinary progress
    before its safety boundary, then pauses controllers, revokes leases, establishes
    the active-output safe barrier, and quiesces new acquisition/control production
    while safety/transport/Recorder/Application progress continues. Every terminal
    transition clears quiesce, resumes scheduling from current time without catch-up,
    and never rearms controllers. Cadence is a live property; definition, address,
    binding, COM, removal, and replacement are restart-required. File stage/reload is
    rejected while an API overlay is active; property override clones that overlay.
14. **Reconnect fencing:** one resource-scoped reconnect advances binding and live
    mapping/correlation fences for every simple instance on the resource; old work is
    fenced, Good samples become unavailable pending reacquisition, safe evidence is
    re-established, and controllers are never automatically restarted.
15. **Provenance:** canonical definition/instance bytes+hash, stable IDs, binding/mapping,
    config revision, safety/control binding, Runtime/deployment identity.
16. **Restart persistence:** file-backed `kind="simple_device"` is the persistent
    baseline; API overlay is process-local only, never rewrites TOML, and is absent
    after restart, which restores that baseline.
17. **Application surface:** one new mutation `stage_simple_device_candidate`; existing
    `configuration_status`, incremental `apply_configuration`, `operation_status`, and
    resource-scoped `reconnect_resource`; one capability `simple_device_provisioning`.
18. **Versioning:** Application envelope remains v1; additive operation version 1 and
    semantic API inventory update when implemented.
19. **Bounds:** frozen in section 17, including 8,192 raw artifact/candidate bytes,
    6,144 canonical definition bytes, exact candidate/envelope lexical maxima 900/909,
    524,288 retained provisioning bytes, one pending/prepared apply, and one post-send
    quarantine.
20. **Slices:** frozen below; none is authorized by this report.

## 22. Implementation decomposition (not authorization)

### M16.2 — read-only declarative device vertical slice

- **Goal:** persistent `[[instruments]] kind="simple_device"` startup/composition with
  one frozen <=8,192-byte definition artifact, compiled fixed READ through the existing
  COM executor, and an ordinary Signal/generic discovery/current/event/history/Recorder
  path.
- **Likely production owners:** new bounded `lab-runtime` simple-device schema/compiler
  and host schedule/codec module; minimal protocol-neutral transaction bound/seam in
  Core transport/runtime; configuration composition arm; Recorder provenance assembly.
- **Red oracles:** exact persistent TOML arm, deployment-relative artifact freeze/hash,
  shared compiler/schema/6,144 canonical bound, exact parameter-level encoding,
  exact tagged request/match/extract/checksum grammar, strict fixed reply, literal and
  instance match/checksum/length failures, shared-resource correlation, timeout/retry,
  integer observation bounds against `i64`, float range conversion, a pending read-only
  apply with unrelated acquisition/control continuing, stale generation, generic
  discovery/event/Recorder, and no Workbench special case.
- **Non-goals:** output, API upload/provisioning, delimiter/ASCII, new resources.
- **Gate:** external review before M16.3.

### M16.3 — writable actuator + ACK/readback

- **Goal:** persistent deployment-level safe profile/optional native controller through
  ordinary OutputAuthority to compiled write, final send fence, strict ACK, optional
  independent exact-raw readback, and exact ambiguity evidence.
- **Likely owners:** Core output/physical correlation and transport, Runtime
  simple-device codec, host scheduling/config validation and provenance.
- **Red oracles:** zero/partial/full byte boundaries, revoke/deadline/generation races,
  checksum-only/nonsemantic/bad/timeout ACK, multi-instance ACK correlation,
  malformed readback, exact integer/raw-bit mismatch, signed-zero normalization,
  NaN/infinity rejection, float-only actuator validation with integer raw encoding,
  explicit rejection of an integer actuator, identical normal/safe verification rules,
  pre-quiesce ordinary progress, post-quiesce suspension of new acquisition/control
  with safety/transport/Recorder/Application liveness, terminal no-catch-up resume and
  zero automatic controller rearm, safe obligation, Required Recorder, resource-kind
  validation, and zero blind retry.
- **Non-goals:** public actuator/raw-write operation, provisioning upload.
- **Gate:** external review before M16.4.

### M16.4 — Application provisioning surface

- **Goal:** implement exactly `stage_simple_device_candidate`, capability advertisement,
  incremental explicit apply, one-slot/revision/dedup semantics, process-local overlay,
  prepared topology, and post-send reconciliation quarantine.
- **Likely owners:** protocol registry, sessions typed mutation, configuration API,
  ServiceHost deployment lifecycle, compiler/candidate ownership, public error mapping,
  hello limits, and docs later after M16 consolidation.
- **Red oracles:** 8,192/6,144-byte, exact 900/909 lexical maxima, and depth/body maximum
  envelope, complete controller/safe-profile schema, exact error envelopes, full typed
  dedup/conflict/unknown, 524,288-byte SessionStore credit exact/over/cleanup,
  candidate capacity/expiry/disconnect, file-reload rejection before slot occupation,
  property-overlay preservation, Accepted-with-progress scheduling, disconnect/status/
  exact-retry behavior, prepared/quarantine cardinality and phase boundaries, failed
  apply atomicity, no TOML write, restart absence, no raw bytes/path/COM settings.
- **Non-goals:** chunks, builders, persistence/export, resource creation.
- **Gate:** external review before M16.5.

### M16.5 — generic system integration acceptance

- **Goal:** prove API-provisioned entities automatically participate in Workbench plot,
  current/events/recent history, Recorder/durable history, controller input/output, and
  resource reconnect.
- **Likely owners:** tests around existing Application, Workbench rebuild/model, Recorder,
  controllers and reconnect; production fixes only for concrete generic-path defects.
- **Red oracles:** source/symbol scan plus end-to-end ordinary identities, one aggregate
  Workbench subscription, live plot points, durable rows, controller operation, two or
  more read-only instances on one resource reconnecting together, output safe recovery
  without controller restart, and no declarative branch in generic consumers.
- **Non-goals:** special Simple Device GUI/editor.
- **Gate:** external review before M16.6.

### M16.6 — faults, bounds, provenance, recovery acceptance

- **Goal:** execute section 18 and every section 17 boundary; verify exact memory/disk
  provenance and prior-deployment preservation.
- **Likely owners:** test-only scripted ByteTransport/COM peers, configuration/API
  harnesses, Recorder SQLite acceptance, reconnect and process restart tests.
- **Red oracles:** malformed grammar, conversions, capacity/deadline, ambiguity, rebind,
  stale mapping, prepared-state capacity/identity/deadline, safe barrier, Required
  Recorder, retained-payload credit, client death, Runtime restart.
- **Non-goals:** new policy or expanded grammar to make a test pass.
- **Gate:** external review before M16.7.

### M16.7 — minimal unknown-device acceptance

- **Goal:** provision a deterministic non-Metakon device with READ temperature, WRITE
  power, ACK, and READBACK through the Application API, then demonstrate full ordinary
  semantic paths without device-specific Runtime/Application/Workbench code.
- **Likely owners:** acceptance fixture/data and scripted physical peer; production only
  if a frozen generic seam is defective.
- **Red oracles:** exact candidate, rediscovery, plot/history/Recorder/controller,
  ACK/readback, reconnect fencing, restart nonpersistence, and no raw public authority.
- **Non-goals:** real Arduino/COM hardware or an Arduino driver.
- **Gate:** external review, then M16 consolidated external review.

### M16 consolidated review

Re-audit architecture, API, schema grammar, bounds, authority, provenance, generic
integration, fault evidence, and absence of raw/public protocol authority before any
real Arduino milestone or deferred M15 documentation resumes.

## 23. Risks retained for implementation review

These are implementation risks, not unresolved architecture decisions:

1. Current Core/Host physical coordination is named and shaped around Metakon. The
   later seam extraction must not duplicate output authority or create a second mutable
   experiment owner.
2. Current live apply assumes fixed topology and synchronous Application dispatch. The
   frozen incremental pending/prepared owner, 15-second deadline, atomic move, and
   post-send authority quarantine require careful implementation and red-oracle tests.
3. Raising the executor/COM byte bound from 38 to 64 affects memory and COM tests;
   Metakon must retain exact 38-byte codec validation.
4. Residual COM bytes are not presently detected as a complete extra frame by the
   exact-length worker. M16 tests must prove the chosen protocol-recovery boundary and
   must not report success from contaminated input.
5. Provenance activation currently uses bounded shared entry/object credits. Maximum
   simple-definition/instance composition must be tested against those existing
   credits, especially with 32 active instances.

None requires a new policy decision, dependency, scripting language, raw API, or schema
change during M16.1.

## 24. Non-goals and authorization boundary

M16.1 does not implement or authorize M16.2–M16.7. It does not modify Application
operations, configuration schemas, Core, Runtime, Workbench, Recorder, Cargo, tests, or
public documentation. It adds no scripting/automation runtime, no language selection,
no device editor, no raw write, no Discard/Forget, and no Steel work.

Final review state requested by this audit:

```text
M15.1–M15.4: ACCEPTED

M15.5–M15.8:
DEFERRED UNTIL M16 CONSOLIDATED ACCEPTANCE
NOT AUTHORIZED

M16.1 declarative simple-device architecture/API audit:
ACCEPTED

M16.2–M16.7:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M16_1_DECLARATIVE_DEVICE_AUDIT_ACCEPTED
```
