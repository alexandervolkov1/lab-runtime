# M14.6B2B1 Exact Retry core and evidence lifecycle

## Review boundary

M14.6B2B1 adds only renderer-neutral and worker-owned Exact Retry semantics. The
accepted M14.6B2A implementation is
`f945d910cfeabbe8552634c53d95016e79a6eafa`; its acceptance/B2B1 authorization
commit is `f8b247691aaa8d8bc27e4d5cce163b5b33235956`.

This slice adds no dependency, Cargo change, Runtime/core change, Application
operation or DTO, Exact Retry GUI button, automatic reconnect, Discard/Forget, or
Steel integration.

## Worker authority and immutable payload ownership

The Application client worker remains the sole owner of active recovery records,
the journal, the socket, correlation, and retry wire construction. A retry command
contains only `MutationIdentity`. `queue_retry` finds the exact active
`RecoveryRecord` and constructs the request from its retained `request_id`, `op`,
and `args`; no caller can supply or replace a payload.

The worker rejects before wire emission unless all of these hold:

- connection state is `Ready` and hello exists;
- the recovery journal is trustworthy and quarantine is empty;
- an exact active record exists for the identity;
- record boot and scope match the attached hello;
- admission is `Pending`, `Accepted`, or `Ambiguous`;
- no status, retry, or mutation exchange for the same identity is in flight.

Completed and Failed records are evidence, not retry candidates. Quarantined
records are never retry authority.

## Renderer-neutral prepare/confirm guard

`PreparedExactRetry` freezes the complete projected recovery record plus attached
boot and scope. `ExactRetryWorkflow::confirm` repeats every eligibility check and
requires the active record to equal the frozen record exactly. A change to boot,
scope, identity, admission, operation, or arguments produces `DraftStale` and zero
submission. Confirmation calls only `retry_mutation(frozen.identity)`.

The fixed confirmation warning states that Exact Retry resends the same identity
and worker-retained payload; an already admitted identity relies on Runtime dedup,
an unadmitted request may execute for the first time, and an evicted outcome may
remain `outcome_unknown`. It makes no claim of a safe retry, guaranteed completion,
non-execution, or cancellation.

The workflow has one current state and no attempt history:

```text
Idle
AwaitingConfirmation
Submitted
Accepted
Completed | Failed
OutcomeUnknown
LocalFailure | ApplicationFailure | Interrupted | DraftStale
```

Any retained local failure text or Application error code is bounded to 512 UTF-8
bytes with character-boundary truncation and a visible marker.

## Terminal evidence retention

A terminal retry no longer removes its recovery record. The worker changes the
same exact record admission to `Completed` or `Failed`, durably replaces that record
in the journal, and publishes the authoritative `RecoveryProjection`. The existing
eight-record bounded terminal-retirement policy may later retire old terminal
evidence only when capacity is required for a new mutation. No unbounded history
was added.

Persistence occurs before the terminal projection is published. Under update-queue
pressure the durable terminal record therefore remains available even if the GUI
does not receive that projection before the connection becomes Stale. Restart
reloads the same identity, operation, arguments, and terminal admission and emits no
automatic status, retry, or mutation request.

## Public errors and outcome_unknown

A `PublicError` for `Purpose::RetryMutation` never removes or reclassifies the
retained record. In particular, `outcome_unknown` means only that Runtime no longer
has the retained outcome: the exact record and its prior admission remain unchanged,
and the renderer-neutral workflow shows `OutcomeUnknown`. It is not translated to
Failed, Completed, not-executed, cancelled, or safe-to-forget. Other retry public
errors likewise retain the evidence and are displayed as bounded application
failures.

## Status/retry mutual exclusion

The worker has one identity-based recovery-action check shared by
`operation_status` and Exact Retry. A pending status rejects retry for the same
identity; a pending retry rejects status and duplicate retry. Different identities
continue to use the accepted global in-flight bound. There is no polling or
automatic submission.

The renderer-neutral prepare path also checks the B1 status tracker. Retry is not
added to `OperatorWorkflow`, `WorkbenchModel.actions`, or request-sequence
allocation.

## Ambiguous operator workflow reconciliation

The worker-owned `RecoveryProjection` is the single reconciliation authority. An
`OperatorWorkflow::Ambiguous` state changes to `ReconciledTerminal` only when the
active projection contains the exact matching identity with `Completed` or `Failed`
admission. The state contains no fabricated Application envelope and can use the
existing acknowledge transition to return to Idle.

Pending, Accepted, Ambiguous, `outcome_unknown`, quarantine, and a terminal record
for another identity do not resolve the workflow. This rule applies equally after
B1 Check Status or Exact Retry because both converge on the same worker projection.

### Pre-Accepted identity correlation

`OperatorWorkflow::Submitted` has no mutation identity before the worker publishes
`MutationAccepted`. If transport continuity is lost first, the workflow initially
becomes `Ambiguous { identity: None }`. A later worker-owned
`ReconciliationRequired` or active `RecoveryProjection` may supply that identity
only when exactly one unresolved active recovery record has both:

```text
record.op   == frozen prepared.operation
record.args == frozen prepared.args
```

Eligible admissions are only `Pending`, `Accepted`, and `Ambiguous`. Correlation is
therefore exact immutable op+args and only when unique. Zero matches, multiple exact
matches, terminal records, and quarantined records retain `identity: None`; no
latest-record, sequence, command-ID, or operation-only heuristic exists. An identity
already learned from `MutationAccepted` remains authoritative and is never replaced
by this search.

Consequently not every Ambiguous workflow receives an identity. Lack of one unique
worker-owned match deliberately leaves it unresolved and triggers no automatic
Check Status, Exact Retry, or mutation.

## Bounds and pressure safety

- active recovery records: at most 8;
- quarantined recovery records: at most 8;
- in-flight exchanges: at most 8;
- renderer-neutral Exact Retry workflows: exactly one current workflow;
- status and retry for one identity: at most one combined recovery action;
- Exact Retry diagnostic text: at most 512 UTF-8 bytes;
- attempt history: none;
- action-history entries created by retry: none.

The terminal-pressure regression fills the ordered update queue, completes an Exact
Retry, proves the terminal record is already durable, applies every visible update,
and proves the model never loses the prior recovery evidence. Restart then reloads
the terminal record without emitting a request.

## Deterministic tests

The focused tests cover:

- exact immutable record capture and identity-only submission;
- stale confirmation on admission, op, args, identity, boot, or scope change;
- renderer-neutral and worker rejection for terminal, quarantine, journal failure,
  and status-in-flight cases;
- status/retry mutual exclusion and duplicate retry single-wire behavior;
- admission and terminal UI states changing only through `RecoveryProjection`;
- durable Completed and Failed terminal retention and restart reload;
- evidence retention for `outcome_unknown` and other public errors;
- terminal completion under ordered-update pressure;
- exact-identity-only reconciliation of an Ambiguous operator workflow;
- unique exact pre-Accepted `op+args` identity recovery after transport loss or
  `ReconciliationRequired`;
- rejection of inexact, non-unique, terminal, and quarantined correlation sources;
- preservation of an identity already learned from `MutationAccepted`;
- non-resolution by pending admission, mismatch, quarantine, or
  `outcome_unknown`;
- acknowledge from reconciled terminal state to Idle;
- 100 repeated failure cycles retaining one bounded state and no action history.

## Real Runtime evidence

The existing reconnect/reconcile process acceptance continues to prove retained-scope
reattach, one-shot status reconciliation, next-sequence continuity, replay, and
Runtime independence. A focused Recorder lifecycle oracle disconnects after
authoritative admission, reattaches, and invokes the worker-owned Exact Retry. The
same retained request identity and payload return the retained run outcome; the
authoritative Recorder projection exposes the same run identity, proving no second
domain execution.

A focused opt-in process oracle advances the retained Runtime outcome window until
the original identity is evicted, then sends the exact retained identity and
payload. Runtime returns `outcome_unknown`; authoritative Reference revision and
target remain unchanged, proving no second domain execution. The Workbench keeps the
original exact record and prior admission.

## Verification

Final command results are recorded after the complete gate below:

```text
cargo fmt --all -- --check                                      PASS
cargo test --workspace --locked                                PASS
cargo test --workspace --release --locked                      PASS
cargo clippy --workspace --all-targets --locked -- -D warnings PASS
cargo test -p lab-workbench --locked (three consecutive runs)  PASS, 125/0/4 each
real Runtime Workbench process acceptances                      PASS, 4/0
native Glow GUI smoke                                           PASS
git diff --check                                                PASS
```

The workspace declares 651 tests. Debug and release gates each ran all 645 enabled
tests successfully and retained six explicit opt-in ignores. The four ignored
Workbench real-process tests were executed explicitly and passed; the pre-existing
Runtime rotation and soak workloads remain unchanged and ignored.

The Glow smoke reported `status=pass`, selected Glow, observed the native window and
minimize/restore, reached Fresh after reconnect, retained live signal/Reference and
operator lifecycle evidence, and proved Runtime survival after both clean and forced
Workbench exit.

## Non-goals retained

M14.6B2B1 does not implement the Exact Retry button or GUI confirmation, automatic
transport-fault reattach, automatic status/retry/mutation replay, quarantine
reactivation or clearing, Discard/Forget, a new journal schema, M14.6B2B2–B4, or
Steel.

## Review state

```text
M14.6B2A: ACCEPTED

M14.6B2B1:
READY FOR FINAL EXTERNAL RE-REVIEW

M14.6B2B2 Exact Retry GUI:
NOT AUTHORIZED

M14.6B3–B4:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M14_6B2B1_PRE_ACCEPT_IDENTITY_READY_FOR_EXTERNAL_REVIEW
```
