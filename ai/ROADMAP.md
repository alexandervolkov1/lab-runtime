# Product roadmap

This file records the persistent milestone sequence. Read `HANDOFF.md` for the
review checkpoint and `WORK.md` for the current model's authorized work.

## Current sequence

- M1: domain foundation — completed.
- M2: central output authority and arbitration — completed.
- M3: bounded transport, narrow Metakon and data-driven instruments — completed.
- M4: thermal plant, EMA, Reference and native PID — original 72-test checkpoint
  corrected and verified at the 92-test W1–W9/R1–R10 gate before Lua dependencies.
- M5: bounded embedded Lua virtual measurement and moving-mean transform —
  implemented and verified at 123 workspace tests. Waiting for external review.
- M6: later milestone; no design or implementation during the present review gate.

The reviewed implementation contracts are
`docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md` and
`docs/implementation/MILESTONE_5_DESIGN.md`; implementation results are in the
corresponding M4/M5 reports. `NEXT_IMPLEMENTATION_PLAN.md` and
`IMPLEMENTATION_ROADMAP_M2_M6.md` retain the earlier sequence and architecture
context; the current `ai/WORK.md` forbids proceeding while review is pending.

## First stable release after M6

The first release also requires M7 durable Recorder and SQLite, runtime/release
hardening, a real Windows serial/COM adapter, a safe read-only physical Metakon
smoke path, repository Markdown consolidation, final documentation, an end-to-end
tutorial and release packaging. These are later phases and are not authorized by
the current M5 implementation instruction. The separate v1 donor stays read-only.
