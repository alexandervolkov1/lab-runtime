# Milestone 9A implementation report

Date: 2026-09-17

```text
Baseline: 69a377be20960fb8037a1a526cb2f2ca863633a4
Implementation commit: bba8f49
M8: ACCEPTED
M9A: IMPLEMENTED
STATUS: READY_FOR_M9A_EXTERNAL_REVIEW
M9B: NOT AUTHORIZED
```

## Result

M9A made the existing managed-component boundary implementation-neutral and added a
first-class native Rust implementation without creating another Core lifecycle.
Runtime still owns admission, correlation, committed state, signal publication,
generation/revision fencing, result validation and failure propagation:

```text
Runtime -> Invocation -> ComponentExecutor -> ComponentCompletion/ComponentResult
        -> Runtime validation -> committed managed state/signal
```

`lab-core` remains independent of Lua, `mlua`, TOML, paths, storage, OS and host
composition.

## Neutral definition and implementation selection

`ComponentDefinition.source: String` was replaced by `ComponentImplementation`:

- a validated semantic `ComponentImplementationId`, at most 64 lowercase ASCII
  identifier bytes;
- `BuiltIn` for compile-time trusted native implementations;
- nonempty `Text`, at most 32 KiB, for a trusted source adapter;
- existing bounded `PlainData` configuration, manifest and lifecycle fields.

Core carries and reports implementation identity but does not interpret the artifact
or know Lua. Trusted host composition admits only registered IDs. Deployment TOML now
requires an explicit `implementation`; `source` is optional and legal for `lua.v1`,
while `native.moving_mean.v1` requires no source. TOML config leaves are converted to
the same validated `PlainData` used by invocations.

## One common executor

`ManagedExecutor` is the only production `ComponentExecutor`. It moved the accepted
worker boundary out of `lab-lua` and provides one compile-time registry for both:

```text
lua.v1                   -> lab_lua::run_bounded
native.moving_mean.v1    -> native MovingMean runner
```

It owns exactly two worker threads process-wide. Each worker has one capacity-1 job
mailbox and one capacity-1 completion mailbox, a 2 MiB stack and a 100 ms absolute
job deadline. Therefore at most two managed jobs are pending/running, no backlog is
hidden behind a busy worker, and no replacement worker is spawned after a hung job.
Cancellation is atomic and nonblocking. Runtime shutdown closes admission, fences
pending work and observes unfinished workers without joining them on the owner lane.
A completed worker retires when its mailbox closes; a stuck trusted implementation
remains reported as unfinished rather than delaying safety service.

The worker owns only an `Invocation`, deadline and cancellation flag. It has no
Runtime, transport, Recorder or `OutputAuthority` handle and its result type cannot
express a physical ACK, readback, transport completion or safe evidence.

## Native reference component

The reference native transform is `native.moving_mean.v1`.

- it accepts exactly one numeric `window` configuration in `2..=64`;
- its manifest must be a Transform and its warm-up count must equal the window;
- it preserves the input engineering unit exactly;
- persistent state is one `PlainValue::Numbers` field named `values`, never longer
  than the configured window and still subject to the 4 KiB `PlainData` bound;
- init publishes no Good value;
- steps remain Warming until the full window is present;
- the mean is computed with checked finite intermediates;
- Core applies the same finite/range/unit/status/state validation used for Lua before
  any commit.

The standard profile retains its frozen Lua source model but now uses native
`MovingMean` as the downstream transform. This exercises a real mixed implementation
graph and the existing native controller consumer.

## Replacement and reload

Native and Lua definitions use the existing staged init and replacement machinery.
A replacement advances generation exactly once, resets state/warm-up, invalidates
dependents and fences old correlation identities. A failed component remains scoped;
it cannot mutate unrelated native safety/control state.

The public lifecycle names are now:

```text
reload_managed_sources
restart_models
```

`reload_managed_sources` rereads only declared source artifacts. It prepares the
source-backed components plus their transitive managed dependents, so a native
transform consuming a replaced source is reinitialized honestly without pretending
that native code had a mutable source file. `restart_models` reinitializes configured
managed components and native virtual models from already frozen definitions without
rereading TOML or source. Neither operation rearms output authority.

## Public API and provenance

Hello now advertises `managed_component`, `managed_source` and
`managed_transform`. Discovery and component status include the stable implementation
ID. The accidental public `lua_source`, `lua_transform`, `reload_managed_scripts` and
`restart_virtual_models` vocabulary was removed pre-v0.1.

New activations record bounded neutral entries:

```text
managed_component_implementation
managed_component_source
```

Implementation provenance includes component ID, semantic implementation ID,
`built_in`/`text` artifact class, package build version and bounded PlainData config.
Native provenance does not claim source bytes. Old SQLite rows named
`managed_lua_source` remain valid historical evidence and Recorder retains bounded
read/write compatibility for that old kind; no archive was migrated.

## Tests-first evidence

The first M9A Core tests were added before the production representation and failed
to compile because `ComponentImplementation` and snapshot implementation identity did
not exist. The implemented focused coverage then proved:

- native definitions need no executable text and Lua uses the same neutral contract;
- a built-in identity cannot bypass Core finite/range validation;
- native MovingMean init, three-sample warm-up, output and bounded state;
- wrong-unit rejection before execution;
- one generation advance and empty warm-up state after replacement;
- finite common-worker shutdown and the fixed two-worker bound;
- source-only reread with dependency-closure replacement and source-independent model
  restart;
- neutral public capabilities, operations, discovery and implementation status;
- truthful Lua text/native built-in provenance;
- retained stale-completion, invalid-result, scoped-failure, controller, Recorder,
  output-safety and Lua sandbox/security regressions.

## Verification

All required gates passed on Windows:

- `cargo fmt --all -- --check`;
- `cargo test --workspace`;
- `cargo test --workspace --release`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- warning-denied `cargo doc --workspace --no-deps`;
- all three actual Babashka A/B process tests with Babashka 1.13.220;
- `bb test-client`: 8 tests, 13 assertions;
- finite `cargo run -p lab-runtime` demo;
- `git diff --check`.

COM5 was not opened. No hardware run occurred. The successful M8 archive remains
SHA-256 `63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80`;
the prior failure oracle remains
`1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023`.

## Remaining legitimate Lua-specific names

Lua-specific vocabulary remains only where it is truthful:

- the `lab-lua` crate, runner, fixtures and sandbox/security tests;
- semantic implementation ID `lua.v1` and Lua-specific adapter validation/errors;
- `.lua` fixture/source paths and source-reload tests that explicitly exercise that
  adapter;
- historical M5 design/report text;
- historical `managed_lua_source` SQLite compatibility and tests.

The common Core contract and public product API no longer define managed components
as Lua. Optional complete `lab-lua` removal remains a later decision after replacement
coverage; it is not part of M9A.

## Gate

M9A is ready for explicit external review. M9B, Presentation API, GUI and Steel work
remain NOT AUTHORIZED.
