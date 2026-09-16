# Current work — M8 corrected real-hardware acceptance

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING

Current model: SOL_HIGH.
Current phase: corrected Metakon 513 read-only hardware acceptance.
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

## Next authorized action

Do not open COM5 until the user explicitly continues from this checkpoint. Then
launch exactly:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

The frozen configuration opens only COM5 at 9600 8N1/no flow, address 5; exposes
only the trusted `channel_type` and `temperature` reads; uses Required recording;
and writes to the new
`examples/metakon-513-com5-corrected-history.sqlite`. Do not issue actuator,
output, configuration-register, reset or speculative protocol operations.

First obtain consecutive plausible real readings, confirm resource status,
Good Signal quality, binding generation and Required SQLite progress, then stop
before the operator's physical disconnect. After a separate explicit user cue,
observe Offline/Unavailable without fabricated Good values, wait for reconnect,
use only `reconnect_resource`, verify a new binding generation and resumed real
measurements, perform the public Babashka/history/provenance checks and shut down
COM plus Recorder cleanly. Reopen SQLite and record durable evidence in the M8
report.

If actual behavior contradicts the trusted protocol/recovery contract, preserve
the observations, set `STATUS: WAITING_FOR_REVIEW` and stop. A fake transport
cannot satisfy the hardware gate. Only after all remaining physical C14/C16/C17/
C19 evidence passes may M8 become ready for external review. Do not begin M9.
