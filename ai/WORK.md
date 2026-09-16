# Current work — M8 tests-first implementation

STATUS: M8_IMPLEMENTATION_IN_PROGRESS

Current model: SOL_HIGH
Current phase: M8 tests-first implementation only.
M7: externally accepted at `f3ff456`.
M8 implementation: explicitly authorized on 2026-09-16.
M9: not authorized.

## Completed design

The authoritative M8 design, lifecycle and tests-first C1-C20 contract is:

```text
docs/implementation/MILESTONE_8_DESIGN.md
```

This phase changed documentation only. No implementation tests, dependencies,
production code, hardware actions or new M7 acceptance claims were added.
External reviewer communication belongs only in `ai/HANDOFF.md`.

## Implementation instruction

Read in full before acting:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
docs/implementation/MILESTONE_8_DESIGN.md
docs/implementation/MILESTONE_7_DESIGN.md
docs/implementation/MILESTONE_7_REPORT.md
```

Recover `git status`, `git log --oneline -20`, unstaged/staged diffs and verify
the accepted M7 checkpoint plus this M8 design. Do not recreate missing commits
from memory. Preserve unrelated user changes and the read-only donor.

SOL_HIGH implements M8 only, tests-first in small logical
increments following design section 12. Preserve M1-M7, Core boundaries, fixed
Recorder budgets, output evidence and Required fail-closed semantics. Do not
silently redesign the accepted architecture. Keep the actual red/green sequence,
C1-C20 named evidence, versions/bounds and limitations in MILESTONE_8_REPORT.md.

The final software gate is the complete latest-HEAD gate in design section 12:
debug/release workspace tests, fmt, warning-free all-target clippy and rustdoc,
finite demo, actual Babashka client tests, existing M6/M7 recording-enabled
process/reopen acceptance, M8 configuration/reload/provenance process acceptance,
and `git diff --check`.

C16/C17/C19 also require actual Windows COM + Metakon read-only measurements,
recording, disconnect/reconnect with justified recovery, public API/Babashka and
SQLite inspection, and clean shutdown. No physical actuator writes. Do not report
fake or skipped tests as hardware evidence. If unavailable, software may complete
but M8 remains incomplete: record `M8_HARDWARE_ACCEPTANCE_PENDING` in the handoff
and stop for the missing bench input. Do not start M9.

For a genuine architectural contradiction involving ownership, OutputAuthority,
Required safety, provenance/evidence, boundedness or shutdown, record the precise
issue in `ai/HANDOFF.md`, set `STATUS: WAITING_FOR_REVIEW`, and stop.

Only after all software and hardware acceptance passes, finalize the M8 report,
set `STATUS: READY_FOR_EXTERNAL_REVIEW` in the coordination files, and stop for
external review. No automatic milestone/model transition.

## Model gate

MODEL HANDOFF

STOP HERE.

Switch:

ASTRA_HIGH -> SOL_HIGH

Resume with:

M8 tests-first implementation against MILESTONE_8_DESIGN.md.

The model gate is crossed. Continue autonomously through the software completion
gate. If real Metakon/COM hardware is unavailable, finish all software evidence,
set M8_HARDWARE_ACCEPTANCE_PENDING, and stop without starting M9.
