# Current work — SOL_HIGH M7 Recorder/SQLite implementation

STATUS: M7_IMPLEMENTATION_IN_PROGRESS

Current model: SOL_HIGH
Authorized work: M7 implementation only.
M7 design and D1-D18 tests-first acceptance contract: complete.
M8: not authorized.

## Paused implementation checkpoint — 2026-09-16

The user requested an interruption after the current logical commit and status
notes. This is a pause inside SOL_HIGH, not a model or milestone handoff. Resume
M7 only when the user asks to continue; keep STATUS:
M7_IMPLEMENTATION_IN_PROGRESS until the complete D1-D18 gate is verified.

Completed at this checkpoint:

- M7 design and D1-D18 acceptance contract are complete; the incoming
  pre-SOL uncommitted production audit is complete. No architectural
  contradiction has been found.
- The Runtime-owned bounded Recorder, Required/BestEffort failure bridge,
  host-only SQLite worker/reopen and bounded history API have substantial
  implemented and tested M7 coverage. Accepted M1-M6 behavior remains in the
  workspace suites run to date.
- New acceptance coverage includes managed state/input lineage, original
  output epoch through partial/recovery/late ACK, lineage memory credit,
  client-event-ring separation, killed-process/new-boot paging, nonreading
  history-client isolation, held-writer shutdown through both host and API,
  and an actual blocked-writer server-process flush watchdog.
- The latest D12 process-watchdog commit is `53880ca`; its named debug/release
  tests, formatting and warning-free focused clippy passed. The working M7
  report records red tests, later coverage and fixture corrections honestly.

Not complete:

- D1-D17 still have specific edge acceptance cases to close, especially
  managed-publication pressure; full binding/provenance reconstruction;
  held-writer M3 first-byte and Required multi-output recovery; nested batch
  and fault accounting; physical storage/startup failures; fatal/Lua shutdown
  combinations; selected-row/history generation checks; and M3 wall-jump
  send-decision comparison. Read the D matrix and later checkpoint notes in
  `docs/implementation/MILESTONE_7_REPORT.md` before selecting the next case.
- D18 final debug/release/fmt/clippy/doc/diff/demo/Babashka verification and
  the final version/limit inventory have not been run against the latest head.
  The M7 report is explicitly in progress, not an acceptance claim.
- No READY_FOR_EXTERNAL_REVIEW request has been made. M8 remains unauthorized.

The user explicitly crossed ASTRA_HIGH -> SOL_HIGH. Implement and verify M7
against docs/implementation/MILESTONE_7_DESIGN.md; do not redesign the accepted
architecture or cross the M8 gate.

## Read before acting

Read in full:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
docs/implementation/MILESTONE_7_DESIGN.md
docs/implementation/MILESTONE_6_REPORT.md
```

Inspect git status, git diff, git diff --staged, and git log --oneline -20.
Preserve unrelated files, especially ai/project_snapshot.txt if present, and
leave D:\rust\com_port_reader read-only.

## Authorized sequence

1. The incoming uncommitted M7 audit is complete. Audit each new change against
   the accepted design; preserve correct work and all M1-M6 contracts.
2. Continue tests-first D1-D18 implementation. When a production change preceded
   its acceptance test, add the required coverage and verify the behavior. Record
   that sequence honestly in MILESTONE_7_REPORT.md; do not invent red evidence.
3. Keep plain domain facts and Required safety in Core; keep SQLite, threads,
   path handling and wire encoding in the host. No M8 TOML/COM/reload, M9 Lua
   workspace/presentation, M10 GUI or later work.
4. Use small logical commits. The earlier stale coordination correction was
   committed separately; keep subsequent documentation checkpoints separate
   from Recorder behavior and do not bundle unrelated cleanup.
5. Complete actual SQLite/reopen/process/Babashka acceptance and all D1-D18 tests.

If a genuine architectural contradiction with OutputAuthority, Required safety,
immutable provenance, evidence, boundedness or shutdown appears, record the
precise issue in ai/HANDOFF.md, set STATUS: WAITING_FOR_REVIEW, and STOP.
Routine implementation defects are to be fixed within M7.

## Completion gate

Write docs/implementation/MILESTONE_7_REPORT.md mapping every D1-D18 item to
named tests, actual red/green or post-code acceptance sequence, resolved versions,
bounds, durability/reopen/process results and remaining limitations.

Run:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo run -p lab-runtime
bb --version
cargo test -p lab-runtime --test babashka_reconnect -- --nocapture
git diff --check
git status --short
```

Also execute bb test-client in clients/babashka and the recording-enabled
actual-process/reopen acceptance. Update coordination to actual verified facts,
set STATUS: READY_FOR_EXTERNAL_REVIEW in ai/HANDOFF.md, print MODEL HANDOFF,
and STOP. Do not begin M8.
