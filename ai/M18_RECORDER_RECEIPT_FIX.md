# M18 Recorder receipt correction

**Historical review record.** The Recorder remediation was subsequently approved
and committed; current M18 scope, verification and remaining review limits are in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). Earlier restrictions,
HEAD/status claims and failed gates below describe their dated diagnostic stage.

Scope: the separately authorized writer high-water correction and regression
evidence. This does not authorize changes to Recorder credit, grouping, Application
operation outcomes, output authority or network policy. M18 remains under review.

## Meaning and safety argument

`persisted_through_sequence` identifies the exact committed and owner-receipted
Recorder prefix. Cumulative released records/bytes/groups return its charged credit
exactly once. `confirmed_submission` is the greatest **original owner submission
time** confirmed in that committed FIFO prefix; it is a progress heartbeat, not a
promise that every submission bearing an earlier timestamp is committed.

The single SQLite worker processes a FIFO, allowing only bounded contiguous Facts
coalescing. An intervening message is deferred, not reordered. Successful SQL work
is required before watermark or credit publication. Thus taking the maximum of
already committed original times cannot commit a later queue element, fill a record
identity hole, acknowledge a failed transaction or include an uncommitted terminal.
Timestamp order and FIFO record order are different: Reference command facts can
retain the preceding scheduler time while an accepted audit uses the current clock.

Required progress deadlines remain `original_submission + REQUIRED_PROGRESS_AGE`
(two seconds), never receipt arrival time. An older committed fact keeps the existing
high-water value; it does not extend the deadline. Queued later work stays charged
and beyond `persisted_through_sequence`. Failed/gap coverage remains sticky.

One receipt belongs to one worker/process boot; a reopened worker starts with a new
boot and empty receipt/generation. Start remains the existing separate SQL boundary:
trusted Runtime submission uses its current monotonic clock and enters Recording
only after durable Start. Owner start-time expectations are unchanged; a synthetic
backwards clock across intervals is not introduced or legitimized by this fix.
Checked activation generations, reserved record IDs and original SQL timestamps
are unchanged. Activation already preserved its submission high-water value.

Owner stale/future sequence, future submission, regressed timestamp and cumulative
release checks are unchanged. Their strictness is why a newer incorrectly regressed
writer receipt previously lost legitimate release/gap evidence.

## Exact change

Only writer successful-commit callbacks change: Facts confirms the maximum of all
original times in the committed batch and the existing receipt; Operation,
Annotation and Probe confirm the maximum of their committed original time and the
existing receipt. A private storage-loop helper documents this distinction. The
public status documentation now describes the high-water meaning precisely.
Start/Stop/Finish, activation generation logic, SQL schema/transactions, owner
reconciliation, limits and Application outcomes are untouched.

## Regression oracles

`tests/recorder_receipts.rs` adds five bounded tests with existing WriterBarrier
synchronization and deadline/yield waiting, without command retries or fixed sleeps:

- A 20 ms accepted audit is fully observed, then 10/11/12 ms Facts commit before
  a held 30 ms terminal. Exactly one group stays charged; repeated receipt polling
  cannot release twice or confirm the held terminal. Another fact and a reserved
  activation remain pending. Wrong activation generation is rejected, no generation
  advances before its SQL commit, and release preserves both operation audits,
  original capture times and measurement generation/revision.
- An older committed fact cannot hide a durable GapSeal. Finish seals the boot,
  releases all credit and preserves Failed/gap rather than claiming healthy coverage.
- A batch ordered 30/10/20 ms confirms 30 ms, preserving all three original times.
- Older operation, annotation and probe receipts retain the confirmed prefix; a
  later valid Start advances the checked run and interval counters after commit.
- Four genuinely uncommitted groups still reject a fifth and do not confirm its
  queued time. Reserved gap and terminal sealing still work.

The existing real deferred-foreign-key COMMIT-failure test additionally asserts
that confirmed time, persisted prefix and charged records/groups do not advance
or release for the rejected transaction. Existing stale/future receipt and
activation/reload/shutdown regressions remain mandatory.

Before the writer change: four of these new tests failed and the genuine-capacity
test passed. After the change: all five and related regression suites pass. The
original failures and all subsequent process/gate evidence are retained under
`target/m18-recorder-fix-20261008/`.

## Remaining acceptance boundary

This correction does not fit five truly outstanding groups into four. A normal
Reference operation still submits accepted audit, domain facts and terminal audit
as separate groups. Real simultaneous-client recording must pass its process matrix
and long GUI/Babashka scenario before operational M18 acceptance. If genuine capacity
still fails, any bounded reservation/grouping proposal requires separate review;
increasing credit, suppressing audits or weakening the failure policy is not a remedy.

## Repeated process matrix

Fresh Release processes, ten independent databases for each controller/subscriber
pair, on Windows and Arch WSL2. The sequence remains RecordingStart -> Subscribe
-> Reference query -> one Retune. No mutation retry, extra synchronization query,
capacity change or discarded assertion. This is an operational FAIL, not acceptance.

| Controller / subscriber | Windows healthy / 10 | Arch WSL2 healthy / 10 |
|---|---:|---:|
| tcp/tcp | 8 | 0 |
| tcp/ws | 6 | 0 |
| ws/tcp | 7 | 0 |
| ws/ws | 6 | 0 |

All 80 mutations were accepted/completed by Application, and all retained the
Reference fact. The 53 failed Recorders had an explicit durable gap, zero remaining
charged groups after receipts, and a sealed boot. Their terminal Retune audit was
not admitted; shutdown truthfully reported Recorder failure and process exit 1.
The 27 healthy cases retained both Retune audits and shut down with flush/exit 0.
All 80 databases passed integrity and foreign-key checks. Reopening each original
database in a new Runtime boot preserved the complete old record/measurement
prefix fingerprint, returned healthy idle Recorder and default Reference revision 1
(no replay), and shut down with flush/exit 0.

The remaining overload is distinct from the repaired receipt regression: two
measurement/domain groups can already be pending, followed by the accepted audit
and command Reference fact. The terminal audit can arrive as the fifth group before
SQL commits are receipted. The writer's unchanged Facts coalescing deadline is
100 ms, with FIFO interruption by an audit. At the fifth admission, the existing
four-group policy fails closed. The exact limit is additionally reproduced with
an explicit writer barrier, independently of scheduler timing. No bounded limit or
owner defensive check was changed. An operation-wide bounded reservation/grouping
proposal would need separate review; a scheduling tweak cannot prove adequate
capacity for this burst. Application completion remains a domain outcome, not
a promise of complete scientific/audit recording.

Evidence: `windows-closed-readers-matrix.json`, `linux-closed-readers-matrix.json`,
all 80 NDJSON transcripts, coherent SQLite copies and per-boot stderr under
`target/m18-recorder-fix-20261008/`. Original test databases remain isolated in
that directory (Windows) or the Arch user's matching `target/` directory.

## Diagnostic failures retained

Early matrix starts under concurrent compilation hit the existing managed startup
deadline. Those aborted runs are retained, not relabelled PASS. A later reopen probe
hit `sqlite: database is locked` on both OSes: the Python harness kept read-only
SQLite connections alive because its connection context manager only ended the
transaction. Explicit `contextlib.closing` fixed the harness; 80 subsequent reopens
passed. This proves this diagnostic lock's cause, not every historical lock report.

The first Linux Debug workspace run failed at `service.rs:3015` with
`RecordingUnavailable` in the existing API-provisioned reconnect test during
concurrent compilation/process validation. A detached original-b546 Runtime lib
run passed 133 tests with one ignored. That pass does not establish the failure
was introduced by this fix or classify its exact Recorder cause. No unrelated
production/test change is made on the feature branch.

## Managed-executor Linux test contention

The later Linux parallel Debug history-api target returned Component(Busy) from
ServiceHost::startup in six existing tests (lines 202, 451, 551, 714, 1130, 1223).
ManagedExecutor::new_with_runner (`managed_executor.rs:124`) admits exactly one
process-wide two-slot pool through compare_exchange(0, WORKERS). Concurrent test
ServiceHosts share that budget, and service startup's existing two-second bound
can expire while another fixture owns the pool. Recorder is opened only after
managed initialization (`service.rs:1099` onward), so these startup errors cannot
be caused by writer receipt handling. The production constructor, managed executor
and history-api test file are unchanged from b546354.

The original-b546 detached baseline reproduced the same history-api result:
3 passed / 6 failed under default parallelism. Its identical test executable
passed all nine with --test-threads=1, as a diagnostic comparison only. That serial
run is not substituted for any mandatory workspace gate. Classification: existing
test resource contention against the process-global executor bound. A minimal
test-only follow-up would coordinate fixtures that instantiate that pool; neither
its production capacity nor startup deadline should be increased to hide the
conflict. No such unrelated test change is included in this writer correction.

## Long Windows Workbench / Babashka / Arch WSL2 session

One planned five-minute active-recording session completed in 305.16 seconds.
Workbench used its real Windows Release GUI in --observe mode and direct loopback
WS forwarding to the isolated Arch Runtime. Babashka kept an independent WS
connection/subscription for 120 seconds before one explicit retune, confirmed
accepted/completed and the authoritative Reference snapshot, then added its own
plot via the local Workbench API. Both clients continued for another 180 seconds.
No measurement relay, mutation retry, fixed diagnostic sleep or Recorder capacity
change was used. Waiting was on actual bounded network/event queues and deadlines.

Recorder retained 6,086 actual virtual-demo measurements, accepted/completed retune
audits and no gaps. Workbench's dispatcher remained Fresh, exposed current good
measurements and presentation revision 3 with the added plot. GUI close exited 0
without stopping Runtime or Recorder. Explicit RecordingStop and RuntimeShutdown
then completed with durable flush/exit 0. A new Runtime boot reopened the same
original database, preserved the old prefix fingerprint, returned idle/default
Reference revision 1 without replay, and exited 0. SQLite integrity and FK checks
passed. Evidence is long-*.json/log/jsonl plus long-coherent.sqlite in target.

Native visual limitation: the screen capture of the actual GUI showed a white
client area. A separate bounded compositor-frame capture also failed to prove
painted contents. The shared GUI model/API update and fresh authoritative data
are verified; visible graph rendering is not claimed as PASS. No renderer or
Workbench production change was made. That additional isolated diagnostic Runtime
and GUI were cleaned up; the user's original Arch PID 1087 remains running.

## Additional Linux Workbench test assumption

Both Debug and Release fail journal_failure_rejects_mutation_before_wire_emission
at worker.rs:2249 (bounded wait timeout). Its fixture places recovery.json beneath
a regular file. An isolated standard-Rust filesystem probe returned NotFound
(OS code 3) on Windows, but NotADirectory (OS code 20) on Linux.
Worker::new/load_startup_recovery thus recognizes a lazy missing journal on Windows,
but a startup journal error on Linux. Worker::run emits that problem before hello
(`worker.rs:527`); the test's wait_for(Hello) consumes/discards it. The already-failed
Linux journal then causes queue_mutation to reject locally at 942, with no new
RecoveryJournalProblem, while the test waits for that second notification.

This is an existing platform-dependent test fixture/event-order assumption, not a
Recorder writer or mutation-safety change. A narrow follow-up could arrange the
write obstruction after hello (then preserve both original problem/rejection and
no-wire assertions), or explicitly test startup-error handling separately. No
Workbench change is included here. The source file is identical to b546354.

## Gate results and review boundary

Windows: git diff --check; cargo fmt --all -- --check;
cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace;
cargo test --workspace --release all PASS. Each workspace mode has 847 top-level
passed, 13 ignored, zero failed, plus two successful child ownership checks
(the child output must not be counted again as distinct workspace tests).
Five receipt tests pass 20 repetitions in each Windows/Linux Debug/Release mode,
400 test evaluations total. The related Windows Recorder regression selection
passes all 48 tests, including actual deferred-FK COMMIT rejection.
Ubuntu Rust 1.95 x86_64-unknown-linux-gnu Runtime Debug/Release builds PASS.

Linux complete default-parallel workspace diagnostics used --no-fail-fast to
retain all target results, without serializing tests: Debug 837 passed, 11 failed,
13 ignored; Release 846 passed, 2 failed, 13 ignored. Debug failed history-api,
process-reopen, shutdown and startup fixtures with Component(Busy), plus the
Workbench journal test above. Release failed the existing reconnect test at
service.rs:3015 (RecordingUnavailable) and that Workbench test. Its exact Recorder
state is established by the detached diagnostic below; no further production fix
is authorized.
The original stopping gate attempts and subsequent full diagnostic logs are retained.

A white-window capture is a visual-validation limitation, not proof of rendered
plots. Real physical LAN, USB/RS-485, protected Tuna E2E and overall M18 acceptance
remain unclaimed. The genuine five-group burst and these Linux gate failures are
not waived by the deterministic receipt fix or the successful five-minute session.

## Instrumented reconnect saturation

A second detached diagnostic worktree contains b546354 plus the same two writer
production files and only a test-panic detail at service.rs:3015. Its first full
Runtime Release lib run passed 133 / 1 ignored; the next default-parallel run
reproduced the failure, with this observed state:

- Recorder Failed, first_error=recorder ingress capacity exhausted, coverage=gap;
- exactly 4 groups / 5 records / 6,148 bytes charged, persisted prefix 7;
- confirmed original submission 37.635011 ms; first_missing_fact=5;
- activation_generation=2, gap not yet receipted at the instant of failure;
- Recorder limits unchanged (4 groups / 1,024 records / 4 MiB);
- reconnect stage LifecycleDurability, old worker finished, replacement spawned
  and OS-open confirmed, one open attempt, core_rebind_crossed=true.

This links the intermittent RecordingUnavailable gate failure to genuine Recorder
saturation during reconnect publication, not the managed-startup pool error. The
operation truthfully reports failure after core rebind instead of granting success;
no automatic retry is safe. It is a separate operational capacity/publication
review item. No owner check, reconnect flow, capacity or publication logic was
changed in the feature working tree. The diagnostic panic is confined to target.
See linux-candidate-lib-repeat-1.log and linux-candidate-lib-repeats.log.

No new local commit is created because the complete Linux gates remain failed.
The feature HEAD remains b546354bddf035c15f78f1f1d1ea1d22eb84ac76, with the narrow
writer correction and this evidence available for external review. No merge, push,
tag or Release is performed. Existing release/diagnostic artifacts are preserved.
