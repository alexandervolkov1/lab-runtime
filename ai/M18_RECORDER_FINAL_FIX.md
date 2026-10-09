# M18 Recorder: accepted second-review remediation

**Final external review: APPROVE (2026-10-09).** The three fixes below are accepted;
no further Recorder production change is required. Pre-admission, capacity and
Recorder architecture are frozen unless an invariant violation is established.
The user authorized a local logical Recorder commit followed by preparation for
consolidated M18 review. The record below describes the pre-commit verification
snapshot; its earlier pending-review statements are historical, not current status.

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
HEAD: `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`; index remains empty.
No commit, merge, push, tag or release. Final external acceptance is not claimed.
This report supersedes the current defect status in `M18_RECORDER_REREVIEW.md`;
the original diagnostic tests, RED logs and databases remain available.

Evidence: `target/m18-recorder-finalfix-20261009/`. The initial 339 source/document
files were copied and fingerprinted before editing. `production-delta.patch`
contains exactly the three production-file changes relative to that snapshot;
`review-delta.patch` also includes tests and documentation. Existing uncommitted
M18 implementation is preserved; neither HEAD nor the index was changed.

## A. Late receipt, safety facts and Reference completion

The old completion path polled Recorder after reserving Reference completion.
`Runtime::confirm_recording_progress` first services the previous Required deadline;
a late receipt can therefore call `recording_failure`, close Required output coverage
and append Controller/Output safety facts to the Core outbox. Recorder itself can
still be Recording. The old final command fence checked only Failed/Closed, so
Reference revision changed, and capturing the mixed group subsequently failed the
single-Reference bound. Accepted remained durable, with a gap and no terminal audit.

The change is a producer-phase boundary, not a split of an already mixed completion:

1. [prepare_reference_recording](../apps/lab-runtime/src/host/recording.rs#L708)
   polls current receipts, then drains the entire existing Core causal group through
   ordinary ingress with Reference capture disabled. No facts are synthesized,
   renumbered, filtered or retagged. Identity, sequence, generation, revision,
   timestamps, lineage and provenance are passed through unchanged.
2. It rejects Required Failed/Closed, and also rejects a closed Core Required gate
   while Starting/Recording. Ingress failure is published through the existing
   failure/gap path before returning `RecordingUnavailable`.
3. [prepare_reference_completion](../apps/lab-runtime/src/host/recording.rs#L674)
   uses this boundary around reservation validation. Crucially, the final
   [command_with_cause](../apps/lab-runtime/src/host/lifecycle.rs#L212) uses it
   immediately before synchronous Core Reference dispatch as well. There is no
   further HostCore receipt poll between successful drain/check and that dispatch.
4. Ordinary worker ingress may observe SQL receipts, but it cannot produce Core
   safety facts. The single owner does not interleave acquisition or probe producer
   work inside synchronous Reference dispatch. The Reference fact then enters the
   existing reserved completion envelope; fact and terminal retain one SQL
   transaction. The exact completion shape/budget is unchanged.

The late-receipt regressions now reject before Reference revision changes. SQLite
contains accepted, then the safety Output Revoked fact, then failed terminal, in
that FIFO order, with no gap. Shutdown drains to zero credits. A second regression
releases the receipt after preparation and observes it at the final synchronous
command boundary, covering the last-poll case explicitly.

## B. Unused probe reservations

The old [fact capture path](../apps/lab-runtime/src/host/recording.rs#L1240) removed
probe tokens from the owner map before choosing Reference capture. On a rejected
capture neither the writer nor the owner retained responsibility for those tokens.

The Reference branch now saves the capture result, cancels every removed unused
probe token, then processes the original result. [poll_recorder](../apps/lab-runtime/src/host/recording.rs#L1069)
also takes/cancels owner-held probe and rebind tokens on Failed/Closed. A second
cleanup cannot find the token. No submitted group is cancelled by these paths.

The original capture-error test retains its failure/gap assertions and now checks
zero outstanding groups, records and bytes after draining, before any test cleanup.
It also proves the removed token can no longer be cancelled. The added failure
transition test holds a real submitted baseline receipt: the unused probe is
released immediately, the submitted baseline remains charged, and only its actual
cumulative SQL receipt releases it to zero.

## C. Required failure during reconnect

The old `reserve_rebind_facts` returned `Ok(true)` for non-Recording state and for
an existing reservation before checking Required failure. A real deferred-FK SQL
COMMIT error could therefore be followed by transport replacement and generation
1 to 2; the later configuration lifecycle fence rejected too late.

[reserve_rebind_facts](../apps/lab-runtime/src/host/recording.rs#L1304) now checks
Required Failed/Closed immediately after the current Recorder poll, before either
shortcut. Existing callers propagate rejection before `replace_transport`, Core
rebind and binding generation changes. Poll cleanup releases unused fact tokens.
[finish_recorded_lifecycle_detailed](../apps/lab-runtime/src/service/configuration.rs#L820)
also cancels an unused activation reservation when its FIFO transfer fails, before
the existing failure/quiesce cleanup. Submitted SQL work remains charged.

Both deferred-FK COMMIT regressions reject with generation still 1, whether rebind
credit was reserved before the failure or not. The only remaining credit is the
one already submitted fact whose transaction failed: one group, one record,
1,920 bytes. It is not reported durable or falsely released. SQLite retains the
last committed prefix; status is Failed/unknown_tail, with no invented terminal
durability. BestEffort still permits rebind, exposes Failed/gap explicitly, and
releases unused tokens; its dedicated regression verifies generation 2 and no
fabricated measurement evidence.

## Invariants and capacity boundary

- Pre-admission, mutation identity, admission fencing, generation validation and
  wire sequencing are unchanged. Required rejection occurs before the Reference
  or rebind side effect; lifecycle failure does not roll back/rearm old transport.
- The pre-effect causal group is transferred intact before the synchronous effect.
  Writer FIFO assignment, SQL transaction code and cumulative receipt validation
  are untouched. `confirmed_submission` remains monotone committed-prefix evidence.
- Cancellation addresses only owner-held, untransferred credit, which has no FIFO
  record identity. Transferred reservations are absent from those owner maps and
  remain charged until a valid cumulative post-COMMIT receipt. Duplicate/stale
  receipts and stale activation tokens retain the existing checks.
- The Reference completion bound remains at most one Reference fact plus terminal;
  failure before the effect uses the terminal-only completion. Reservation does
  not guarantee SQL success. Actual overload and SQL failure retain gap/unknown-tail
  diagnostics and Required fail-closed behavior.
- The approved 13 groups / 1,545 records / 4 MiB and SQL coalescing policy are
  unchanged. Four acquisition groups + five reconnect groups + two Reference
  pairs fit the supported envelope. Only Reference headroom is protected;
  acquisition, safety, rebind/probe and ordinary audits share ordinary capacity.
  Reconnect has no separate protected budget or arbitrary-load availability
  guarantee. No new queue, worker, wait, retry or automatic replay was introduced.

## Test fixture corrections and retained failures

No test was disabled, assertion weakened or production deadline widened.

- Parallel manual-time Recorder unit fixtures allowed real periodic ClockAnchor
  admission to precede activation or occupy a held FIFO position. This changed
  asserted prefix numbers and could wait behind their own barrier. A `#[cfg(test)]`
  manual owner clock now fixes periodic admission at the fixture's chosen time;
  the storage worker keeps its real clock and real SQLite/barriers. Clock and
  normal-process tests retain production timing. The seven initial Linux library
  failures and SQLite proof of the extra ClockAnchor are retained.
- `recording_boundary_and_ordered_revisions_reconstruct_current_config` used
  scheduler turns while waiting for Recorder activation/drain. Under parallel load
  it produced unrelated periodic acquisitions and a genuine capacity gap. It now
  polls only receipts during those waits. The existing injected measurement failure
  already publishes the authoritative fact; all original provenance assertions
  remain, with explicit Recording/no-gap checks added. Intermediate failed fixture
  attempts (private-method compile error and an unnecessary extra-read assertion)
  are retained, as are the original gap database and log.
- `insufficient_event_capacity_fails_before_durable_activation` scheduled the old
  transport while waiting for its read-only apply phase, allowing discovery status
  to change independently from idle to recovering. Its loop now advances only
  the relevant lifecycle/Recorder phases. The complete before/after discovery
  assertion, no-publication checks and exact one-durable-activation assertion remain.
  The original Windows Debug failure log is retained.

## Verification

Final targeted and workspace results (`gate-summary.json`, individual logs/result
JSON, and `recorder-suite-summary.json`):

| Gate | Windows Debug | Windows Release | Linux Debug | Linux Release |
|---|---:|---:|---:|---:|
| Original `m18_review_` regressions | 6 PASS | 6 PASS | 6 PASS | 6 PASS |
| Entire workspace | 883 PASS | 883 PASS | 882 PASS | 882 PASS |
| Existing ignored tests | 13 | 13 | 13 | 13 |

Runtime library: 154 PASS / 1 existing ignored in all four workspace gates.
The 24 Recorder integration suites give Windows 172 PASS and Linux 171 PASS in
each profile, with zero failed or ignored. Both platforms pass
`cargo fmt --all -- --check` and workspace/all-target Clippy with `-D warnings`.
`git diff --check` passes. No additional ignores or disabled tests were introduced.
`sqlite-evidence.json` indexes 16 coherent databases from the four targeted runs;
all pass integrity and foreign-key checks. Twenty-seven earlier fixture diagnostic
databases were also backed up to D: with their original paths indexed separately.

One earlier Linux Release workspace attempt failed the unchanged Workbench
`wss_proxy_requires_key_trusted_certificate_and_matching_hostname`: one instead
of two negative connections reached its authorization callback. Unauthorized
success was not observed. The isolated test passed (0.41 s), and the final complete
Linux Debug/Release gates passed. The cause of that earlier failure is **unresolved**,
not classified as fixed. Its original full log/result remain
`linux-workspace-release-wss-failed.*`; Workbench/WSS source is unchanged in this
remediation. This is a residual M18 test-stability issue outside these three fixes.

The six original regressions retain their original assertions, strengthened with
exact zero-credit or retained-submitted-credit checks where appropriate:

| Scenario | Source |
|---|---|
| Prior mixed Measurement/Reference group drains before admission | [reference_completion_tests.rs:440](../apps/lab-runtime/src/host/reference_completion_tests.rs#L440) |
| Late receipt safety facts survive; Required Reference effect rejected | [reference_completion_tests.rs:477](../apps/lab-runtime/src/host/reference_completion_tests.rs#L477) |
| Capture error leaves no orphan probe reservation | [reference_completion_tests.rs:660](../apps/lab-runtime/src/host/reference_completion_tests.rs#L660) |
| Reconnect/probe activation fence excludes Reference overlap | [reconnect_recorder_ordering_tests.rs:564](../apps/lab-runtime/src/host/reconnect_recorder_ordering_tests.rs#L564) |
| Real SQL failure before the first rebind reservation blocks effect | [service.rs:5605](../apps/lab-runtime/src/service.rs#L5605) |
| Real SQL failure with an existing rebind token blocks effect | [service.rs:5610](../apps/lab-runtime/src/service.rs#L5610) |

Three additional tests cover [late receipt at final dispatch](../apps/lab-runtime/src/host/reference_completion_tests.rs#L482),
[failure with an unused probe and submitted baseline](../apps/lab-runtime/src/host/reconnect_recorder_ordering_tests.rs#L679),
and [BestEffort Failed rebind](../apps/lab-runtime/src/host/reconnect_recorder_ordering_tests.rs#L749).
Existing tests retain normal Reference fact/terminal atomicity, final-poll failure,
genuine overload, SQL COMMIT rollback, exact 13/1545 occupancy, forged/duplicate
receipt rejection, generation fencing, drain/shutdown and reopen coverage.

## Two-client TCP/WS process matrix

**80/80 PASS:** Windows/Linux, Debug/Release, all four control/observer TCP/WS
pairs, five repetitions each. Each case runs one virtual-demo Runtime with both
listeners and isolated SQLite, then two independent client scopes. Both clients
mutate sequentially; overlapping requests use the same expected revision, so one
completes and one fails with the expected revision conflict. No mutation is replayed.

`matrix-summary.json` and per-case transcripts/SQLite copies retain:

- 640 actual wire Accepted responses including lifecycle and reopen shutdown;
  320 Reference accepted SQL audits, 240 completed and 80 expected failed terminals.
  Every Reference accepted has its terminal audit. The wire counter uses transport
  frames only, excluding duplicate `dual_response` harness annotations; original
  transcripts and the pre-correction metric JSON are preserved.
- Busy **0**, gaps **0**, unexpected Recorder failures **0**, integrity/FK **PASS**.
- Observed credit peaks: Windows Debug **6**, Windows Release **4**, Linux Debug
  **5**, Linux Release **5**, within 13. The deterministic capacity test separately
  exercises exact 13-group / 1,545-record saturation and release.
- 2,912 logged SQL commit measurements: median **1.485 ms**, p95 **2.936 ms**,
  maximum **23.489 ms**. These are measured SQL transaction elapsed times, not an
  end-to-end latency guarantee.
- **80/80** clean shutdown/drain/reopen. Every row in every prior-boot table,
  including full audit identities and payloads, is unchanged after reopening;
  SQLite integrity/FK checks pass. Fresh-boot Reference revision is 1, proving
  no replay of prior retunes. All owned processes exited successfully.

Binary SHA-256, per-case logs and coherent databases are indexed by platform/profile.
This is same-laptop Windows + Ubuntu WSL2 evidence, not physical two-computer
qualification or a new Tuna/GUI acceptance claim.

## Disk and build organization

No additional build-directory deletion was needed during this remediation. The
previous verified cleanup freed 31.24 GiB inside ext4; its exact paths and retained
artifacts remain documented in `M18_RECORDER_ADMISSION_CAPACITY.md`. This is not
claimed as newly reclaimed Windows disk space.

All Windows gates reuse `D:\rust\lab-runtime\target`. Every Linux gate reuses
`/root/lab-runtime-target/m18-implementation-20261009` with two build jobs. No new
isolated build tree was created. Linux test SQLite/scratch uses
`/root/lab-runtime-evidence/m18-recorder-finalfix-20261009/tmp`; coherent evidence
and initial source snapshots are retained on D: under the evidence root.

Gate preflight/result files record C: free space, VHDX allocation, ext4 free space,
target size and running Cargo/rustc. The VHDX remained **49,793,728,512 bytes**
through all builds. The C: 5 GiB stop threshold was never reached. Useful current
Debug/Release binaries and shared dependencies remain available for review.
Final snapshots: C: **8,434,851,840 bytes (7.86 GiB)** free; ext4
**1,003,661,029,376 bytes (934.73 GiB)** available. The common Linux target is
**18,587,369,472 bytes (17.31 GiB)**, versus 14.41 GiB at the start of this
remediation. Growth is inside the one reusable Debug/Release cache; VHDX size
did not grow. Final process checks found no Cargo/rustc, Runtime, Workbench or Tuna
processes on either platform.
No toolchain, registry, source, user SQLite or unrelated distro was removed or
moved, and no WSL shutdown/export/import/compaction was performed. Free ext4 blocks
remain a reuse opportunity, not a guarantee against future VHDX growth.

## Review boundary

The local production delta is three files, **84 added / 29 removed lines** relative
to the start-of-remediation snapshot. Seven test-only files and four coordination
documents complete this review delta. The audit fingerprints all 339 original
files and checks the allowlist; 326 are byte-for-byte unchanged. The ServiceHost
production prefix and Reference worker production prefix are separately verified
unchanged. In particular, existing transport/LAN/Workbench/Tuna/GUI work is preserved.

Final Git status: **39 modified / 13 untracked**, index empty, HEAD unchanged.
The tracked working-tree diff includes the earlier M18 work; its final stat is in
`final-diff-stat.txt`. `working-tree.patch` also preserves untracked source/docs,
while `production-delta.patch` isolates only this review's three production fixes.

Recorder remains a final-review candidate. No external acceptance, new contract,
protected reconnect guarantee, unconditional SQL terminal guarantee or physical
two-computer qualification is claimed. Stop at this review boundary; broader M18
work and the unresolved Workbench WSS test stability issue are not silently folded
into these three fixes.
