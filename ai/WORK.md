# Current work — M8 corrected real-hardware acceptance

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING

Current model: SOL_HIGH.
Current phase: post-reconnect-fix real-hardware acceptance pending.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed corrections complete
through reconnect gate commit `0ae7b3c` on 2026-09-16.
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

The C1-C20 software implementation remains complete. The corrections preserve
the strict decoder/raw I16 contract, explicit device scale, deterministic
deployment paths, finite recovery, resource-scoped reconnect fencing and every
prior SQLite bench archive. Exact red/green history, hashes and gates are in
`MILESTONE_8_REPORT.md`.

## Corrected checkpoint and next authorized action

The post-recovery physical run passed real 23--25 °C acquisition against the
operator's 25 °C display. Physical USB removal produced exactly one durable
Unavailable row with no value, finite Offline at the configured two-second
deadline, zero queue growth and no fabricated Good. The first explicit reconnect
then returned terminal `invalid_configuration` while its replacement generation
nevertheless acquired new Good 26/27 °C rows. The cleanly sealed evidence archive
is preserved unchanged at SHA-256
`6afe7924e64326a0dedc37b3d860428a4a351c8f35803c56d51ba271f8caa0e4`.

External review authorized and SOL_HIGH completed the narrow reconnect correction
at `0ae7b3c`. A bounded resource-scoped gate now blocks only target ordinary
acquisition from pre-retirement through compatibility and durable lifecycle
completion. The rebind Unavailable is a probe baseline rather than an immediate
failure; only the target channel-type probe is admitted. Failure preserves the
advanced generation, retires/quarantines the replacement finitely, stays
Unavailable and requires another explicit reconnect using the current generation.
Success releases acquisition only after probe and lifecycle completion and resets
the cadence without catch-up. The debug workspace gate passes 407 named entries;
workspace Clippy, fmt and diff checks pass. Release/rustdoc remain deferred until
the corrected M8 hardware gate.

Do not reopen COM5 until the user explicitly continues. The next run uses the
new, absent archive
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`, TOML SHA-256
`200acfe4a2cd36b4375213db9369518d751e0ad43c8401f0b456cb0a03ef6b7c`,
and unchanged definition SHA-256
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Preserve all three older SQLite evidence sets. On explicit continuation, use
this approved read-only launch command:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

Repeat the approved read-only acquisition/disconnect/explicit-reconnect sequence.
No ordinary post-rebind temperature may appear before successful channel type 3
and reconnect completion. Do not start M9.

If actual behavior contradicts the trusted protocol/recovery contract, preserve
the observations, set `STATUS: WAITING_FOR_REVIEW` and stop. A fake transport
cannot satisfy the hardware gate. Only after all remaining physical C14/C16/C17/
C19 evidence passes may M8 become ready for external review. Do not begin M9.
