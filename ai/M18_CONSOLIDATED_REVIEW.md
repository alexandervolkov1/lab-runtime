# M18 consolidated review candidate

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
Recorder admission/capacity and the three final remediation fixes are externally
APPROVED. Consolidated M18 acceptance is requested, not assumed. No merge, push,
tag or GitHub Release. All commits below are local.

## Preservation and commit boundaries

Before assembly, all 340 source/document files matched the approved candidate's
hash manifest, HEAD was `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`, and the index
was empty. The complete candidate was copied to ignored evidence before staging.
Full staged patches, file inventories, checks and committed-blob comparison are
under `target/m18-consolidated-20261009/`. Partial staging split Recorder tests,
LAN production and unrelated fixture corrections in shared files without rewriting
the working source. All five new Recorder source/test modules were included.

| Commit | Logical contents | Diff stat |
|---|---|---|
| `55bbf3006f05b4f8356b863c4c1261b5c4255f9b` | Reference pre-admission, quantified capacity, FIFO receipts, safety-fact separation, unused probe cleanup, Required rebind fence, tests and Recorder documentation | 28 files, +4008 / -129 |
| `08e1ee697afd45f5bde33f9a44e1607122cbc334` | Explicit trusted LAN TCP opt-in in Runtime/config mode, Workbench and bundled Babashka example; policy tests and deployment docs | 11 files, +476 / -75 |
| `e524e393d856be7a15ef374d90607973687eaa8e` | Test-only pool/startup/child readiness, temporary file and event-order corrections across Windows/Linux | 10 files, +97 / -27 |
| `907edbf681a4ebb98e85a4bec5050e4c4c24f4ab` | Windows-only Workbench ownership error variants/display gated for Linux warnings-denied builds | 1 file, +4 / -0 |
| `bb22d57f6f3f0eb32bc576a015c5a94e4d3ba058` | Reproduced normal-close/false-panic race: completion/liveness observation order, regression and diagnosis | 3 files, +139 / -1 |
| `fe9057d140ab18a333379219e731f946f1aea58b` | Stronger per-case WSS negative assertions and sanitized failure diagnostics; intermittent early failure is not claimed fixed | 1 file, +33 / -5 |

The already committed M18 client boundary remains separately reviewable:
`e1737897eb02f0d9a46e25a83cfdcf5c2fdfefd6` contains the bounded TCP/WS/WSS
Workbench adapter and observation mode;
`b546354bddf035c15f78f1f1d1ea1d22eb84ac76` contains the distributed Babashka
validation example and deployment guide. Workbench API ownership is unchanged.
The new LAN commit only adds explicit remote TCP policy to that client path.

## Recorder and closure follow-up

The accepted Recorder proof and original six regressions are in
[M18_RECORDER_FINAL_FIX.md](M18_RECORDER_FINAL_FIX.md). The full budget is
13 groups / 1545 records / 4 MiB: the supported overlap is four acquisition
groups, five reconnect groups and four Reference groups. Only Reference has
protected credit; reconnect shares ordinary credit and has no independent
five-group reserve or guaranteed admission under arbitrary acquisition pressure.
Reservation does not promise SQL success. Required failure rejects side effects;
BestEffort explicitly exposes missing recording. Submitted credits remain charged
until a valid cumulative SQL receipt, and Reference fact/terminal remain atomic.

The first post-commit Linux Debug gate exposed a normal writer completion being
misclassified as panic. The race was deterministically reproduced before changing
production; its SQL boot was sealed and integral. This qualifies for the user's
established-invariant exception to the freeze. The separate correction orders
completion/liveness observations and acquires completion visibility, without
changing admission, capacity, receipts, SQL or transport semantics. The RED test,
coherent SQLite copies and exact change are described in
[M18_RECORDER_CLOSE_RACE.md](M18_RECORDER_CLOSE_RACE.md). This additional fix is
included for consolidated review, not represented as part of the prior approval.

## Final verification

Final source HEAD: `fe9057d140ab18a333379219e731f946f1aea58b`. Gates ran after all
source commits, with default test parallelism and offline dependencies. Windows
and Linux final workspace gates ran sequentially to avoid cross-platform build
contention. The final documentation commit does not change any tested source.
Evidence root: `target/m18-consolidated-20261009/`; `gate.ps1` records the commands,
and `gate-summary.json` contains source HEAD, results, timings and disk snapshots.

| Gate | Windows | Ubuntu Linux |
|---|---|---|
| Workspace Debug | 884 PASS / 0 failed / 13 ignored | 883 PASS / 0 failed / 13 ignored |
| Workspace Release | 884 PASS / 0 failed / 13 ignored | 883 PASS / 0 failed / 13 ignored |
| Workspace/all-target Clippy `-D warnings` | PASS | PASS |
| `cargo fmt --all -- --check` | PASS | PASS |
| Original six `m18_review_` regressions | 6/6 in each full profile | 6/6 in each full profile |
| Focused worker regressions, Debug and Release | 14/14 each | 14/14 each |
| Panic/Required/shutdown integrations, Debug and Release | 18/18 each | 18/18 each |

`git diff --check` and each staged `git diff --cached --check` pass. No tests were
disabled or ignored for this work; 13 ignored tests are the existing inventory.
All 215 final source/build inputs are fingerprinted; the post-commit audit checks
they still match. The prior full-gate failures and intermediate successful runs
are retained separately, not overwritten or counted as final. An intermediate
fmt failure while adding the diagnostic case table was corrected with `cargo fmt`;
the final checks above pass.

The intermediate Linux Debug run after the close fix completed with 882 PASS,
one FAILED and 13 ignored: the existing WSS test observed one rather than two
authorization denials. No invalid connection was accepted. Its old negative
loop accepted any transport failure before checking the aggregate denial count.
The test now additionally requires an authorization rejection for missing/wrong
keys and a TLS validation failure for untrusted CA/wrong hostname, reporting the
sanitized category per case. All original no-Hello, secret-redaction, denial/
acceptance count and successful authorized-query assertions remain. This is
stronger diagnostic coverage, not a claimed fix for the intermittent early failure.
Ten full Linux Workbench suites with default parallelism passed after this change.
The failed full run is `linux-workspace-debug-wss-failed.log`; targeted repetitions
are `linux-wss-diagnostic-{1..10}.log`. Production transport is unchanged.

## Current Recorder process matrix

The new `matrix-summary.json` records **80/80 PASS**: two operating systems,
Debug/Release, four TCP/WS controller/observer pairings, five repetitions each,
with two independent mutation clients. Every case shuts down normally and reopens
the same database. Complete comparisons of all original-boot rows pass 80/80,
including audit identities/payloads and provenance; no mutation is replayed into
the new boot. SQLite integrity and foreign-key checks pass in every case.

| Metric | Result |
|---|---|
| Reference audits | 320 accepted, 240 completed, 80 expected revision-conflict failed |
| Wire accepted, including lifecycle operations | 640 |
| Busy / gaps / foreign-key violations | 0 / 0 / 0 |
| Observed peak groups, Windows Debug / Release | 6 / 6 |
| Observed peak groups, Linux Debug / Release | 5 / 5 |
| SQL COMMIT samples | 2886 |
| Commit latency median / p95 / maximum | 1.472 / 2.987 / 19.381 ms |

These measured peaks/latencies describe this workload, not a guaranteed SQL timing
bound or protected reconnect budget. Exact zero-credit accounting is asserted in
the deterministic tests; process cases additionally prove successful drain/flush,
shutdown and reopen. Binary SHA-256 values and full per-case transcripts, logs,
coherent databases and comparisons are retained. This matrix ran on the same
Windows laptop and Ubuntu WSL2, not two physical computers.

All 1279 files fingerprinted in `target/m18-recorder-finalfix-20261009/` remain
byte-for-byte unchanged. The accepted prior 80-case matrix and the new 80 cases
are distinct evidence sets.

## LAN, GUI and protected Tuna evidence

Existing real-process evidence is preserved under
`target/m18-recorder-capacity-20261009/`. These are dated E2E results from this same
Windows laptop and Ubuntu WSL2; neither is claimed as a second physical machine.
The public Tuna tunnel was shut down after testing. No production deployment or
current external endpoint availability is claimed by these historical results.

- `gui-lan-2/`: Windows Workbench used explicit remote TCP opt-in to an Ubuntu
  config-mode Runtime. The visible native GUI plotted authoritative data during
  a 305-second run with an independent Babashka client. Reference revision changed
  1 to 2, Workbench reached Fresh, and presentation stayed client-owned. The bundled
  TCP example's `--host` / `--allow-remote-tcp` path also passed. Runtime/Required
  Recorder survived GUI close; shutdown exited 0. SQLite contains 2917 measurements,
  all expected audits, no gaps, integrity OK and no foreign-key violations.
- `tuna-e2e-2/`: one Ubuntu virtual-demo Runtime exposed loopback TCP 8765 and
  WS 8766. Local hello/latest passed on both listeners with one boot identity.
  Tuna 0.36.1 used the existing access key through the child environment, disabled
  inspection, upstream `http://127.0.0.1:8766` and matching Host rewrite. Missing key
  and wrong key returned HTTP 401; correct key returned 101 over verified TLS.
  Workbench and Babashka concurrently used WSS, performed the virtual Reference
  mutation, authoritative Workbench refresh and local presentation API mutation.
  Explicit disconnect stayed disconnected; manual reconnect retained scope and
  sequencing and rebuilt Fresh without automatic mutation replay. Runtime/Recorder
  survived GUI close; clean shutdown exited 0. SQLite contains 212 measurements,
  expected accepted/completed audits, no gaps and integrity/FK PASS. All owned
  Runtime, GUI and tunnel processes were stopped.

The transport/client production sources used in these E2E runs are unchanged by
closure assembly. Final Recorder source is covered by the fresh focused, workspace
and TCP/WS process checks, rather than relabelling old GUI/Tuna binaries as current.
Raw remote TCP remains an explicit trusted-network opt-in; WS Runtime bind remains
loopback. Internet access uses authenticated WSS. No credential was staged or committed.

## Disk policy and remaining limits

The earlier authorized cleanup removed only verified obsolete Cargo profile
artifacts, freeing 31.24 GiB inside ext4 while retaining source/worktrees, SQLite,
diagnostics and useful old binaries. This closure phase performs no further cleanup,
WSL move, compaction, shutdown, export or import. It does not claim reclaimed
physical Windows space. The common Linux target remains
`/root/lab-runtime-target/m18-implementation-20261009`, with two Cargo build jobs;
Windows uses `D:\rust\lab-runtime\target`. Separate evidence directories contain
logs/databases, not isolated multigigabyte build trees. Every heavy Linux gate
checks C:, VHDX, ext4, target usage and Cargo/rustc; free ext4 alone is not a
guarantee against VHDX growth.

Final observation (2026-10-09 16:37 +03:00): C: free **8,263,987,200 bytes
(7.70 GiB)**; VHDX **49,793,728,512 bytes**, unchanged throughout closure.
The shared Linux target uses **18,884,984,832 bytes (17.59 GiB)**; ext4 reports
1,003,277,262,848 available bytes. Other old target containers retain only small
diagnostic files (largest 9.8 MiB), not old profile builds. Windows and Ubuntu
have no remaining Cargo/rustc, Runtime, Workbench or Tuna test processes;
8765/8766/17867 have no test listeners. Ubuntu's system DNS listeners remain.
No new source files remain untracked. The final documentation-only commit includes
this report and historical reviews; final commit inventory/stat and clean Git
status are captured under the evidence root after that commit.

Remaining review/qualification limits:

- Physical Windows-PC/Linux-mini-PC LAN and public WSS qualification is unperformed.
- The Workbench WSS authentication test again observed only one of two expected
  failed-auth attempts in an intermediate Linux Debug workspace run. Stronger
  category assertions and ten full Workbench repetitions did not reproduce the
  early failure. No unauthorized upgrade occurred. The scheduling/fixture cause
  remains unresolved and the failed logs are preserved. Older unreproduced Linux
  timing failures also remain documented, not silently declared fixed.
- The newly established close-observation defect has its own RED/green evidence
  and is part of this review. Recorder architecture and Application API stay frozen.
- Consolidated M18 approval and formal release/legal sign-off are still gates.
  No merge, push, tag or release publication is authorized.
