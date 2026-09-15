# Current work — external review gate

Read in order:

1. `ai/HANDOFF.md` — the sole external-review communication file;
2. `ai/ROADMAP.md` — milestone sequence;
3. this file — current authorized phase;
4. `AGENTS.md`, `PROJECT_BRIEF.md`, the M4 lifecycle review and M5 design/report
   when assessing implementation or architecture.

```text
STATUS: WAITING_FOR_EXTERNAL_REVIEW
Completed model: SOL_HIGH
Completed implementation: M4 W1–W9/R1–R10 corrections and M5 L1–L22 Lua proof
Next: external architecture/safety reviewer response in ai/HANDOFF.md
STOP: Do not begin M6 design or implementation automatically.
```

The completed SOL_HIGH instructions and their red/green implementation history are
retained in Git from the earlier `ai/WORK.md` revision; do not reimplement M4/M5
because an earlier file or roadmap passage still describes them as next steps.

The external reviewer should evaluate the corrected M4 lifecycle and bounded M5
implementation against
`docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md`,
`docs/implementation/MILESTONE_5_DESIGN.md`,
`docs/implementation/MILESTONE_4_REPORT.md` and
`docs/implementation/MILESTONE_5_REPORT.md`. Put the decision, unresolved
architectural forks and approved next-phase instructions in `ai/HANDOFF.md`.
Do not treat a clean test run or prior authorization through M6 as permission
to cross this review/model gate.

Continue to preserve M1–M4 safety and M3 ACK != readback. The separate v1 donor
stays read-only. No physical actuator tests, M6 design/implementation, M7 Recorder,
long soak, product-parity or pre-release Markdown cleanup belong to this phase.
A future implementation phase requires the user-authorized review/model handoff.
