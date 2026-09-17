# Milestone 9B implementation report

## Status

```text
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3 discovery / measurements / history / subscriptions: COMPLETE
M9B.4 Reference and controller/PID API: COMPLETE
M9B.5: NOT STARTED
M9C+: NOT AUTHORIZED
```

This report is cumulative through M9B.4. It does not claim that
the complete M9B Application API is finished or externally accepted.

## M9B.2 scope and architecture

M9B.2 introduced one small host-side `protocol` module between bounded NDJSON
framing and Application dispatch:

```text
bounded NDJSON frame
        -> protocol operation registry and validation
        -> Application dispatch
        -> Runtime/domain operation
        -> typed result or bounded public error
        -> bounded protocol response/event
```

The module owns public protocol identity, operation declarations, capability
declarations, advertised limits and the public error taxonomy. It does not own
Runtime state and does not expose Rust enum layouts or `Display` text. `lab-core`
remains independent of JSON, TCP, sessions and host composition.

No queue, worker, transport, Recorder, OutputAuthority or Runtime scheduling model
changed. Socket work remains on the fixed nonblocking reactor; full client queues
detach the client instead of blocking the Runtime owner.

## Protocol identity and versions

Hello now distinguishes three identities:

- protocol identity `lab-runtime.application` and envelope version `1`;
- semantic Application API version `0.1-pre`;
- Runtime package version from `CARGO_PKG_VERSION`.

These are independent of Recorder/SQLite schema identity and native executable
provenance. There is no negotiation state machine: version 1 is accepted; another
version receives a bounded structured `version_mismatch` error and that connection
closes. Hello retains the top-level `v`, boot/session cursors and event window needed
by existing protocol clients while adding the explicit identity objects.

## Authoritative operations and capabilities

`protocol::OPERATIONS` is the source used by both wire validation and hello. Each
entry declares the stable name, query/mutation kind, allowed top-level arguments and
composition availability. Hello therefore no longer maintains a second handwritten
operation list. Recorder, configuration, managed-source reload and physical-resource
reconnect operations are advertised only when the frozen service composition can
perform them.

Capabilities are bounded structured records with `name`, `version` and `stability`.
They describe semantic protocol support, not a demo fixture or backend. In
particular, new hello responses do not advertise `recorder_sqlite_v1`,
`history_raw_paged_v1`, `managed_source`, Lua or Babashka names. The temporary
`managed_source_reload` capability is explicitly `transitional`; the operation
remains only while the text-backed adapter exists before M9C.

M9B.2 does not advertise any M9B.3+ discovery, history, emulator or resynchronization
feature that is not implemented.

## Correlation, admission and reconnect semantics

`msg_id` remains a bounded connection-local exchange correlation. It may be reused
only after the preceding exchange is no longer pending on that connection.

Queries are synchronous and have no `request_id`. Mutations carry a process-local
`{scope, seq}` request identity. The session store still provides:

- consecutive admission and payload-aware deduplication;
- one retained `accepted` state followed by at most one terminal result;
- replay of the same known outcome without re-execution;
- conflict for the same identity with different work;
- honest `outcome_unknown` after finite retention;
- accepted authoritative work and retained outcomes independent of TCP lifetime.

A synchronous rejection is a response with `type=error` and `accepted=false`. An
admitted operation first returns `type=operation,state=accepted` and later returns
exactly one `completed` or `failed` state carrying the same `request_id`. A failed
terminal operation now has the same structured error fields as a synchronous
rejection. Simple queries remain synchronous.

Session scopes and operation outcomes are process-local. A client may reconnect with
an unexpired scope from the same boot; process restart or finite eviction produces an
explicit unknown/resynchronization result rather than exactly-once claims across
restarts. Full reconnect reconstruction remains M9B.8 work.

## Public error model

Every public error has:

- an allowlisted stable `code` (specific when existing semantics are precise);
- a stable broad `category`;
- a fixed human-readable `message` of at most 256 bytes;
- `retryable` and `resync_required` booleans;
- optional trusted structured details admitted only up to 2,048 encoded bytes.

The category set implemented in M9B.2 is:

```text
invalid_request
unsupported_operation
invalid_configuration
revision_conflict
not_found
unavailable
transport_unavailable
recording_unavailable
timeout
capacity_exhausted
operation_failed
protocol_error
```

Known precise codes such as `unknown_signal`, `history_cursor_expired` and
`event_gap` remain available while mapping into those categories. Unknown internal
codes and arbitrary internal text map to the fixed `operation_failed` record; Rust
type names, filesystem details and uncontrolled `Display`/`Debug` text are never
copied into a response.

Unknown bounded operation names now reach Application dispatch and receive a
nonfatal structured `unsupported_operation`. Malformed JSON/schema/version input is
rejected by the reactor without entering the Runtime owner. Duplicate in-flight
`msg_id` remains deterministic and connection-scoped. A complete oversized incoming
frame is rejected/closed under the existing 16 KiB framing policy. If an owned
Application response unexpectedly exceeds that same bound, the server substitutes
one small correlated `response_too_large` rejection instead of terminating the
whole service loop.

## Published common bounds

Hello derives these values from the constants production code uses:

| Surface | Bound/policy |
|---|---:|
| NDJSON frame including LF | 16,384 bytes |
| JSON depth / values / string | 16 / 1,024 / 512 bytes |
| semantic name / capabilities | 64 bytes / 32 |
| public error message / details | 256 / 2,048 bytes |
| clients / owner-reactor mailbox | 8 / 64 |
| requests / replies / events per client | 8 / 8 / 16 |
| nonblocking network sweep | 8,192 bytes |
| hello, partial-frame, blocked-write deadline | 2,000 ms absolute |
| scopes | 16 |
| pending operations | 8 per scope / 64 total |
| retained terminal operations | 32 per scope / 256 total |
| terminal outcome / retention | 4,096 bytes / 600 s |
| detached scope retention | 1,800 s |
| event replay ring / event | 1,024 records / 4,096 bytes |

Operation-specific collection/page limits remain the responsibility of their M9B
domain slices. M9B.2 neither enlarges them nor creates unbounded client-controlled
allocation.

## Regression coverage

Focused tests prove:

- coherent protocol/Application/package identities and published limits;
- hello operations equal the registry filtered by actual composition;
- capabilities are structured, bounded and free of backend/client leakage;
- unknown operations are structured, nonfatal rejections;
- malformed, overdeep, overpopulated, oversized and incompatible frames remain
  bounded and panic-free;
- arbitrary internal error text cannot reach the wire;
- error message/detail bounds;
- synchronous rejection differs from accepted asynchronous failure;
- accepted and terminal responses retain one stable operation identity;
- a session record accepts exactly one terminal result;
- deduplication, replay, finite retention and disconnect independence remain intact;
- an oversized outgoing result becomes a bounded correlated client error;
- a slow/nonreading client remains isolated from Runtime progress.

The existing Recorder disconnect test continues to prove that an accepted mutation
and its durable terminal outcome survive client disappearance. Existing process,
Babashka A/B and owner-progress suites remain regression evidence only; Babashka is
not made part of the future product API.

## Deferred M9B work

M9B.4 is complete. Later slices own Recorder projections,
resource/configuration projections, emulator publication, full fault acceptance and
the remaining operation-specific naming cleanup. `reload_managed_sources` and
`restart_models` were deliberately not redesigned in M9B.2.

No COM port or hardware path was opened. M8 evidence was not modified.

## M9B.3 discovery, measurements, history, and subscriptions

Discovery is now an immutable, owner-derived projection of the actual Runtime and
host composition. `discover` freezes records for resources, instruments, and signals;
`discovery_page` continues the same revision. Records are deterministically ordered,
contain stable domain IDs and bounded domain metadata, and classify implementations
as `physical`, `virtual`, `emulated`, or `managed` without exposing Rust type names or
transport handles. A page contains at most 64 records and 8 KiB. The existing single
frozen-projection slot and five-second TTL apply per connection.

The public signal identity is exactly `{instrument, parameter}` everywhere:
discovery, `latest`, `measurements_current`, `measurement_window`, durable history,
and signal events. Array position is never an identity. Object identity is distinct
from generation: physical binding, native-model, and managed-component replacement
fences are projected as `generation`.

`latest` and the paged `measurements_current` operation use one complete measurement
DTO. It always contains signal, value (possibly null), unit, quality, explicit
status/failure, publication time, source/freshness time, and generation. A signal
that has never been attempted is explicitly `quality=unavailable`,
`status=not_observed`, and `failure=not_observed`; clients do not infer state from
missing JSON. Current values come only from committed Runtime snapshots, never
SQLite. Current pages share the 64-record/8-KiB projection limits.

`measurement_window` exposes the Runtime's bounded recent attempts, oldest first,
with the same measurement DTO, a maximum request of 128 records, explicit source and
truncation metadata, and an 8-KiB encoded-result check. Durable `history_read`
remains the distinct Recorder-backed operation: 1..=128 measurement rows or 1..=32
runs, frozen checkpoint ordering, at most 8 KiB per retained page, a two-second job
deadline, five-second page retention, and thirty-second continuation retention.
Empty selections succeed with an empty terminal page. Cursor tokens remain bound to
database/checkpoint/filter and are now also bound to the session scope, so a client
may reconnect to the same retained scope without inheriting another connection's
cursor.

The pending-history disconnect defect was a missing terminal transition. `detach`
removed the connection-keyed job and queued worker cancellation, but left the
already-admitted session operation pending indefinitely. Disconnect now records a
bounded `client_disconnected` terminal failure before cancelling only the
connection-local query delivery job. It removes page/snapshot/subscription delivery
state, does not mutate durable archive or experiment state, and cannot publish an
orphan result into a reused connection. Reconnect observes the retained terminal
outcome and starts a fresh explicit query.

Live delivery retains one aggregate subscription per connection, at most eight event
kinds and sixteen typed targets. The owner scans at most 32 retained events and
offers at most four per pump into the existing 16-frame client event queue. Runtime
acquisition, control, safety, and Recorder work never wait for that queue. Ring loss
or an invalid installed replay produces explicit `event_gap` with
`resync_required=true` and removes the subscription; a full network queue detaches
the slow client. Because the 1,024-record semantic ring is independent of the client,
a reconnect can resume from a still-retained cursor or receives an explicit gap. A
fresh connection reconstructs state with hello, discovery/current projections,
optional recent/durable history, and a new subscription. Subscription objects are
not preserved magically across disconnect.

Hello now advertises stable `structured_discovery`, `current_measurements`,
`recent_measurement_history`, `measurement_history`, and `live_subscriptions`
capabilities only when their operations are present. It also publishes the discovery,
current, recent/durable history, subscription, event queue, and page byte bounds.
The transitional storage-oriented history capability name is not present.

Focused M9B.3 tests cover owner-derived ordered discovery, stable signal identity,
Good and Unavailable current states, units, quality and generation, bounded recent
windows/pages, structured missing-signal errors, subscription admission/unsubscribe,
capability bounds, disconnect terminalization, no stale result delivery, and fresh
connection capacity. Existing protocol, session, durable-history, replay/gap, and
slow-client suites remain regression gates.

## M9B.4 Reference and controller/PID Application API

References retain their domain `ReferenceId` across discovery, `reference` current
queries, mutations, and reference events. The bounded current DTO explicitly reports
`kind`, `value`, engineering unit, `revision`, `status=valid`, configurability, and
the last monotonic evaluation time; Ramp projections additionally report target and
rate. `reference_configure` replaces the complete Fixed or Ramp policy while
retaining identity and the deployment-defined unit. A Ramp starts at the current
committed value. Core performs finite/rate validation, active-controller exclusion,
and the expected-revision comparison before one atomic commit. Failed validation or
revision conflict leaves the prior policy unchanged. The historical Ramp-only
`reference_retune` remains as a compatible narrow operation but is no longer the
advertised semantic capability.

Controllers likewise retain `ControllerId` across discovery, current queries,
operations, and events. The current projection reports `kind=pid`, lifecycle state,
explicit valid/failed status, active and paused booleans, configuration revision,
last tick/latest output, read-only input/Reference/output bindings, and the complete
supported PID, EMA, freshness, tick-gap, lease-lifetime, and proposal-TTL policy.
Bindings remain deployment-owned and are not rewritable through this slice. No
worker, channel, lease token, transport object, or mutable Runtime storage is
projected.

`controller_configure` is the typed full-policy operation. It preserves the current
bindings and engineering unit, requires an expected revision, parses duration and
count fields as bounded decimal strings, and delegates all finite, numeric, timing,
unit, lifecycle, and cross-reference validation to the existing atomic Core
reconfigure command. It never starts or rearms a controller. The earlier
`controller_configure_pid` remains a valid narrow update for existing callers; the
capability advertises the coherent full configuration surface.

The public lifecycle is exactly the lifecycle implemented by Core:
`controller_start`, `controller_pause`, `controller_resume`, and
`controller_reset_failed`. There is no invented Stop transition. Pause is the
existing safe transition through OutputAuthority; resume returns through warmup;
reset acknowledges a safely failed controller into Paused. Invalid states,
unavailable/stale input, rejected output, invalid configuration, missing identity,
and revision conflict use the bounded M9B.2 error taxonomy. An admitted transition
is Runtime-owned and is not cancelled on client disconnect.

The Application layer can request only these typed Runtime commands. Its registry
contains no generic output mutation, accepts no lease/ACK/readback fields, and has
no API for constructing an `OutputLease`. Native control still produces an
`OutputProposal`; Core and OutputAuthority retain finite leases, epoch fencing,
final pre-send authority checks, safe transitions, and ambiguous-write policy.
Configuration and Reference changes preserve their existing durable operation
intent and terminal provenance.

Reference and controller events use the same complete projections as current-state
queries. They remain snapshots of semantically relevant state rather than per-tick
PID telemetry, and use M9B.3's bounded replay ring, per-client queue, explicit gap,
and resynchronization rules. A reconnect reconstructs authoritative state with
hello, discovery, Reference/controller/measurement current queries, and fresh
subscriptions; controller state is never client-owned.

Hello now advertises `reference_read_write`, `controller_status`,
`controller_configuration`, and `controller_lifecycle`. Single-object Reference and
controller results are bounded to one record. PID configuration has exactly five
numeric fields; full controller policy has six bounded top-level groups. Common
64-byte semantic identifiers, 512-byte strings, 1,024 JSON values, 16-KiB frames,
4,096-byte events, operation/session limits, and subscription queue limits continue
to apply. No client-controlled collection was added.

Focused tests cover stable Reference/controller identity across discovery, current
state and events; complete Reference projection; atomic finite/rate and revision
validation; exact revision advancement; binding visibility; full PID/policy
round-trip; invalid configuration and conflict atomicity; start/pause/resume and
invalid transition behavior; disconnect-independent transitions; explicit failed
reset; capability/result bounds; and rejection of forged lease/ACK input. Existing
Core controller tests continue to cover scoped failure, reset success, generation
and authority fencing, safe pause, finite leases, final authority recheck, and
nonblocking control progress. Protocol, deduplication, lifecycle, subscription/gap,
process, Recorder, and workspace suites remain regression gates.

M9B.5 and later work remains deferred: Recorder public-contract refinements,
resource/configuration/reconnect expansion, external emulator API, full adversarial
fault acceptance, and final API freeze. No COM port or hardware path was opened, M8
evidence was unchanged, and M9C was not started.
