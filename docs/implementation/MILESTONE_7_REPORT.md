# Milestone 7 implementation report

Status: M7 D1-D18 implementation and latest-head verification complete on
2026-09-16. Ready for external review. The accepted contract is
[MILESTONE_7_DESIGN.md](MILESTONE_7_DESIGN.md). M8 has not begun.

## Incoming working-tree audit

The explicit SOL_HIGH switch occurred after uncommitted M7 production work had
already been written. This report distinguishes existing checks from acceptance
coverage added during the SOL_HIGH audit. The initial uncommitted implementation
must not be described as tests-first merely because tests now exist.

The audited changes were confined to Core fact capture/Required gating; the host
Recorder worker, SQLite schema/projections/provenance/history; startup CLI and
lifecycle; local API/session/wire additions; and the two host-only dependencies
rusqlite and sha2. No M8 TOML, COM or reload, GUI, persistent Lua workspace,
donor mutation, or unrelated user file change was found. The original Core,
transport, output-authority and managed-worker boundaries remain in their
accepted crates. No architectural contradiction requiring WAITING_FOR_REVIEW was
found. Incomplete or incorrect M7 behavior was fixed against the design.

The preexisting test files were named recorder_facts, recorder_evidence,
recorder_sqlite, recorder_transactions, recorder_reopen, recorder_provenance,
recorder_backpressure, recorder_isolation, recorder_required,
recorder_operations, recorder_history, recorder_history_api,
recorder_shutdown and recorder_startup. They are behavioral beginnings, not
proof that each D row is complete. Some production code preceded these tests;
the exact earlier red/green sequence cannot be reconstructed and will not be
invented.

## Final D1-D18 acceptance mapping

The following matrix is the final signoff mapping. Detailed red/green history,
including fixture-only corrections and production code that preceded later
acceptance coverage, remains below; no retroactive tests-first claim is made.

| ID | Final named evidence | Result |
| --- | --- | --- |
| D1 | `native_and_managed_batches_reopen_through_public_indexed_history_with_exact_sql_ids`; `explicit_stop_waits_for_held_pending_fact_and_reopens_its_wal_prefix_and_seal` | Real SQLite batches, WAL prefix, interval seals, checkpoints and public reopen/history match exact independent identities. |
| D2 | `same_publication_time_failure_preserves_both_original_facts_after_reopen`; `native_sensor_sentinel_reopens_as_unavailable_without_reusing_previous_good`; `all_scalar_kinds_and_delayed_publication_reopen_without_coercion` | Good/Unavailable, delayed observation/publication, every scalar kind, i64 extremes and nonfinite rejection remain exact. |
| D3 | `delayed_managed_transform_reopens_original_upstream_identity_and_state_revision`; `replaced_managed_generation_reopens_with_failure_and_ignores_old_late_result`; `activation_binding_and_rebound_output_rows_reconstruct_exact_m3_provenance` | Units, generations, definition/state revisions, lineage and M3 binding revisions are immutable at capture. |
| D4 | `partial_m3_attempt_reopens_as_old_epoch_ambiguous_before_distinct_safe_ack`; `late_m3_ack_after_safe_epoch_reopens_only_as_old_ordinary_evidence`; `unconfirmed_m3_recovery_keeps_safe_evidence_unknown_and_never_sends_queued_safe_bytes`; Core `recorder_evidence`/`recorder_facts` suites | Requested/authorized/send/ACK/readback/uncertain/ambiguous/safe/revoke facts retain original correlation and never promote ACK to readback or physical proof. |
| D5 | all four `recorder_operations` tests; `killed_retune_after_durable_acceptance_has_unknown_terminal_and_is_not_replayed`; public history API tests | Accepted and terminal facts span held writers, disconnect/reconnect and crashes without duplicate execution, invented outcomes, evidence injection or history-read recursion. |
| D6 | all eleven `recorder_backpressure` tests, especially `four_full_causal_groups_commit_exactly_the_256_record_batch_limit` and `four_delayed_managed_lineage_groups_charge_nested_scratch_at_batch_peak`; receipt-fence unit tests | Count, bytes, Vec capacity, escaping, nested lineage scratch, groups, FIFO IDs, cadence, history pressure and cumulative receipts remain bounded and atomic. |
| D7 | all six `recorder_isolation` tests; `two_real_lua_slots_and_held_sqlite_leave_socket_native_renewal_and_m3_recovery_alive` | Held SQLite cannot block native PID, three finite lease renewals, M3 first byte/recovery, loopback Queries, safety work, or two independent Lua slots. |
| D8 | all four `recorder_failure` tests; `core_outbox_overflow_marks_best_effort_gap_without_stopping_native_control`; startup checkpoint/unopenable-storage tests | First failure, loss boundary/count, watermark, stable repeated status and persisted/not-persisted distinction survive overflow, panic, SQL/encoding/checkpoint and unavailable storage; event-ring eviction is separate. |
| D9 | all four `recorder_required` tests; `required_deadline_safes_virtual_output_but_keeps_partial_m3_recovery_unknown`; Core exact-deadline/direct-dispatch tests | Required opens only after durable start, trips at two seconds, enumerates multiple outputs, revokes manual/native authority, fences queued bytes, keeps unknown physical recovery unknown and never auto-rearms. |
| D10 | all eighteen `recorder_transactions` tests; all five `recorder_startup` tests; commit-before-receipt process cases | Real WAL/FULL/FK transactions, constraint/commit/checkpoint failure, collision, schema/index/encoding validation, one-GiB quota, corrupt/foreign/oversized startup and exclusive ownership preserve the exact committed prefix. |
| D11 | `shutdown_seals_active_interval_and_closes_worker_before_reporting_flush_success`; `running_controller_shutdown_drains_held_measurement_before_final_safe_seal`; `public_shutdown_operation_waits_for_held_recorder_prefix_then_reports_durable_flush`; multi-run/full-credit shutdown tests | Producer barrier, safe evidence, accepted prefix, interval/run/boot seal, worker close and public terminal result are FIFO, finite and nonduplicating. |
| D12 | shutdown insert/close/late-receipt/blocked-writer tests; `blocked_writer_child_exits_nonzero_after_the_public_flush_watchdog`; `fatal_owner_time_fault_still_seals_healthy_recorder_before_nonzero_terminal`; `failed_m3_recovery_and_two_stalled_workers_still_flush_honest_recorder_evidence` | Safety, Recorder durability, fatal owner state, M3 ambiguity and Lua cleanup remain separate; timeout/failure exits nonzero without joining a blocked worker or falsifying a seal. |
| D13 | all ten `recorder_process_reopen` tests and four `recorder_reopen` tests | Kills before commit and after commit-before-receipt preserve only actual rows, create new identities, reject stale scope/event/history state, restore no lease/output/control work and retain unknown tails. |
| D14 | all eight `recorder_history` and eight `recorder_history_api` tests; `nonreading_history_client_cannot_block_native_service_while_sqlite_is_held`; history cancellation unit tests | Invalid work is rejected before SQL, selection visits returned rows plus at most one lookahead, VM/time/row/byte/frame/job/TTL bounds hold, and stale/nonreading clients cannot retain work or block safety. |
| D15 | frozen-keyset/equal-time/escaped/Unavailable/gap tests in `recorder_history`; restart/public paging tests; `killed_equal_time_archive_pages_exactly_the_committed_prefix_under_a_new_boot` | Frozen-W keyset pages have exact independent identity unions across append, byte shortening, gap metadata, empty/end ranges, expiry and process restart. |
| D16 | all nine `recorder_provenance` tests, especially loaded-byte SHA/dedup, serving activation, configuration reconstruction and M3 rebind reconstruction | Loaded bytes, typed content roles, activation root, limits, full PID/Reference baseline/revisions, binding descriptors and output-at-revision facts reconstruct the active composition. |
| D17 | all nine `recorder_time` tests, especially `m3_first_byte_recovery_and_safe_send_decisions_ignore_wall_clock_jumps` and live worker wall-read failure | PID/EMA/Reference/leases/M3 bytes and fact order use monotonic time only; signed/pre-epoch/failed wall reads remain metadata with stable boot mappings and actual anchors. |
| D18 | 345 named Rust tests; full debug/release workspace; fmt; clippy; rustdoc; finite demo; actual M6 and recorded M7 Babashka process tests; `bb test-client`; diff/status checks | Complete on latest HEAD. The first final debug attempt exposed a real pause/tick credit race; after test-client tick alignment, three focused debug runs, focused release, and the complete latest-head debug/release gates passed. |

## Observed red/green in this SOL_HIGH audit

This section is a chronological audit log. Statements that a case “remained
open” describe that intermediate checkpoint and are superseded by the final
D1-D18 matrix and latest-head verification above.

Real observed red cases included: a BestEffort run reopened as required; Required
stop accepted a manual lease; worker close was reported as flush without a boot
seal; Core outbox overflow still allowed manual acquire; history budget API and
separate boot_id field were absent; incompatible record encoding, multiple active
runs, checkpoint mismatch, missing index and mixed float/integer columns were
accepted; duplicate provenance entries changed the activation root. The
corresponding targeted checks passed after fixes. Several red cases were compiler
errors for new API seams; they are recorded as such, not as runtime failures.

Further observed runtime reds: durable `operation_events` were empty after a real
retune/disconnect/reopen; a serving fixture used a different public boot ID from
its Recorder archive; the serving startup had no loaded Lua provenance or
`object_snapshots`; SQLite stored the Celsius display symbol instead of its
stable unit ID; a writable ingress overflow could not finish the run or boot
with a gap seal; and Required stop accepted a second manual output lease because
the host had not tracked its successful profile binding. Their targeted tests
passed after the corresponding fixes. The initial large M7 working tree remains
prior production work and is not retroactively called tests-first.

A full workspace run then failed the history API fixture with an empty page:
the owner status was `Recording` but fact credit and checkpoint stayed zero
after a successful post-start refresh. The worker could receipt Start between
host polls, and a fact-drain helper copied its new status before the transition
handler enabled capture and the Required gate. The accepted measurement-history
test was red under this race and passed after state changes were centralized in
the transition handler. The parallel history fixture initially also saw the
shared Lua supervisor Busy during this failed two-second wait; after the race
fix both independent tests completed in under a tenth of a second.

Two further observed D1 reds were a post-boundary observation dropped while
the actual SQLite Start transaction was held, and a new run with no boundary
snapshot of the already committed latest sample. The subsequent tests pass:
Starting now admits its FIFO facts under the new IDs, and a durable
`boundary_snapshot` contains the old sample's original time without inserting
it as a new `measurements` row. Existing checkpoint/history tests were updated
for the intentional first boundary record sequence; those initial red assertions
were stale test expectations, not new transaction failures.

The next D4 tests were red in three distinct ways: Core facts lacked typed
attempt/dispatch fields (compile-time red); a reopened virtual attempt had NULL
`attempt_id` and `dispatch_id` despite its four ordered stages (runtime red);
and the real fake-M3 ACK kept NULL correlation after its requested stage
(runtime red). A candidate with a mismatched unit then failed without any
pre-send attempt fact (compile-time red for the required stage), as did an
accepted proposal at its exclusive expiry. Each targeted test passed after
capture and typed SQLite projection changes. The original unbounded dispatch
lookup was replaced before commit: one current virtual dispatch is retained
per binding, while M3's immutable queued intent carries its original attempt
through terminal and recovery events. This work covers only the D4 cases named
above; supersession and the other evidence cases remain open.

The D17 pure mapping test first failed to compile because no time-anchor domain
type existed. The real-file test then failed to compile for a missing anchored
open method and again for separate occurrence/publication/capture storage. A
later test found `clock_anchors` were independent rows with no canonical
`records` or `runtime_events` identity; the projection now shares one
transaction/checkpoint. Actual periodic and start/end worker tests were red
for the missing worker clock-origin seam and missing run wall fields. They pass
with the serving M6 origin and worker-side UTC capture. Existing worker credit
and gap tests initially used literal record watermarks from before the
intentional interval-start clock fact; their assertions were revised to
require actual post-start progress and the accepted prefix sequence, not to
weaken their durability checks. Additional reopened checks found NULL boot
estimates on the boundary, accepted/completed operation rows and a writable
coverage-gap row. These were real runtime reds, not adjusted fixture
expectations. Each row now stores the initial boot-mapped display estimate and
its `boot_anchor` basis; the independent actual UTC anchors never rebase it.

The D6 held-writer test was red with five checkpoints for start, two facts,
stop and boot seal. Two causal fact groups now share one SQLite transaction and
publish one cumulative receipt for both groups; its four expected checkpoints
and two reopened measurement rows pass. The Store batch path was written before
the additional D10 malformed-second-group test: that test then observed SQLite
coerce NaN to NULL and accept the transaction. The subsequent validation made
that test pass with no valid first-group row or checkpoint advancement. This
sequence is recorded honestly; the batch rollback coverage was added after
the first batch implementation.

D10's page-cap test first failed to compile for the missing storage-page seam;
the accepted real-file cap and smaller-reserve test then passed. The WAL test
failed to compile for absent sidecar/threshold observations; after implementing
a worker-only checkpoint before the next ordinary transaction, it passed with
two real truncation checkpoints and the actual nonzero WAL bytes. No test yet
claims that `journal_size_limit` is a hard WAL allocation cap.

The D11 archive test was red with a committed terminal boot seal but no
`outputs` safety facts in its exit summary. The worker also passed monotonic
zero to the seal and left `ended_wall_us` NULL. After the accepted observation
is frozen by the host at the end of its safety phase, a bounded summary and
actual worker UTC anchor share the boot-end seal transaction and checkpoint.
The reopened test now checks confirmed safe evidence, no lease, managed-worker
count, actual end UTC and nonzero monotonic shutdown publication. The full
shutdown suite, debug workspace suite and warning-free clippy pass after this
slice. It does not yet prove full-ingress acceptance or final seal failures.

D2's new same-time plant failure test was red with `NonMonotonicTime` after a
Good observation at that same owner instant. The Core tail now uses the already
reviewed trusted invalidation rule: it becomes Unavailable at the same instant,
while the earlier Good and later Unavailable Recorder facts keep their separate
record sequences. Both archive rows survive boot seal and reopen. The scalar
archive test initially failed to compile because the validated native sample
factory was absent and the derived factory was Core-private. Those factories
now allow finite trusted sample construction and preserve the captured source
time; the real SQLite round trip passes for all five scalar kinds and both i64
extremes. A first fixture attempt also held a live exclusive store while
opening a second SQLite connection, and another compared the accepted 8-byte
monotonic encoding against an incorrect 16-byte fixture. Those were test
errors, corrected before the final green run. The debug workspace and clippy
passes include this slice.

The D11 full-credit test initially requested 64 groups, while the accepted
implementation's explicit hard group bound is four. The fifth was correctly
rejected as an ingress overflow; this was a fixture mistake, not a Recorder
regression. With four full charged groups and the actual first SQLite batch
held behind a confirmed barrier, Stop still enters the reserved physical FIFO
capacity, drains the four-row accepted prefix, and Finish commits a sealed boot
before close. The focused three-test shutdown suite passes.

D5's real API Start/Stop test was red with an empty durable lifecycle list
despite four correct process-local accepted/completed replies. The Host now
keeps at most 64 pending request identities. Its small deferred Start intent
is captured before the boundary and transferred after committed Start; Stop
acceptance enters before its seal, and its terminal fact enters the boot
lifecycle after the seal. Reopen observes the four original request phases
and the Start request in `pending_operations`. This does not yet cover a
process kill between accepted and terminal or pending Stop summary. The new
D17 anchor-ID oracle separately observed `[1,2,3,5]` under actual worker
Start/Stop/Finish; the terminal transaction now uses the next free ID, and
the reopened oracle sees contiguous IDs. Focused D5/D17 tests, the full debug
workspace suite and warning-free clippy pass after these slices.

D15's escaped equal-time archive fixture was red because three 1,000-byte
NUL text values fit the old two-byte-per-source estimate, although each NUL
requires six bytes in JSON. The worker selection now charges a conservative
six-byte escape bound per UTF-8 source byte plus fixed row/envelope headroom.
It shortens a raw page before returning a cursor and advances only through the
returned row. Two rows appended at the same publication time between pages
cannot enter the frozen checkpoint; the three old record identities have no
duplicate or omission, and a fresh traversal finds all five. The first green
attempt incorrectly expected all five fresh rows on one page despite the
intentional shortening; the fixture now traverses bounded continuations. The
focused history/API tests and warning-free clippy pass. An actual public
encoded-page oracle was then added after the production fix; it is green across
all three public page/cursor/release operations with whole-frame checks, and
is recorded as added coverage rather than a fabricated prior red case.

D12 added a real SQLite trigger that aborts only the serving boot's terminal
`records.shutdown` insertion. The owner still reaches virtual safe evidence,
but reports `recorder_error`, no successful flush and nonzero exit. The failed
seal transaction leaves the serving boot active with no terminal record;
reopen classifies it interrupted. This new test was green with existing rollback
semantics after correcting its fixture: the first setup boot had been left
active and was separately interrupted, so the initial expectation of exactly
one interrupted boot was a test error. The setup boot is now deliberately
sealed before fault injection. Commit/close and process watchdog cases remain.

The additional D2 native M3 fixture was green against existing trusted
publication behavior: a valid 123.4 Celsius read is followed by the explicit
sensor-fault sentinel, and reopen preserves Good and `SensorFault` Unavailable
as two separate rows with no stale value substitution. The D3 managed fixture
was red: a successfully committed generation replacement invalidated the
Core's latest Good sample but emitted only the old Good to Recorder. Core now
captures the replacement's Unavailable fact with generation 2/state revision
0, and similarly captures dependent invalidations with their original
generation/revision. A late generation-1 completion produces no new
measurement. Real SQLite rows reopen as Good generation 1 and ComponentFailure
Unavailable generation 2; the full debug workspace and clippy suites pass.

D5/D11's new public Stop archive assertion was red with `QueryReturnedNoRows`:
the previous worker only changed the interval row's state and wrote an actual
end clock anchor. Stop now atomically writes a canonical `interval_seal` record
and runtime event, with the host-frozen list of at most 64 pending identities,
the accepted prefix through the prior record and honest coverage. The terminal
interval row points to that seal and the checkpoint includes it. The API test
reopens the pending Stop request; its Completed operation record appears later
as boot lifecycle metadata rather than being guessed in the seal. Earlier
tests with literal pre-seal checkpoints were red as expected and updated to
check the actual sealed prefix. Focused history, transaction, time, shutdown,
backpressure and reopen suites then passed. The full debug workspace and
warning-free clippy checks also passed after this slice.

D14's first private worker test was red because `cancel_history` made only one
nonblocking attempt to remove a job from three mutex-protected structures.
Holding the active-slot lock at cancellation left one retained slot and a late
run result. Deferred cancellation now retries on owner polls, and publication
holds the short active-slot check through mailbox insertion. All eight deferred
jobs remain within the eight-slot credit while the lock is held. A second
private test was red when eight already-completed job IDs filled that deferred
set and displaced cancellation of a live ninth job. The owner now tracks only
its at most eight live admitted identities, ignores retired IDs and releases a
slot and mailbox result together; admission failure also removes its slot while
the same short lock is held. All three cancellation tests, the full debug
workspace suite and warning-free clippy pass. Public expiry/reconnect and
large-fixture execution cases remain open.

D15's archived-gap contract initially failed to compile because `HistoryPage`
had no loss metadata beyond its coverage label. The SQLite adapter now reads
the single run interval through a dedicated `(boot_id,run_no)` index, validates
the already committed bounded loss summary, and returns known source-fact
identities and cutoff without inventing absent data. A missing lookup index in
a version-one archive is rejected unchanged, as the accepted schema requires.
The public API fixture then returned `history_failed` for a valid 512-byte NUL
reason: JSON escaping expanded the stored summary beyond the provisional 1-KiB
read bound. The corrected 4-KiB bound and conservative page charge pass through
real service reopen, with complete loss metadata, one Good row and whole-frame
limits. A second real-file fixture was green after correcting its test's
lowercase `disabled` expectation to Core's stored `Disabled`: Unavailable stays
raw, equal-time append after frozen W cannot enter prior pages, and a new query
finds all five identities. The full debug workspace suite and warning-free
clippy pass after this slice. Expiry/reconnect and delayed-observation union
coverage remain open.

D14/D15's public cursor fixture was green for reconnect and explicit TTL
invalidation: a page token and continuation from connection 1 could not be
used by connection 2, and the new first page recovered the same immutable
record identity. Its first cleanup attempt failed to delete the still-open
worker database on Windows; the fixture now waits at most two seconds for the
background close. A newly added same-database run mismatch test was red:
the application scheduled SQL and returned only generic `history_failed` after
the worker rejected the old cursor's filter. The application now compares the
retained continuation's archived boot/run and original signal/range before
scheduling work. It returns `history_archive_mismatch` or
`history_cursor_mismatch` immediately; later expiry returns
`history_cursor_expired`. The full debug workspace and warning-free clippy
checks pass. A held-writer pending-job reconnect oracle remains open.

D14's new test-only large indexed fixture was green against the current
implementation: 20,000 committed measurements are inserted in 80 bounded
groups, then a near-tail 11-row range returns 8 and 3 complete rows under
separate 10,000-VM-op cooperative budgets, well below the 100,000-op production
limit. It is later acceptance coverage added after the indexed selection code,
not an invented prior red test. The full debug workspace and warning-free
clippy checks pass. Exact query-plan and cancellation under a held writer
remain open.

D4's late-evidence Core oracle was runtime-red after a trusted ordinary virtual
dispatch had started, then `RequestSafe` advanced authority epoch before its
ACK arrived. Rust safety correctly kept the new procedure SafePending, but the
old ACK's Recorder fact was mislabeled with today's epoch. Core now freezes
the successful completion's original `Dispatch` before authority mutation and
emits its scalar, dispatch ID and original epoch; a new rustdoc accessor exposes
that immutable epoch. A later green real-SQLite archive test reopens the old
ACK under its original epoch and 16-byte dispatch correlation, with no safe
ACK/readback row. The stale old readback is rejected after a distinct new safe
dispatch begins, and safe confirmation stays false. Fifteen Core fact tests,
the M3/virtual SQLite evidence suite, full debug workspace, optimized focused
evidence test, formatting and warning-free clippy pass. This covers direct
virtual completion; actual late M3 wire ACK/recovery remains an open D4/D9
acceptance case.

A separate later-green actual M3 fake-byte oracle holds the normal reply after
all seven request bytes have been accepted, advances Rust authority with
`RequestSafe`, then delivers the delayed old protocol ACK before the next safe
request is sent. Core stays SafePending with `safe_confirmed=false`; real SQLite
reopen shows exactly one ordinary ACK under the old epoch and one distinct
safe ACK under the newer epoch after the queued safe transaction runs. The
fixture's transport satisfies the M3 recovery-generation contract: it does
not inject pre-recovery bytes into a post-recovery generation. All three
focused evidence tests pass in debug and optimized release with warning-free
clippy. This test follows the existing M3 intent-fencing production code and
does not claim a prior red case; late ACK coverage is now explicit, while
failure to recover and held-writer first-byte isolation remain open.

D4/D9's unconfirmed-recovery oracle initially failed because the fixture
assumed Rust could queue a safe dispatch while the old M3 ordinary transaction
still occupied the resource. The correct bounded behavior is Busy until the
adapter establishes a clean generation; the reviewed safety invariant does
not permit a second frame to overtake uncertain bytes. The adjusted real M3
fake adapter returns `RecoveryStatus::Pending` repeatedly after a two-byte
ordinary prefix. A Required Core gate is opened and then explicitly failed;
the authority revokes its lease and fault-latches, ordinary Acquire is denied,
the safe request remains pending and no further byte or ACK/readback appears.
Actual SQLite reopen retains `transport_uncertain` and `safe_requested` without
fabricated ordinary or safe evidence. All four evidence tests pass in debug
and optimized release with fmt and clippy. This uses the trusted Core failure
seam plus a real file, not a full host Recorder receipt-failure trigger. A
failed-recovery Required host integration remains open.

D8's later-green separation oracle starts a real file-backed Recorder interval,
then publishes more than the 1,024-entry transient client event-ring limit
through the owner-local semantic event API without creating Core recording
facts. The old client cursor reports a replay gap, while Recorder stays
Recording/complete with no missing fact; after Stop/shutdown, reopened SQLite
has a complete interval and zero durable gap rows. The first red runs were
fixture mistakes: a zero start time did not match the serving monotonic origin,
cached Recorder status was read without taking an owner service opportunity,
and the test queried a nonexistent `intervals` table rather than
`recording_intervals`. The corrected behavioral oracle passes all four focused
failure tests in debug/release with fmt and clippy. It follows preexisting
separate ring/Recorder production boundaries, so it is later acceptance
coverage, not retroactive tests-first implementation evidence.

D15's killed-process paging oracle uses a real Recorder child process and
confirmed SQLite after-commit/before-receipt barrier. The child commits three
equal-publication-time finite measurement fixtures in one batch, and its parent
kills it before the owner receives the watermark. A new boot opens the same
database ID; three one-row keyset pages keep one frozen W, report the old run's
`unknown_tail`, and return exactly record IDs 3/4/5 with values 21/22/23.
An independent SQL scan under the old boot matches that union. This adds a
real process-death/restart page oracle after the existing worker and keyset
implementation; it is later green acceptance coverage, not a retroactive
tests-first claim. All ten process-reopen tests pass in debug and optimized
release with fmt and clippy. Public cursor invalidation and Unavailable/gap
union are covered by separate fixtures; a combined killed-process public page
case remains open.

D14's later-green real loopback nonreader oracle holds the actual SQLite
writer, starts native BestEffort PID control, and sends a burst of 32 bounded
`history_read` run requests from a socket that stops reading after Hello. A
second socket still receives Controller Queries under one second; native ticks
increase across 6.3 seconds and the Controller remains Running after three
finite lease lifetimes. Recorder's expected writer-held ingress gap is visible
as Failed without waiting on the slow peer. Four focused isolation tests pass
in debug/optimized release with fmt and clippy. Existing eight-slot/cancel
tests provide the precise job-retention bound; this process fixture establishes
that a nonreading client's pending history replies do not block owner safety
or healthy client service. The bounded peer/reactor/history implementation
preceded this test, so it is not claimed as tests-first.

D11's later-green active shutdown oracle gets a real native Controller to
Running, commits the Recorder Start boundary, then holds the actual SQLite
writer before a newly accepted measurement can commit. With no client present,
ServiceHost requests shutdown while that record remains charged. After the
writer barrier releases, Rust confirms virtual safe readback, the accepted
measurement and safe evidence commit, the interval and boot seal follow in
FIFO record order, and the worker closes before `recorder_flushed=true` is
returned. Reopened SQL checks measurement < safe evidence < interval seal <
shutdown record. All eight focused shutdown tests pass in debug/optimized
release with fmt and clippy. Production shutdown sequencing preceded this
oracle, so the test is later acceptance coverage. The API-triggered pending
case is covered below; the real process exit watchdog is still open.

D11's separate public API shutdown oracle holds one accepted measurement at
the actual SQLite pre-commit barrier, sends `runtime_shutdown` through the
application boundary, and observes only an accepted response while the worker
is held. After release, the durable prefix and one accepted `shutdown`
operation precede the final sealed boot record; the process-local terminal
response is completed only after safe evidence, seal receipt and worker close,
with `recorder_flushed=true`. The first SQL assertion was red because the test
incorrectly required a `runtime_shutdown` completed operation row after the
worker had already closed. Section 9 of the accepted design explicitly makes
terminal network publication separate from durable sealing, and forbids the
seal from claiming a future network result. The corrected oracle verifies the
durable accepted intent/boot seal and process-local terminal separately. This
was a fixture-contract mistake, not a genuine architectural contradiction.
All nine shutdown tests pass in debug/optimized release with fmt and clippy;
the worker/API implementation preceded this acceptance test.

D12's real server-process watchdog oracle uses a child test process running
the actual loopback `server::run` with a trusted held pre-commit SQLite writer.
The parent sends `runtime_shutdown` over the public wire API. It receives an
accepted response followed by a failed terminal result with Rust safe evidence
confirmed, `recorder_flushed=false`, `recorder_unfinished=true` and unsuccessful
exit. The child process exits nonzero within eight seconds without joining the
blocked storage thread. A fresh SQLite boot retains the old interrupted run as
`unknown_tail` under the same database ID. Both debug/release process tests,
fmt and warning-free clippy pass. This uses a test-only writer barrier rather
than an option in the deployable CLI; it is later acceptance coverage after
the existing finite shutdown implementation. Fatal owner, two stalled Lua
slots and unconfirmed physical safe recovery combined with this process path
remain open D12 cases.

The D15 expected-identity oracle was then tightened. One unavailable/equal-time
fixture had derived its expected old IDs by running the same history query
again, which did not independently prove completeness. Both that fixture and
the escaped-text fixture now assert the known `record_seq` values assigned by
their offline setup: boundary record 1, original observations 2–4, and later
observations 5–6. The first attempt incorrectly counted the boot anchor as a
canonical record and expected 3–5; Store keeps the initial boot bracket in
boot metadata/clock anchors, so those fixture assertions were red and corrected
to the actual independent setup sequence. All eight focused history tests pass.

The first gap-history implementation incorrectly created its new lookup index
on reopen if a version-one file lacked it. Section 7.2 of the accepted design
forbids repairing an incomplete nonempty schema. A new test was red: dropping
`intervals_by_run` still allowed reopen and changed the archive. Reopen now
validates all seven required indexes and the new index's exact two columns
before any boot mutation. The index is created only with a new schema. The
targeted reopen test passes and confirms the missing index and single old boot
remain unchanged after rejection.

D13's first real two-process test was green against current pre-commit failure
behavior. The child holds one actual SQLite writer batch, confirms that owner
credit remains outstanding and its receipt watermark has not advanced, then
the parent kills A. B reopens the unchanged database identity with a new boot;
the old raw selection is empty with `unknown_tail`, and B's own new run has one
independent measurement. A second test first failed to compile for the missing
post-commit/pre-receipt barrier. The worker now exposes this trusted test stage
after a successful atomic batch transaction and before writing its cumulative
receipt. Killing A there leaves the owner unconfirmed; B reads the one committed
WAL measurement and its exact durable checkpoint, while the old lifecycle
remains interrupted. Its first attempt opened a second SQLite connection
while B held the accepted exclusive connection lock and was red for a test
fixture `DatabaseBusy`; closing B before direct checkpoint inspection fixed
the fixture. The full debug workspace passes; clippy's first pass found one
needless borrowed path argument in the test helper, corrected on the next pass.
Public API B and authority non-replay cases remain open.

D10's source-identity test was red: a committed Measurement with source
`fact_seq=7` could be followed by a mixed batch containing a changed
`fact_seq=7` payload; both new rows committed under distinct `record_seq`
values. A version-one partial unique index on `(boot_id,fact_seq)` for non-NULL
source IDs now rejects that collision. The whole mixed transaction rolls back,
its valid leading fact is absent, the prior Good remains and reopen observes
the original checkpoint. No automatic schema repair is introduced.

The first full-suite pass after that index was red in the unchanged finite
shutdown test: the boot seal and worker close had completed, but an owner poll
cloned the prior receipt just before the worker set Closed and read `alive=false`
afterwards. It labeled that normal close `recorder worker exited unexpectedly`.
A new private test first failed to compile for the final reconciliation seam;
its controlled stale clone is now green. Poll reapplies the worker's final
cumulative receipt after observing it closed, including credit and terminal
seal fields, before declaring an exit fault. An intermediate refactor edit
misplaced a Rust brace and failed compilation; corrected before the passing
targeted and full debug workspace runs. Clippy first flagged a test-only
Default-field reassignment, corrected on the next warning-free pass. This
fix does not infer flush success from `alive=false` alone.

D6's new encoded-byte test was red: one 512-byte NUL text fact fit a
2,500-byte worker limit because the old charge counted only the source
String capacity. The owner admission now charges the transferred Vec's full
allocation and a conservative six-byte JSON representation per source byte,
plus fixed envelope/scratch headroom, before sending an atomic group. A later
green test with one scalar fact in a Vec of capacity 32 verifies that unused
allocated slots also consume bounded ingress credit; it was added after the
production fix and is reported as such. The unchanged writable-gap fixture
was then red because its 1-KiB fault budget no longer admitted even the
intended first prefix fact. Its test-only byte limit was raised to 4 KiB while
the one-group saturation remains the trigger for the rejected second fact.
Focused and full debug workspace suites plus warning-free clippy pass.

D5's public annotation test was initially red because `experiment_annotate`
was unknown on the version-one wire. The accepted design's bounded name/data
validator now runs before session admission, including conservative JSON
escaping, depth, item and string limits. The same validator also protects
direct Recorder admission. A real held SQLite writer keeps the confirmed
watermark unchanged while the API completes with `durability:pending` and a
reserved `record_seq`; duplicate request returns that same completed result.
After release and clean shutdown, reopen finds exactly that annotation row,
its original actor scope and Runtime-assigned `local_client` origin. Oversize
and too-deep candidates were added after the production operation and are
green; they leave the logical request sequence available. Additional top-level
`origin`, ACK, safe-evidence and readback fields were also added after the
wire change; strict known-field validation rejects them. The annotation's
nested informational data is never interpreted as transport or safety
evidence. Full debug workspace and warning-free clippy pass. This covers one
D5 slice only; process-crash acceptance/unknown and spanning-interval cases
remain open.

D5's accepted-only process fixture was compile-red for a trusted terminal-only
writer barrier. That seam now allows the accepted operation SQL transaction to
commit, then holds the next terminal operation before SQL. The child performs a
real public Reference retune and verifies its completed domain result and a
confirmed acceptance watermark. Its parent kills the process at the held
terminal stage. After B reopens the database under a fresh boot, SQL contains
the original request's accepted row only, and the interrupted run has
`unknown_tail` coverage. A fresh Runtime Reference still has revision 1; the
old retune is never replayed. The first two runtime attempts failed for test
fixture `DatabaseBusy` while a second connection probed an exclusive live
Store; moving the first proof to the owner receipt and closing B's Store
before direct archive SQL fixed the fixture. Full debug workspace and
warning-free clippy pass. The accepted-only record is not described as a
failed domain action or a completed durable terminal fact.

The D5 no-recursion assertion was added after the historical-read production
path. During an active interval, the public API completes an explicit indexed
run-history selection; after shutdown and reopen, the `operation_events` table
contains no `history_read` audit row. This green regression test does not
retroactively count as tests-first production work.

D6's owner-identity test was initially compile-red because whole-group
admission returned no assigned `record_seq` range. Runtime owner ingress now
reserves an exact contiguous range only after the bounded FIFO accepts the
complete group. The SQLite worker verifies that the next committed identity
matches the reservation, including boundary, periodic clock, operation, gap,
stop and terminal seal records. The held-writer test verifies assigned IDs
before the corresponding SQL commit and finds the same IDs after reopen. A
second held-writer test was added after this production change; it is green
and verifies that an owner-reserved periodic clock record sits between two
facts in FIFO order, even while the first writer transaction is held. This
second test did not precede its implementation. Full debug workspace and
warning-free clippy pass. Annotation admission and all remaining D6 budget
oracles are still open.

D17's deterministic native comparison was added after the existing wall-time
storage implementation. Four real virtual Host runs use identical supplied
monotonic service times while their separate file-backed Store records normal,
forward-jumped, backward-jumped and unavailable UTC anchors. Their complete
PID/EMA state, Reference, lease expiry, sample and virtual output decision
traces match. Reopened SQL keeps each independently supplied actual anchor but
uses the unchanged boot mapping for the `clock_anchor` fact. The first fixture
assertion expected `999999` microseconds at 1 second; the signed pre-epoch
boot mapping actually yields `-1` microsecond and the assertion was corrected.
The second fixture compared global process lease-instance counters across
independent Hosts; it now compares lease expiry and decisions rather than that
allocation identity. This is a green test added after production, not a new
tests-first implementation. Actual live worker wall-read fault injection and
M3 transport decision comparison remain open. Full debug workspace and
warning-free clippy pass.

D1's native/managed integration test was added after the recording and history
production paths. Service A starts an interval with the real virtual plant and
two initialized Lua components, observes separate native and managed Good
publications and at least three distinct confirmed storage watermarks, then
stops and completes a finite shutdown. An independent SQL read after close
finds the exact source values and verifies the old boot checkpoint equals its
maximum committed `record_seq` and one interval seal exists. Service B opens
the same SQLite file under a different boot, discovers the old run through
bounded public `history_read` pages and matches every SQL identity, value and
quality in order for both signals. Focused and full debug workspace suites and
warning-free clippy pass. The fixture uses a bounded 3-second publication
deadline; its sleep spaces real Lua execution, while SQL and public pages are
the actual oracle. It is a green test added after production, not a retroactive
tests-first implementation. Pending-before-Stop WAL coverage remains open.

D1's explicit pending/Stop test was added after the worker's FIFO and WAL
production code. It holds the actual SQLite writer before one accepted fact's
transaction, queues Stop while the owner still reports only the committed
start watermark, then releases the writer. A nonempty live WAL sidecar proves
that the fact/seal commits used the real file-backed journal before close.
After reopen under a new boot, the original fact has its owner-assigned ID,
precedes the interval seal, and the old boot checkpoint equals its committed
maximum. The first expected value `31.0` was a fixture mistake: this trusted
virtual source produces `base 31.0 + 1 second = 32.0`; the known-value
assertion was corrected. Focused and full debug workspace suites plus
warning-free clippy pass. The test was added after production and is reported
as a green regression oracle.

D6's maximum ordinary-credit reserve test was added after the existing
failure-seal production path. A confirmed held writer retains four accepted
groups and eight scheduled history jobs; a fifth group fails atomically and
leaves its ordinary charge at zero. After release, the same file contains the
accepted four-fact prefix, exactly one gap row and a terminal boot seal, while
live status retains its first missing fact and sticky Failed state. The first
test attempt waited for state Closed after worker close; the accepted contract
keeps failure sticky and reports close separately through `worker_closed`, so
the fixture was corrected. Full debug workspace and warning-free clippy pass.
The test is green coverage added after production, not tests-first evidence
for that earlier implementation. In-flight encoder scratch and receipt fault
accounting remain open.

D6/D9 receipt fencing began with two private red tests. Before the fix, an
unreserved future `persisted_through` ID was accepted as committed and a
future `confirmed_submission` moved a Recording owner's progress time despite
no such owner submission. The owner now checks both before applying the
cumulative receipt, uses its own bounded `record_seq` reservation and latest
accepted submission, and latches a sticky Recorder failure for future values.
Older clones are ignored rather than decreasing a previously confirmed
watermark. A stale-probe regression was added after production; it is green
and verifies that a prior 0-second confirmation cannot roll back a real
1-second durable probe. The two original tests were red first, and the later
stale test is reported honestly as after-production coverage. Focused tests,
full debug workspace and warning-free clippy pass. This private receipt
failure oracle does not yet establish D9's complete held-writer multi-output
safe transition.

D13/D15's restart fixture was added after the public history and session
production paths. An archived two-row equal-time run is committed, Service A
retains its first-page cursor, then A closes and Service B opens the same DB
under a new boot. B explicitly rejects A's scope (`instance_changed`), event
cursor (`instance_changed`) and retained history cursor
(`history_cursor_expired`), while a fresh indexed read returns the old IDs
`[2,3]`, complete coverage, an empty nonmatching range with no cursor, and
the archived run in run discovery. These are API oracles after real reopen,
not direct Store guesses. A separate after-production extension to the
existing frozen-W fixture adds a delayed Good publication observed two
seconds earlier than its publication time. Unavailable remains a separate
row; append between pages does not alter the exact old identity union
`[2,3,4,5]`, and a fresh query returns the appended rows. The first edit
accidentally targeted another test's identical `append_facts` line; it was
restored before the focused and full debug workspace runs. Both cases were
green after the earlier production change and are reported that way.
Warning-free clippy passes. Killed-process public paging remains open.

D16's pre-recording configuration test was red against reopened SQL: Recorder
activation had already frozen the initial profile at attach, but subsequent
accepted PID and Ramp retunes before Start left only revision number 2 in the
start boundary. The archive had no full values for that pre-interval change.
The serialized Runtime owner now queries committed `ControllerConfig` and
`Reference` snapshots while constructing the frozen boundary, before enqueuing
Start. The boundary preserves its existing revision arrays and additionally
contains complete PID gains/limits, controller bindings and timing, and the
Reference's value/target/rate/unit/progress time at revision 2. The same SQL
test reopens and checks known `kp=4.5`, `output_min=5.0`, `target=57.0` and
`rate=1.5` at Start. Focused and full debug workspace plus warning-free clippy
pass. This closes the pre-start reconstruction gap; during-interval revision
replay and rejected-candidate oracles were still open at that point. A later
extension of the same test was green against existing production code: accepted
in-interval PID/Reference revision 3 carries full PID and Ramp values, an
invalid revision-4 PID candidate is absent, and an unavailable measurement
record follows both revision records in owner-assigned order. Its first fixture
burst filled the four ordinary group credits, so shutdown correctly failed
with an ingress gap. The fixture now waits for confirmed credit release before
shutdown. This test was added after its production path and is reported
honestly. Complete object/binding lineage, output-at-revision reconstruction
and provenance pressure limits remained open at that point. A subsequent red
real SQLite oracle showed that three manifest references with identical loaded
bytes but different role/encoding reopened with only one typed content row.
The version-one table now keys immutable content by `(SHA-256, kind, encoding)`;
lookups and root lookup use those typed identities. Reopen validates the key
shape before WAL/exclusive setup and rejects an old single-hash interim M7
schema without rewriting its sentinel bytes. The first full debug run exposed
a validator fixture bug: `table_info` returns physical column order, while the
primary-key order is different. Sorting by PK ordinal fixed the two red archive
tests. The full debug workspace and warning-free clippy pass. A separate red
32-KiB managed-source test found Store accepted 32 KiB+1 bytes despite the
existing M5 source bound; Store now rejects before SQL. Green negative tests
also reject zero/129 entries and total owned manifest pressure with no new
activation rows. These production changes followed their red acceptance tests.
Complete object/binding lineage and output-at-revision reconstruction remain
open.

A later green D10 admission-negative fixture reopens three independent real
files: a recognized database with future `user_version=2`, a truncated/corrupt
SQLite header, and an SQLite database bearing a foreign application ID and a
donor note. All three are rejected before any conversion/repair, with exact
main-file bytes unchanged. This coverage was added after the existing startup
validation; the focused 15-test transaction suite and warning-free clippy
pass. It does not replace a separate crash/corruption process oracle.

D11's later green two-Run host oracle was added after the existing lifecycle
production path. A real serving host starts Run 1, explicitly stops it, then
starts Run 2 and shuts down while its interval is active. Reopened SQL shows
two distinct sealed/complete Run identities and exact FIFO order:
`Boundary1 < Seal1 < Boundary2 < Seal2 < Shutdown`. The durable checkpoint
equals the final Shutdown record; reopening under a new boot leaves the two
old Run IDs intact. The focused five-test shutdown suite and warning-free
clippy pass. Full accepted-prefix/pressure and public shutdown archive cases
remain open.

D12's first late-seal extension was runtime red for a fixture reason: holding
an earlier fact kept Stop unconfirmed until the process timeout, so Finish had
never been admitted and no late boot seal was owed. That extension was removed
from the existing stalled-writer test. A separate tests-first seam was compiler
red until `held_finish` was added. It holds only the already admitted Finish
immediately before its SQL transaction. The process returns a finite,
safe-confirmed but recorder-unfinished/nonzero terminal result after its flush
grace; releasing the writer later produces one durable `shutdown` record and
a sealed old boot. A new boot reopens that committed seal without retroactively
turning the earlier process result into success. The six-test shutdown suite
and warning-free clippy pass. Close failure and actual watchdog/process cases
remain open.

D13's later green public-B oracle was added after the existing process and
history production paths. The parent kills a real A after its fact transaction
commits but before owner receipt. B starts the full ServiceHost on the same
database ID, with a distinct boot, and public `history_read/history_page`
returns A's exact Good value and `unknown_tail` coverage. A stale A event
cursor is rejected as `instance_changed`; B's native controller is freshly
Ready and its virtual output has no lease. The six-test process-reopen suite
and warning-free clippy pass. The killed A in this case used the Store worker
directly rather than holding an output lease, so the stronger active-authority
replay oracle and a real old scope/history cursor from killed A remain open.

D7's later green real-socket isolation oracle was added after existing
concurrency production boundaries. A service owner is constructed on its own
thread because Runtime intentionally does not implement `Send`; the test first
failed to compile when it tried to move an already constructed HostCore across
threads. SQLite then holds one admitted controller fact while the native PID
runs for 6.3 seconds, beyond three 2-second lease lifetimes. Before release,
the loopback client gets Controller/Output/Reference Queries in under one
second, observes an advancing native tick, automatic output ownership and a
finite renewed lease. BestEffort overflow reports failed coverage while native
control continues. The first fixture expected `server::run` to return `Ok`
after that failed recording coverage; it correctly returns an incomplete
shutdown error, so the test now asserts that outcome. The three-test
isolation suite and warning-free clippy pass. Actual M3 recovery and both
stalled Lua worker slots under the same writer hold remain open.

D10's later green checkpoint-failure oracle uses a real SQLite trigger at the
final `durable_checkpoints.persisted_through_seq` update, after the new fact's
measurement projection has been inserted. The entire attempted fact transaction
rolls back; the previous boot's durable fact and final checkpoint remain, and
the new boot stays at its Start boundary checkpoint. This test was added after
the existing transactional production path. Its first expectation used the
old boot's pre-Stop sequence instead of its final boot checkpoint; correcting
that fixture made the focused and 12-test transaction suite green. Warning-free
clippy passes. An actual commit failure and visible quota status remain open.

D8's worker-panic test first failed to compile because the trusted seam was
absent. After adding a panic immediately before a fact SQL transaction, it
failed at runtime: the Rust thread died before its final alive/receipt stores,
and repeated owner polls still reported `Recording`. The owner now checks the
thread's finished state without joining; a panic reports `Failed`,
`unknown_tail`, `worker_closed` and the stable first error. The last confirmed
Start watermark remains unchanged, the unconfirmed fact credit is not released,
and repeated polls do not invent a receipt. Reopen under a new boot marks the
old run interrupted with zero old measurement rows. Focused and full debug
workspace suites and warning-free clippy pass. Other disk/encode/stall failures
and the complete D8 scope remain open.

A later green D8 test added after the existing worker SQL-error path uses a
real SQLite `RAISE(ABORT)` trigger on the fact projection. Start is committed
and confirmed before the fact arrives. The worker surfaces its stable first
SQL error, closes, leaves `failure_persisted=false`, and never advances the
owner's Start watermark for the attempted fact. Reopen discovers an
interrupted/unknown-tail run with zero measurements. The focused three-test
failure suite and warning-free clippy pass. This trigger exercises a real
SQLite insert failure; physical disk failure remains a separate open oracle.

D9's later green Required-host integration was added after the worker-panic
detection path. With a confirmed Start, the native controller accepts Start and
its first fact then kills the storage thread before SQL. A subsequent owner
service unit observes `Failed` Recorder coverage, faults the controller,
revokes its output lease and latches the output fault while independent Query
still responds. The owner does not wait for SQLite. The four-test Required
suite and warning-free clippy pass. Multiple controllers/outputs, a failed
physical safe recovery and queued M3 bytes remain open.

D10's public status test was red after a confirmed real-file Start: the cached
`recording_status` response had no ingress limits or storage footprint. The
SQLite worker now samples logical main pages, checked main quota, actual WAL
sidecar bytes, threshold and successful checkpoint count into its cumulative
receipt after SQL boundaries. The Runtime owner copies only that receipt;
`recording_status` performs no hidden disk work. After a quiet confirmed Start,
the test matches reported WAL bytes against the live sidecar and checks the
1-GiB quota, 16-MiB threshold and 1024-record/4-MiB/4-group limits. Its first
request failed because the test had not established an M6 hello session;
after adding hello, the missing fields were the observed runtime red. The
focused and full debug suites plus warning-free clippy pass. Public checkpoint
counter/failure and large-file/corrupt-process cases remain open.

D10's later green actual-COMMIT oracle was added after the existing production
transaction path. A real SQLite deferred foreign key, inserted by a trigger
after the attempted measurement and checkpoint update, accepts all preceding
SQL statements but rejects `COMMIT`. Direct Store rolls back the entire new
fact and deferred probe; the old boot's final checkpoint and the new Start
boundary checkpoint remain. The same file then triggers a worker COMMIT
failure: owner status closes with `Failed`/`unknown_tail`, never advances the
Start watermark for the attempted fact, and reopen under a fresh boot finds
one old durable measurement and an interrupted failed run. The first fixture
used a 34-character boot ID; a later edit accidentally placed the worker
extension in the checkpoint-update test. Both fixture errors were corrected
before the focused and 14-test transaction suite passed. Warning-free clippy
passes. The separate committed-before-receipt kill oracle is recorded under
D13; public checkpoint failure status and corrupt/large-file startup remain
open.

D14's later green query-plan oracle was added after the existing indexed
archive selection. The same 20,000-row real SQLite fixture reopens externally
after its bounded 11-row/two-page tail read. `EXPLAIN QUERY PLAN` for the
equivalent narrow boot/run/signal/time keyset selection names the mandatory
`measurements_history` index. The focused eight-test history suite and
warning-free clippy pass. Explicit selected-row instrumentation, malformed
field matrix and slow/nonreading peer pressure remain open.

A later green D14 public matrix was added after the existing wire/application
validation: raw frames with SQL/path/downsampling/unsupported-envelope or
unknown nested fields fail `decode_frame`; unterminated and overlarge frames
also fail there. With a real hello session, malformed max-record bounds,
noncanonical/overflow time IDs, reversed/equal ranges, overlong cursor,
unsupported mode, mismatched run boot and negative signal ID each return one
error without an accepted history operation. The seven-test history API suite
and warning-free clippy pass. This covers admission validation but does not
yet prove selected-row instrumentation or slow-peer isolation.

The full debug workspace suite passed after the first safety/storage fixes,
including the unchanged real Babashka process test. That pass does not close
the M7 D1-D18 gate. This report will be completed with exact final commands,
versions, limits, durability/process evidence and limitations only after all
remaining contract rows pass.

D17's live-worker UTC failure test was red at compile time because no trusted
one-shot wall-read fault seam existed. The SQLite worker now permits the harness
to fail one periodic UTC read after a valid boot anchor. Its periodic record
retains a nullable actual UTC and bounded `wall_read_failed` reason, while the
record's estimate stays tied to the boot anchor and Recorder remains Idle until
a normal Finish closes it. The focused eight-test time suite and warning-free
clippy pass. This test was added before the seam; complete D17 transport send
decisions and cross-boot ordering remain open.

D7's system isolation fixture was extended after the already implemented
Recorder path. Both real Lua worker slots enter confirmed barriers; the real
M3 fake-byte adapter enters its timeout/recovery path; a separate SQLite worker
commits Start and then reaches a confirmed held fact SQL stage. While all three
workers remain held or recovering, the loopback client receives Controller and
Output Queries, the native controller reaches Running and its finite lease
deadline advances without epoch/owner changes, and M3 recovery attempts progress.
The held SQLite fact is released only after those assertions. All three
`lua_network_isolation` tests and warning-free clippy pass. This is later green
coverage, not a claim that its production path was originally tests-first.
D7's distinct-input/delivery and full three-lease M3 combination remain open.

D14's explicit selected-row assertion was compiler-red because an indexed
page exposed only returned rows. The worker now counts decoded selection rows,
including at most one lookahead, as a bounded `HistoryPage` diagnostic. In the
real 20,000-row archive's 11-row tail range, the first 8-row page decodes nine
rows and the terminal 3-row page decodes three. The existing independent query
plan and 10,000-op budget still verify the indexed narrow selection. The
application's retained completed-history enum boxes the now larger measurement
page to keep its variants bounded and warning-free. The eight-test history and
seven-test public archive suites plus clippy pass. This red/green addition does
not yet cover slow peers or pending-job generation fencing.

D10's public checkpoint fault fixture was compiler-red for the trusted low-WAL
threshold seam. Its first attempted external reader failed for a fixture reason:
the accepted Recorder connection uses exclusive SQLite ownership, so an
independent read connection cannot hold a WAL snapshot during the serving boot.
The test now injects one failure only after the worker observes an actual
above-threshold WAL sidecar and after Start has completed at least one successful
explicit checkpoint. The failed next fact does not advance the owner watermark;
`recording_status` retains the previous successful checkpoint count and stable
first error. A first API assertion used the wrong public key
`persisted_through_sequence`; the corrected `persisted_through_seq` response
passes. The three-test startup suite, full debug workspace and warning-free
clippy pass. The checkpoint error is a trusted gate injection, not a physical
disk error; large-file/process startup and physical fault cases remain open.

D3/D4's virtual SQL expectation was runtime-red: `output_events.unit_key`
was NULL for a requested percent value. Core now freezes the engineering unit,
Rust authority epoch and applicable resource/binding generation/mapping revision
in each output fact before handing it to the host; SQLite projects those typed
fields rather than looking up today's binding. A later green Core M3 fake-wire
test and a new real-file M3 ACK archive both preserve percent, the original
manual epoch, resource one and generation/revision one through all four stages.
The M3 terminal row remains `transport_protocol` ACK with no readback row.
This fixes production code after a red virtual acceptance assertion; the M3
archive coverage was added after the fix and is reported as later green.

A second runtime-red virtual SQL assertion found that a rejected Celsius
proposal had been recorded under the actuator's expected percent unit. The
Core proposal capture now stores the proposed unit on requested/rejected facts,
while later authorized/send/evidence facts use the trusted actuator unit.
Another red storage-negative test showed a direct output fact with no trusted
unit could commit as NULL. The Store now rejects missing unit/epoch or partial
resource/binding identities before a transaction advances its checkpoint; a
subsequent valid virtual fact commits. The full debug workspace, focused Core
and SQLite suites, formatting and warning-free clippy pass. Supersession,
binding-generation changes, physical partial/late evidence and complete
output-at-revision reconstruction remain open.

D4's queued-proposal review found no fact when a successful `RequestSafe`
invalidated the pending ordinary attempt. A compiler-red test introduced a
distinct `superseded_before_send` stage; the first runtime green still had no
separate revoke row and assigned the old attempt the new authority epoch.
The corrected capture records old requested/superseded under the original
attempt and epoch, then `revoked` and `safe_requested` under a distinct current
safe attempt/epoch. A later green real SQLite archive confirms those four rows,
their unit, attempt IDs and epoch bytes through reopen.

A new runtime-red manual Release test initially reopened only `requested`.
Release and Trip now use the same explicit safe transition capture, with
supersession of a queued candidate where present. Safe attempt-counter exhaustion
is scoped to audit: the one lost correlation makes the bounded Core outbox
sticky-incomplete, while a private Core test confirms `RequestSafe` and safe
dispatch still execute. A separate runtime-red exact lease-watchdog test had
only `requested`; both direct Output commands and the shared safety service
now capture `expired_before_send` for the old queued attempt before revoke and
new safe intent. Every changed authority is recorded immediately even if the
enclosing command later fails. The full debug workspace and warning-free clippy
pass. Physical first-byte/recovery and older in-flight ambiguity still require
additional D4/D9 oracles; this does not establish physical safe evidence.

D5's later green public Start/Pause fixture uses the existing trusted writer
seam to commit `controller_start` acceptance while its terminal SQL remains
held. The domain reply is already truthful `completed/warming`; disconnecting
the first client neither cancels the pending durability nor executes the
command again. After release, a new client pauses that controller, and a
duplicate pause request returns the retained completed result without a second
mutation. Clean shutdown/reopen finds exactly four FIFO operation rows:
Start accepted/completed under scope A, then Pause accepted/completed under
scope B. All four `recorder_operations` tests and focused clippy pass. This
coverage was added after production operation capture, so no retroactive
tests-first claim is made; other cross-interval terminal states remain open.

D2's existing reopened scalar oracle now checks constructor rejection for NaN,
positive infinity and negative infinity before storage admission, alongside its
exact i64 extremes and delayed observation/published times. The three-test
quality suite, formatting and focused warning-free clippy pass. This extends
acceptance coverage after the preexisting finite-value validation and is not
a retroactive tests-first claim; managed-publication pressure remains open.

D18's real recorded Babashka process fixture was added before extending the
Babashka acceptance entry point. It initially failed with an unknown `record-a`
mode. After the new mode existed, one isolated run passed, but repeated paired
M6/M7 runs exposed real Required ingress saturation: the fixed four-group
Recorder credit could fill during a burst of application acceptance/terminal,
native facts and safety transition facts. Recorder correctly latched Failed and
native control failed closed; the first isolated green did not prove acceptance.
The Babashka recorded variant now drains transient subscription events and polls
the public `recording_status` Query for released bounded group credit before
each burst of operator mutations. Polling is limited to three seconds, sleeps
between retries, and throws immediately on Recorder Failed; it neither requests
larger production limits nor changes output/safety authority. A second failure
at B's stop was likewise diagnosed as ingress saturation after Pause, not a
recording run identity mismatch or a missing safe readback. B now waits for the
already verified safe output and released credit before Stop.

The unchanged recording-disabled M6 Babashka A/B fixture and new M7 fixture pass
together in three consecutive focused runs. The M7 fixture uses actual Babashka
and Runtime processes: A starts Required recording and is killed while its native
controller is Running; B reconnects after more than six seconds, observes the
same run and autonomous plant movement, pauses to Rust-verified safe readback,
stops recording and shuts down. A new boot opens the same SQLite file and pages
bounded Good measurement history, including rows after A's checkpoint. These
green runs are a focused acceptance slice; full debug/release and `bb test-client`
remain outstanding, and Required ingress fail-closed is still an intentional
behavior under unpaced or stalled production workloads.

D12's post-seal close-fault oracle was compiler-red because the trusted worker
had no close-stage injection seam. The seam now simulates a close error only
after the actual SQLite Finish transaction commits its boot seal. The public
Service shutdown result still confirms virtual Rust safe readback, but reports
Recorder error, no successful flush and a nonzero exit result. Owner status
retains `terminal_seal_committed=true`, Failed, worker closed and a bounded close
error. A new boot opens the actual file and finds exactly one sealed old boot;
the failed close receipt neither removes the durable seal nor claims that the
worker closed successfully. This is a synthetic close-stage fault after real
SQL durability, not a demonstration of a physical SQLite `close()` error. All
seven focused shutdown tests and warning-free clippy pass. D12's real process
watchdog, commit-failure and M3/Lua combined safety paths remain open.

The actual `bb test-client` task also passes eight Babashka tests and thirteen
assertions after the recorded process-mode additions. It verifies the public
client helpers; the separate process A/B fixture supplies the recording/reopen
oracle. The task is part of the final gate and should be rerun with final code.

D6's later green full-batch oracle charges four independently captured 64-fact
groups while a confirmed SQLite writer stage is held. The owner reports exactly
256 outstanding records, four groups and less than the fixed 4-MiB credit; after
release, one real SQL fact transaction commits all 256 distinct measurement IDs.
The final durable commit number is four for Start, that one fact batch, Stop and
Finish, and the receipt releases all charged records/groups before either seal.
All nine focused backpressure tests and warning-free clippy pass. The worker's
batch path preceded this acceptance test; this is not tests-first evidence for
that original production path. Peak encoder scratch, nested payload accounting
and a precise oldest-group cadence fault remain open.

D8's private Host/Core outbox oracle was runtime-red after synthetic 257-fact
publication without the normal per-command Host drain: Core marked its first
lost fact, Recorder latched a gap, but `HostCore::recording_status` still returned
Recording because the owner copied its status only at a later poll. The fixed
owner admission path polls immediately after both successful and failed handoff.
The corrected test sees Failed/gap and the same first missing fact in that owner
unit, with a stable `core fact outbox overflow` first error. While the SQLite
writer is held, the BestEffort native controller remains eligible; after release
it reaches Running, and Finish persists exactly one real SQLite gap row. This
uses a trusted private batch to pressure the Core outbox, so it tests the Host
failure bridge rather than claiming a normal client can issue 257 facts in one
unit. The full debug workspace, warning-free clippy and formatting pass. Physical
disk/encoding/stall faults and repeated diagnostic exhaustion remain open.

The first full optimized release run was red at D18 despite the earlier debug and
focused green runs. The recorded Babashka A procedure started Required coverage
before its initial Reference/PID configuration burst. In the optimized process,
that burst could fill the four fixed ingress groups even with inter-command
credit polling; Recorder correctly failed closed with a durable gap. The accepted
D18 behavior does not require recording these safe pre-start configuration
commands. The recorded variant now configures Reference and PID while Required
control remains closed, opens a committed recording interval, and only then
starts the native controller. It still records autonomous Running progress,
continuous retune, B's pause/safe/stop/shutdown and the reopened archive. Both
M6 and M7 process tests pass together in three consecutive optimized focused
runs and a full optimized workspace run. The original normal-load failure is
recorded here rather than hidden by a single later green run. The Required
four-group fail-closed rule and fixed limits are unchanged.

D4's new actual M3 partial-byte Core acceptance was runtime-red first for a
fixture expectation: `PollTransports` alone does not automatically request a
new safe procedure after recovery. The test now explicitly invokes trusted
`RequestSafe` and polls its queued byte work, as required by the reviewed output
lifecycle. Once that was corrected, the captured recovery `ambiguous` row still
used the authority's later epoch rather than the frozen ordinary intent's epoch.
The Runtime now retains the immutable `OutputIntent` with an unresolved M3
dispatch and emits M3 send/ACK/uncertain/recovery facts from its original unit,
epoch, resource and binding generation/revision. The ordinary two-byte prefix is
never retried or recorded as ACK/readback; recovery marks it ambiguous under the
old attempt/dispatch/epoch. A separate safe attempt is requested and receives
trusted protocol ACK under its own correlation, with no ordinary ACK or invented
readback. The test was red before this production correction and now passes.

A later green host-storage integration uses an actual Core M3 fake-byte adapter,
not hand-made output rows. The real SQLite file reopens under a second boot with
nine ordered output stages: requested, authorized, send started, transport
uncertain, ambiguous, revoked, safe requested, safe send started and safe ACK.
SQL projections retain the old attempt/dispatch/epoch and original percent,
resource one and binding generation/revision one through ambiguity. The safe
attempt's ID and epoch differ, and only its ACK has `transport_protocol`
evidence. The full debug workspace, optimized focused archive test, Core suite,
formatting and warning-free clippy pass. Late ACK/readback after a newer epoch,
physical recovery failure, and first-byte fencing with a held SQLite worker
remain open D4/D9 coverage.

D13's active-authority process oracle was runtime-red before its child fixture
existed: the parent could not observe an actual held SQLite stage. The new child
opens a real file-backed BestEffort worker, confirms a fact transaction has
committed but its owner receipt is held, and independently verifies that its
native controller is Running with an active Rust lease. The parent kills that
actual process at the printed barrier checkpoint. A fresh ServiceHost B opens
the same database ID under a new boot, reports Ready control, no lease, no
pending or in-flight ordinary output and an Idle Recorder. After B's clean
shutdown, the old run is `interrupted/unknown_tail` rather than resumed or
replayed. The child uses the existing bounded process-kill harness; no donor or
physical hardware is involved. All eight focused process tests pass in debug
and optimized release, with formatting and warning-free clippy. Old scope/event
cursor rejection is covered by earlier A/B API fixtures; a killed active A's
old history cursor and archived output-evidence inspection remain open.

D3/D16's managed-provenance slice began with a compiler-red acceptance case:
the archived measurement row had no separate managed state revision. The new
Core facts and SQLite projection keep immutable definition revision 1 distinct
from state revisions 1, 2, and replacement 0. A delayed Transform test freezes
its Source's generation, definition revision, state revision, exact value/unit
and observed/published times before the worker runs. Source progresses from
42 to 43 while the Transform completion is held; the reopened derived row and
bounded history page still name the original Source revision/value, not the
later live state. This is genuine tests-first for the new state/lineage fields,
but existing managed measurement capture and SQLite worker code preceded it.
An additional later green storage-negative case rolls back a malformed lineage
before advancing the checkpoint.

The first full debug workspace run after this change was red in the actual
recorded Babashka A/B fixture: Required Recorder failed with `inconsistent
transform input lineage`; an isolated rerun showed the exact stable first
error. A successful Transform observation uses its captured input freshness
time, while an Unavailable/Warming attempt records its own attempt time and may
still retain the valid captured input. The original validator incorrectly
required both times to match for every quality. A new real-SQLite Warming
Transform test was runtime-red under that rule; after restricting the equality
to Good rows it reopens Unavailable/ProcessingWarmup with an original captured
input and no fabricated Good value. The full debug and optimized release
workspace suites now pass, including the paired real Babashka fixtures. This
is a corrected implementation defect, not an architectural contradiction or
a change to the accepted M7 design. Public history lineage, rebind and broader
binding-at-revision acceptance are still open.

A separate later-green public API test pages a reopened managed Transform row
after an unrelated ServiceHost restart. Its `history_read` Operation followed
by cached `history_page` Query returns the typed state revision and original
upstream generation/revision/value/unit, while a native row's lineage stays
null. This verifies the wire projection after the production field existed;
it is not retroactively tests-first.

D6's new lineage-memory test was runtime-red: a small byte-credit profile
accepted one managed Transform fact even though its separately allocated
SQLite lineage encoder envelope was not charged. Admission now reserves the
envelope's maximum 512 bytes in addition to the inline fact/Vec allocation.
The group is rejected atomically before worker transfer, status keeps zero
outstanding bytes and the reopened file has no measurement row. The full debug
workspace, focused optimized backpressure and paired Babashka process suites,
formatting and warning-free clippy pass. This is a tests-first correction to
the new lineage charge, not proof of all D6 nested/batch peak requirements.

D10's later-green same-ID collision oracle commits a source fact, then attempts
a second two-row group containing the same `fact_seq` with a different value.
The existing unique index rejects the collision and rolls back that whole
later group/checkpoint; reopening retains only the original 21.0 value. The
first test run was red because its independent raw SQL reader was opened while
the reopened `SqliteStore` still held ownership; dropping that fixture owner
before the reader corrected the test without production change. All 18 focused
transaction tests pass in debug and optimized release, with fmt and clippy.
This covers a collision acceptance case added after the index existed; it is
not retroactively tests-first. Large-file startup and physical storage faults
remain open.

## Historical user-requested pause checkpoint — 2026-09-16

The current logical code commit is `53880ca` (`test(recorder): exit blocked
writer process after flush watchdog`). It adds a real server child process with
held SQLite precommit, a public Shutdown request, a finite flush timeout and an
unsuccessful process exit without joining the blocked writer. A fresh process
reopens the same database and observes the old run's `unknown_tail`. Its named
debug and release tests, formatting and focused warning-free clippy pass. This
is later acceptance coverage for an existing watchdog, not an invented
tests-first production sequence.

Other recent accepted slices add managed state/input lineage through SQLite and
public history, charge bounded lineage scratch, reject repeated source fact
identity atomically, retain the original output epoch for partial/recovery and
late M3 ACK, keep Required M3 recovery evidence unknown, separate durable
coverage from client-ring eviction, page a killed process's committed prefix
under a new boot, isolate native control from a nonreading history peer, and
exercise held-writer Shutdown/Finish through host and public API. Some were
later-green tests of existing production; observed red tests and fixture
corrections are described above and in the commit history. No architectural
contradiction with the accepted M7 design has been found.

Those pause-time items were subsequently closed by the named tests in the final
matrix. The continuation added actual held-writer M3 first-byte and Required
multi-output recovery, delayed managed-lineage peak accounting, physical startup
and one-GiB archive rejection, fatal/M3/two-worker shutdown, stale connection
generation fencing, exact M3 wall-jump decisions, and complete M3 activation and
rebind provenance. The provenance acceptance first found a real missing M3
activation binding; Core gained only a read-only binding snapshot and the host
now freezes it before storage. No transport handle, authority or I/O crossed
into provenance.

The first final debug workspace run found another real acceptance failure rather
than a production safety violation: recorded Babashka B could issue Pause while
a 100-ms native tick raced for the fourth fixed Recorder group. Required correctly
failed closed. The acceptance client now aligns the pause immediately after a
new public `last_tick`, waits for zero outstanding groups, and verifies the tick
did not change before issuing Pause. Limits and production fail-closed behavior
were not enlarged or weakened. Three consecutive focused debug runs, one focused
release run, and the later complete debug/release gates passed.

## Final versions and fixed limits

Verified tool and dependency versions:

- rustc 1.95.0 (59807616e 2026-04-14)
- cargo 1.95.0 (f2d3ce0bd 2026-03-21)
- Babashka 1.13.220
- rusqlite 0.40.2
- libsqlite3-sys 0.38.2 with bundled SQLite 3.53.2
- sha2 0.11.0

Implemented M7 bounds match the design: Core outbox 256 facts/256 KiB; Recorder
ingress 1,024 records/4 MiB/four groups; one group and worker batch 256 records/
512 KiB; 100-ms oldest-group cadence; 250-ms committed progress probe; two-second
Required progress and shutdown flush deadlines; eight history jobs; 128 rows,
8 KiB page data, 100,000 VM instructions and 50-ms history execution; two-second
job submission, five-second completed result and 30-second cursor TTL; 16,384-byte
wire frame; 16-MiB WAL checkpoint threshold; checked one-GiB main database quota
with five-percent reserve. Nested lineage scratch, owned Vec capacity and escaped
text are included before admission. These are application-owned bounds, not a
whole-process RSS or SQLite page-cache promise.

## Final latest-head verification

The completion gate was run on 2026-09-16. After the documentation-only rustdoc
link correction, the full debug and optimized release suites were rerun on the
final source HEAD. Results:

```text
cargo fmt --all -- --check                                      PASS
cargo test --workspace                                         PASS (345 named tests)
cargo test --workspace --release                               PASS (345 named tests)
cargo clippy --workspace --all-targets -- -D warnings           PASS
RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps        PASS
cargo run -p lab-runtime                                       PASS (finite demo)
bb --version                                                    PASS (1.13.220)
cargo test -p lab-runtime --test babashka_reconnect -- --nocapture
                                                                PASS (2 real process tests)
bb test-client  (clients/babashka)                              PASS (8 tests, 13 assertions)
recording-enabled A kill / B reconnect / Pause / Stop / Shutdown /
SQLite reopen and bounded history inspection                   PASS
git diff --check                                                PASS
git status --short                                              clean before report edits
```

The recording-enabled process oracle starts Required recording, kills actual
Babashka A while the native controller remains Running, leaves the Runtime and
finite lease renewal autonomous for more than six seconds, reconnects actual
Babashka B, pauses to Rust-confirmed safe evidence, stops/seals/shuts down, opens
the same SQLite archive under a new boot and reads Good rows committed after A's
checkpoint. `bb.exe` remains optional and is not required by normal Runtime
startup.

## Remaining limitations

M7 does not certify physical actuator effects, power-loss atomicity, arbitrary
remote security, downsampling, configuration reload, real Windows COM, GUI or a
persistent Lua workspace. M3 fake-byte fixtures distinguish ACK, readback and
unknown physical evidence; they are not hardware qualification. Injected panic,
checkpoint, close and SQL-trigger faults exercise exact software boundaries but
are not claims about every OS/filesystem failure. Raw history is bounded keyset
paging only. Required deliberately fails closed when fixed recording credit or
durable-progress deadlines are exceeded; it does not auto-recover or auto-rearm.

No architectural contradiction was found. M7 is ready for external review, and
M8 remains unauthorized until that review and the next explicit gate.
