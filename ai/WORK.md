# Current work — M8 Metakon protocol discrepancy review

STATUS: WAITING_FOR_REVIEW

Current model: SOL_HIGH.
Current phase: review actual Metakon 513 register-1 value interpretation.
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

## Review stop

The actual COM5 bench reached ready after the strict read-only channel-type probe,
then repeatedly decoded register-1 raw I16 values 23/24 as 2.3–2.4 °C under the
accepted 0.1 scale. The physical Metakon 513 display was approximately 21 °C.
Required SQLite evidence, deterministic frame reconstructions and clean shutdown
are recorded in `MILESTONE_8_REPORT.md`. The relative `--config` launch also
failed before COM open because its resolved Recorder path remained relative.

Do not resume hardware operations, change scale/register/address, run the
disconnect test or attempt any write/reset without explicit reviewed direction.
Consult only authoritative documentation or read-only donor evidence. M9 remains
unauthorized.

## Remaining acceptance after review

If review establishes the correct trusted read-only mapping, update it tests-first
and rerun the explicit real-hardware checklist without actuator operations. The
remaining physical disconnect/reconnect, public history/provenance inspection and
final close/reopen evidence still must pass. Only after every hardware row passes
may the coordination files be set to `READY_FOR_EXTERNAL_REVIEW`. Do not begin
M9 and do not cross a model or milestone gate automatically.
