# M18 consolidated review candidate

## Home-PC final review (2026-10-09)

This section supersedes the old machine/disk descriptions below. The original
closure record remains historical evidence; its ignored raw artifacts are not
present on this home PC and were not independently reopened here.

### Git and documentation boundary

The initial worktree was clean at the requested
`458240229363cb1a48651c72655e428bcae13e5f`, on
`feature/m18-distributed-workbench`. Read-only `git ls-remote` confirmed the same
remote feature tip. Local and remote main were
`8f7cb98de194205c5ffcaaa9292ea5ce6845e623`. All seven final commits were inspected:
the six implementation/test commits in the historical inventory below plus
`4582402` (the consolidated evidence/documentation commit).

Before installing Arch, local commit
`96a6ff25ab412d588882a2ef702219d4538b5917` corrected README and the Workbench,
configuration, getting-started, architecture and distributed deployment guides.
It also adds the existing distributed guide and Babashka example to the preview
package's explicit public-file inventory: otherwise the corrected guides link to
files absent from the package. No production Rust, dependency, Cargo profile or
security assertion was changed. Rust/Cargo inputs used by the workspace gates
still match `fe9057d`.

The source comparison covers all eleven Workbench CLI options, transport-specific
validation, explicit LAN opt-in in both Runtime startup modes, one worker across
TCP/WS/WSS, observation admission, recovery/quarantine, local presentation and
Required Recorder capacity/durability. The existing packaging script's Markdown
quality/link checks pass for 23 public Markdown files, both against the repository
and a documentation-only projection of the actual package inventory. External
links are not claimed checked. Diff checks and a staged secret review pass.

Deployment instructions now explicitly cover:

- A: Runtime, Workbench and Clojure on the same Windows PC; independent Runtime
  scopes and a local presentation API.
- B: Windows Runtime/Workbench and remote Clojure over explicitly opted-in trusted
  LAN TCP or authenticated WSS/Tuna. The Workbench presentation API remains
  IPv4-loopback-only and has no direct remote access; remote scripts use Runtime
  only, and presentation commands run on the Workbench host.
- C: a separate Linux Runtime with Workbench/Clojure on Windows; both independently
  connect to Runtime, while presentation calls remain local to Windows.

These are supported placements, not a claim of physical two-computer E2E.

### Recorder close-race review

The exact `bb22d57` diff is narrowly scoped to observation order, an acquire fence
and a deterministic test hook/regression. Completion is sampled before liveness.
This prevents the old combination of pre-close `alive=true` and post-close
`is_finished=true` from classifying a normal writer close as panic. Normal worker
return publishes its final receipt and clears liveness with Release ordering;
actual panic bypasses that path and remains failure evidence. The implementation
was cross-checked against Rust 1.95's standard-library thread completion path.

SQL failure remains sticky and fail-closed, including failure after seal. Closure
alone is not durability: host `recorder_flushed` still requires Closed, committed
seal, no first error and zero outstanding records. Credits are released only by
validated cumulative receipts; this fix adds no credit release. Existing tests
cover real panic, SQL commit/close failure, stale/future/invalid receipts, drain,
seal and database reopen. No new Recorder production defect was established.

### WSS security and stability review

The local TLS proxy binds its listener before its worker is spawned. The Runtime
fixture has a readiness signal. Accepted sockets are blocking with bounded
timeouts. TLS runs before the HTTP Upgrade authorization callback: TLS may succeed
before that callback, or certificate/hostname rejection may end a connection
without calling it. Counting exactly two denials is a test coverage assertion for
the missing/wrong-key cases, not an Application security invariant.

`fe9057d` preserves and strengthens the important outcomes: missing/wrong keys must
produce authorization rejection; untrusted CA/wrong hostname must produce TLS
validation failure; every negative attempt must produce no Hello; no invalid
attempt may be authorized; the valid key/trust/hostname path must complete Hello
and authoritative queries. Secret-redaction assertions remain. Runtime upstream
connection occurs only after the proxy's authorized Upgrade succeeds.

The serial proxy's two-second I/O timeouts and bounded client connect deadline
leave a plausible load-sensitive fixture explanation, but code inspection does not establish that
as the historical root cause. The committed evidence describes an intermediate
Linux Debug count failure (the user's recollection also mentioned Release); its
raw failed log is unavailable here. No authentication/TLS bypass was established.
Passing repetitions must not be relabelled as fixing the unresolved stability issue.

### Other frozen boundaries

TCP/WS/WSS share Application session/scope/sequencing and the same Workbench worker.
Recovery rebuilds authoritative observations, never automatically replays mutation,
status or Exact Retry. Old-boot/scope evidence remains quarantined; starting a new
scope is explicit. Observation mode blocks Workbench Runtime mutation and Exact
Retry before mailbox admission without blocking an independent script's own scope.
Presentation remains client-owned and its unauthenticated API remains loopback-only.

Remote TCP requires explicit Runtime/client opt-in and remains unauthenticated
plaintext for a trusted LAN, not an Internet transport. Runtime WS stays loopback;
Tuna supplies authenticated WSS with `X-Token` and normal certificate/hostname
validation. Recorder remains Runtime-owned: 13 groups / 1545 records / 4 MiB, only
Reference protected credit, no universal reconnect reserve, no durability promise
from admission or successful transport delivery. No new architecture blocker found.

### Arch environment and bounded verification method

The home PC already had WSL 3.0.1.0 and enabled Windows WSL/VirtualMachinePlatform
components, but no registered distribution. The actual installed WSL help supports
`--location`, `--version`, `--vhd-size` and `--no-launch`; its online list includes
official `archlinux`. Installation used:

```powershell
wsl --install -d archlinux --location E:\M18-WSL\Arch --version 2 --vhd-size 16GB --no-launch
```

The registration and VHDX are on E:, not AppData. No other distribution, Windows
component update, administrator elevation or reboot was needed. This follows the
[official Arch WSL installation route](https://archlinux.org/download/) and the
[documented WSL location option](https://learn.microsoft.com/en-us/windows/wsl/basic-commands).
No pre-existing `.wslconfig` existed: the new file disables WSL swap and redirects
crash dumps to E: with their count disabled, using the
[documented WSL settings](https://learn.microsoft.com/en-us/windows/wsl/wsl-config).
It does not overwrite an existing user configuration.

Arch uses its normal signed packages and one minimal Rust 1.95.0 Linux toolchain
(rustc, std, Cargo, rustfmt and Clippy). Existing Windows dependency archives were
only read, checked against Cargo.lock SHA-256 and copied where useful; missing
dependencies were fetched normally. No existing Windows toolchain/cache was edited.
Provisioning retries, including an incomplete new Rust toolchain reinstallation
to avoid unwanted local rust-docs, are retained in setup logs and are not test runs.

Build/test/Clippy invocations use `--locked`, one `/root/m18-target`, two build jobs,
`CARGO_INCREMENTAL=0`, offline dependencies after fetch and `/root/m18-tmp` for
temporary files. Debug remains unoptimized with debuginfo and Release optimized;
no Cargo profile, feature selection or security assertion is altered. Source is
read directly from `/mnt/d/rust/lab-runtime`; no duplicate repository or large new
C:/D: target is created. Existing tests write small ignored TLS/recovery fixtures
under the repository's target, also included in disk accounting.

`E:\M18-WSL\measure.ps1` measures Windows allocated file sizes, including VHDX,
all task-created E: setup/cache/download/log files, the new configuration file and
those small D: fixtures. Cargo/toolchain sizes inside VHDX are not added twice.
Before each heavy operation and every two seconds during it, the wrapper checks
this total and C:/E: free space. It refuses new heavy work at 18 GB and terminates
only the new task-owned Arch distro at 17.5 GB as a conservative stop reserve;
16 GB triggers a warning. C: must retain at least 5 GiB. Linux `df` is informational,
not the physical-disk budget proof. Diagnostics remain under `E:\M18-WSL`.

### Fresh Arch targeted test results

All entries below are new runs on this home PC's Arch x86_64/WSL2, not renamed
Ubuntu results. No test was disabled or ignored. Filters deliberately select the
reviewed boundaries rather than pretending to be a full workspace run.

| Targeted check | Arch Debug | Arch Release |
|---|---|---|
| Runtime `--lib recorder::worker::` (including deterministic close race) | 14 PASS | 14 PASS |
| Runtime `--lib m18_review_` | 6 PASS | 6 PASS |
| Runtime `recorder_failure`, `recorder_required`, `recorder_shutdown` integrations | 18 PASS | 18 PASS |
| Workbench `distributed_tests::` (parity, observation, WSS, recovery/quarantine) | 5 PASS | 5 PASS |
| Workbench `client::endpoint::tests::` (LAN opt-in, plaintext/URL policy, token handling) | 3 PASS | 3 PASS |
| Runtime `tcp_lan` | 2 PASS | not repeated |
| Runtime public registry/documentation projections | 2 PASS | not repeated |
| Workbench public operation table/source inventory | 1 PASS | not repeated |
| Exact WSS test, four concurrent processes plus two CPU load workers | 20/20 PASS | 20/20 PASS |

The 40 extra WSS invocations used the already-built binaries in the single target,
five batches of four independent processes with distinct ephemeral ports/fixtures.
Debug repetitions also overlapped the two-job Release build; Release repetitions
overlapped targeted Clippy. Every transcript contains exactly one passed test,
zero failed and zero ignored. All original authentication/TLS/hostname/no-Hello and
positive authoritative-query assertions ran unchanged. This does not reproduce or
resolve the historical intermittent early-connection failure; no minimal fixture
fix is justified by the new evidence, and no transport change is proposed.

Evidence commands and complete output are retained as `debug-checks.sh`,
`release-checks.sh`, `static-checks.sh`, `wss-repeats.sh`,
`debug-targeted.{stdout,stderr}.log`, `release-targeted.{stdout,stderr}.log`,
`static-checks.{stdout,stderr}.log` and `wss-{debug,release}-{batch}-{slot}.log`
under `E:\M18-WSL`. The separate four-test Debug failure suite is in `static-checks`.

Full Windows/Ubuntu workspace Debug/Release and workspace/all-target Clippy gates
below remain historical evidence. `git diff fe9057d -- apps crates Cargo.toml
Cargo.lock` is empty, including the working tree. Those gates were not repeated
solely for this documentation/review task. No full Arch workspace gate is claimed;
it was intentionally not required by this unchanged-source, targeted review, not
reported as failing or as impossible within 20 GB. No required targeted test was
omitted because of the disk limit, and no build profile was reduced to obtain PASS.

`cargo fmt --all -- --check` passes on Arch. Scoped Clippy passes with warnings
denied for `-p lab-runtime --lib` and `-p lab-workbench --bin lab-workbench`, both
with `--locked`. These are not relabelled as fresh workspace/all-target Clippy.

### Final outcome, disk and independent review

**READY FOR MERGE** for this bounded consolidated review. No new production
correctness, safety, durability or authentication blocker was established. This is
not release publication, physical qualification or external legal approval.

After the checks and filesystem sync, at 2026-10-09 21:58:28 +03:00:

| Disk item | Bytes |
|---|---:|
| Accounted new task files, including allocated VHDX | 8,535,433,320 |
| `E:\M18-WSL\Arch\ext4.vhdx`, logical and allocated size | 8,191,475,712 |
| Single Cargo target inside that VHDX (`du`) | 4,887,732,224 |
| Cargo home inside VHDX | 659,144,704 |
| Rustup home inside VHDX | 1,387,479,040 |
| Free E: | 172,709,392,384 |
| Free C: | 53,840,871,424 |

The task-file total excludes unrelated pre-existing user data. As a conservative
cross-check, the entire C: free-space decrease since the initial observation is
236,777,472 bytes; even charging all of that unrelated/system activity to this
task keeps the combined figure below 8.78 GB. Small subsequent review text/Git
objects do not approach the 16/18/20 GB thresholds. No heavy operation reached the
16 GB warning. Swap is absent (`/proc/swaps` empty), and no Cargo/rustc, test Runtime,
Workbench or CPU-load process remains. Arch itself is left installed and usable.

Remaining risks are explicit: the historical WSS early-failure/denial-count issue
is unresolved despite new 40/40 stress PASS; physical two-PC LAN/WSS qualification
is unperformed; the pre-existing 15-package license-text/legal release gate remains.
No new production, Recorder, transport or recovery development is opened.

Suggested independent inspection, in priority order:

1. `bb22d57`: `recorder/worker/lifecycle.rs` observation/fence and `worker.rs`
   deterministic race test, alongside the existing real-panic/SQL-failure tests.
2. `fe9057d`: Workbench `distributed_tests.rs` per-case failure categories, no-Hello,
   authorization counters and the positive trusted-key/TLS/query path.
3. `08e1ee6`: Runtime `service.rs`, Workbench `client/endpoint.rs` and their LAN
   opt-in tests; `e173789` worker/transport recovery and observation boundaries.
4. `96a6ff2` and this documentation follow-up: the six public guides, especially
   the three placements and scenario B limitation, and the two added package files.

Initial remote feature/main identities remain unchanged. Only local documentation
commits follow the expected initial HEAD; the follow-up records this review and
clarifies that config-mode port zero uses the explicitly selected bind address.
No merge, push (including feature), tag or Release was performed. Work stops here.

## Historical closure on the previous Windows laptop

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
