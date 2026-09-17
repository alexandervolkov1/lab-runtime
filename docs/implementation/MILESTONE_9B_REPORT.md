# Milestone 9B implementation report

## Status

```text
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3: NOT STARTED
M9C+: NOT AUTHORIZED
```

This report is cumulative. This revision records only M9B.2. It does not claim that
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

M9B.3 remains not started. It owns owner-derived discovery, normalized current
measurement DTOs, bounded signal windows, history page semantics and subscription
coverage. The known pending-history disconnect/page-retention correction is also
deferred to that history slice as explicitly authorized; it is not required to
define common operation identity.

Later slices own Reference/controller completeness, Recorder projections,
resource/configuration projections, emulator publication, full reconnect/resync and
the remaining operation-specific naming cleanup. `reload_managed_sources` and
`restart_models` were deliberately not redesigned in M9B.2.

No COM port or hardware path was opened. M8 evidence was not modified.
