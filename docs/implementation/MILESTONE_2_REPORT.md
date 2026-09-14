# M2 completion report — 2026-09-14

Status: complete. Scope: deterministic virtual output authority, not physical I/O.
M1 behavior is retained. M3 implementation has not begun.

## Delivered boundary

Runtime owns a six-state per-actuator authority: Unverified, SafePending, Disarmed,
ArmedManual, ArmedAuto, FaultLatched. Each has one exclusive bounded lease, checked
epoch, one pending normal proposal, one in-flight dispatch and a reserved safe path.
Explicit profiles choose limits, safe value, lease/TTL bounds and required evidence.
Generic configuration cannot actuate. Manual and automatic producer identities share
the same validation; automatic identity does not implement a controller.

Queue/send/complete are separate deterministic commands. Final send validates the
current token, instance, epoch and exclusive deadline. Revoke invalidates authority
before requesting safe; it cannot undo an in-flight operation. Instance-scoped
dispatch IDs reject completions from other runtimes, including identical logical IDs.
Requested, sent, acknowledged and readback values are separate. Safe recovery needs
correlated evidence; fault acknowledgement does not arm. No universal zero policy.

Public additions are in `lab_core::output`, Command::Output, CommandResult::Output,
Query::Output, QueryResult::Output and Error::Output. There is no external wire API.
The private state machine is in `crates/lab-core/src/output/authority.rs`; Runtime
remains the single mutable owner. Allocation of process-local instance IDs uses one
checked atomic counter; no worker threads or shared device state were introduced.

## Acceptance evidence

The first eight M2 integration tests were committed before implementation (5545c55).
The red run failed with six missing-output-API compilation errors. Implementation
then made them pass. Subsequent review added six integration and two private tests.
One regression genuinely failed before its fix: a new explicit safe request was
lost when an older safe dispatch failed (`Output(NothingPending)`, 13 pass/1 fail).
The fix preserves the newer recovery request without accepting the old evidence.

| Contract | Executable evidence in milestone2.rs (unless noted) |
| --- | --- |
| Unverified/Disarmed deny ordinary writes; explicit nonzero safe | unverified_and_disarmed_have_no_ordinary_output_authority |
| Exclusive modes and bounded queue/in-flight capacity | one_owner_and_one_bounded_pending_proposal |
| Type, unit, finite value, range and generic-setter gate | both_modes_share_strict_validation_and_generic_setter_is_not_a_bypass |
| TTL/lease expiry and revoke before send | expiry_and_revoke_fence_queued_work_at_final_send |
| Stale epochs/results cannot restore authority | old_epoch_and_late_result_cannot_restore_authority_or_confirm_new_safe |
| Failed/ambiguous/insufficient safe evidence never means safe | safe_failure_ambiguity_and_insufficient_evidence_never_become_disarmed |
| Explicit watchdog, release, trip; pure queries | lease_watchdog_release_and_fault_are_explicit_not_query_side_effects |
| Invalid binding, profile and backwards time | invalid_profiles_time_and_unknown_output_are_rejected |
| Cross-runtime token/completion fencing | tokens_and_completions_from_another_runtime_instance_are_rejected |
| Old safe completion cannot consume new recovery | a_new_safe_request_waits_for_and_is_not_confirmed_by_the_old_safe_result |
| Normal failed/ambiguous delivery revokes without auto-rearm | normal_failure_revokes_and_requires_safe_recovery_without_rearming |
| ACK is not readback; explicit profile evidence policy | acknowledgement_policy_is_explicit_and_not_readback |
| Exclusive TTL boundary, lease clipping, overflow | proposal_deadline_is_exclusive_and_cannot_outlive_the_lease; profile_duration_and_checked_time_boundaries_are_enforced |
| Final check independent of eager queue cleanup | authority unit: final_epoch_check_protects_even_if_eager_queue_cleanup_is_missed |
| Epoch exhaustion fails closed without wraparound | authority unit: epoch_exhaustion_never_wraps_to_an_old_owner |

## Verification

All commands exited 0 on Windows:

- `cargo fmt --all -- --check`
- `cargo test --workspace`: 38 passed, 0 failed/ignored.
- `cargo test --workspace --release`: 38 passed, 0 failed/ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`: no warnings.
- `cargo run -p lab-runtime`: existing finite M1 demo, five values 20–24 Celsius,
  last three samples retained, normal exit. This is not an M2 hardware demo.
- `cargo tree --workspace --edges all`: two packages, no external dependencies.
- `git diff --check`.

Test count: 16 M1 + 5 values + 1 host + 14 M2 integration + 2 authority unit tests.
Rust source documentation is English. Workspace missing_docs is enabled and enforced
by warning-free clippy. M1 public documentation was improved in a separate commit.

## Commits

- 96e9582 — authorized plan, M2 design and initial handoff.
- 3707809 — English M1 public documentation and missing-docs lint.
- 5545c55 — M2 acceptance tests before implementation.
- 9abdd4f — deterministic authority and Runtime integration.
- 37ecdce — fencing/recovery/deadline regressions.
- 8f79150 — independent final-epoch defense and exhaustion tests.

This report and final contract/handoff are subsequent documentation commits, visible
in `git log`; they do not add later-milestone production code.

## Limitations and deviations

No transport, controller, Lua, Babashka service, recorder or GUI was implemented.
The M1 executable intentionally remains finite. Time/watchdog progress requires
explicit host commands; this slice does not guarantee autonomous timely shutdown.
No physical safety, hardware readback, persistent interlock, evidence-age policy or
long-soak claim is made. An unfinished simulated operation stays unresolved.

Complete is a trusted simulation input. Terminal simulated ambiguity means unknown
past effect but no later effect; real timeout cannot assume that and belongs to M3
recovery. Dispatch is an observation, not a reusable permission to send bytes.
IDs are local to this process, not durable or remote authentication credentials.

The donor remained read-only: final git status clean, HEAD unchanged at
50d3d1e3de84c650e1aa0ffbf1625044f794d315. Selected Metakon framing/register knowledge
was inspected only for the upcoming documentation contract; no donor code was run
or changed in this milestone.

The direct user instruction requires M3 design/acceptance before the first model
switch. It overrides section 16 of the later supplied IMPLEMENTATION_ROADMAP_M2_M6.md,
which says not to design M3. That file is preserved as supplied; its M5 section ends
mid-question at `Answ`. No missing later handoff policy is inferred.

## Stop

M2 is complete. [M3 design/acceptance](MILESTONE_3_DESIGN.md) is documentation only.
Update the sole special handoff file, AI_HANDOFF.md, request Astra High -> Sol High,
and stop before any M3 production code or tests.
