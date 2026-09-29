# M14.6B1 — recovery UI and one-shot operation status

## Status and scope

```text
M14.6A: ACCEPTED

M14.6B1 recovery UI + one-shot operation_status:
READY FOR EXTERNAL REVIEW

M14.6B2–B4: NOT AUTHORIZED

M13.2 Steel: BLOCKED / NOT AUTHORIZED

STATUS: M14_6B1_RECOVERY_STATUS_UI_READY_FOR_EXTERNAL_REVIEW
```

The accepted recovery/fault audit is commit
`4e18801930939404a8e856521b86c55c20d71dc9`; its acceptance and this slice's
authorization are recorded by commit
`9d805cc9000552812dc1bad25c39942a936fc0e7`.

M14.6B1 adds only renderer-neutral presentation of worker-owned recovery records,
manual one-shot `operation_status` submission, bounded caller-local correlation,
and a compact GUI section. It changes no Runtime or Application semantics, Cargo
dependency, transport behavior, durable recovery ownership, or mutation identity.

## Ownership

The existing Application client worker remains the only owner of the socket,
`msg_id`, retained scope, mutation sequencing, recovery records, recovery journal,
`operation_status` wire request, and authoritative `RecoveryState` projection.

`RecoveryStatusTracker` owns only bounded UI correlation for a manual status query:
the exact `MutationIdentity`, its caller-local status command ID while outstanding,
and one typed current notice. It cannot mutate or replace a recovery record. The
Workbench model continues to replace its recovery projection only from
`ClientUpdate::RecoveryState`.

Widgets do not build arbitrary queries. The tracker accepts an exact retained
identity and invokes only the existing `ClientHandle::operation_status(identity)`
boundary. Status queries do not enter `OperatorWorkflow`, operator action history,
or mutation sequence allocation.

## Eligibility and attachment

Check Status is enabled only when all of these are true:

- the client connection is `Ready` and an authoritative hello exists;
- hello advertises `operation_status`;
- the recovery journal has no visible failure;
- record and hello have the same boot and retained scope;
- admission is `Pending`, `Accepted`, or `Ambiguous`;
- no request is already pending for the exact identity.

`Completed` and `Failed` records remain visible under the worker's existing
retention policy but are not query candidates. No hello, boot mismatch, scope
mismatch, missing operation, stale connection, journal failure, and terminal state
are distinct renderer-neutral eligibility values. No quarantine semantics are
inferred from an ineligible record.

## Bounds and failure behavior

- The tracker holds at most `MAX_IN_FLIGHT == 8` identity slots, matching the
  authoritative recovery-record bound.
- At most one status request is outstanding per exact identity. Different retained
  identities may be checked concurrently within the worker's existing bounds.
- A slot contains only current typed state; it is replaced in place and removed when
  its authoritative recovery record disappears. There is no request history.
- A retained local diagnostic is at most 512 UTF-8 bytes. Truncation occurs at a
  character boundary and is visibly marked.
- The GUI drains the already accepted bounded worker update queue. M14.6B1 adds no
  intermediate event queue.

Submission failure is local and does not alter known admission. A correlated
`LocalRejected` clears pending state and exposes a bounded local failure. Transport
loss, a transport-lost resnapshot, or a non-ready terminal client state changes a
pending status request to `Interrupted`; it never resubmits it.

## Reply and `outcome_unknown` semantics

An `operation_status` result clears the caller-local pending state. `accepted`,
`completed`, and `failed` become visible as authoritative admission only when the
worker publishes the corresponding complete `RecoveryState`; the tracker does not
derive or write admission from the reply.

`outcome_unknown` leaves the existing recovery admission unchanged and produces a
typed visible notice. It is not translated to completion or failure. A public error
is retained as a bounded code and likewise does not modify admission.

There is no polling timer, automatic query on hello, automatic query after
reconnect, automatic mutation replay, or automatic exact retry.

## GUI

The compact Recovery / reconciliation section displays the bounded projected
records with scope/sequence identity, operation name, known admission, boot/scope
attachment, and current status-query notice. It distinguishes pending,
`outcome_unknown`, local/application failure, interrupted, terminal, and ineligible
states. Arguments are not an editable surface and Exact Retry is absent.

Nothing in this section claims physical application, output safety, device ACK,
Recorder durability, or experiment completion beyond the authoritative Application
state shown.

## Deterministic evidence

Focused tests prove:

- presentation never fabricates a GUI command ID;
- exact identities are sent once and duplicate outstanding clicks emit no command;
- different identities are bounded to eight;
- operation advertisement, connection, boot, scope, journal, and terminal-state
  eligibility;
- accepted/completed/failed display changes only after authoritative
  `RecoveryState`;
- `outcome_unknown` retains unresolved admission;
- local rejection and transport loss clear pending state with no automatic resend;
- repeated status/result cycles retain one slot, create no operator action, and do
  not grow history;
- local diagnostics retain valid UTF-8 within 512 bytes with visible truncation.

The complete focused `lab-workbench` suite passed three consecutive debug runs:
`99 passed; 0 failed; 2 ignored` in each run. The two ignored tests are the existing
opt-in real-process acceptances, which were then run explicitly and both passed
(`2 passed; 0 failed`).

## Verification

From the final M14.6B1 working tree:

```text
cargo fmt --all -- --check                                      PASS
cargo test --workspace --locked                                PASS
cargo test --workspace --release --locked                      PASS
cargo clippy --workspace --all-targets --locked -- -D warnings PASS
cargo test -p lab-workbench --locked (three consecutive runs)  PASS, 99/0/2 each
real Runtime Workbench process acceptances                      PASS, 2/0
native Glow GUI smoke                                           PASS
git diff --check                                                PASS
```

The Glow smoke observed a native Glow window, Fresh → Stale → Fresh lifecycle,
live signal points, Reference visibility, confirmed/accepted/completed mutation,
authoritative refresh, stale control disabling, post-rebuild re-enabling, finite
clean close, forced Workbench termination, and Runtime survival after both exits.

No dependency, Cargo manifest/lockfile, Runtime/core source, Application operation,
DTO, session, deduplication, or transport change was made.

## Non-goals retained

M14.6B1 does not implement Exact Retry UI, editable retained payloads, automatic
transport-fault reattach, automatic status polling, automatic mutation replay,
quarantine persistence, destructive discard/forget, M14.6B2–B4, or Steel.
