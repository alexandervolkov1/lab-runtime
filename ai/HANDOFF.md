# AI handoff

## Current state

```text
M8: ACCEPTED
M9A: AUTHORIZED
Current phase: M9A — neutral managed components and native Rust execution
```

M7 Recorder/SQLite is externally accepted at `f3ff456`. M8 implementation, final
read-only hardware acceptance and final software gates are externally accepted at
`92aec535e7279ef9931247bd4ced190fc18ebebe`. Production hardware code is the accepted
`8da89ab` baseline; the final release-only correction changed tests and
coordination/reporting only.

## Preserved hardware evidence

Successful archive (do not modify, reuse or reopen through Runtime):

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80
```

It proves generation-1 Good acquisition, one Unavailable after power-off, one public
reconnect, the generation-2 channel baseline and Good `channel_type = 3`, durable
lifecycle before ordinary generation-2 Good acquisition, sustained acquisition,
Babashka A-kill/B-resume independence, a live-safe revision without rebind, zero
outputs, zero gaps, complete seals and clean COM/Recorder shutdown.

The earlier failed reconnect oracle also remains immutable:

```text
examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

No further M8 hardware rerun is required. COM5 is not part of current work.

## Final release-test correction

The reproducible release-only first error was `recorder ingress capacity exhausted`.
The shared test helper waited for `outstanding_groups == 1`, which did not prove that
three earlier charged fact groups and their receipts had drained before a live
activation reserved the fourth group. Production correctly failed closed.

The correction is test-only: wait for zero outstanding groups plus a durable
`confirmed_submission` through the Offline/Unavailable fact before reserving
activation. An existing `WriterBarrier` now deterministically recreates the exact
three-group backlog. No production code, Recorder invariant, limit or timeout changed.

Repeated exact release tests, repeated Recorder/reconnect release subsets, complete
debug and release workspace suites, formatting, warning-denied Clippy and rustdoc,
actual Babashka tests, `bb test-client`, finite demo and diff checks all pass.

## Next step

Implement only the detailed M9A authorization in `ai/WORK.md`. M9B remains NOT
AUTHORIZED until M9A receives explicit external acceptance; M10 and M11 remain
future milestones. Detailed chronological M8 evidence remains in
`docs/implementation/MILESTONE_8_REPORT.md`.
