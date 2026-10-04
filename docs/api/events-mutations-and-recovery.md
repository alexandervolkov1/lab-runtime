# Events, mutations, and recovery semantics

Applicable to Application protocol v1 and the current repository/v0.1 product.

This page defines protocol semantics. Operator-facing troubleshooting is deliberately
outside this reference.

## Snapshots before events

Events are ordered changes, not a substitute for authoritative current state. A
typical client:

1. completes hello and records `event_latest`;
2. reads required current/frozen projections;
3. creates one aggregate subscription after a valid snapshot/fence cursor;
4. catches up and then applies live events in sequence.

If continuity cannot be proven, rebuild from queries. Never bridge old and new boots,
or infer unseen state across a gap.

## Frozen projection pages

Three query families use a connection-local frozen projection:

| Start | Continuation | Contents |
|---|---|---|
| `discover` | `discovery_page {projection,index}` | discovery records |
| `measurements_current` | `measurements_page {projection,index}` | current observations |
| `configuration_properties` | `configuration_page {projection,index}` | typed properties |

The first result and each continuation contain `projection`, a boot/event
`revision`, `records`, `next_index`, and `complete`. A continuation is tied to
its connection, projection kind, frozen records, and five-second lifetime. It never
silently switches to newer state. `snapshot_expired` requires a new start query;
`snapshot_capacity` means one record could not fit the bounded page.

There is one shared frozen-projection slot per connection across all three families,
not one slot per family. Starting any of the three start queries replaces the
connection's previous frozen projection. A continuation using the replaced token or
wrong family then fails with the existing `snapshot_expired` behavior.

Recorder history uses a different pattern and must not be treated as a projection
token. `history_read` is a mutation because archive work is scheduled and retained
as an operation. Its completed result yields a connection-local `page_token`;
`history_page` reads that page and `history_release` frees it. History continuation
cursors are tied to archive/filter selection and have their own bounded retention.

For Recorder history, a connection can own at most one pending history job and at
most one completed retained history page. While either slot is occupied, another
`history_read` on that connection follows the existing admitted-operation path and
terminates with `history_busy`. Continuation cursors are a separate bounded retained
resource. Page tokens and continuation cursors are connection-local; disconnect
removes them, so they are not valid after reconnect. If disconnect occurs while a
history job is pending, Runtime cancels/fences the job and retains the admitted
operation as terminal `failed` with `client_disconnected`.

## One aggregate subscription

`subscribe` accepts:

- `after:{boot_id,seq}`, which must identify this boot and a retained cursor;
- `filter:{kinds,targets}`, bounded to eight selected kinds and sixteen targets.

The current protocol recognizes event kinds `signal`, `controller`, `reference`,
`component`, `output`, `operation`, `host`, `recorder`, `resource`, and
`configuration`. An empty kind/target selection acts as an aggregate wildcard.
Workbench uses one aggregate subscription; its chosen filter is a client strategy,
not a second protocol registry.

Only one subscription may exist per connection. The result returns a token and
`accepted_cursor`. `unsubscribe` is connection-local and returns whether the
token was removed.

Each event carries `boot_id`, monotonic sequence, publication time, kind, target,
data, and optional mutation cause `request_id`. A filtered scan that advances
without matching an event emits `subscription_progress` so the client can advance
its applied cursor.

The event ring is bounded. If `after` is older than retained replay,
`event_gap` returns current oldest/latest cursors, removes invalid incremental
authority, and requires a new snapshot/rebuild. A cursor from another boot produces
`instance_changed`; a future cursor is invalid. Events do not make a partial rebuild
Fresh.

## Mutation admission and lifecycle

Runtime decodes a mutation into a normalized typed value before admission. For a new
consecutive identity, SessionStore reserves the exact payload and returns:

```text
request
  |
  v
accepted  ---- transport may be lost ----> client uncertainty
  |
  +----> completed(result)
  |
  `----> failed(public error, optional bounded result)
```

`accepted` is authoritative admission evidence. It is not terminal completion and
does not by itself describe physical effect. `completed` means that operation's
defined domain action reached its authoritative terminal result. `failed` is a
terminal operation outcome, not a fabricated projection or physical state.

Transport continuity and operation state are independent. A disconnect before the
caller reads `accepted` does not prove non-admission. A disconnect after `accepted`
does not cancel admitted work. Runtime may retain a terminal result after the socket
has gone away; recovery uses the exact scope/sequence evidence below.

The `operation_status` query accepts an exact `request_id` in the currently
attached scope and returns:

- `accepted` for retained nonterminal admission;
- `completed` with retained result;
- `failed` with public error fields and optional result;
- `outcome_unknown` when Runtime currently has no retained operation-state record for
  that exact identity.

The status lookup does not compare the requested sequence with the scope high-water
mark. Therefore `state:"outcome_unknown"` does not prove prior admission, execution,
non-execution, or that the sequence is already past.

## Client recovery evidence

`Pending` and `Ambiguous` are client/recovery evidence concepts, not Runtime
terminal operation states:

| Evidence | Meaning |
|---|---|
| Pending | client durably retained exact work before possible wire emission; Runtime admission is not yet known |
| Ambiguous | some mutation bytes may have been emitted, but admission/outcome is not known |
| Accepted | Runtime authoritatively admitted exact identity/payload |
| Completed | Runtime retained authoritative terminal success/result |
| Failed | Runtime retained authoritative terminal failure |
| Status `outcome_unknown` | Runtime currently has no retained state evidence for that exact identity; prior admission/execution is not established |
| Mutation error `outcome_unknown` | submitted sequence is already at/below high-water with no retained record and will not execute again |

A socket failure cannot turn Pending/Ambiguous into Accepted, Completed, or Failed.
Conversely, losing a reply does not roll back a Runtime-admitted mutation.

```text
operation outcome != current physical/projection state
Fresh observations != mutation reconciliation
```

ACK, readback, and physical effect remain distinct unless a specific operation result
explicitly contains the corresponding authoritative evidence.

## Deduplication

Mutation identity is `{scope,seq}`; equality also includes the normalized typed
operation/payload retained under that identity.

```text
same retained request_id + same normalized typed operation/payload
    -> return retained current outcome
    -> do not execute twice

same retained request_id + different operation/payload
    -> request_conflict

already-past sequence with no retained record
    -> mutation error code outcome_unknown
    -> do not execute that old identity

new sequence above next_seq
    -> sequence_gap
```

Deduplication is process-local and retention-bounded. A scope from another boot is
invalid. A detached scope may be reattached only while retained and not attached
elsewhere.

The mutation error above is distinct from an `operation_status` result containing
`state:"outcome_unknown"`. Neither is Completed or Failed, proof of physical state,
or permission for blind retry.

## Exact resubmission

There is no `exact_retry` Application operation. “Exact Retry” is a client workflow
that deliberately resubmits:

```text
same request_id
same operation
same normalized typed arguments
```

The client must use retained exact evidence, not reconstruct or edit the payload. An
exact resubmission may return a retained outcome without execution. It may also be the
first Runtime admission if the original bytes never reached admission and that exact
identity is still the next admissible sequence. Therefore “retry” does not prove prior
execution.

**Automatic mutation retry is not part of the Application contract.** Workbench makes
Check Status and Exact Retry explicit/manual and never resubmits mutation/status/retry
work merely because a socket reattached.

## Scope and boot invalidation

- `scope_in_use`: the same retained scope is attached elsewhere; no new authority is
  created.
- `scope_unknown`: Runtime cannot attach the scope; old evidence cannot be assigned
  to a new scope.
- `instance_changed`: the scope/cursor belongs to another Runtime boot.

These conditions do not erase client evidence and do not authorize sending an old
request identity under a new scope. See [Protocol and sessions](protocol-and-sessions.md)
and [Public errors](errors-and-limits.md).
