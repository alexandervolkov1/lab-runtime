# Current work — M8 corrected real-hardware acceptance

STATUS: WAITING_FOR_REVIEW

Current model: SOL_HIGH.
Current phase: review actual Windows COM disconnect/recovery contradiction.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed corrections complete
through `867f985` on 2026-09-16.
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

The C1-C20 software implementation remains complete. The correction phase kept
the strict decoder/raw I16 contract, made the actual Metakon 513 thermocouple
scale explicit, resolved deployment-owned relative paths from runtime.toml, and
preserved the first SQLite bench archive. Exact red/green history, hashes and
391-test gates are in `MILESTONE_8_REPORT.md`.

## Review stop

Corrected acquisition passed at 27/28 °C against the operator's 28 °C display.
After the operator physically disconnected the device/cable, resource 1 entered
`recovering` but did not reach Offline or publish Unavailable for more than 25
seconds despite a configured 2-second recovery timeout. Transaction 498 remained
active and the bounded queue filled to 32. The old Good timestamp did not advance,
so there was no fabricated new Good sample.

Reconnect was not attempted. Finite shutdown flushed Recorder but reported one
unfinished, unclosed transport and exited unsuccessfully. The sealed corrected
SQLite archive has complete coverage/no gaps, 381 Good and zero Unavailable
temperature rows, zero output events, and an honest recovering-resource boot
seal. Full evidence and hashes are in `MILESTONE_8_REPORT.md`.

Do not reconnect or reopen COM5 until external review determines the correction
scope. Preserve the corrected SQLite/SHM/empty-WAL files and the original evidence
files. Do not start M9.

If actual behavior contradicts the trusted protocol/recovery contract, preserve
the observations, set `STATUS: WAITING_FOR_REVIEW` and stop. A fake transport
cannot satisfy the hardware gate. Only after all remaining physical C14/C16/C17/
C19 evidence passes may M8 become ready for external review. Do not begin M9.
