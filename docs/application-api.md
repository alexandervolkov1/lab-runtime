# Application API reference

## Protocol

The current adapter is loopback TCP with newline-delimited JSON (NDJSON): one UTF-8
JSON object followed by LF. TCP supplies transport only; the Application API defines
the domain semantics.

| Field | Value / rule |
|---|---|
| protocol ID | `lab-runtime.application` |
| protocol version | `1` |
| Application API version | `0.1-pre` |
| request envelope | `v`, `msg_id`, `op`, `args`, and `request_id` for mutations |
| query response | `{"v":1,"msg_id":"...","type":"result","result":...}` |
| mutation response | one `type:"operation"` accepted state, then completed or failed |
| ordinary error | `type:"error"`, `accepted:false`, plus the fields described below |

Requests use strict field allowlists and reject duplicate JSON keys. IDs and u64
counters are canonical decimal strings on the wire unless a field is explicitly a
JSON number. `msg_id` correlates one connection exchange; it is not a mutation
deduplication key.

The first request on a connection must be:

```json
{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}
```

`hello` returns a process boot ID, a server-issued scope, `next_seq`, current
operations/capabilities, event cursors, and all advertised limits. Supplying a
retained scope attempts process-local reattachment.

## Mutations and deduplication

A mutation adds:

```json
"request_id":{"scope":"<hello scope>","seq":"1"}
```

Sequence numbers are consecutive within the scope. The Runtime admits an immutable
typed mutation before execution, replies with `state:"accepted"`, and later stores a
bounded terminal `completed` result or `failed` public error. Repeating the same
`(scope, seq)` and same typed mutation replays the retained outcome rather than
executing it twice. Reusing the ID for different work is rejected. This guarantee is
process-local and retention-bounded; it is not cross-process exactly-once delivery.

Disconnecting a client does not roll back an admitted mutation. A reply can be lost,
so a reattached client should use `operation_status` where the outcome remains
retained.

## Operations

The authoritative registry contains exactly 42 operations: 22 queries and 20
mutations. Feature-gated operations are advertised only when the active composition
supports them. In the tables, `id` means a canonical decimal string.

### Session, discovery, and projections

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `hello` | query | `scope`: null or retained string | Boot/protocol/Application identity, scope/next sequence, supported operations, capabilities, bounds, oldest/latest event cursor. | `already_hello`, `scope_unknown`, `scope_in_use`, or capacity; required before all other operations. |
| `discover` | query | none | First frozen page of instrument, signal, component, controller, Reference, output, resource, and configuration records. | `snapshot_capacity`; frozen projection is connection-local. |
| `discovery_page` | query | `projection`, `index` | Next page of the same frozen discovery view. | `snapshot_expired`/`snapshot_capacity`; 5 s, 64 records, 8 KiB/page. |
| `describe` | query | `instrument` | Instrument descriptor: stable ID, name, and complete parameter descriptors. | `unknown_instrument`; one bounded descriptor result. |

Frozen-page results contain `projection`, a boot/event `revision`, `records`,
`next_index`, and `complete`. A later page never silently switches to newer state.

### Resources and configuration

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `resource` | query | `resource` | Physical resource state, availability, generations, bound instruments, revision, and failure. | `unknown_resource`; advertised only with configured reconnect resources. |
| `configuration_status` | query | none | Active/configured state, revision/source, staged candidate, and overlay count. | Feature-gated; reports disabled state rather than exposing a filesystem handle. |
| `configuration_properties` | query | none | First frozen page of generic resource/instrument/component property records. | `configuration_disabled`/`configuration_capacity`; 256 records maximum. |
| `configuration_page` | query | `projection`, `index` | Next property page. | `snapshot_expired`/`snapshot_capacity`; 64 records, 8 KiB. |
| `stage_configuration` | mutation | none | Candidate ID, base revision, expiry, and classified effects. | `invalid_configuration`, conflict, or Required Recorder failure; one candidate for 30 s. |
| `apply_configuration` | mutation | `candidate_id`, `expected_revision` | New committed configuration revision. | `revision_conflict`, invalid/expired candidate, unsafe lifecycle, Recorder/transport unavailable. |
| `reload_configuration` | mutation | none | Reads, validates, stages, applies; returns new revision. | Atomic `invalid_configuration`, conflict, Required Recorder, or transport failure. |
| `property_configure` | mutation | `target:{kind,id}`, `property`, scalar `value`, `expected_revision` | Target/property/new revision; deployment source is not auto-rewritten. | `not_found`, `revision_conflict`, invalid type/range/access, or lifecycle failure; 32 overlays. |
| `reconnect_resource` | mutation | `resource`, `expected_binding_generation` | Resource and new binding generation after retire/open/probe/rebind. | `unknown_resource`, `revision_conflict`, `transport_unavailable`, timeout; never rearms control. |

Property records contain stable owner/property identity, value type, current value,
unit, access, mutation class, constraints, and configuration revision. Unsupported
or unsafe changes are rejected atomically.

### Measurements and history

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `latest` | query | `signal:{instrument,parameter}` | Current attempt or explicit `not_observed`, with identity/value/unit/quality/times/generation. | `unknown_instrument`, `unknown_parameter`, `unknown_signal`; exactly one current record. |
| `measurements_current` | query | none | First frozen page of all current signals. | `snapshot_capacity`; connection-local frozen projection. |
| `measurements_page` | query | `projection`, `index` | Next frozen current page. | `snapshot_expired`/`snapshot_capacity`; 64 records, 8 KiB. |
| `measurement_window` | query | `signal`, `max_records` (1..128) | Oldest-first in-memory Runtime history. | unknown signal or `response_too_large`; not durable history. |
| `history_read` | mutation | See below | Schedules Recorder query; terminal result gives a checkpoint-frozen page token. | Recorder feature only; `history_busy`, unknown archive, cursor conflict/expiry, 2 s job timeout. |
| `history_page` | query | `page_token` | Retained durable page. | `history_page_expired`; 5 s page retention and 8 KiB result. |
| `history_release` | query | `page_token` | `released:true`. | `history_page_expired`; explicitly frees the connection-local page. |

`history_read` supports two strict modes:

```json
{"mode":"runs","database_id":"<32-hex>","max_records":32,"cursor":null}
```

```json
{"mode":"measurements","database_id":"<32-hex>","boot_id":"<32-hex>",
 "run_id":{"boot_id":"<same boot>","run_no":"1"},
 "signal":{"instrument":"1","parameter":"1"},
 "from_ns":"0","to_ns":"1000000000","max_records":128,"cursor":null}
```

Run pages contain run identity, label, policy, state, coverage, and optional next
cursor. Measurement pages contain raw durable rows, watermark, coverage/loss data,
and optional next cursor. A continuation cursor is tied to the scope/filter and
expires after 30 s. At most eight history jobs/pages and eight cursors are retained.

### References, controllers, and output

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `reference` | query | `reference` | Fixed/ramp state, revision, value/target/rate, unit, and last evaluation. | `unknown_reference`; one bounded record. |
| `reference_configure` | mutation | `reference`, `expected_revision`, and either `kind:"fixed",value` or `kind:"ramp",target,rate` | Complete committed Reference projection. | `revision_conflict`, unknown Reference, invalid/nonfinite/rate/configuration. |
| `reference_retune` | mutation | `reference`, `expected_revision`, `target`, `rate` | New ramp revision/value/target/rate and commit time. | `revision_conflict`, wrong Reference kind/state, invalid finite values/rate. |
| `controller` | query | `controller` | Lifecycle, status/failure, revision, bindings, complete policy, last tick/output. | `unknown_controller`; one bounded record. |
| `controller_configure_pid` | mutation | `controller`, `expected_revision`, `pid:{kp,ki,kd,output_min,output_max}` | Updated controller projection. | conflict, invalid state/configuration/range; safe lifecycle required. |
| `controller_configure` | mutation | controller/revision, `pid`, `ema:{time_constant_ns,warmup_samples}`, and four timing fields | Updated full policy projection; bindings/units stay deployment-owned. | conflict, invalid lifecycle/durations/PID/EMA/output range. |
| `controller_start` | mutation | `controller` | Ready -> Warming controller projection. | unknown/invalid state, unavailable input/output, Recorder policy failure. |
| `controller_pause` | mutation | `controller` | Committed safe-transition controller state. | unknown/invalid state or output safe transition failure. |
| `controller_resume` | mutation | `controller` | Paused -> Warming with fresh authority. | invalid state, absent safe evidence, stale/unavailable input/output. |
| `controller_reset_failed` | mutation | `controller` | Failed -> Paused after valid safe evidence. | invalid state or unconfirmed/outstanding safe obligation; never resumes. |
| `output` | query | `actuator:{instrument,parameter}` | Complete read-only authority/evidence snapshot. | `unknown_actuator`; no mutation/raw-write authority. |

The API intentionally has no raw write, `set_output`, or register-write operation.

### Components and virtual instruments

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `component` | query | `component` | Implementation/binding/generation/revision/state, good steps, pending, diagnostics. | unknown component; one bounded record. |
| `emulator_publish` | mutation | `signal`, Good finite `value` or `unavailable`, `expected_generation` | Committed virtual signal/generation/state/time. | invalid/physical target, nonfinite/range/type, or stale generation; one record/request. |
| `virtual_models_restart` | mutation | none | Model count and new generation. | unsupported when absent; configuration/lifecycle/Recorder failure; no controller auto-rearm. |

Virtual publication cannot target physical signals or fabricate resource Ready,
transport completion, ACK, readback, or output-safety evidence.

### Recorder and experiment

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `recording_status` | query | none | Complete policy/archive/run/interval/admission/prefix/coverage/ingress/provenance/failure projection. | Always bounded; reports `unconfigured` rather than inventing a Recorder. |
| `recording_start` | mutation | nonempty `label` (<=128 bytes) | Database/run/interval identity and committed start/provenance flags. | `recording_unavailable`, invalid state, capacity, or durable failure; completes only after commit. |
| `recording_stop` | mutation | `run_id:{boot_id,run_no}` | Drained/sealed/committed flags and run identity. | wrong run is `revision_conflict`; invalid state/storage failure; worker/boot close are separate. |
| `experiment_annotate` | mutation | `name` (1..64 bytes), bounded JSON `data` | Record sequence and `durability:"pending"`. | invalid/oversized data, Recorder unavailable, ingress capacity; admission is not commit. |

See [Recorder and SQLite](recorder-sqlite.md) for lifecycle and durability semantics.

### Operations, events, and process

| Operation | Kind | Request `args` | Result | Important errors / bounds |
|---|---|---|---|---|
| `operation_status` | query | `request_id:{scope,seq}` | `accepted`, `completed`, `failed`, or `outcome_unknown`. | scope must match; outcome retention is bounded to 32/scope, 256 total, 600 s. |
| `subscribe` | query | `after:{boot_id,seq}`, `filter:{kinds,targets}` | Aggregate token and accepted cursor. | `subscription_busy`, instance/future cursor, or `event_gap`; 1/client, 8 kinds, 16 targets. |
| `unsubscribe` | query | `subscription` | `removed:true/false`. | Idempotent false for unknown token; connection-local only. |
| `runtime_shutdown` | mutation | none | Finite truthful cleanup/safety/Recorder/output terminal projection. | duplicate/in-progress shutdown or incomplete cleanup/safe evidence; reply delivery is best effort. |

Subscription kinds are `signal`, `controller`, `reference`, `component`, `output`,
`operation`, `host`, `recorder`, `resource`, and `configuration`. A target is a
typed stable identity. Event records carry boot ID, sequence, kind, target, data,
publication monotonic time, and optional mutation cause. A filtered scan may emit
`subscription_progress` so the applied cursor can advance.

Only one aggregate subscription exists per client. The event ring retains 1,024
records. If a cursor is older than retained replay, the subscription is removed and
the client receives `event_gap` with `resync_required:true` and current oldest/latest
cursors. Recover by reading current/frozen state and subscribing from a valid cursor;
never infer a contiguous sequence across a gap.

## Capabilities

Each advertised capability has `version:1` and `stability:"stable"`. The complete
registry is:

| Capability | Representative operation | Capability | Representative operation |
|---|---|---|---|
| `operation_lifecycle` | `operation_status` | `structured_discovery` | `discover` |
| `live_subscriptions` | `subscribe` | `current_measurements` | `measurements_current` |
| `recent_measurement_history` | `measurement_window` | `instrument_queries` | `describe` |
| `reference_read_write` | `reference_configure` | `controller_status` | `controller` |
| `controller_configuration` | `controller_configure` | `controller_lifecycle` | `controller_start` |
| `managed_components` | `component` | `output_status` | `output` |
| `runtime_shutdown` | `runtime_shutdown` | `recording_status` | `recording_status` |
| `recording_control` | `recording_start` | `measurement_history` | `history_read` |
| `resource_status` | `resource` | `configuration_read` | `configuration_status` |
| `configuration_properties` | `configuration_properties` | `configuration_write` | `property_configure` |
| `deployment_configuration` | `stage_configuration` | `resource_reconnect` | `reconnect_resource` |
| `virtual_instruments` | `discover` | `emulator_publication` | `emulator_publish` |
| `virtual_model_lifecycle` | `virtual_models_restart` |  |  |

Capabilities and operations are filtered to the actual composition. The static
registries contain exactly 25 and 42 entries respectively.

## Public errors

Every mapped error contains stable `code` and broad `category`, a fixed bounded
`message`, `retryable`, and `resync_required`; trusted structured `details` may be
present. The 12 categories are:

| Category | Meaning |
|---|---|
| `invalid_request` | Envelope, IDs, arguments, sequence, or strict schema are invalid. |
| `unsupported_operation` | Operation/history mode/feature is not supported by this composition. |
| `invalid_configuration` | Configuration, state transition, or output request violates domain rules. |
| `revision_conflict` | Caller revision/cursor/archive selection conflicts with authoritative state; normally resnapshot. |
| `not_found` | Stable object or retained page/cursor is absent or expired. |
| `unavailable` | Required input/service/lifecycle is temporarily unavailable. |
| `transport_unavailable` | Required physical transport is unavailable. |
| `recording_unavailable` | Required Recorder service is unavailable or failed. |
| `timeout` | A bounded operation deadline expired. |
| `capacity_exhausted` | A fixed queue, projection, history, or service capacity is full. |
| `operation_failed` | Bounded fallback for an internal failure not safely exposed in detail. |
| `protocol_error` | Framing/version/encoding or incremental-state resynchronization failure. |

Messages are at most 256 bytes; structured details at most 2,048 encoded bytes.
Untrusted payloads and raw internal/SQLite/OS errors are not echoed.

## Bounds

`hello.result.limits` is authoritative for the running build. Current fixed values:

| Resource | Bound / behavior |
|---|---|
| complete NDJSON frame | 16 KiB including LF |
| JSON depth / values / string or key | 16 / 1,024 / 512 UTF-8 bytes |
| concurrent clients | 8 |
| owner/reactor mailboxes | 64 messages each |
| pending inbound requests/client | 8 |
| reply frames/client | 8 |
| event frames/client | 16 |
| network sweep | 8 KiB/turn |
| handshake, partial input, or queued-output deadline | absolute 2 s |
| terminal delivery after server shutdown | best effort for 200 ms |
| retained scopes | 16; detached retention 1,800 s |
| pending operations | 8/scope, 64 total |
| terminal operations | 32/scope, 256 total; 600 s retention |
| terminal result | 4 KiB |
| event replay / event record | 1,024 records / 4 KiB |
| frozen discovery/current/config page | 64 records / 8 KiB; 5 s retention |
| recent/durable measurement page | 128 records maximum |
| history work / cursors | 8 jobs/pages / 8 cursors |
| subscription | 1/client; 8 kinds; 16 targets |
| configuration properties / overlays | 256 total records; 32 runtime overlays |
| staged configuration | 1 candidate retained 30 s |
| emulator | 64 targets; 1 record/request; 8 pending mutations/scope |

Queue pressure is scoped. Slow or malformed clients can lose delivery or be
detached, but they do not become experiment owners and cannot block required native
Runtime progress indefinitely.
