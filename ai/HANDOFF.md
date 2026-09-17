# AI handoff

## Current state

```text
M8: ACCEPTED
M9A: IMPLEMENTED
STATUS: READY_FOR_M9A_EXTERNAL_REVIEW
Current phase: external M9A review
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

## M9A implementation

Core now carries an explicit bounded `ComponentImplementation` (semantic ID plus
`BuiltIn` or bounded `Text` artifact) instead of requiring source text. One common
two-worker `ManagedExecutor` registry dispatches both frozen `lua.v1` and the native
`native.moving_mean.v1` transform through the existing invocation, completion,
validation, generation, revision and failure lifecycle.

Deployment selection is explicit. Native `MovingMean` uses a 2..=64 sample window,
bounded `PlainData` state, honest warm-up and unit preservation. Public capability,
discovery and lifecycle vocabulary is neutral (`managed_component`,
`reload_managed_sources`, `restart_models`). New provenance records neutral
implementation/source identities; historical `managed_lua_source` archives remain
unchanged.

The first external review found three focused M9A blockers, now corrected. Source
reload prepares and validates only the selected source-backed dependency closure and
commits its schedule delta without a fallible global activation after mutation.
Text-backed component objects carry a content-addressed SHA-256 link to their exact
executed source blob. Built-in component metadata carries a cached streaming SHA-256
of the actual running executable; package version is descriptive only. Deterministic
regressions cover the unrelated-Failed reload oracle, pre-commit rejection, source
swaps/exact recovery and native binary association.

The complete debug/release/fmt/Clippy/rustdoc/Babashka/client/demo gates pass after
the review corrections. The implementation report is
`docs/implementation/MILESTONE_9A_REPORT.md`.

## Next step

Perform only external M9A re-review under `ai/WORK.md`. M9B remains NOT AUTHORIZED
until M9A receives explicit external acceptance; M10 and M11 remain future milestones.
Detailed chronological M8 evidence remains in
`docs/implementation/MILESTONE_8_REPORT.md`.
