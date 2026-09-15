# Current work — SOL_HIGH M7 Recorder/SQLite implementation

STATUS: M7_IMPLEMENTATION_IN_PROGRESS

Current model: SOL_HIGH
Authorized work: M7 implementation only.
M7 design and D1-D18 tests-first acceptance contract: complete.
M8: not authorized.

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

1. Audit every existing uncommitted M7 production change against the accepted
   design. Preserve correct work, fix contradictions, remove only clearly invalid
   or out-of-scope changes. Preserve all M1-M6 contracts.
2. Continue tests-first D1-D18 implementation. When a production change preceded
   its acceptance test, add the required coverage and verify the behavior. Record
   that sequence honestly in MILESTONE_7_REPORT.md; do not invent red evidence.
3. Keep plain domain facts and Required safety in Core; keep SQLite, threads,
   path handling and wire encoding in the host. No M8 TOML/COM/reload, M9 Lua
   workspace/presentation, M10 GUI or later work.
4. Use small logical commits. Keep this stale coordination correction separate
   from Recorder implementation changes. Do not bundle unrelated cleanup.
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
