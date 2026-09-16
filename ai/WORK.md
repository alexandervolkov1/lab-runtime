# Current work — M8 corrected real-hardware acceptance

STATUS: WAITING_FOR_REVIEW

Current model: SOL_HIGH.
Current phase: post-reconnect-fix real-hardware review stop.
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

## Corrected checkpoint and review stop

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

The corrected real run used the previously absent
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`. Normal acquisition
matched the operator's 28-degree front panel. Physical disconnect produced one
durable value-less Unavailable row, finite Offline, unchanged generation, zero
queue growth and no fabricated Good.

After physical reconnection, exactly one explicit reconnect request was issued.
It reconciled as terminal `failed/invalid_configuration`; ordinary acquisition
remained quiesced, Offline and Unavailable, so the escaped-acquisition defect did
not recur. During the same operation Required Recorder failed with
`fact record reservation mismatch`, leaving an unknown tail after durable record
472. Accepted normal shutdown exited finitely and honestly with code 1 and
`safe shutdown incomplete`.

The archive is preserved unchanged at SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.
Exact public and SQLite evidence is recorded in `MILESTONE_8_REPORT.md`. Do not
reopen COM5, retry reconnect, reuse this archive, implement another correction,
run the final M8 gate or start M9 until external review supplies a new explicit
authorization.
