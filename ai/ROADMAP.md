# Product roadmap

This file records the persistent milestone sequence. Read `HANDOFF.md` for the
review checkpoint and `WORK.md` for the current model's authorized work.

## Current sequence

- M1: domain foundation — completed.
- M2: central output authority and arbitration — completed.
- M3: bounded transport, narrow Metakon and data-driven instruments — completed.
- M4: thermal plant, EMA, Reference and native PID — corrected and verified at the
  92-test W1–W9/R1–R10 gate before Lua dependencies; external review approved.
- M5: bounded Lua model observations and moving-mean transform — implemented,
  verified at 123 workspace tests; external review approved.
- M6: autonomous headless host, monotonic scheduler, bounded local API/events,
  reconnect and actual Babashka slice — implemented and verified at 189 Rust
  workspace tests plus real bb A/B process acceptance; external review pending.

The fixed M6 contract is [the M6 design](../docs/implementation/MILESTONE_6_DESIGN.md);
[the M6 report](../docs/implementation/MILESTONE_6_REPORT.md) is the reviewable
implementation evidence, with [ai/WORK.md](WORK.md) as the current review-stop
instruction. Existing M4/M5 contracts and reports remain the accepted baseline.
`NEXT_IMPLEMENTATION_PLAN.md` and
`IMPLEMENTATION_ROADMAP_M2_M6.md` retain historical context; they do not bypass
current model/review gates or authorize future milestones.

`READY_FOR_EXTERNAL_REVIEW` is the current HANDOFF state. SOL_HIGH has stopped;
no M7 design or implementation is authorized at this review gate.

## First stable release after M6

The first release still requires M7 durable Recorder and SQLite, runtime/release
hardening, a real Windows serial/COM adapter, a safe read-only physical Metakon
smoke path, repository Markdown consolidation, final documentation, an end-to-end
tutorial and release packaging. These are later phases. The separate v1 donor
stays read-only and never becomes a workspace dependency.
