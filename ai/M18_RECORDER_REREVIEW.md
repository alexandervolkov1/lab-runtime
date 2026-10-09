# M18 Recorder: diagnostic re-review — REQUEST CHANGES

**Subsequent remediation:** the user accepted these findings and authorized three
minimal production fixes. Implementation and final verification are recorded in
[M18_RECORDER_FINAL_FIX.md](M18_RECORDER_FINAL_FIX.md). The historical diagnostic
record and its original RED evidence below are preserved; external acceptance of
the remediation is APPROVE. See [M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md)
for subsequent commit assembly and final gates.

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
HEAD: `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`; index empty.
Scope: the user's two review questions, deterministic regressions and capacity
wording only. No production changes, commits or full workspace gates in this step.

All 338 files fingerprinted at the previous handoff matched before this work.
The initial working tree was 37 modified / 11 untracked. The diagnostic delta
contains only three test modules and four coordination/documentation files;
existing production changes remain intact. `service.rs` changes are wholly inside
its existing `#[cfg(test)] mod reconnect_preparation_tests`; its production prefix
was reconstructed from the previous patch and verified unchanged.

## 1. Reference completion and concurrent facts

**Ordinary Measurement/probe concurrency is excluded on the current Application
path, but the broader single-fact assumption is false.**

- [Application admission/dispatch](../apps/lab-runtime/src/application.rs#L488)
  holds the sole mutable ServiceHost through pre-admission, acceptance, preparation,
  synchronous Reference dispatch and terminal recording. It does not run the
  scheduler, poll transports/components, yield to another mutation or reconnect
  inside that sequence. Storage and serial workers cannot append to the Core outbox.
- [HostCore::reserve_reference_operation](../apps/lab-runtime/src/host/recording.rs#L641)
  drains pre-existing Core facts before reserving the Reference token. The new
  mixed Measurement/Reference pre-admission test confirms both survive in SQLite
  separately from the completion transaction.
- [RecorderWorker::reserve_reference_operation](../apps/lab-runtime/src/recorder/worker/reference_completion.rs#L76)
  refuses while `pending_activation_generation` exists. A production reconnect's
  reserved compatibility probe is inside that activation window. The new probe
  test proves Reference admission is refused both while the token is held and
  after the probe result arrives, until the activation receipt is durable. The
  probe token is transferred exactly once; Reference succeeds after the fence
  clears, both audits persist and shutdown leaves zero credits.
- [Reference dispatch](../apps/lab-runtime/src/application/references.rs#L60),
  Core `RetuneRampReference` / `ReconfigureReference` in
  [Runtime::command](../crates/lab-core/src/runtime/dispatch.rs#L472), and pure
  `observe`/query do not themselves produce Measurement or probe facts.

The exception is **owner-side safety progress during receipt polling**:

1. Start opens Required coverage at time zero. A previous fact commits on the
   writer, but `WriterBarrier::held_after_fact_commit()` holds its receipt.
2. At logical time 2.001 s, pre-admission reserves a Reference operation and its
   accepted audit is queued. No scheduler turn is inserted inside the operation.
3. Release the writer and wait for its actual prefix-four receipt, without polling
   HostCore. This fixes receipt arrival before completion preparation.
4. [prepare_reference_completion](../apps/lab-runtime/src/host/recording.rs#L674)
   calls [poll_recorder](../apps/lab-runtime/src/host/recording.rs#L1050).
   [confirm_recording_progress](../crates/lab-core/src/runtime.rs#L780) services
   the old Required deadline before extending it.
   [recording_failure](../crates/lab-core/src/runtime.rs#L809) closes Required and
   appends an Output Revoked fact (also Controller Failed facts if controllers
   are active). Recorder itself is still `Recording`, so fact capture remains on.
5. Preparation returns success. The final
   [command_with_cause fence](../apps/lab-runtime/src/host/lifecycle.rs#L212)
   checks Recorder Failed/Closed, not the newly closed Core Required gate. Retune
   succeeds, changing Reference revision 1 to 2 and appending its Reference fact.
6. [admit_recording_facts](../apps/lab-runtime/src/host/recording.rs#L1196) passes
   both facts to [capture_reference_completion](../apps/lab-runtime/src/recorder/worker/reference_completion.rs#L251).
   Its exact-one-Reference check fails. SQLite retains accepted, the earlier fact,
   and a gap for fact identities 2..3 (`known_count=2`); no terminal audit persists.

This regression uses production **13-group defaults**, actual SQLite receipts and
a logical clock crossing; it does not depend on a two-second sleep or a racing
worker schedule. Both Windows and Linux reproduce the same missing terminal.
Thus exclusion of ordinary concurrent acquisition does not establish the required
completion invariant.

**Probe token cleanup is independently defective if capture rejects.**
`admit_recording_facts` removes matching tokens from `probe_fact_reservations`
before selecting the Reference branch. That branch neither transfers nor cancels
them. The error path sets a gap but does not release those unused reservations.
Normal `cancel_configuration_activation(None)` cannot find the removed token.
The negative-path regression deliberately injects a Measurement while a Reference
token is active; this is **not claimed as a reachable concurrent probe scenario**.
After committed accepted/gap receipts, it observes an orphan charge of exactly
**1 group / 256 records / 524,288 bytes**. The test alone retains the token and
cancels it for fixture cleanup before asserting the contract failure. No submitted
SQL credit is released by that manual cleanup.

Production fix is needed for the reachable safety-fact mixture. A correction must
handle Required safety transitions at the final pre-effect boundary and route/account
non-Reference facts separately without losing FIFO order or the reserved terminal.
Simply removing `facts.len() != 1`, discarding extra facts, increasing group count,
or skipping safety polling does not establish the existing record/byte guarantee.
Token cleanup also needs a narrow defensive correction: every removed, unused probe
token must be cancelled or transferred on every capture outcome, exactly once.
No implementation choice has been applied; these remain for the requested review.

## 2. Required writer failure before core rebind

**The upper lifecycle fences reject success too late to prevent the side effect.**

The two tests execute the real ServiceHost reconnect phase methods in the same
order as [reconnect_resource_with_factory](../apps/lab-runtime/src/service/reconnect.rs#L24).
They split the phases solely to control the writer failure, rather than adding a
production test hook. They do not claim an additional process/TCP qualification.

A deferred foreign-key violation in the test SQLite makes the writer's first
fact transaction fail at SQL COMMIT. The held writer is released only after
[begin_recorded_lifecycle_with_scope](../apps/lab-runtime/src/service/configuration.rs#L719)
has successfully reserved activation. Both cases wait for actual Failed and
`worker_closed` receipts before the rebind attempt:

| Failure position | reserve_rebind_facts after failure | Core rebind | Final lifecycle |
|---|---|---|---|
| After lifecycle admission, before first fact reservation | `Ok(true)`, no token | `Ok(())`, generation 1 → 2 | `Err(Commit)` |
| After fact reservation and old-adapter retirement, before rebind | `Ok(true)`, existing token | `Ok(())`, generation 1 → 2 | `Err(Commit)` |

[reserve_rebind_facts](../apps/lab-runtime/src/host/recording.rs#L1255) polls
failure and then returns true either for the existing token or for any non-Recording
state. [rebind_configured_transport](../apps/lab-runtime/src/host/configuration.rs#L585)
uses that result before `replace_transport` and Core generation changes. Failed
polling has disabled Core fact capture; the rebind baseline is consequently absent.
With a prior token, converting the now-empty facts cancels the unused reservation
and returns success as well.

[finish_recorded_lifecycle_detailed](../apps/lab-runtime/src/service/configuration.rs#L820)
rejects activation afterward. It prevents successful lifecycle completion/acquisition
release, but cannot undo the already changed generation. Both tests preserve the
quiesced resource, retire the test replacement and verify zero generation-two
measurements/activation lifecycle records in reopened SQLite (`integrity_check=ok`).
No Completed reconnect or physical output was asserted by this test; the demonstrated
defect is the unrecorded core rebind despite failure being observed beforehand.

Minimal proposed production correction: immediately after polling in
`reserve_rebind_facts`, reject Required Failed/Closed **before both** the existing-token
shortcut and the non-Recording shortcut. The same check is reached immediately before
`replace_transport` by the inner rebind path, even when an outer reservation previously
succeeded. Existing cancellation must release only unused tokens and leave submitted
credits charged. Keep Idle/unconfigured behavior and documented best-effort semantics;
do not globally reject all non-Recording states. Reservation still cannot guarantee
SQL success for a failure arriving after the final poll. This fix is proposed only.

## 3. Capacity wording and test results

Thirteen groups fit the stated **4 acquisition + 5 reconnect + 4 Reference** envelope.
Only Reference has protected credit. Acquisition and reconnect share the other nine
groups plus ordinary record/byte credit. Reconnect reservations are incremental;
there is no protected five-group reconnect pool, priority service or guaranteed
deadline success under arbitrary acquisition pressure. Updated
`M18_RECORDER_ADMISSION_CAPACITY.md` and `../docs/recorder-sqlite.md` make that distinction.

New test functions (all named with the `m18_review_` prefix):

| Test suffix and location | Windows | Linux |
|---|---|---|
| [reference_admission_drains_prior_mixed_measurement_group](../apps/lab-runtime/src/host/reference_completion_tests.rs#L439) | PASS | PASS |
| [reference_admission_is_fenced_while_reconnect_probe_is_reserved](../apps/lab-runtime/src/host/reconnect_recorder_ordering_tests.rs#L555) | PASS | PASS |
| [reference_completion_must_preserve_safety_facts_from_late_receipt](../apps/lab-runtime/src/host/reference_completion_tests.rs#L476) | RED | RED |
| [capture_error_must_not_orphan_removed_probe_reservation](../apps/lab-runtime/src/host/reference_completion_tests.rs#L565) | RED | RED |
| [required_writer_failure_before_first_rebind_reservation_blocks_effect](../apps/lab-runtime/src/service.rs#L5590) | RED | RED |
| [required_writer_failure_after_rebind_reservation_blocks_effect](../apps/lab-runtime/src/service.rs#L5595) | RED | RED |

Targeted command on each platform:
`cargo test --workspace --lib m18_review_ -- --nocapture --test-threads=1`.
Each final run: **2 PASS, 4 FAILED, 0 ignored**, exit 101. These assert the required
contracts and are intentionally left RED pending production changes; no ignore,
`should_panic` or changed expectation hides the defects. Full workspace gates and
the process matrix were not repeated. `git diff --check` and workspace fmt-check PASS.

Initial fixture mistakes (wrong Unit constant; attempting SQLite trigger DDL after
the live writer acquired the database lock) were corrected only in tests, with
their failed logs retained. They are not counted as product defect evidence.
The first Linux run's `/tmp` databases were unavailable at collection, so only the
six filtered tests were repeated with persistent `TMPDIR` under
`/root/lab-runtime-evidence/m18-recorder-rereview-20261009`. Both Linux runs have the
same outcomes. Eight coherent SQLite backup copies on D: have integrity `ok`.

Evidence: `target/m18-recorder-rereview-20261009/` contains final Windows/Linux logs,
initial failures, `sqlite-evidence.json`, eight databases, source fingerprints,
`diagnostic-only.patch`, status and disk observations. Linux reused the existing
`/root/lab-runtime-target/m18-implementation-20261009`; no isolated target or cleanup
was required. C: stayed above 8 GB free and VHDX stayed at 49,793,728,512 bytes.
No Runtime/Tuna infrastructure was started; fixture workers/listeners have exited.

Stop for the requested short external review. The broader M18 work is not resumed
and the previous green full gates do not supersede these newly exposed failures.
