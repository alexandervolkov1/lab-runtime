# Current work — M8 real-hardware acceptance pending

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING

Current model: SOL_HIGH.
Current phase: M8 read-only Windows COM/Metakon hardware acceptance only.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73` on 2026-09-16.
M9: not authorized.

## Authoritative contract

Read before acting:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
docs/implementation/MILESTONE_8_DESIGN.md
docs/implementation/MILESTONE_8_REPORT.md
```

The M8 software implementation and completion gate are finished. The report
maps C1-C20, records the actual tests-first sequence, resolved versions, fixed
bounds and the 388-test debug/release gates. Do not reopen software design or
add unrelated features unless the remaining bench exposes a real defect.

## Remaining authorized work

Only the explicit design-section-12 real-hardware checklist remains:

1. Obtain an operator-confirmed actual Metakon, Windows COM binding, firmware,
   wiring, serial settings and justified clean recovery boundary.
2. Run read-only compatibility and repeated real measurement acquisition through
   the production COM adapter. Never issue actuator/register writes.
3. Record actual observations under Required recording, inspect them through the
   public API/Babashka and SQLite, and retain exact configuration provenance.
4. Physically disconnect/reconnect, observe explicit failure, use the explicit
   reconnect operation only after the justified boundary, and confirm resumed
   real measurements with a new binding generation.
5. Cleanly shut down COM plus Recorder and reopen the database.

Record the date, implementation commit, non-private environment details, hashes,
observations and shutdown result in `MILESTONE_8_REPORT.md`. Fake, loopback or
deterministic transports cannot satisfy C16/C17 or the hardware portion of C19.

If hardware or an operator step remains unavailable, keep
`M8_HARDWARE_ACCEPTANCE_PENDING` and stop. Only after every hardware row passes
may the coordination files be set to `READY_FOR_EXTERNAL_REVIEW`. Do not begin
M9 and do not cross a model or milestone gate automatically.

If the bench reveals a genuine architectural contradiction involving Runtime
ownership, OutputAuthority, Required recording, evidence, provenance,
boundedness or shutdown, write the precise issue to `ai/HANDOFF.md`, set
`STATUS: WAITING_FOR_REVIEW`, and stop.
