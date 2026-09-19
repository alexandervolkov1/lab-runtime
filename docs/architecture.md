# Architecture and concepts

## System boundary

```text
physical instruments ── protocol adapters ──┐
virtual instruments ────────────────────────┤
native managed components ─────────────────┤
                                            v
                                      Core Runtime
                                      authoritative state
                                            |
                  ┌─────────────────────────┼──────────────────────┐
                  v                         v                      v
          Host/Service orchestration    Recorder/SQLite       Application
          scheduling and adapters      durable history       semantic API
                                                                  |
                                                                  v
                                                          external clients
```

The governing rule is:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.
```

The repository therefore contains no workspace, plot, panel, or other presentation
model. Those belong to clients outside `lab-runtime`.

## Ownership

| Layer | Responsibility | Does not own |
|---|---|---|
| `lab_core::Runtime` | Instruments, committed signals and recent history, References, controllers, `OutputAuthority`, resource executors, component state, semantic Recorder fact outbox | OS serial ports, SQLite, sockets, presentation |
| `HostCore` | Composes one Runtime with schedules, adapters, Recorder admission, event projection, and configuration catalogs | A second copy of experiment state |
| `ServiceHost` | Process startup, loopback listener, deployment lifecycle, reconnect, and finite shutdown progression | Controller or measurement semantics |
| `Application` | Sessions, deduplication, operation correlation, frozen pages, subscriptions, and public projections | Experiment authority |
| `RecorderWorker` | Bounded fact ingress and exclusive SQLite worker ownership | Runtime decisions or output authority |
| diagnostic logger | Best-effort troubleshooting records | Scientific history or control decisions |

Queries clone bounded committed projections. Commands and Application mutations
advance state through the serialized owner. A query never performs hidden polling.

## Instruments, parameters, and signals

A **resource** is a bounded transport endpoint, such as one configured COM port. An
**instrument** is a semantic device or model. Its descriptor contains stable
parameter identities, value types, units, access modes, roles, and write effects.

A readable measurement parameter may expose a **signal** identified by the pair:

```json
{"instrument":"41","parameter":"1"}
```

IDs are stable semantic identities; list positions and display names are not IDs.
Each current measurement contains:

- a value and engineering unit when quality is `good`;
- `quality` (`good` or `unavailable`) and a failure reason;
- `observed_at_ns`, the Runtime monotonic publication/attempt time;
- `source_at_ns`, the original observation/model time used for freshness;
- a Runtime-owned generation.

Before the first attempt, the public status is `not_observed`, with no value or
timestamp. An `Unavailable` attempt is explicit and cannot carry an old successful
value. A cached `Good` value can still be too old for control: quality and freshness
are separate facts. Generation replacement fences late completions from an old
resource or model instance.

Periodic scheduling uses monotonic deadlines. A delayed slot receives one
actual-time opportunity rather than replaying missed historical deadlines. This is
bounded scheduling, not a hard-real-time guarantee.

## Acquisition and native progress

The physical Metakon path is:

```text
Host scheduler
  -> Runtime QueueMetakonRead
  -> typed Metakon READ
  -> bounded ResourceExecutor
  -> serial worker
  -> strict frame/CRC decode
  -> Runtime measurement commit
  -> current/recent history, events, controller input, Recorder fact
```

One Host service turn checks Recorder receipts, safety and transport progress,
shutdown/configuration fences, virtual models, physical-read admission, References,
controllers, and managed components in a fixed safety-first order. Non-authoritative
component or client work cannot make required native progress wait indefinitely.

## Control and physical output

Controllers calculate proposals but cannot write transports. The trusted path is:

```text
fresh measurement + Reference
  -> native controller
  -> OutputProposal
  -> Runtime-owned OutputAuthority
  -> bounded ResourceExecutor reservation
  -> final lease/epoch/generation check
  -> typed transport WRITE
  -> strict ACK
  -> separate register readback
```

An `OutputProposal` is intent, not permission. Authority is tied to one authority
instance, owner, revocation epoch, finite lease, binding generation, mapping
revision, and proposal deadline. See [Safety and failure behavior](safety-and-failures.md).

## Managed components

A managed component is bounded Runtime-invoked computation:

```text
Runtime -> Invocation -> native worker -> ComponentResult -> Runtime validation
```

The current implementations are trusted compile-time Rust registrations. A
component has bounded `PlainData` configuration/state and validated inputs/results.
It has no Runtime, serial, Recorder, client, or `OutputAuthority` handle. A dead
worker becomes explicit dead capacity; it is not respawned in an unbounded loop.

## Application and sessions

The Application API is semantic and transport-independent in design; the current
adapter is local TCP with NDJSON serialization. One connection first obtains or
reattaches a process-local scope with `hello`. Queries return bounded committed
snapshots. Mutations have accepted/terminal operation states and process-local
deduplication by `(scope, seq)`.

The event ring and connection queues are bounded. Event loss is reported as a gap
that requires a current snapshot/resynchronization; delivery is not guaranteed.
Client lifetime is deliberately independent of experiment lifetime.

## Recorder and time

The Runtime emits semantic facts. `RecorderWorker` charges bounded ingress credit,
and one storage thread owns SQLite transactions. Admission is not a durability
claim; the committed prefix advances only after the storage receipt reaches the
owner. See [Recorder and SQLite](recorder-sqlite.md).

Monotonic time is used for scheduling, ordering, freshness, PID `dt`, leases, and
deadlines. Wall-clock time is used for human-readable archive context and export.
Wall-clock changes do not alter control behavior. Clock-anchor records relate the
two domains without making wall time authoritative.

## Boundedness and shutdown

All long-lived queues, histories, sessions, workers, projections, and cursors have
explicit limits. On exhaustion the subsystem rejects, evicts according to its
documented policy, reports a gap, or fails closed; it does not switch to an
unbounded fallback.

Shutdown is a bounded progression, not an instantaneous state:

```text
request -> revoke/control safety -> transport retirement -> Recorder seal/flush
        -> worker/client cleanup -> truthful terminal result
```

`shutdown requested` does not mean every resource is already closed. An unfinished
or physically ambiguous resource remains visible in the terminal result; software
shutdown does not manufacture proof of physical safety.
