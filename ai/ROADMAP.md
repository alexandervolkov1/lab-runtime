# Product roadmap

This file records the persistent milestone sequence. Read `HANDOFF.md` for the
review checkpoint and `WORK.md` for the current model's authorized work.

## Current sequence

- M1: domain foundation — completed.
- M2: central output authority and arbitration — completed.
- M3: bounded transport, narrow Metakon and data-driven instruments — completed.
- M4: thermal plant, EMA, Reference and native PID — original 72-test checkpoint
  completed; reviewed warming and native lease-renewal corrections remain required.
- M5: bounded embedded Lua virtual measurement and moving-mean transform — implement
  only after the corrected M1–M4 verification gate. Stop for external review after M5.
- M6: later milestone; no design or implementation during the current SOL_HIGH phase.

The milestone implementation contracts are
`docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md` and
`docs/implementation/MILESTONE_5_DESIGN.md`. `NEXT_IMPLEMENTATION_PLAN.md` and
`IMPLEMENTATION_ROADMAP_M2_M6.md` retain the earlier sequence and architecture
context; the current `ai/WORK.md` governs this phase.

## First stable release after M6

The first release also requires M7 durable Recorder and SQLite, runtime/release
hardening, a real Windows serial/COM adapter, a safe read-only physical Metakon
smoke path, repository Markdown consolidation, final documentation, an end-to-end
tutorial and release packaging. These are later phases and are not authorized by
the current M5 implementation instruction. The separate v1 donor stays read-only.
