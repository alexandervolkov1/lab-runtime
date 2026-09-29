# M14.6B3 bounded fault reattach

Status: ready for final external re-review.

`STATUS: M14_6B3_POST_FENCE_ADMISSION_READY_FOR_EXTERNAL_REVIEW`

## Scope and ownership

M14.6B3 implements the accepted M14.6A distinction between an explicit operator
disconnect and an unexpected loss of an already attached Runtime continuity domain.
The single TCP worker remains the sole mutable owner of sockets, connection-local
requests, retained scope, retry timing, and the current reattach episode. The GUI
`RebuildCoordinator` no longer submits a competing `Connect` after a
`connection_lost` update; it waits for a worker-published `Hello` and then performs
the ordinary bounded projection rebuild.

No dependency, Cargo manifest, Runtime, Application operation/DTO, core, recorder,
recovery-journal schema, or presentation-document format changed.

## Episode state and timing

The worker replaces the former optional retained-scope deadline with one typed
state:

```text
None
ManualRetained { deadline }
AutomaticFault { deadline, cause }
```

`cause` is a bounded enum, not an OS-error string or history. It classifies EOF,
read, write, partial-frame timeout, request timeout, protocol failure, and ordered
update overflow. Only one current episode exists.

An automatic episode captures `deadline = now + 3 seconds` on the first fault from
`Ready`. Every later TCP failure, hello/protocol failure, `scope_in_use`, partial
progress, or timeout reuses that `Instant`. A connect attempt receives
`min(2 seconds, deadline - now)`. Every scheduled retry is at least 10 ms after the
preceding failed attempt. Expiry atomically removes the retry timer and episode and
publishes `Disconnected`; only a later manual `Connect` can open another socket.

## Explicit disconnect and shutdown

The client boundary accepts Disconnect by atomically publishing one coalescing
out-of-band pending bit. Disconnect does not enter or compete for space in the
ordinary command mailbox, so a full 32-command mailbox still yields `Ok(())`. The
only rejection retained is a worker already known to be stopped; that check occurs
before the control word is changed and therefore has zero reconnect-policy effects.

The same bounded control word provides the linearization order. Bit zero is the
single worker's connect-attempt claim and bit one is the accepted Disconnect fence.
A connect attempt can claim only the zero word. If the claim wins first, that attempt
logically predates Disconnect; after `connect_timeout` returns, the worker observes
the pending bit and discards the socket or error result. If Disconnect wins first,
the claim fails and `TcpStream::connect` is never called.

Disconnect acceptance also closes ordinary command admission for the old connection
epoch. One shared client helper guards Connect, Query, Mutation, RetryMutation,
OperationStatus, Subscribe, Unsubscribe, and reference bootstrap submission against
both worker stop and the pending bit. A third constant-size control bit claims the
short nonblocking mailbox submission interval. If submission wins immediately before
Disconnect, the worker keeps the fence until that claim is released and then drains
the racing command. If Disconnect wins, the submission claim cannot be acquired and
the public call fails locally without consuming a command id or mailbox slot.

On first observing the pending bit, the worker clears `retry_at`, removes either
reattach mode and retained scope, closes/reset the transport, discards
connection-local work, and preserves bounded recovery/journal/quarantine evidence.
The bit remains set while a dedicated drain rejects queued ordinary commands in the
unchanged bounded batches of `COMMANDS_PER_TURN = 8`. The drain never invokes normal
command handlers, so a queued mutation cannot enter pre-wire journal admission and
status/retry/subscription/bootstrap commands cannot queue wire work. With ordinary
post-fence admission closed, at most the existing 32 mailbox entries remain; four
eight-command drain batches plus one empty observation complete Disconnect in at
most five worker turns. Once empty with no racing submission claim, the worker
publishes `Disconnected`, acknowledges the bit, and permits a new explicit Connect.
Repeated Disconnect calls coalesce into the same constant-size bit and cause one
cleanup.

Disconnect and worker shutdown emit no `runtime_shutdown`, `recording_stop`,
`controller_pause`, mutation, `RetryMutation`, or `operation_status` request. A
scripted listener observes no second socket for longer than the former three-second
fault window after an explicit disconnect.

## Fault and hello behavior

The common automatic episode path covers EOF/reset, read failure, write or blocked
write failure, partial-frame timeout, request/reply timeout, malformed/oversized or
otherwise invalid protocol input, and ordered-update overflow. A fault closes the
attempt, marks cached observations non-authoritative through existing updates and
states, and exposes `Reattaching` while time remains.

`scope_in_use` closes only the current attempt and schedules the next attempt inside
the same deadline. `instance_changed` and `scope_unknown` terminate the episode,
quarantine exact old recovery evidence through the accepted B2A path, remove retry
authority, and leave the client disconnected for a manual new-scope connection.
No second automatic episode begins from either invalidation.

A same-boot/same-scope hello clears the episode and retry timer and publishes
`Ready` plus `Hello`. It does not make observations Fresh. The existing bounded
discovery/current/detail/fence/subscription/catch-up rebuild remains responsible for
Freshness. After a successful hello, a later independent continuity fault may own a
new three-second episode.

## Non-replay and same-connection recovery

Transport reset discards all connection-local query, subscription, status, and
mutation exchanges. Isolated ordinary queries are not replayed. Rebuild may issue
its normal bounded projection queries only after a new authoritative hello.

Possibly transmitted mutation evidence follows the existing worker/journal rule:
Pending becomes Ambiguous where required, Accepted remains Accepted, and exact
identity/payload evidence remains bounded and durable. Automatic reattach sends no
mutation, no `RetryMutation`, and no `operation_status`. Pending Check Status and
Exact Retry UI workflows are interrupted by existing renderer-neutral fault updates
and are never resubmitted. A user can explicitly invoke those accepted B2B1/B2B2
actions after hello and full rebuild restore their authority.

Application `event_gap` remains a same-connection resnapshot. It clears the affected
subscription/bootstrap state and starts the existing bounded rebuild without a TCP
connect attempt or transport episode.

## Ordered-update overflow

Ordered update saturation now enters the same typed automatic fault episode as every
other continuity loss. There is no overflow-specific deadline or reconnect queue.
The existing single deferred `ResnapshotRequired { connection_lost: true, ... }`
remains the fail-closed GUI notification; the worker retains only one `retry_at` and
one typed episode.

## Visible state and bounds

Explicit disconnect ends in `Disconnected`, enabling manual Connect with no
background activity. An active unexpected-fault episode exposes `Reattaching`; the
existing model makes cached observations non-authoritative and disables mutations,
Check Status, and Exact Retry until hello and rebuild restore authority. Expiry ends
in `Disconnected`.

No reconnect history or additional queue was added. Existing command, update,
in-flight, recovery, quarantine, frame, and envelope bounds are unchanged. The only
retained fault diagnostic is the small typed current cause. Raw bounded Application
envelopes are still parsed semantically before any presentation handling.

## Deterministic evidence

Focused tests cover:

- explicit Disconnect from Ready and from both episode modes, including no socket
  open beyond the former three-second window;
- due AutomaticFault and ManualRetained retries fenced with eight preceding commands,
  plus a near-capacity case with 31 preceding commands and an old queued Connect;
- full 32-command mailboxes in both episode modes, accepted out-of-band Disconnect,
  constant-size coalescing, one cleanup, and no socket open;
- a full mailbox starting from Ready with query, mutation, status, retry,
  subscription, unsubscribe, and bootstrap commands, all rejected by the drain with
  no wire exchange or recovery-journal byte change;
- every public ordinary API rejected after the fence without mailbox or command-id
  growth, including repeated producer attempts and bounded completion;
- deterministic pre-fence submission-claim race cancellation without normal
  mutation, wire, or journal semantics;
- both connect-claim/Disconnect linearization orders, including disposal of a
  pre-fence connect result and no connect call when the fence wins;
- rejected WorkerStopped Disconnect with unchanged episode, timer, scope, and state;
- exact Accepted/Ambiguous evidence preservation across the backlogged Disconnect,
  followed by exactly one permitted manual attempt after completed Disconnect;
- initial `scope:null` failure without retry;
- typed EOF/read/write/partial/request/protocol/overflow triggers;
- one immutable three-second deadline across repeated transport and hello failure;
- 10 ms retry spacing and `min(2s, remaining)` connect timeout arithmetic;
- `scope_in_use`, deadline expiry, manual recovery, successful same-scope hello, and
  a later independent fault;
- `instance_changed`/`scope_unknown` quarantine termination;
- no ordinary-query, Check Status, or Exact Retry/mutation replay and preservation of
  exact Accepted/Ambiguous evidence;
- `event_gap` same-connection rebuild ownership;
- one overflow fault mechanism and the pre-existing deferred fail-closed regression;
- worker shutdown without any Runtime shutdown operation; and
- no reconnect history or unbounded diagnostic state.

Final verification for the review bundle includes three consecutive Workbench test
runs, the locked debug and release workspace suites, warning-denied all-target
Clippy, all four existing ignored real-Runtime Workbench acceptance tests, and the
native Glow GUI smoke.

Recorded results:

```text
cargo fmt --all -- --check: PASS
cargo test -p lab-workbench --locked, three consecutive runs:
    148 passed; 4 ignored; 0 failed (each run)
cargo test --workspace --locked:
    668 passed; 6 ignored; 0 failed
cargo test --workspace --release --locked:
    668 passed; 6 ignored; 0 failed
cargo clippy --workspace --all-targets --locked -- -D warnings: PASS
four real Runtime Workbench acceptance tests: 4 passed; 0 failed
native GUI smoke: PASS (renderer=glow, native_window_observed=true)
git diff --check: PASS
```

## Non-goals

M14.6B3 adds no automatic mutation/status/Exact Retry replay, Discard/Forget action,
new Runtime or Application semantics, dependency, generalized reconnect history,
M14.6B4 process matrix, or Steel work. M14.6B4 remains unauthorized and M13.2 Steel
remains blocked/not authorized.
