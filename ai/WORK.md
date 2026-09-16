# Current work — M8 corrected real-hardware acceptance

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING

Current model: SOL_HIGH.
Current phase: post-fix real-hardware disconnect/reconnect acceptance pending.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed corrections complete
through recovery commit `71d4e56` on 2026-09-16.
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

## Corrected checkpoint and next authorized action

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

External review authorized and SOL_HIGH completed the narrow recovery correction
at `71d4e56`. Transaction and protocol recovery now use the configured finite
monotonic deadline; ordinary admission and old queued work are fenced at the
resource owner; the first physical read failure publishes one normal
`Unavailable`/Recorder fact; failed recovery becomes Offline without generation
advance; and shutdown starts nonblocking adapter retirement from every executor
state without weakening honest `cleanup_incomplete`. The full debug workspace
gate passes 402 named tests. Release/rustdoc gates remain deferred until the
corrected M8 hardware gate.

Do not reopen COM5 until the user explicitly continues. The next run uses the
new, absent archive
`examples/metakon-513-com5-recovery-corrected-history.sqlite`, TOML SHA-256
`5fec1c546f81b8a29f7a21360deb33575fe98a87c987fbbd1a28094bd8e01def`,
and unchanged definition SHA-256
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Preserve both older SQLite/SHM/WAL evidence sets. On explicit continuation, use
this approved read-only launch command:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

Verify corrected Offline/Unavailable, then wait for the operator's reconnect cue
and use only `reconnect_resource`. Do not start M9.

If actual behavior contradicts the trusted protocol/recovery contract, preserve
the observations, set `STATUS: WAITING_FOR_REVIEW` and stop. A fake transport
cannot satisfy the hardware gate. Only after all remaining physical C14/C16/C17/
C19 evidence passes may M8 become ready for external review. Do not begin M9.
