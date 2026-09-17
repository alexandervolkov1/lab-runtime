# Milestone 9A implementation report

Date: 2026-09-17

```text
Baseline: 69a377be20960fb8037a1a526cb2f2ca863633a4
Initial implementation commit: bba8f49
External-review correction commit: b7d8f40
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
`built_in`/`text` artifact class, package version and bounded PlainData config. Each
text-backed component additionally records `source_content_sha256`, calculated from
the exact bounded bytes supplied to its executor. That digest addresses the matching
`managed_component_source` blob directly; entry ordering and deduplication are not
part of the association. Each built-in component instead records
`runtime_binary_sha256`, the SHA-256 of the actual running executable. The executable
is streamed through a fixed 64 KiB buffer and the result, including a bounded typed
failure, is cached process-wide. Package version remains descriptive metadata and is
not treated as build identity. Native provenance does not claim source bytes. Old
SQLite rows named `managed_lua_source` remain valid historical evidence and Recorder
retains bounded read/write compatibility for that old kind; no archive was migrated.

## External-review corrections

The first external review reproduced three blockers.

First, `reload_managed_sources` committed the selected source-backed branch and then
ran global configured-component activation. An unrelated component already in
`Failed` state could therefore clear the global schedule and make the public
operation return `OwnerFailure` after the selected generation had advanced. Reload
now validates the source bundle, implementation/artifact and complete affected
dependency closure; prepares only that branch's schedule delta; establishes binary
identity and Recorder/lifecycle capacity; and only then commits the prepared Core
replacement, selected schedule delta and already-validated source bundle. No global
post-commit schedulability check remains. An unrelated failed or warming component
is unchanged and cannot turn a successful selected reload into an error. The
selected generation still advances exactly once, state/warm-up reset, old completion
identities remain fenced, and no controller or output authority is rearmed.

Second, the earlier neutral provenance kept exact source blobs but did not bind a
component activation to its concrete blob by immutable content identity. Recorder
objects now carry an optional bounded `source_content_sha256`; Recorder accepts it
only for managed components and only when the activation contains an exact
`managed_component_source` blob with that digest. The durable object
`source_hash` column stores this digest, while `definition_hash` continues to address
the component implementation metadata. Swapping source A and B between two component
IDs now swaps their per-component source hashes and produces unambiguous historical
recovery even when the source set and its sorting/dedup order are unchanged.

Third, `CARGO_PKG_VERSION` alone did not identify native executable bytes. Built-in
implementation metadata now links every native component to the cached
`runtime_binary_sha256`. Failure to resolve, open or read the actual executable is a
bounded explicit provenance failure; version text is never substituted as equivalent
build identity.

Deterministic regressions reproduce the original reload oracle with an independent
failed native component and a second independent component initially in `Warming`,
prove invalid selected source fails before generation mutation, prove source A/B
component swaps and exact byte recovery, reject a missing source blob atomically,
hash controlled byte fixtures, and verify native Recorder provenance links to the
current executable digest without a fabricated source hash.

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
