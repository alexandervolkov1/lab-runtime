# M14.6B2B2 Exact Retry GUI

## Scope

M14.6B2B2 adds only the GUI integration around the accepted renderer-neutral
`ExactRetryWorkflow`, `PreparedExactRetry`, and worker-owned
`ClientHandle::retry_mutation(identity)` boundary. It adds no dependency, Runtime
operation, Application DTO, journal schema, automatic reconnect, evidence discard,
quarantine reactivation, or Steel integration.

## Ownership and ordered update handling

`WorkbenchApp` owns exactly one `ExactRetryWorkflow`. It is bounded to one current
interaction and retains no attempt history. Ordered worker updates are consumed in
this order:

```text
WorkbenchModel
RecoveryStatusTracker
ExactRetryWorkflow
OperatorWorkflow
RebuildCoordinator
```

Both recovery workflows observe the worker's authoritative atomic
`RecoveryProjection`; neither modifies it. Exact Retry is absent from
`WorkbenchModel.actions`, normal operator command tracking, mutation-identity
allocation, and `PresentationDocument`.

## Renderer-neutral eligibility

Recovery rows ask `ExactRetryWorkflow::can_begin`, which delegates to the same
renderer-neutral preparation path used by `begin`. An Exact Retry is available only
for an active, unresolved, attached record while the client is Ready, journal
authority is trustworthy, quarantine is empty, and Check Status is not pending for
the identity. Completed, Failed, quarantined, stale/disconnected, boot/scope-mismatched,
or journal-blocked evidence is not offered.

`OperatorWorkflow::allows_exact_retry` permits recovery alongside an idle ordinary
workflow or an `Ambiguous` workflow for the same retained identity. It refuses a
second recovery interaction beside unrelated confirmation, submitted, or accepted
operator work.

## Confirmation and immutable submission

Clicking `Exact Retry…` calls only:

```text
ExactRetryWorkflow::begin(model, recovery_status, identity)
```

It sends no command. The confirmation displays the frozen scope, sequence,
operation, known admission, and bounded read-only retained args. It never exposes a
payload editor. Confirm remains enabled only while the complete prepared recovery
record, current Runtime boot/scope, journal/quarantine state, and per-identity status
authority remain unchanged. Confirmation calls only
`ExactRetryWorkflow::confirm`, whose narrow submitter sends the frozen identity; the
worker retrieves the immutable op, args, and request identity.

The exact warning is:

> Exact Retry resends the same request_id and the same worker-retained payload. If Runtime already admitted that identity, deduplication must not execute it twice. If it was never admitted, this may be its first execution. If Runtime no longer retains the outcome, the result may remain outcome_unknown.

A changed record or session transitions visibly to `DraftStale`; it is never rebased.
Only an explicit `Confirm Exact Retry` click can submit. Startup, hello, reconnect,
rebuild, recovery projection, reconciliation notification, `outcome_unknown`, and an
ambiguous operator workflow perform zero automatic retry submissions.

## Status/retry exclusion

`ExactRetryWorkflow::blocks_status` disables Check Status for the exact identity from
the moment its confirmation is open through every retained retry notice. Conversely,
a pending B1 Check Status request makes the shared renderer-neutral Exact Retry
preparation fail. Different identities retain independent Check Status eligibility
within the accepted worker bounds. The worker's accepted mutual-exclusion checks
remain authoritative.

## State wording

The GUI distinguishes `Submitted`, `Accepted`, `Completed`, `Failed`,
`OutcomeUnknown`, `LocalFailure`, `ApplicationFailure`, `Interrupted`, and
`DraftStale`. Submitted and Accepted make no completion or physical-effect claim.
Completed and Failed describe only the retained Application operation outcome.
`OutcomeUnknown` explicitly preserves unresolved original evidence.
`ApplicationFailure` describes the retry request and does not reclassify the original
mutation. `Interrupted` states that no automatic retry occurred. Terminal/notable
states require an explicit acknowledgement and then return the single workflow to
Idle.

## Original operator reconciliation

The already accepted `OperatorWorkflowState::ReconciledTerminal` is now presented as:

```text
Previously ambiguous operator action reconciled: Completed
Previously ambiguous operator action reconciled: Failed
```

The exact retained scope/sequence is displayed. The state comes only from the
authoritative terminal recovery projection, fabricates no Application envelope, and
can be acknowledged to Idle. It works whether B1 Check Status or Exact Retry caused
the worker projection to become terminal.

## Deterministic tests

Focused tests prove:

- unresolved active rows offer Exact Retry while terminal and quarantined rows do not;
- Ready/session/journal/status rules use the renderer-neutral preparation path;
- a pending Check Status blocks retry for the same identity while another identity
  remains independent;
- opening confirmation sends nothing and exposes exact read-only identity/op/args and
  the fixed warning;
- changed evidence produces `DraftStale` with zero sends;
- Confirm emits exactly one retained identity and duplicate Confirm emits no second
  command;
- Submitted/Accepted retry blocks Check Status for that identity;
- terminal, unknown, local, Application, interrupted, and reconciled-terminal wording
  preserves the authority boundary;
- Exact Retry creates no model action entry, and the accepted core repeated-cycle test
  proves there is no retry history growth;
- GUI source contains neither raw mutation calls nor direct `retry_mutation` calls;
- the accepted finite worker shutdown path sends no Runtime shutdown, controller,
  Recorder, mutation, or recovery replay action.

All accepted M14.2-M14.6B2B1 tests remain in place.

## Verification

Final evidence is recorded after running the complete requested gate:

```text
cargo fmt --all -- --check                                      PASS
cargo test --workspace --locked                                PASS
cargo test --workspace --release --locked                      PASS
cargo clippy --workspace --all-targets --locked -- -D warnings PASS
cargo test -p lab-workbench --locked (three consecutive runs)  PASS, 130/0/4 each
four real Runtime Workbench process acceptances                 PASS, 4/0
native Glow GUI smoke                                           PASS
git diff --check                                                PASS
```

The workspace declares 656 tests. Debug and release gates each ran all 650 enabled
tests successfully and retained six explicit opt-in ignores. The four ignored
Workbench real-process tests were executed separately and passed. The pre-existing
Runtime rotation and soak workloads remain unchanged and ignored.

The Glow smoke reported `status=pass` with Glow, observed the native window and
minimize/restore path, reached Fresh after retained-scope reattach, retained live
signal and Reference evidence, observed confirmation/accepted/completed/authoritative
refresh states, disabled stale controls, and proved Runtime survival after clean and
forced Workbench termination.

## Non-goals

M14.6B2B2 does not implement automatic transport-fault reattach, polling, automatic
status or retry, Discard/Forget, quarantine clearing/reactivation, new recovery
persistence, Runtime/Application changes, M14.6B3-B4, or Steel.

## Review state

```text
M14.6B2B1: ACCEPTED

M14.6B2B2:
READY FOR EXTERNAL REVIEW

M14.6B3:
NOT AUTHORIZED

M14.6B4:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M14_6B2B2_EXACT_RETRY_GUI_READY_FOR_EXTERNAL_REVIEW
```
