# Current work — M8 corrected real-hardware acceptance

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING

Current model: SOL_HIGH.
Current phase: Recorder-ordering-corrected physical reconnect gate pending.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed corrections complete
through Recorder FIFO correction commits `fdf8a73` and `2ec104b` on 2026-09-16.
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
deployment paths, finite recovery, resource-scoped reconnect fencing, Required
Recorder fail-closed semantics and every prior SQLite bench archive. Exact
red/green history, hashes and gates are in `MILESTONE_8_REPORT.md`.

## Current checkpoint

The latest physical run proved corrected finite disconnect and that a failed
generation-2 reconnect did not release ordinary acquisition. It then exposed a
Recorder FIFO identity hole: lifecycle capacity reservation assigned an unsent
record ID before the resource-scoped compatibility probe emitted a normal fact.
The SQLite worker correctly rejected the later ID with
`fact record reservation mismatch`; Required recording failed with an unknown
tail and the reconnect surfaced as `invalid_configuration`. The archive is
preserved unchanged at
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`, SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.

External review authorized and SOL_HIGH completed the narrow correction. Live
activation now reserves only fixed capacity and generation before side effects.
It assigns the current FIFO-tail record identity only when the Activation is
immediately enqueued. Intervening probe facts remain contiguous; cancellation
does not rewind or skip IDs. Existing bounds, one writer, exact worker validation,
resource-scoped quiescing and Required failure behavior remain unchanged.
Recorder lifecycle failures now use the existing public
`recording_unavailable` category rather than being masked as configuration
validation failures.

Commit `fdf8a73` contains production and tests. The debug workspace passes 414
named tests; all 136 Core tests, targeted Recorder/reconnect/COM suites, actual
Babashka A/B, fmt, workspace all-target warning-denied Clippy and diff checks
pass. Full release and warning-denied rustdoc remain deferred to the successful
final hardware gate under the accepted review instruction.

The next archive is
`examples/metakon-513-com5-recorder-corrected-history.sqlite`; it was confirmed
absent and has not been opened. Commit `2ec104b` selects it. Exact hashes are:

```text
runtime.toml  405d99056fd4ba265bb8776f18360a33cd4be81e65c7e03976196acbf374056b
definition    b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb
```

Do not reopen COM5 until the user explicitly continues from this checkpoint.
The next authorized launch command is:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

Continue only the remaining read-only M8 hardware acceptance. Do not reuse or
modify prior SQLite evidence, issue actuator/configuration writes, probe another
register, enumerate COM ports, start M9 or perform post-M8 cleanup.
