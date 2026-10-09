# M18 Recorder admission/capacity — continuation review candidate

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
Base HEAD: `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`.
No new commit, index staging, merge, push, tag or release. External acceptance of
this candidate is not claimed. This record supersedes the *current-status* claims
of earlier M18 review records while retaining their failures as historical evidence.

**Current follow-up: external Recorder review APPROVE.** The three fixes in
[M18_RECORDER_FINAL_FIX.md](M18_RECORDER_FINAL_FIX.md) are accepted and committed.
Pre-admission and the capacity envelope below remain unchanged. Commit assembly,
the separately reproduced close-observation race and final gates are tracked in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). The pre-commit state and
review restrictions below are historical.

**Historical external re-review: REQUEST CHANGES.** The diagnostic-only investigation
is in `M18_RECORDER_REREVIEW.md`. Six new tests have two PASS and four intentional
RED contract regressions. Earlier green gates below are historical evidence for
the previous test inventory, not acceptance of the newly exposed paths. Production
code is unchanged during this diagnostic review.

## Recovery and preservation

Recovery found 31 modified and 10 untracked files, rather than the last reported
28/9. The saved pre-admission review snapshot and the later
`target/m18-recorder-admission-20261009` logs identify the interrupted session's
additional pre-admission/capacity implementation and eight admission regressions.
HEAD and the empty index matched. No unknown commit or unrelated edit was adopted.

Evidence root: `target/m18-recorder-capacity-20261009` (ignored generated evidence).
`initial-working-tree.patch`, `initial-status.txt`, and
`initial-files-sha256.json` preserve the recovered state, including untracked files.
Cleanup verified that all 337 initially fingerprinted files remained unchanged.
Older review files and all failure logs remain available.

## Disk policy and cleanup

Only verified generated profile artifacts were removed, with no Cargo/rustc running:

- `/root/lab-runtime-target/debug` and `release`;
- `/root/lab-runtime-target/x86_64-unknown-linux-gnu/debug` and `release`;
- `/root/lab-runtime-target/m18-20261009/debug` and `release`;
- `/root/lab-runtime-target/m18-pre-admission-20261009/debug`;
- `/root/lab-runtime-target/m18-recorder-admission-20261009/debug`.

`cleanup-linux.json` records each checked path and allocation. Useful old top-level
binaries and dependency descriptors were copied with hashes to D: under
`preserved-linux-binaries`. Sources, Git worktrees, nested diagnostic/evidence
directories and 54 SQLite files were retained. No registry/toolchain or user files
were removed. No WSL shutdown, relocation, export/import or VHDX compaction occurred.

Cleanup freed **33,547,329,536 bytes (31.24 GiB)** inside ext4. Target allocation
fell from 44,532,805,632 to 10,985,476,096 bytes. This is ext4 reuse capacity,
not reclaimed physical Windows space. Ubuntu initially had about 943 GiB available
after cleanup. The VHDX started at 49,807,360,000 bytes and did not grow through the
subsequent reboot/revalidation; its post-reboot size was 49,794,777,088 bytes.

Windows gates reuse `D:\rust\lab-runtime\target`; all Linux gates reuse
`/root/lab-runtime-target/m18-implementation-20261009`, with two Cargo build jobs.
No additional isolated Linux compilation tree was created. SQLite/process evidence
uses `/root/lab-runtime-evidence/m18-recorder-capacity-20261009` and coherent copies
on D:. The gate runner records C: free space, VHDX size, ext4 space, target size and
Cargo/rustc processes before each heavy Linux gate. It refuses C: below 5 GiB or
VHDX growth exceeding 512 MiB from the initial observation. Free ext4 blocks alone
are not treated as proof that VHDX growth is impossible.

## Root cause and final implementation

The earlier receipt correction is retained: `confirmed_submission` is the maximum
original owner timestamp in the FIFO committed prefix, not the final timestamp of
a SQL-sorted batch or a claim that an entire timestamp range is durable.

Four-group capacity had a separate real problem. Three pending acquisition groups
plus the accepted Reference audit filled the pool. Reserving completion only then
could reject the side effect but could not admit the mandatory failed terminal
audit. Failed Recorder also let a later preparation path return success without a
reservation, allowing a subsequent Required Reference mutation to change state
without its required history.

The new internal `SessionStore::admit_with_gate` invokes the Recorder gate only for
a new, otherwise admissible identity, before changing the high-water sequence or
inserting Accepted. Known exact identity, conflict, old/unknown sequence, gap and
scope checks retain their existing meanings. Recorded synchronous Reference
retune/configure reserve their accepted audit plus fact/terminal envelope atomically.
Insufficient capacity returns existing `busy`, `accepted: false`, no sequence use,
no side effect and no artificial Recorder gap. Wire sequencing is unchanged.

The single owner holds at most one not-yet-transferred Reference token. It binds
scope, sequence, command, run, interval and activation generation. The accepted audit
is transferred separately in FIFO order. The completion group contains the optional
single Reference fact plus the completed/failed terminal audit in one SQL transaction.
The existing Application Accepted/Completed lifecycle does not become a SQL receipt.

Required Failed/Closed is checked at pre-admission, completion preparation and the
final HostCore command poll immediately before the Core Reference change. The final
check covers a newly observed writer failure between preparation and dispatch.
Best-effort retains the documented ability to continue otherwise valid control,
with explicit Failed Recorder state and a stopped durable prefix. It does not claim
that the omitted history was recorded.

Rebind baseline and compatibility probe facts use owner-local bounded reservations
before their producer effects. Pure fallible baseline queries precede reservation.
Existing activation generation fences, publication ordering and cleanup remain in
force. Ordinary operation strings are compacted before their allocation charge;
unused caller String capacity cannot invalidate the byte-budget proof.

## Quantified operating envelope

The default capacity is **13 groups, 1,545 records, 4 MiB accounted bytes**, with
the existing 512 KiB maximum causal group and eight history jobs. SQL coalescing
remains at most four fact groups and its existing 100 ms window. There is no new
worker, blocking owner wait, public setting or forced pressure-commit policy.

| Work awaiting receipts | Groups | Record budget |
|---|---:|---:|
| Four full acquisition groups | 4 | 4 × 256 = 1,024 |
| One reconnect: accepted, activation, baseline, probe, terminal | 5 | 1 + 1 + 256 + 256 + 1 = 515 |
| Two independently admitted Reference accepted/completion pairs | 4 | 2 × 3 = 6 |
| Total | 13 | 1,545 |

Four groups/six records and two complete Reference byte envelopes are protected
from ordinary traffic. Thus ordinary acquisition/rebind cannot consume the two
Reference pairs' headroom. A third pair outstanding is bounded pre-admission Busy.
This provides bounded control opportunity, not unlimited per-client fairness or a
guarantee against an arbitrarily stalled disk. Smaller trusted test limits retain
their shared pool to exercise actual saturation.

The thirteen-group calculation is a supported simultaneous-work envelope, not
a protected reconnect service guarantee. Only Reference credit is protected.
The other nine groups/ordinary record and byte budget are shared by acquisition,
rebind, probe, activation and ordinary audits. Reconnect reserves its pieces
incrementally; it has no atomic five-group reservation, acquisition throttle or
priority/fairness entitlement. With more than the stated four acquisition groups
outstanding, admission can wait or fail at the existing absolute deadlines. The
capacity regression proves that the stated combination fits; it does not prove
reconnect availability for every arrival order or sustained acquisition load.

On x64, accepted envelope A = 16,704 bytes and completion envelope C = 18,016
bytes. Protected Reference bytes = 2(A+C) = 69,440. A conservative whole-envelope
bound is 6 × 512 KiB + 64 KiB activation + 2A ordinary reconnect audits + 2(A+C)
= **3,314,112 bytes**, below 4 MiB. Tests deliberately provide audit Strings with
large spare allocations and prove that transfer compaction preserves this bound.

The channel remains bounded: 13 ingress + 8 history + 6 lifecycle/control slots.
One production owner serializes admissions; logical credit is reserved before
FIFO identity assignment. The reserved fact token set is bounded by ingress credit.
No measurement queue or deferred mutation collection is made unbounded.

Reference admission also remains fenced while a new activation generation is
pending. Reserved capacity does not waive rebind/publication identity fences.

## Credit, durability and identity proof

1. Reservation charges all required dimensions in one owner action before admission.
   New mutation sequence and Core revision do not change on refusal.
2. Reservations assign no SQL record ID. FIFO transfers assign consecutive ranges;
   Reference fact and terminal are indivisible in the writer transaction.
3. Only never-transferred portions may be cancelled. Submitted groups remain charged
   until cumulative committed receipts release them, including on SQL COMMIT failure.
4. Receipt validation bounds Reference releases by submitted Reference credit and
   ordinary releases by the separately charged ordinary budget. Forged cumulative
   totals cannot release an unused Reference token or spend the other pool.
   Duplicate receipts cannot double-release. Generation and revision fences remain.
5. A SQL failure is still a real failure. No reservation is presented as a promise
   of successful commit; committed prefix and SQL durability remain authoritative.
   Genuine ordinary overload retains gap/failure evidence and Required fail-closed.
6. Disconnect/reconnect does not replay a mutation. Known identity/status and
   retained sequence survive pressure and reconnect; core rebind fences stale tokens.
7. Clean stop/shutdown drains accepted history before seal. Reopen verifies integrity
   and the unchanged previous boot prefix, with no hidden replay.

## Targeted regressions and test corrections

Nine Application admission regressions cover three acquisition groups + Reference;
two independent scopes with overlapping writer credits; Required/best-effort after
Failed; real thirteen-credit occupancy, third-pair Busy and identity/sequence rules;
reduced four-credit pre-admission rejection; genuine ordinary saturation with a gap;
SQL COMMIT failure and the second mutation under both policies; 32 cycles of four
facts/two References without Busy; and configure/failed-retune terminal audits.

Worker/host regressions also prove the full 13/1545 envelope, exact release to zero,
monotone receipt prefix, forged-receipt rejection, final-poll Required failure fence,
activation/rebind/probe reservations, shutdown/flush/reopen and SQL failure retention.
Windows final Reference filter: 11 PASS; admission: 9 PASS. Linux targeted suite:
library 145 PASS/1 ignored, admission 9, receipts 5, backpressure 11, reload budget 4,
shutdown 10, transactions 18 PASS. The final full gates include these tests again.

Proven fixture/platform corrections, without disabling tests or widening production deadlines:

- Windows Release emulator tests collided on the same PID + wall-clock-nanosecond
  pathname; instrumentation captured two test threads using exactly the same path.
  Reuse the existing unique temporary-database helper for their TOML instead.
- The checkpoint-failure fixture allowed unrelated acquisition while awaiting Start,
  racing its prefix-two expectation. Receipt-only polling now isolates the injected
  checkpoint and explicit fact. A discarded frozen-clock attempt failed monotonic
  time validation; its log is preserved.
- Linux Clippy rejects unused Windows-only ownership error variants; those variants
  and matching display arms are now Windows-gated with no runtime behavior change.
- One early Linux reconnect cleanup `recorder_flushed` failure did not reproduce in
  20 focused and five full-library repeats. Two later Linux process readiness
  timeouts retain their original failed full-gate log. Test diagnostics now retain
  child status/stderr and scratch evidence on panic. Their cause is not claimed
  proven merely because the subsequent full gate passes.
- The first complete Linux Release run failed the managed-worker panic fixture at
  `standard_components_initialized()`. Its 21 fast polls advanced a fake clock by
  200 ms without awaiting the real initialization callback; ten isolated repeats
  passed, confirming schedule-sensitive behavior rather than a deterministic setup
  failure. The fixture now holds experiment time fixed while polling actual Init
  and shutdown completion under a separate two-second wall-clock bound. It also
  observes the real worker count fall after Step before advancing native controller
  time, so an artificial invocation timeout cannot substitute for a worker panic.
  This change is entirely inside `#[cfg(test)]`; managed execution and Runtime
  deadlines are unchanged. The failed full gate and original repeat logs are retained.
  The first correction's frozen-clock `service()` loop was itself insufficient:
  polling is gated by the scheduler's 10 ms slot, so a completion arriving after
  the first poll remained unread. Its failed full Release run is retained too.
  The final fixture explicitly calls owner `PollComponents` at the fixed instant
  for Init and actual panic completion; shutdown observes real worker termination.
  Final focused Release lifecycle tests are 4/4, followed by three ordinary parallel
  full Runtime-library Release runs, each 145 PASS/1 ignored/0 failed.

## Protected Tuna, real GUI and independent clients

After reboot, Ubuntu and Arch were stopped; no Runtime/Tuna or listeners on 8765/8766
survived. Only Ubuntu was started. An existing Linux debug Runtime reported package
0.1.0, Application protocol 1/API 0.1-pre (the CLI has no `--version` switch).
The expected old access-key location was absent; an existing key was found under
the user's `.config/lab-runtime`, and existing Tuna agent configuration/token under
LocalAppData. Neither was modified. Tuna 0.36.1 was used without upgrade.

The single owned Runtime command was:

```text
lab-runtime --serve --profile virtual-demo --bind 127.0.0.1 --port 8765
  --record-db /root/lab-runtime-evidence/m18-recorder-capacity-20261009/tuna-e2e-2/history.sqlite
  --record-policy required --ws-port 8766 --ws-origin http://127.0.0.1:3000
```

Both listeners belonged to Linux PID 396; Windows Tuna PID 12472 forwarded the
known public endpoint to `http://127.0.0.1:8766`, rewriting Host to `127.0.0.1:8766`.
`TUNA_KEY_AUTH` was set only in its child environment, using the existing key;
`--inspect=false`, `TUNA_INSPECT=false`, and disabled TUI prevented inspection.
Raw agent/HTTP logs were not saved; only event classifications and HTTP statuses
were retained. This uses the documented equivalent of `--key-auth`:
[Tuna environment variables](https://tuna.am/docs/guides/environment-variables/),
[HTTP tunnel](https://tuna.am/docs/tunnels/http/).

| Check | Result |
|---|---|
| Local TCP and WS hello/latest, same boot | PASS |
| Missing X-Token / wrong X-Token | HTTP 401 / 401 |
| Correct X-Token, verified TLS, WS subprotocol | HTTP 101; hello/discover/latest PASS |
| Direct WSS retained-scope reconnect | Same boot/scope/next sequence; no replay |
| Windows Workbench + independent Babashka over public WSS | PASS |
| Babashka safe virtual Reference mutation | revision 1 → 2; target 50 → 51 |
| Workbench authoritative update | Fresh Reference revision 2/target 51 visible |
| Local presentation mutation | Plot committed through API 127.0.0.1:17867 |
| Native visible GUI | Actual cyan measurement traces observed in saved screenshots |
| Explicit Disconnect | Disconnected/Stale after four seconds; no auto reconnect |
| Explicit Connect | Same scope/next sequence, complete rebuild to Fresh |
| Workbench close | Runtime and active Required Recorder remain alive/healthy |
| Stop/shutdown | Recording stop completes; safe/flush/transport shutdown; exit 0 |
| SQLite | integrity OK, no FK errors/gaps; 212 measurements; run/boot sealed complete |
| Reference audit | Exactly one accepted and one completed |

`tuna-e2e-2/result.json`, `sqlite-validation.json`, `runtime-transcript.jsonl`,
`workbench-api-transcript.jsonl`, `babashka.log`, and actual `tuna-final.png` capture
the final PASS. Workbench PID 5996 exited cleanly. Only these owned processes were
stopped; the tunnel was terminated before Runtime shutdown. No tunnel remains.
An evidence scan found no access-key content in generated E2E files.

The first full harness compared the entire evolving ramp DTO after reconnect,
incorrectly including current value/timestamps. That failure is preserved under
`tuna-e2e-1`, with a separate valid identity/revision/target/rate and shutdown check.
The corrected complete run above passes. No production change was made for Tuna.
This is a real protected public endpoint from the **same Windows laptop plus WSL2**,
not a PASS from a second physical computer and not exhaustive WAN fault acceptance.

## Full gates, process matrix and remaining review

The current config-mode LAN candidate also completed a **305-second** actual GUI
session: Windows Workbench over explicitly opted-in TCP to Ubuntu's selected
172.31.158.229 interface, independent Babashka WS subscription/retune, and local
Workbench presentation mutation. The bundled TCP Babashka smoke separately passes
with `--host` and `--allow-remote-tcp`. Native GUI traces were visually inspected,
Fresh remained authoritative, and Recorder stayed Required/Recording with no
failure before and after closing Workbench. Stop/shutdown exit 0; SQLite has 2,917
measurements, no gaps/FK errors, complete sealed run/boot, and all six lifecycle /
Reference audit rows. Evidence: `gui-lan-2/long-gui-babashka-result.json`,
`sqlite-validation.json`, `gui-babashka-tcp-smoke.log`, and `long-gui-final.png`.
This is WSL networking on one laptop, not physical LAN qualification. An earlier
GUI launch failed because the standalone executable was stale; an explicit build
of the current Workbench fixed that test setup without a GUI source change.

Windows workspace Debug and Release: **874 passed, 0 failed, 13 ignored** each;
warnings-denied workspace/all-target Clippy passes. After the final fixture fix,
Linux fmt, Clippy, full workspace Debug and Release all pass; each test mode has
**873 passed, 0 failed, 13 ignored**. Final Windows revalidation also passes:
**874 passed, 0 failed, 13 ignored** per mode, fmt and warnings-denied Clippy PASS.
`git diff --check` passes. Platform totals differ because of platform-specific
tests; no test was newly ignored or disabled. `final-summary.json` binds gate logs
and source hashes, and the sequential final gate runner exited zero.

Windows Release process matrix: 40/40, ten repeats for each TCP/WS control/observer
pair. There are 200 accepted operations including shutdown/reopen, no Busy, no
gaps, and 40/40 Reference accepted/completed audit pairs. All integrity, FK, exact
old-prefix reopen and clean-exit checks pass. Peak charged groups: 4; measured SQL
COMMIT elapsed time p95 1,378 us, maximum 3,104 us (not end-to-end queue latency).
The deliberate full-budget deterministic regression separately reaches 13 groups.

Windows dual-client Release: 4/4 transport pairs. Sequential plus overlapping
requests preserve 16 accepted Reference audits, 12 completed and four expected
stale-revision failed audits. Busy/gaps: 0; integrity/reopen PASS; peak groups 4;
SQL COMMIT p95 1,452 us, maximum 1,640 us. Overlapping means both clients send
before awaiting a result; the sole Runtime owner still dispatches serially.

These final process metrics are in `process-metrics.json`; each case preserves its
executable hash, transcript, coherent SQLite backup and explicit reopen comparison.
The earlier Debug process smoke and dual-client smoke also passed and remain as
separate, earlier-candidate evidence.

Linux Release process matrix: 40/40 with the same four transport pairs and ten
repeats each. Accepted operations 200; Busy/gaps 0; all 40 Reference accepted and
completed audits present. Peak groups 5; SQL COMMIT p95 3,854 us, maximum 59,187 us.
Linux dual-client Release: 4/4; 32 accepted operations including lifecycle/reopen;
Reference audits 16 accepted, 12 completed, four expected failed; Busy/gaps 0;
peak groups 5; COMMIT p95 3,440 us, maximum 4,149 us. Integrity/FK/prefix-preserving
reopen and clean process exit pass in every case. These runs overlapped a bounded
two-job compilation; that load is recorded rather than removed from the latency
statistics. The measured maxima are observations, not latency guarantees.

Combined original-shape Windows/Linux matrix: **80/80** with no failures or Busy,
versus the preserved earlier 53/80 failed recordings. The additional dual-client
matrix is **8/8**, preserving both successful and rejected-domain terminal audits.
Process production code was unchanged by the subsequent managed-worker test-only
fixture correction. No automatic retry was introduced to make any case pass.

The last rebuilds changed executable hashes despite only changing a test fixture.
Both final Release artifacts were therefore rechecked with all four transport
pairs and the sequential/overlapping two-client flow: another **8/8 PASS**, no
Busy/gaps and complete accepted/completed/expected-failed audits. Their exact binary
hashes and metrics are in `{windows,linux}-final-release-matrix.json` and
`process-metrics.json`.

An additional read-only check compares every row in every boot-keyed SQLite table
against the coherent pre-reopen backup, including full operation audit identities
and payloads: **96/96 unchanged**, integrity/FK PASS. Its hashes/counts are in
`post-reopen-all-boot-rows.json`; it does not merely compare measurement counts.

Remaining acceptance limits include external review of this Recorder candidate,
physical two-machine deployment qualification, and formal release/legal sign-off.
Direct remote WS remains unauthorized by the frozen Runtime loopback policy; LAN
uses explicit trusted-network TCP opt-in. No transport/recovery rewrite is proposed.
The earlier isolated Linux cleanup/readiness failures remain a documented stability
follow-up: their non-reproduction is not represented as a proven root-cause fix.
No reproducible production Recorder failure remains in the completed final gates
and operating envelope above. This is review readiness, not M18 consolidated
acceptance or physical production certification.

## Final environment and review artifacts

At 13:36 MSK, C: had **8,656,449,536 bytes (8.06 GiB)** free. Ubuntu had
**1,007,343,722,496 bytes (938.16 GiB)** available in its virtual ext4 filesystem.
The entire Linux target root occupied **15,026,348,032 bytes (13.99 GiB)** after all
Debug/Release gates. Useful current caches are retained; no isolated build trees
were added. VHDX size was **49,793,728,512 bytes (46.37 GiB)**, below the initial
observation, with no compaction or physical Windows space recovery attempted.

Final process checks find no Cargo/rustc, Runtime, Workbench or Tuna processes from
this work and no test listeners on 8765/8766/17867. Ubuntu has only its existing DNS
listeners; Arch was not started or modified. Existing Tuna config/key modification
times remain 2026-10-06 / 2026-10-08. All test SQLite and workspace evidence is kept.

- `recorder-production.patch`: Runtime Recorder/owner/session/reconnect source diff
  against HEAD, including new worker modules and adjacent inline regressions.
  `service.rs` also contains the previously authorized startup TCP opt-in.
- `review-working-tree.patch`: complete diff including all untracked source,
  tests and review documents, preserving the pre-existing M18 work.
- `review-diff-stat.txt`, `final-status.txt`, `final-files-sha256.json`,
  `final-summary.json`: review scope and provenance; the live Git index is empty.
- `preserved-transport-source.json`: seven recovered transport/client/manifest
  files are byte-identical to their initial snapshot; no Tuna-specific production
  workaround was introduced.

Final Git state: **37 modified + 11 untracked**, HEAD and feature branch unchanged.
No commit, merge, push, tag or release was made. The full diff stat is saved in
`review-diff-stat.txt`; it includes the existing uncommitted work, not only edits
made during this continuation.
