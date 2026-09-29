# M14.6A — Workbench recovery and fault architecture audit

## Status and scope

```text
M14.5: ACCEPTED

M14.6A recovery/fault audit:
READY FOR EXTERNAL REVIEW

M14.6B implementation:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M14_6A_RECOVERY_FAULT_AUDIT_READY_FOR_EXTERNAL_REVIEW
```

This is a read-only architecture audit. It was performed from coordination HEAD
`a41e18af16ee7904515b81dba91e6a83300b972e`. The accepted M14.5 implementation is
`ab097ed5207ea426cbbd48611015da12ce534a43`.

M14.6A changes no Rust source, test, Cargo manifest, lockfile, Runtime operation,
Application DTO, public error, session rule, deduplication rule, transport, queue
capacity, persistence format, or experiment semantic. It specifies the contract that
a later, separately authorized M14.6B implementation must prove.

## Boundary and authoritative owners

The accepted boundary remains:

```text
GUI / future Steel
        |
        | bounded typed commands
        v
ONE Workbench Application client worker
        |
        | TCP / NDJSON
        v
ONE Runtime Application / ONE SessionStore / Runtime
```

The worker alone owns the socket, `msg_id`, retained scope, mutation sequence,
pending exchanges, one subscription, event cursor, recovery records, durable journal,
and exact retry payload. `WorkbenchModel` projects that state for display; it does
not reconstruct a second recovery lifecycle. The rebuild coordinator owns bounded
snapshot/subscription orchestration. Operator UI owns intent and confirmation, not
admission or outcome.

These invariants are unchanged:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
script lifetime != experiment lifetime
```

Workbench close, crash, disconnect, reconnect, and rebuild never imply Runtime
shutdown, experiment restart, controller pause, Recorder stop, or mutation retry.

## Already accepted M14.2–M14.5 behavior

- TCP/NDJSON Application JSON is at most 16,383 bytes and a complete frame is at
  most 16,384 bytes including LF. Framing, partial writes, and partial reads are
  bounded.
- The client has one worker, a 32-command mailbox, at most eight in-flight exchanges,
  a 64-update ordered mailbox, and one aggregate subscription.
- The worker owns a checked, connection-local `msg_id` counter. Mutation identity is
  the retained `(scope, seq)` and is never replaced by `msg_id` or GUI command ID.
- A mutation record containing exact `op`, `args`, boot, scope, sequence, and known
  admission state is durably written before possible wire emission. At most eight
  records and 64 KiB are retained.
- Exact retry reads only the worker-owned record. Journal load, hello, reconnect, and
  rendering never send a mutation automatically.
- The model consumes the worker's complete bounded `RecoveryState` projection.
- Ordered-update overflow closes the transport and emits a self-contained
  transport-lost resnapshot notification. An event gap on a usable connection does
  not claim transport loss.
- Disconnect retains cached live points as visibly stale. Starting a rebuild after
  lost continuity clears live buffers and truncation counters, creating a new display
  epoch. This is display policy, not Recorder/history loss.
- `Ready` transport is not `Fresh` Workbench state. Fresh requires the explicit
  projection/fence/single-subscription catch-up barrier.
- Operator controls require Ready/Fresh authoritative projections and no unresolved
  recovery fault. Only `track_operator_intent` creates action bookkeeping, which is
  bounded to 64 entries and never evicts nonterminal state.

## M14.6A decisions

### Intentional disconnect and automatic reattach

An explicit user Disconnect and an unexpected continuity loss are different events
and must remain distinguishable in the client state machine.

An explicit Disconnect is terminal for the current connection attempt. It closes
only the Workbench connection, marks observations stale, retains cached live points,
and never starts automatic reconnect. Any transmitted or accepted mutation is kept
for later reconciliation. The operator must choose Connect to resume.

An unexpected continuity loss may start exactly one automatic retained-scope
reattach episode. The episode reuses the accepted absolute three-second reattach
deadline. Each connect attempt receives `min(2 seconds, remaining)`, retries are
separated by at least 10 ms, and hello plus `scope_in_use` retry progression remains
inside the same absolute deadline. The episode is not restarted by partial progress,
another socket failure, or `scope_in_use`. Once it expires, the client remains
Disconnected and requires a user action. Thus there is no infinite reconnect loop;
even immediately failing attempts have a conservative time-derived ceiling of 301
within one episode.

M14.6B must extend this policy consistently to ordinary unexpected transport failure.
The existing ordered-update overflow path already requests a retained-scope
reconnect. An initial `scope:null` connection failure and explicit Disconnect remain
manual-connect cases. Startup with a valid journal-retained scope may consume one
bounded reattach episode, but never chains a second episode automatically.

### Mutation reconciliation UI

For an unresolved worker-owned `MutationIdentity { scope, seq }`, the UI may expose
only these semantic actions:

```text
Check Status
    -> one operation_status request for the exact identity

Exact Retry
    -> one RetryMutation command containing only the exact identity
    -> worker sends its immutable retained op/args/request_id
```

Only one status or retry request for an identity may be outstanding. All such
requests also consume the existing 32-command and eight-in-flight bounds and the
five-second request deadline. There is no polling loop. Check Status never fabricates
admission or outcome. Exact Retry is disabled unless the attached hello has the same
boot and scope and the worker has the exact record; the payload is never editable or
regenerated. Neither action runs automatically after hello, reconnect, restart, or
event replay.

`outcome_unknown` leaves the record unresolved. Completed and failed status results
are authoritative terminal evidence. Accepted is authoritative admission but still
needs terminal reconciliation. A terminal record may remain visible and may be
retired only by the existing bounded capacity policy after the replacement journal
state has been durably written.

### Recovery journal lifecycle

The journal is Workbench-owned, version 1, limited to 64 KiB, eight records, and
512-byte identity/operation strings. Reads are bounded; writes use a flushed and
synced sibling temporary file followed by replacement. This does not claim parent
directory fsync or protection from external deletion. The accepted single-active-
Workbench workspace mutex is the process ownership rule.

Safe cases are:

- a missing journal at startup means there is no Workbench-retained mutation record;
  it creates an empty recovery projection and does not itself block ordinary use;
- an unsent record may be removed only when the worker proves no possible byte was
  emitted and the removal is durably persisted;
- a terminal record may be retired under the accepted capacity policy only after the
  candidate journal replacement succeeds;
- an empty journal may be removed only after no exact records remain.

Fail-closed cases are corrupt JSON, invalid schema/version/identity, bounded-read
failure, pre-wire write failure, admission-state update failure, or retirement
failure. The file is not overwritten with an empty/default journal. The condition is
visible, all new mutations are blocked, and any in-memory exact record remains
projected. Observational queries and a bounded rebuild may proceed, but Fresh
observations do not clear the recovery warning or restore mutation authority.

A different boot, `instance_changed`, `scope_unknown`, or a different attached scope
turns old records into quarantined uncertainty evidence. They are not retry authority
and must not disappear merely because a new Runtime can be observed. The current
worker clears its live recovery vector on some hello errors while surfacing a journal
problem; M14.6B must close that projection gap by retaining a bounded visible
quarantine/disposition instead of presenting an empty recovery state as resolution.
M14.6A does not authorize a destructive Discard/Forget action.

### Freshness and continuity

On disconnect, cached semantic projections and existing live points may remain
visible only as stale. A reattach/new-connect hello begins a new bounded rebuild,
clears the prior live display epoch, and keeps mutation controls disabled. Global
Fresh returns only after all selected projection queries, the fence, the one aggregate
subscription, catch-up, and required coalesced detail refreshes complete. A boot
change never reuses an old live epoch or draft authority.

An event gap on a still-usable connection starts the same bounded rebuild without a
reconnect. Ordered-update overflow and malformed/closed transport require a new
hello before rebuilding. No individual widget issues recovery queries.

## Frozen and proposed finite bounds

| Resource | Bound and owner | Overflow/failure rule |
|---|---|---|
| Command mailbox | 32, caller to one worker | `Busy`; command did not enter worker |
| Commands serviced | 8 per worker turn | Later commands remain in the bounded mailbox |
| In-flight exchanges/outgoing work | 8, worker | Ninth rejected before wire |
| Recovery records | 8, worker and journal | Retire only terminal record durably, otherwise reject before wire |
| Ordered updates | 64, worker to GUI | Close transport; one deferred transport-lost resnapshot |
| GUI drain | 64 per frame | Repaint/wake continues; no intermediate queue |
| Runtime subscription | 1 per connection | Second/pending subscription rejected |
| Bootstrap event buffer | 64 | Resnapshot required; no fabricated continuity |
| Operator actions | 64 | Evict oldest terminal only; fail closed if all nonterminal |
| Interactive operator workflow | 1 | New workflow rejected until current is acknowledged/cancelled |
| Status/retry per mutation | 1 outstanding (M14.6B decision) | Reject duplicate locally; no polling |
| Journal | 64 KiB, 8 records, strings 512 bytes | Reject candidate; preserve prior valid file |
| Application body/frame | 16,383 / 16,384 bytes | Connection-local protocol failure |
| Network work | 8 KiB read per turn; fixed frame/write state | Continue later or hit finite deadline |
| Live points | 4,096 per signal | Drop oldest display point and count local truncation |
| Rebuild identities | 64 References, 64 controllers, 64 resources | Fail rebuild visibly |
| Dirty detail refresh | 64 controllers and 64 resources; config uses two booleans | Coalesce; fail rebuild on identity overflow |
| Connect/hello | 2 s / 2 s | Disconnect or remain manual-connect |
| Partial frame/blocked write | 2 s / 2 s | Close connection; preserve mutation ambiguity |
| Request/reply | 5 s | Close connection; preserve mutation ambiguity |
| Reattach episode | One absolute 3 s episode, >=10 ms retry interval | Then Disconnected/manual Connect |
| Worker shutdown | 3 s | Visible join failure; never wait on Runtime shutdown |
| Stored local diagnostic | Proposed 512 UTF-8 bytes in M14.6B | Truncate on a character boundary with marker; raw protocol envelope remains frame-bounded |

All maps and queues introduced by M14.5 are already bounded by protocol cardinality,
the explicit 64-entity/detail bounds, bounded replacement, or the single current
workflow. M14.6B must not add a reconnect history, retry history, diagnostic log, or
poll queue.

## Fault matrix

In the matrix, **AR** means automatic reattach and **MR** means automatic mutation
retry. `MR: never` applies to every row. The final column states the authoritative
evidence needed for Fresh, the relevant bound, and the required acceptance oracle.

### Connection and continuity faults

| # / trigger | Worker, session, scope, journal | Model, live data, operator workflow | Recovery policy and user-visible action | Fresh evidence, bounds, acceptance oracle |
|---|---|---|---|---|
| 1. Runtime unavailable at initial Connect | A `scope:null` attempt goes `Connecting -> Disconnected` with no scope allocation; a journal-scope startup may use one bounded reattach episode; journal remains loaded | Observations Stale/Rebuilding, no new live epoch data; draft controls disabled | AR: no for `scope:null`, at most one episode for a retained startup scope. MR: never. Show failure and enabled manual Connect after exhaustion | A later manual Connect must receive hello and finish the complete barrier. Two-second new-scope attempt/three-second retained episode. Test Runtime absent then started |
| 2. Unexpected disconnect while idle | Worker closes socket and subscription, preserves retained scope/cursor/journal | Connection Stale/Disconnected; cached projections and live points remain visibly stale; idle workflow stays idle | AR: one three-second retained-scope episode. MR: never. Show reconnecting then manual Connect if exhausted | Successful hello plus full barrier; test Runtime stays alive and retained-scope path is finite |
| 3. Disconnect during ordinary query | Pending query is discarded with connection state; no mutation journal change | Query result is not fabricated; model becomes Stale, cached live points remain | AR: one episode. MR: never. Do not replay the isolated query; rebuild coordinator may reissue its bounded projection work | New hello and complete rebuild. Eight pending/five-second deadline. Test lost query never appears as result |
| 7. Ordered-update overflow | Worker records one deferred `connection_lost=true` resnapshot, closes transport, retains scope and journal | Model cannot remain Ready; cached values/points become stale until rebuild begins, then live epoch clears; active workflow becomes Ambiguous when applicable | AR: one retained-scope episode. MR: never. No query on closed transport | New hello and explicit barrier. Queue 64 and one deferred update. Preserve deterministic saturation test |
| 8. `event_gap` on usable connection | Socket and scope remain Ready; subscription is abandoned/recreated by coordinator; journal unchanged | Begin Rebuilding immediately and clear live epoch; stale presentation references remain visible; controls disabled | AR: no. MR: never. Same-connection bounded resnapshot | Fresh only after snapshot/fence/new single subscription catch-up. Test exact gap and no Connect command |
| 9. Runtime boot change | Old scope/recovery is not authority in new boot; old records become bounded quarantine evidence | Prior observations/drafts Stale, live epoch cleared on new hello; observational rebuild may complete but recovery warning persists | AR stops after identity rejection; manual new-scope Connect may be offered. MR: never; Exact Retry refused | Fresh observations need new-boot barrier; mutation authority additionally needs reviewed recovery disposition. Test equal IDs/revisions do not revive old authority |
| 10. Retained-scope reattach succeeds | Same scope/boot hello reconciles `next_seq`; records remain worker-owned; subscription is new | Ready then Rebuilding; old points clear at rebuild begin; unresolved workflow remains visible | AR episode ends. MR: never. Offer Check Status/Exact Retry only as applicable | Complete barrier for Fresh and status/retry evidence for records. Three-second episode. Test scope and next sequence continuity |
| 11. `scope_unknown` | Retained attach fails; record cannot be queried/retried in that session and must be quarantined, not silently resolved | Disconnected/Stale; cached values retained stale; workflow Unknown with identity-loss explanation | AR ends; manual new-scope connection is separate. MR: never; no exact retry | New-scope barrier may restore observations, not old mutation authority. Test visible quarantine and zero sends |
| 12. `instance_changed` | Same as different boot: old journal identity is non-authoritative and quarantined | Old observations/drafts stale, old live epoch never reused | AR ends; manual new instance connection only. MR: never | New-boot barrier for observations; explicit future disposition needed for mutations. Test old record remains visible |
| 13. `scope_in_use` | Attempt socket is closed; retained scope/journal unchanged; retry uses same absolute deadline | `Reattaching`; cached values/points stay stale; workflow unchanged/Unknown as applicable | AR continues only inside current three-second episode. MR: never | Same-scope hello then barrier. Test deadline is not reset and manual state follows exhaustion |
| 21. Runtime restarts while Workbench survives | TCP fails; bounded old-scope episode reaches `instance_changed`/unknown; old journal quarantined | Connection stale, boot-A observations stale, live epoch cleared before boot-B data; confirmations invalid | AR: one episode, then manual new-scope Connect. MR: never | Boot-B hello/barrier restores observations only; test Runtime restart and no old mutation send |
| 22. Disconnect during projection rebuild | Abort coordinator phase and pending snapshot work; retain scope/journal | Overall never becomes Fresh; already cached entities stale; old live points stay only until next rebuild begins | AR: one episode for unexpected loss. MR: never. Explicit disconnect remains manual | New hello restarts entire finite barrier. Test first partial reply never completes rebuild |
| 29. Malformed/oversized protocol data | Frame/JSON/order violation closes only Workbench transport; mutation ambiguity is journaled | Disconnected/Stale; no malformed value reaches projections; active mutation becomes Unknown if possibly sent | AR: one episode for unexpected server fault, then manual. MR: never. Show protocol error | New valid hello/barrier. 16,383/16,384 and two-second partial bound. Test healthy Runtime remains authoritative |
| 30. Explicit Disconnect vs unexpected loss | Explicit command closes socket and cancels retry timers; unexpected loss retains permission for one AR episode | Both show Stale, but intent/recovery cause remains distinct; explicit disconnect keeps cached live points and does not begin rebuild | Explicit AR: no; fault AR: one episode. MR: never in both. UI shows Disconnected vs Reconnecting/fault | Only user Connect restarts explicit case; fault case may hello automatically. Test no immediate reconnect after explicit action |

### Mutation and reconciliation faults

| # / trigger | Worker, session, scope, journal | Model, live data, operator workflow | Recovery policy and user-visible action | Fresh evidence, bounds, acceptance oracle |
|---|---|---|---|---|
| 4. Mutation failure before wire | Caller Busy/local validation creates no record; worker pre-wire failure may retire only a proven-unsent record after durable removal | Observations unchanged; workflow stays Awaiting on caller Busy or shows local Failed; never Accepted | AR only if a real transport fault occurred. MR: never. State explicitly “not submitted” only when zero-byte proof exists | No Runtime evidence is needed for proven-unsent work; journal retirement must succeed. Test zero mutation bytes and no Pending claim |
| 5. Mutation transmitted, admission unknown | Exact pre-wire record becomes Ambiguous; same boot/scope retained; sequence remains blocked where required | Connection stale; workflow Ambiguous; no optimistic projection; cached live points stale | AR: one episode after transport loss. MR: never. Offer Check Status and eligible manual Exact Retry | `operation_status` or exact dedup reply. Eight records/five-second request. Test restart from durable record |
| 6. Mutation Accepted then connection lost | Accepted record and identity remain durable; admitted Runtime work outlives client | Workflow Ambiguous/Accepted-needs-reconciliation; observations stale; no terminal success claim | AR: one episode. MR: never. Prefer Check Status; Exact Retry remains exact and manual | Terminal `operation_status`/dedup outcome plus rebuild for Fresh. Test no second execution |
| 23. `operation_status` terminal completed | Worker changes exact record to Completed and publishes RecoveryState | Workflow/recovery display Completed; Runtime projections still change only via query/event; live state unaffected | AR: no. MR: never. Permit acknowledgement; record may be boundedly retained/retired | Terminal Application result is outcome evidence; projection barrier/event is state freshness evidence. Test model/worker projection equality |
| 24. `operation_status` terminal failed | Worker changes record to Failed; preserves structured error/outcome | Workflow visibly Failed; no projection mutation; controls may resume only when other recovery gates clear | AR: no. MR: never. No automatic resubmit | Terminal Application result plus current observations. Test exact error and bounded retirement |
| 25. `operation_status` says `outcome_unknown` | Record remains Pending/Ambiguous/Accepted as previously known; sequence is not invented | Workflow stays Unknown/reconciliation-required | AR: no. MR: never. Keep Check Status manual; Exact Retry may be offered only under same boot/scope and exact record | A later terminal status or exact dedup reply. One outstanding status. Test no state downgrade to failed/completed |
| 26. Exact retry requested | Worker looks up immutable record, verifies current boot/scope and no same identity in flight, then sends stored op/args/request_id | Workflow shows Submitted/Accepted/terminal distinctly; no editable payload | AR: no solely for click. MR: never; this is an explicit user action. Disable duplicate click while pending | Authoritative dedup response. Pending pool eight/request five seconds. Test semantic op/args/request_id identity and no second execution |
| 27. Retry rejected after boot/scope change | Worker refuses `recovery_identity_not_attached` or missing/quarantined record; journal is not rewritten as a new identity | Workflow remains Unknown with visible local rejection; no projection change | AR: no. MR: never. Do not offer or enable retry when identity mismatch is known | Only a reviewed disposition can clear the old uncertainty. Test zero bytes and unchanged quarantine |

### Persistence, process, pressure, and lifecycle faults

| # / trigger | Worker, session, scope, journal | Model, live data, operator workflow | Recovery policy and user-visible action | Fresh evidence, bounds, acceptance oracle |
|---|---|---|---|---|
| 14. Recovery journal missing | Startup recovery is empty; no session is inferred; hello remains authoritative for new scope/sequence | Visible recovery count zero; normal observational rebuild; no fabricated historical outcome | AR: normal connection policy. MR: never. Missing file is not an error by itself | Hello/barrier for Fresh. Test no file creation until mutation record is needed |
| 15. Journal corrupt | Load rejects JSON/schema/version/bounds; worker enters journal-failed mode and keeps file untouched | Recovery problem visible; mutation controls blocked; observations may be rebuilt but warning persists | AR may connect for observation. MR: never. No auto-delete/default overwrite | Fresh observations require barrier; mutation authority remains blocked. 64 KiB/eight records. Test corrupt bytes preserved |
| 16. Journal read failure | Bounded open/metadata/read error becomes journal-failed | Same as corrupt: visible recovery uncertainty and disabled mutations | AR may observe. MR: never. Show actionable path/error without dumping unbounded OS text | Successful later restart/read plus authoritative reconciliation; test access-denied/read error |
| 17. Journal write/durability failure | Pre-wire save failure sends nothing; later state/retire failure retains in-memory exact state, sets journal-failed and blocks sequencing | Visible recovery problem; workflow is local Failed if unsent or Unknown if prior send; observations are not authority for recovery | AR only for independent transport fault. MR: never. No further mutation | Successful durable write plus status resolution in a later reviewed flow. Test pre-wire zero bytes and post-admission fail closed |
| 18. Workbench clean close | Stop accepting commands, close socket best effort, persist ambiguity, stop worker within three seconds; Runtime untouched | Model ends Stopping/Stopped if consumed; presentation/journal remain client-owned; active workflow is not called Completed | AR: no. MR: never. Exit is not experiment control | Process exit after worker join; test no forbidden Runtime operation and Runtime remains alive |
| 19. Workbench forced termination | OS closes socket and releases workspace mutex; last successfully replaced journal is recovery evidence | No final UI transition is claimed; Runtime and experiment continue; presentation last save is independent | AR: impossible in dead process. MR: never. Next launch loads journal visibly | Restart hello plus manual status/retry. Test Runtime alive and second process can acquire mutex |
| 20. Restart with retained recovery record | Worker validates bounded journal but sends nothing; hello classifies same boot/scope vs quarantine | RecoveryState and reconciliation warning visible; live/model rebuild starts independently; workflow has no resurrected GUI command ID | AR: at most one retained-scope startup episode. MR: never. Offer status/exact retry only after valid hello | Same-session hello and authoritative status. Test exact payload survives process restart |
| 28. Command/in-flight/update capacity pressure | Command 32 returns Busy; ninth pending rejected; recovery eight rejects before wire unless terminal retirement persists; update 64 closes transport | No fake action for untracked query rejection; action history remains <=64; overflow makes model non-Ready | AR only for update-overflow continuity loss. MR: never. Show local Busy/capacity cause | Normal queue drain or new hello/barrier. Deterministic tests for every bound and no unbounded collection |

## Required M14.6B process acceptance plan

The later implementation must provide deterministic unit/state-machine tests first
and then these real-process or fault-injection scenarios:

| Scenario | Required proof |
|---|---|
| A. Runtime absent, later available | Initial attempt ends visibly Disconnected without a loop; manual Connect later reaches hello and Fresh |
| B. Workbench disconnect/reconnect | Explicit Disconnect performs no automatic reconnect and Runtime continues; manual retained-scope Connect rebuilds Fresh |
| C. Forced Workbench termination | Runtime/controller/Recorder continue, workspace mutex releases, journal remains usable |
| D. Runtime killed/restarted | Boot A becomes stale/quarantined, no old retry occurs, boot B uses a clean live epoch and complete rebuild |
| E. Mutation ambiguity and Workbench restart | Exact pre-wire journal survives, restart sends nothing, identity/payload remain visible |
| F. Status resolves ambiguity | One manual `operation_status` transitions worker and model projection to terminal without replay |
| G. Exact retry identity | Manual retry sends the retained semantic op/args/request_id only and cannot execute twice |
| H. Event gap | Connection stays usable, old live epoch clears, one subscription rebuild reaches Fresh without reconnect |
| I. Ordered-update overflow | Model becomes non-Ready, exactly one bounded retained-scope episode occurs, hello/barrier restores Fresh |
| J. Corrupt journal | File remains untouched, error is visible, observational connection may work, every mutation workflow is blocked |

Additional focused oracles must cover disconnect during a query, before/after first
mutation byte, Accepted-before-loss, terminal failure, `outcome_unknown`, scope race,
scope/instance invalidation, partial rebuild loss, malformed/oversized frames, each
mailbox limit, and explicit Disconnect while recovery records exist.

Tests should use response/state predicates, scripted peers, journal fault injection,
and process exit handles. Sleeps are not correctness barriers. Runtime source must not
be changed to manufacture Workbench faults.

## M14.6B implementation gaps and proposed slices

The source audit identifies these later implementation needs; none is authorized by
this report:

1. Generalize the one-episode automatic reattach policy from the accepted overflow
   path to unexpected transport loss while preserving explicit Disconnect as manual.
2. Add renderer-neutral, bounded Check Status and Exact Retry workflow states with
   at most one outstanding recovery action per identity.
3. Preserve a visible quarantined recovery disposition across boot/scope rejection;
   do not let clearing the live worker vector imply that uncertainty was resolved.
4. Bound stored local diagnostics to 512 UTF-8 bytes without truncating the bounded
   raw Application error envelope needed for structured display.
5. Add deterministic journal I/O fault seams and the real-process fault matrix.

A small proposed decomposition is:

```text
M14.6B1 recovery UI and one-shot operation_status
M14.6B2 exact-retry and restart/quarantine reconciliation
M14.6B3 explicit-disconnect versus bounded fault-reattach UX
M14.6B4 real-process recovery/fault acceptance
```

These names are planning aids only. M14.6B and every sub-slice remain unauthorized
until external review accepts M14.6A.

## Open questions requiring review before destructive behavior

- M14.6A intentionally does not define an in-GUI Discard/Forget operation for a
  corrupt journal or old-boot quarantine. Adding one would destroy recovery evidence
  and needs a separately reviewed confirmation/export policy.
- Parent-directory crash durability remains the honestly documented M14.3 limitation;
  M14.6 does not claim stronger filesystem guarantees.
- Repeated identical-version full-detail refresh does not itself invalidate accepted
  M14.5 authority guards; this audit does not reopen that rule.

None of these questions requires a Runtime/Application semantic change. If a future
implementation finds that session, deduplication, Runtime ownership, Recorder
durability, OutputAuthority, or the public error taxonomy must change, it must stop
for external review.

## Exact non-goals

M14.6A does not authorize production code, tests that change behavior, a second
Application owner, another subscription, automatic mutation replay, unbounded
reconnect/polling/history, Runtime shutdown UX, Runtime/API changes, new dependencies,
GUI redesign, M14.6B, M13.2, or Steel.

```text
M14.5: ACCEPTED

M14.6A:
READY FOR EXTERNAL REVIEW

M14.6B:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M14_6A_RECOVERY_FAULT_AUDIT_READY_FOR_EXTERNAL_REVIEW
```
