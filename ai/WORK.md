# Current work — external M9A review

```text
M8: ACCEPTED
M9A: IMPLEMENTED
STATUS: READY_FOR_M9A_EXTERNAL_REVIEW
Current phase: external M9A review
M9B: NOT AUTHORIZED
```

This is the only current authorization. Review the completed M9A implementation;
do not begin M9B, Presentation API, GUI, Steel or unrelated release work.

## Review objective

Confirm that the shared managed-component architecture is implementation-neutral and
that native Rust executes through the same bounded Runtime-owned lifecycle rather
than through a second subsystem or direct state mutation.

Review together:

```text
crates/lab-core/src/managed.rs
crates/lab-core/src/runtime.rs
crates/lab-lua/src/runner.rs
apps/lab-runtime/src/managed_executor.rs
apps/lab-runtime/src/configuration.rs
apps/lab-runtime/src/host.rs
apps/lab-runtime/src/service.rs
apps/lab-runtime/src/application.rs
docs/implementation/MILESTONE_9A_REPORT.md
```

## Required findings

Verify:

1. `ComponentDefinition` selects an explicit bounded implementation without requiring
   executable text for a native component.
2. `lab-core` remains independent of Lua, OS, filesystem, TOML and host adapters.
3. Frozen `lua.v1` and `native.moving_mean.v1` use the same `Invocation`,
   `ComponentCompletion`, `ComponentResult`, Core validation and generation/revision
   fences.
4. One common executor owns exactly two worker threads, one job and one completion
   mailbox per worker, a 100-ms job deadline and no replacement-worker churn.
5. Native `MovingMean` has a validated 2..=64 window, bounded `PlainData` state,
   honest three-sample standard-profile warm-up, unit preservation and no direct
   Runtime/output/evidence capability.
6. replacement advances generation once, rejects old completion identity and resets
   warm-up/state; failures remain component-scoped.
7. `reload_managed_sources` rereads only source-backed artifacts and reinitializes
   their dependency closure; `restart_models` reinitializes without rereading source.
8. public capability/discovery/status/lifecycle vocabulary is implementation-neutral.
9. new provenance truthfully distinguishes built-in implementation identity from a
   bounded text source, while historical `managed_lua_source` evidence remains
   unchanged.
10. M5 Lua sandbox/security coverage, controllers, Recorder, deployment and output
    safety regressions remain green.

## Preserved evidence and scope

Do not modify or reopen through Runtime:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

No COM5 or hardware run is required. Do not perform the deferred `.gitattributes`
hardening correction in this review.

## Gate

If no technical or documentation blocker remains, return explicit M9A acceptance.
M9B remains NOT AUTHORIZED until that external acceptance. Do not cross the milestone
gate automatically.
