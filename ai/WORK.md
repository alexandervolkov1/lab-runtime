# Current work — M6 external-review stop

## Gate and authority

```text
Current state: READY_FOR_EXTERNAL_REVIEW
Completed model: SOL_HIGH
Completed work: M6 implementation and H1–H28 verification
Requested next step: external architectural/implementation review
Authorized implementation now: none
Do not begin: M7 design or implementation
```

The user explicitly authorized ASTRA_HIGH -> SOL_HIGH and M6 implementation.
SOL_HIGH followed the fixed [M6 design](../docs/implementation/MILESTONE_6_DESIGN.md)
and wrote [the M6 implementation report](../docs/implementation/MILESTONE_6_REPORT.md).
`ai/HANDOFF.md` is the single reviewer communication file. Read it, this file,
`ai/ROADMAP.md`, `AGENTS.md` and `PROJECT_BRIEF.md` before any later phase.

## Reviewable checkpoint

M1–M5 remain the externally approved baseline: corrected M4 W1–W9/R1–R10
and M5 L1–L22 were preserved. M6 has 189 named Rust workspace tests in the
final debug/release suites. `cargo fmt --all -- --check`, warning-free workspace
clippy, finite `cargo run -p lab-runtime`, Babashka client checks, and the actual
Rust-host/real-bb A/B killed-client reconnect test passed. The real-process run
measured 6,138 ms without an ordinary client and 6,305 ms finite lease-expiry
advance, then B paused safely and shut down cleanly. Tool versions and H1–H28
test mapping are in the report.

M6 is a bounded virtual/software proof: one Runtime owner, explicit safety-first
monotonic scheduler, two fixed isolated real Lua workers, strict loopback-only
version-one NDJSON, pure queries, retained operation outcomes, semantic events,
frozen snapshot/replay/gap recovery, and a thin bounded Babashka client. The
default M1 executable is still finite; the autonomous host requires explicit
`--serve --profile virtual-demo --port <u16>`.

The separate v1 donor remains read-only and never a workspace dependency. No
Recorder/SQLite, GUI, physical production control, real COM adapter, long soak
or M7 design/implementation is part of this checkpoint. The pre-existing
user-staged `ai/project_snapshot.txt` was not edited or included in any M6
commit; preserve it during review.

## Required stop

SOL_HIGH has completed its authorized phase and must STOP after printing MODEL
HANDOFF. External review should assess the design/report/source/test evidence
and record its response in `ai/HANDOFF.md`. This file does not authorize crossing
the review gate or starting M7. A later model/phase instruction requires an
explicit new user authorization. If review finds a genuine architectural fork,
record the precise issue as `WAITING_FOR_REVIEW` in `ai/HANDOFF.md` and stop.
