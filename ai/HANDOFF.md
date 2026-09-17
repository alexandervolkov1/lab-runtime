# AI handoff

## Current state

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
Current phase: M8 external acceptance gate
M9: NOT AUTHORIZED
```

M7 Recorder/SQLite is externally accepted at `f3ff456`. M8 implementation and its
final read-only hardware acceptance are complete. Production hardware code is the
accepted `8da89ab` baseline; the final release-only correction changes tests and
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

Do not open COM5 or perform another hardware run unless a later review explicitly
authorizes it.

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

External M8 review only. Do not start M9A/M9B until M8 is explicitly accepted.
Detailed chronological evidence is in
`docs/implementation/MILESTONE_8_REPORT.md`.
