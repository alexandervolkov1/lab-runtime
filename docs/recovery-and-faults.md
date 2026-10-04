# Recovery and fault handling

This is the operator and client guide for lost continuity, uncertain operations,
Recorder faults, resource reconnects, and retry decisions. The exact Runtime wire
shapes remain in the [Application API reference](api/README.md); the separate
[Workbench API reference](workbench-api.md) defines its local boundary.

The governing rule is:

```text
absence of evidence != evidence of non-execution
```

In particular, a closed socket, timeout, missing ACK, or missing readback is not
proof that an output-affecting action did not occur.

## Start with the authority boundary

- Runtime owns experiment state, mutation identity, device/resource generations,
  output authority, and Recorder.
- A direct client owns its connection correlation and retains any exact request it
  may deliberately need to reconcile.
- Workbench owns presentation and a bounded durable client recovery journal. That
  journal is evidence about its Runtime client; it is not Runtime authority.
- SQLite contains Runtime-written scientific/audit history. It is not a live control
  surface.

Treat three questions separately:

1. Did Runtime admit and finish this mutation?
2. Is my current state projection continuous and fresh?
3. What current physical evidence exists?

An answer to one does not answer the other two.

## Direct Runtime mutation recovery

Runtime mutation identity is `{scope,seq}`. The scope is issued by one Runtime boot;
`seq` is consecutive within that scope. The TCP/WebSocket connection and `msg_id`
are not mutation identity.

```text
send mutation
     |
     +-- accepted seen --> retain {scope,seq} --> status
     |
     `-- no accepted --> preserve exact request; admission is unknown
```

Use this decision model after continuity loss:

| Situation | What is known | Safe next action |
|---|---|---|
| Socket lost before `accepted` was observed | The client does not know whether admission occurred. | Keep the exact request. Reattach the same scope when possible, then query `operation_status` for the exact identity. |
| Socket lost after `accepted` | Runtime admitted that identity and payload. Delivery loss does not cancel it. | Reattach and query status; do not submit a new identity for the same physical intent. |
| Scope reattaches | The same in-process scope and high-water mark still exist. | Use `operation_status`; continue with the returned `next_seq` only for new independent work. |
| Scope is unknown | Runtime cannot attach that process-local scope. | Quarantine the old evidence. Do not transplant its sequence into a new scope. |
| `boot_id` changed | The old Runtime process/session store is gone. | Rebuild current state and keep old operation evidence explicitly unresolved unless durable/physical evidence settles it. |
| Status is `completed` | Runtime retained a terminal completed outcome. | Consume the result, then separately refresh current state if needed. |
| Status is `failed` | Runtime retained a terminal failed outcome. | Use its code/result; failure still may not prove absence of physical effect for a post-send output fault. |
| Status is `outcome_unknown` | No retained operation-state record is available for that identity. | Do not infer admitted/not-admitted or executed/not-executed. Rebuild state and use domain/physical evidence or operator judgment. |
| Exact request is retained and the same scope is attached | The client can reproduce the exact typed mutation. | A deliberate exact resubmission may recover a retained result or become first admission; see below. |

### Exact resubmission

Exact resubmission means all three remain identical:

- the Runtime request identity `{scope,seq}`;
- the operation name;
- the normalized typed arguments.

If Runtime retains that identity and equal payload, it returns the retained state
without a second execution. If the original request never reached admission and the
identity is still the scope's next admissible sequence, the resubmission may become
the first admission. A different payload for a retained identity is a conflict.

An edited value, reconstructed arguments, a new scope, or a new sequence is a new
operation, not exact resubmission. Runtime has no automatic mutation retry. Exact
resubmission is a deliberate client/operator workflow and is unsafe as a generic
"retry on socket error" rule.

### Two `outcome_unknown` forms

These have different wire roles but neither supplies physical certainty:

```json
{"v":1,"msg_id":"status-1","type":"result","result":{"state":"outcome_unknown"}}
```

An `operation_status` result with `state:"outcome_unknown"` says only that Runtime
has no retained operation-state evidence for the queried identity. Status lookup
does not prove whether it was admitted or executed.

```json
{"v":1,"msg_id":"m-1","type":"error","accepted":false,"code":"outcome_unknown","category":"invalid_request","retryable":false,"resync_required":false,"message":"<bounded message>"}
```

A mutation error with `code:"outcome_unknown"` means that sequence is at or below
the scope high-water mark but its retained record is gone. Runtime will not execute
that old identity again. It still does not prove the physical outcome of the old
attempt.

## State continuity is not mutation continuity

```text
state continuity          mutation continuity
snapshot + events         request_id + status
        |                         |
        +---- not interchangeable +
```

State/projection recovery uses a current snapshot, an event cursor, `subscribe`, and
ordered events. `event_gap` means the requested cursor predates retained replay;
discard the incremental projection and rebuild from authoritative snapshots.

Mutation recovery uses scope, `{scope,seq}`, retained outcomes, and
`operation_status`. A fresh projection does not settle an uncertain mutation. A
completed mutation does not prove that an observation is still fresh or that a
physical effect occurred.

Frozen projection page tokens and Recorder history tokens are also different:

- snapshot page tokens are connection-local frozen projections;
- history page tokens and continuation cursors are connection-local Recorder query
  state;
- all are discarded on disconnect and must not be treated as durable identities.

## Workbench recovery

Workbench keeps at most the accepted bounded set of exact mutation records in its
own durable recovery journal. Each record retains Runtime boot/scope/sequence,
operation, exact arguments, and the last known admission state:

| State | Meaning |
|---|---|
| `Pending` | Exact payload was durably prepared before possible wire emission; Runtime admission is not known. |
| `Ambiguous` | Some request bytes may have been emitted before continuity was lost; admission/outcome is unknown. |
| `Accepted` | Runtime authoritatively reported admission; terminal outcome is not yet known. |
| `Completed` | Runtime reported a terminal completed outcome. |
| `Failed` | Runtime reported a terminal failed outcome. |

`Pending` and `Ambiguous` are intentionally different. Pending preserves a
pre-emission exact payload. Ambiguous means transmission may have begun, so treating
the operation as "not sent" is unsafe.

Workbench may perform one bounded retained-scope reattach episode after unexpected
continuity loss. It never automatically sends the mutation, checks status, or
performs Exact Retry. Manual **Check Status** and confirmed **Exact Retry** are
available only where retained identity and session fences permit them.

Records from a different Runtime boot, an unknown scope, or an attached boot/scope
mismatch are quarantined. Quarantine preserves uncertainty evidence but removes
retry/status authority for the current session.

For the external Workbench API:

- `workbench_id` identifies the current Workbench process;
- `recovery_generation` versions its current recovery projection;
- `recovery_get.expected` is required (explicit `null` for the first read);
- later enumeration and `lab_operation_status` use exact Workbench, generation,
  Runtime boot/scope, and active-record fences.

A Workbench restart creates a new `workbench_id` even when its workspace and recovery
journal remain readable. `recovery_generation` is not Runtime mutation sequence.
See [Workbench API recovery](workbench-api.md#recovery-get) for the exact wire form.

## Physical output uncertainty

The evidence ladder is ordered but no stage silently implies a later one:

```text
requested -> authorized -> send-start -> ACK -> READBACK
                                                |
                                                v
                                  downstream physical effect
```

- Requested is caller/controller intent.
- Authorized is a finite central `OutputAuthority` decision.
- Send-start means the first possible output byte may have reached the transport.
- ACK is protocol-response evidence.
- READBACK is a separate observed-state transaction.
- Neither ACK nor register readback proves every downstream physical effect.

A failure before send-start can establish that this attempt did not begin transport
output. After send-start, timeout or disconnect can leave the physical outcome
ambiguous. Do not blindly resend either the requested value or the safe value: either
could repeat a physical action. Preserve evidence, revoke authority through Runtime's
central path, re-establish compatible current transport/readback evidence, and require
the applicable explicit controller/operator recovery.

## Safe transition is not rearm

After transport loss, reconnect, controller fault, ambiguous output, or restart,
Runtime may establish safe evidence. That does not automatically resume a controller
or issue a new lease. Controllers follow their explicit lifecycle (for example,
Failed to acknowledged/reset Paused, then explicit Resume/Start as applicable).
Reconnect never grants output authority by itself.

## Reconnect and stale evidence

```text
old generation          new generation
send-start                    reconnect
    |                              |
late ACK --------------------------X
                               fresh probe/read
```

Runtime's resource owner creates a new binding generation on accepted reconnect and
performs compatibility/safe-evidence work before releasing normal activity. Late
old-generation READ, WRITE completion, ACK, or READBACK cannot become current
evidence. Mapping/configuration revision and authority epoch/lease fences apply in
addition where relevant. Recorder provenance keeps the generation/revision attached
to the historical evidence instead of rewriting it as current.

## Recorder failures

Recorder policy is configured at startup; see the
[Recorder guide](recorder-sqlite.md#recorder-modes-and-policy).

| Situation | Experiment/control consequence | Evidence and operator action |
|---|---|---|
| Recorder disabled | No run accepts facts; normal non-Recorder experiment behavior remains available. | `recording_status` is `unconfigured`; Recorder/history operations are unavailable. Configure a database and restart if durable history is required. |
| Database cannot open at startup | Runtime startup fails before listener readiness, for either policy. | Correct the local absolute path, permissions, compatibility, corruption, or storage problem; do not assume a Recorder-less fallback. |
| Required Recorder fails while operating | Ordinary critical control is failed closed; controllers/output enter the accepted safe/fault path and coverage closes conservatively. | Inspect `recording_status`, preserve the archive, resolve safety first, then restart/recover explicitly. |
| Best-effort Recorder fails | Recorder becomes sticky `failed`; fact admission stops and committed-prefix/coverage evidence remains truthful. Unrelated valid native acquisition/control is not stopped solely by storage failure. | Treat the run as incomplete; repair storage and start a new process/archive workflow. Do not claim missing facts were recorded. |
| History job fails/times out | The admitted history operation terminal-fails; experiment authority is unchanged. | Release/restart the selection as a new independent read if appropriate. Do not replay an old mutation identity blindly. |
| Archive identity/filter mismatch | No page is returned for the mismatched selection. | Refresh `recording_status`/run selection and issue a new correctly fenced history request. |
| Archive is corrupt/incompatible | Open validation fails; no completeness claim is made. | Preserve the file and investigate offline; do not bypass validation or write through external SQL. |
| Shutdown seal/flush fails | Process reports cleanup failure; committed prefix may remain, but clean completeness is not established. | Preserve failure/coverage evidence and inspect after reopen. Do not describe the run as cleanly sealed. |

`best_effort` never means errors are hidden. `required` does not mean every unrelated
subsystem instantly exits; it means Recorder failure participates in the central
fail-closed safety path.

## Transport and device faults

Keep these evidence domains separate:

| Fault | Observation availability | Operation/physical consequence |
|---|---|---|
| Resource unavailable or reconnect fails | Signals become unavailable/stale. | No new authority is created; explicit reconnect/recovery is required. |
| Queue/transaction timeout | No fresh accepted sample from that transaction. | If output had not started, the attempt can fail before send; after send-start it can be ambiguous. |
| Malformed response | No false `Good`, ACK, or readback is published. | Protocol error does not prove device state. |
| Disconnect | Current resource generation goes offline. | Active control fails closed; late completions are fenced. |
| Compatibility probe fails | Replacement remains quiesced/offline. | Acquisition/output is not released. |
| Readback mismatch | Requested/ACK evidence remains distinct from observed state. | Treat required evidence as unsatisfied; do not auto-rearm or blindly retry. |

## Process and connection loss

| Loss | What survives | What does not survive |
|---|---|---|
| Direct Runtime client socket | Runtime experiment, admitted operations, Recorder, and retained scope/outcomes within their finite TTL/bounds. | Connection `msg_id`, subscriptions, snapshot/history pages and history cursors. |
| Runtime process restart | SQLite committed archive prefix; external client/Workbench evidence kept outside Runtime. | Old boot's in-memory scopes, subscriptions, event replay window, operation store, controller authority, and freshness. |
| External Workbench caller socket | Workbench process, presentation, its Runtime client, journal, and admitted Runtime work. | Caller-local `call_id` correlation and undelivered caller results. |
| Workbench process restart | Runtime experiment and Recorder; persisted workspace and bounded recovery journal if valid. | Old `workbench_id`, external caller sessions/call IDs, process-local presentation/recovery generations, live projections. |

After Runtime restart, old observations are not Fresh. After Workbench restart, a
caller performs a new hello and initial `recovery_get` with `expected:null`.

## Shutdown

Normal Runtime shutdown stops admitting new work, drives controllers/output through
the bounded safe path, settles/closes transports, drains and seals Recorder, and then
returns a terminal shutdown outcome where delivery remains possible. A clean Recorder
close requires the terminal boot seal, no outstanding records, and no Recorder error.

Abnormal process death is different. SQLite may recover a committed WAL prefix, but
active runs/intervals reopen as interrupted with conservative unknown-tail coverage.
Ctrl+C or process exit is not proof that every external physical action was reversed.

Workbench shutdown first stops its external endpoint and then its existing Runtime
client. It never synthesizes `runtime_shutdown`; Runtime and its experiment continue.

## Operator playbook

| Situation | Know | Do not assume | Action |
|---|---|---|---|
| Runtime socket dropped during a query | No mutation identity was created. | Response delivery or current state. | Reconnect, hello, and repeat the safe query. |
| Dropped before mutation `accepted` | Admission is unknown. | "Not executed." | Retain exact request; reattach/status, then consider deliberate exact resubmission. |
| Dropped after `accepted` | Runtime admitted the identity. | Terminal result or current physical state. | Reattach and status the exact identity. |
| Status `completed` | Retained terminal domain completion. | Current freshness or physical effect. | Consume result; refresh relevant state/evidence. |
| Status `failed` | Retained terminal domain failure. | No post-send physical effect. | Follow code/result and output evidence. |
| Status `outcome_unknown` | No retained status evidence. | Admitted, unadmitted, executed, or not executed. | Rebuild and use domain/physical evidence or operator decision. |
| Runtime restarted | New boot boundary. | Old scope, cursor, authority, or freshness. | New hello; quarantine old evidence; rebuild snapshots/subscription. |
| `event_gap` | Incremental replay continuity is lost. | Mutation outcome changed. | Resnapshot/rebuild, then resubscribe. |
| Observation stale/unavailable | No fresh accepted measurement. | Device unchanged. | Diagnose resource; do not run control on stale input. |
| Required Recorder failed | Critical control is fail-closed; coverage is incomplete/unknown. | Later rows are durable. | Resolve safety, preserve archive/evidence, then restart explicitly. |
| Best-effort Recorder failed | Experiment may continue; Recorder does not. | Missing data exists elsewhere. | Mark coverage incomplete and repair before a new recording workflow. |
| Resource reconnected | New binding generation exists after accepted probe. | Old completions or automatic rearm. | Wait for fresh current evidence; recover controller explicitly. |
| Ambiguous output after send-start | Bytes may have reached device. | Failure or success. | No blind retry; reconcile via readback/device/operator procedure. |
| Readback mismatch | Observed state differs from required evidence. | ACK proved effect. | Keep authority failed closed and investigate. |
| Workbench record quarantined | Evidence belongs to another continuity boundary. | Status/retry authority in current session. | Preserve and resolve separately; do not transplant identity. |
| Presentation `revision_conflict` | Workbench document changed since expected revision. | Runtime mutation failure. | Fetch `presentation_get`, reapply intent to current document if still desired. |

## Retry and resynchronization taxonomy

The public `retryable` bit classifies an error; it is not permission to replay a
mutation automatically.

- **Repeat safe query:** reconnect if needed and ask for current authoritative state.
- **Resynchronize projection:** discard stale pages/events, take new snapshots, then
  subscribe from the returned barrier.
- **Reattach scope:** use the same Runtime boot/scope before status or exact retry.
- **Query operation status:** use a known exact `{scope,seq}`.
- **Exact resubmission:** manual, same identity/op/normalized args, only with retained
  exact payload and appropriate continuity.
- **New independent operation:** allocate the advertised next sequence only when the
  new intent is genuinely independent, not a disguised retry of uncertain output.
- **Operator intervention:** required for unresolved physical ambiguity, incompatible
  restart/reconnect evidence, or safety policy that cannot establish current state.

## Error examples

Runtime synchronous failures use the DOC-4 error envelope. For example:

```json
{"v":1,"msg_id":"start-1","type":"error","accepted":false,"code":"recording_unavailable","category":"recording_unavailable","retryable":true,"resync_required":false,"message":"Required recording service is unavailable."}
{"v":1,"msg_id":"reconnect-1","type":"operation","request_id":{"scope":"<scope>","seq":"7"},"state":"failed","code":"transport_unavailable","category":"transport_unavailable","retryable":true,"resync_required":false,"message":"The required transport is unavailable."}
{"v":1,"msg_id":"history-1","type":"operation","request_id":{"scope":"<scope>","seq":"8"},"state":"failed","code":"timeout","category":"timeout","retryable":true,"resync_required":false,"message":"The operation reached its bounded deadline."}
{"v":1,"msg_id":"sub-1","type":"error","accepted":false,"code":"event_gap","category":"protocol_error","retryable":false,"resync_required":true,"message":"Incremental client state is no longer valid; resynchronization is required."}
{"v":1,"msg_id":"hello-2","type":"error","accepted":false,"code":"instance_changed","category":"protocol_error","retryable":false,"resync_required":true,"message":"Incremental client state is no longer valid; resynchronization is required."}
```

Workbench has its own envelope and codes. A stale UI edit is correlated by
`call_id`, for example:

```json
{"v":1,"type":"error","call_id":"ui-2","error":{"domain":"workbench","code":"revision_conflict","message":"presentation expectation is stale","retryable":false,"resync_required":true}}
```

Do not compare Runtime and Workbench error text. Use the code/category/fences defined
by their respective API references.

## Related documentation

- [Recorder and SQLite archive](recorder-sqlite.md)
- [Safety and failure behavior](safety-and-failures.md)
- [Runtime mutation, event, and recovery protocol](api/events-mutations-and-recovery.md)
- [Runtime errors and limits](api/errors-and-limits.md)
- [Workbench recovery UI](workbench.md#recovery-and-reconciliation)
- [Workbench external recovery API](workbench-api.md#recovery-get)
- [SimpleDevice reconnect/output rules](simple-device.md#reconnect-and-stale-evidence)
- [Native-driver fault obligations](developer/full-driver-tutorial.md#step-12-failure-cleanup)
