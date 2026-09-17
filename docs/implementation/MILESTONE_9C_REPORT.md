# Milestone 9C report — remove Lua and obsolete client baggage

## Result

M9C removed the active embedded Lua implementation and obsolete first-party client
surface without replacing them. The active v0.1 product remains a headless Rust
Runtime with one bounded, language-neutral Application API.

```text
M9C: READY_FOR_EXTERNAL_REVIEW
M10: NOT AUTHORIZED
```

## Removed product code and dependencies

- Removed the `crates/lab-lua` workspace member, its VM/sandbox/fixture code and the
  `mlua` dependency graph (`mlua-sys` and `lua-src`).
- Removed executable-text `ComponentImplementationArtifact::Text`, source-path
  configuration, source freezing/reload and active text-source provenance emission.
- Removed all active `lua.v1` registration and execution paths.
- Removed `clients/babashka`, its workflow/tests and the Babashka-only process test.
- Removed Lua-only Runtime integration tests. Required lifecycle, process, reconnect,
  multi-client and Recorder isolation coverage remains in Rust/raw-NDJSON tests.

`cargo tree` contains no `lab-lua`, `mlua`, `mlua-sys` or `lua-src` package.

## Managed-component architecture retained

The active path is:

```text
ComponentDefinition + ComponentImplementationId (BuiltIn)
      ↓
native implementation registry
      ↓
Invocation
      ↓
bounded ComponentExecutor
      ↓
ComponentCompletion / ComponentResult
      ↓
Runtime validation, generation fencing and commit
```

`PlainData`, bounded state/diagnostics, warm-up, finite/range/unit validation,
generation/revision fencing and stale-completion rejection remain unchanged.
`native.moving_mean.v1` is the canonical production implementation. It is projected
through generic discovery, signal/current/history/subscription and property surfaces;
there is no component-specific wire operation.

The unconfigured demonstration composition now feeds the native moving mean from the
normal virtual thermal-plant signal. It no longer creates a hidden scripted source.

## Application API cleanup

Removed Lua/source-reload vocabulary:

- mutation `reload_managed_sources`;
- capability `managed_source_reload`;
- source-reload request, lifecycle and scheduling state.

Removed the transitional aggregate snapshot convenience:

- query `runtime_snapshot`;
- query `snapshot_page`;
- query `snapshot_release`;
- capability `runtime_snapshot`.

Structured discovery, paged current measurements and domain-specific current queries
already provide the reconstructable state needed after reconnect. Discovery,
measurement and configuration projection tokens remain bounded connection-local
delivery state; they are not the removed aggregate snapshot contract.

The final registry contains 42 operations: 22 queries and 20 mutations. The final
capability registry contains 25 semantic capabilities. Exact ordered inventories are
asserted directly against the authoritative registry in `protocol` tests. Requests
using any removed operation receive structured `unsupported_operation` responses.

Queries:

```text
hello, discover, discovery_page, describe, resource,
configuration_status, configuration_properties, configuration_page,
latest, measurements_current, measurements_page, measurement_window,
controller, reference, component, output, operation_status,
subscribe, unsubscribe, recording_status, history_page, history_release
```

Mutations:

```text
reference_configure, reference_retune,
controller_configure_pid, controller_configure,
controller_start, controller_pause, controller_resume, controller_reset_failed,
runtime_shutdown, stage_configuration, apply_configuration,
reload_configuration, property_configure, emulator_publish,
virtual_models_restart, reconnect_resource, recording_start, recording_stop,
experiment_annotate, history_read
```

Capabilities:

```text
operation_lifecycle, structured_discovery, live_subscriptions,
current_measurements, recent_measurement_history, instrument_queries,
reference_read_write, controller_status, controller_configuration,
controller_lifecycle, managed_components, output_status, runtime_shutdown,
recording_status, recording_control, measurement_history, resource_status,
configuration_read, configuration_properties, configuration_write,
deployment_configuration, resource_reconnect, virtual_instruments,
emulator_publication, virtual_model_lifecycle
```

## Configuration and provenance

Active deployment accepts only registered native managed implementations. A
`lua.v1` candidate or a removed `source` field is rejected before Runtime mutation.
There is no executable-source path to reload and no Runtime overlay is persisted to
TOML.

New component provenance records the semantic native implementation ID, bounded
configuration and SHA-256 identity of the actual Runtime binary. It does not emit a
managed executable-source blob or source hash.

## Historical compatibility

Historical evidence was not rewritten. Milestone reports and immutable SQLite
archives remain unchanged. Recorder provenance validation retains the bounded names
`managed_component_source` and `managed_lua_source` only to validate and interpret
old content-addressed archive records. The operation-kind reader also accepts old
`reload_managed_sources` / `restart_models` facts so historical archives remain
readable. None of those names is generated by the active API or active configuration.

```text
active product surface != historical compatibility/evidence
```

## Replacement acceptance coverage

Rust tests continue to prove:

- native moving-mean initialization, bounded window/warm-up/state and replacement;
- stale generation/completion fencing and atomic validation;
- generic property discovery/configuration and signal/current/history/subscriptions;
- native binary provenance and Recorder integration;
- removed Lua implementation rejection and removed-operation rejection;
- real loopback hello/discovery, disconnect/reconnect, request replay and finite
  shutdown;
- multi-client isolation, slow-client gap handling, history cleanup, emulator
  pressure and Recorder progress through the M9B.8 acceptance suite.

No product client crate was introduced for testing.

One pre-existing Windows cleanup race reproduced twice in the full parallel debug
suite: after Recorder shutdown had completed and the same archive had already been
reopened successfully, immediate fixture deletion could still receive sharing
violation 32. This was a test-harness cleanup defect, not a Recorder lifecycle or
durability failure. Cleanup now waits on the observable file-deletion predicate
within the existing four-second shutdown budget; no production timeout or Recorder
code changed. The exact test then passed five consecutive runs.

An optimized cursor-capacity test also exposed an invalid aggregate latency
assumption: it admitted eight independent history jobs and required the entire queue
to finish inside one shared two-second wall-clock window. Cursor capacity has no such
latency contract. The test now establishes each of the eight retained cursors after
its observable terminal result, then exercises ninth-client exhaustion and release.
Production history code and bounds were unchanged; the corrected release test passed
ten consecutive focused runs.

## Safety and scope

No controller, OutputAuthority, transport, Recorder durability, emulator or physical
instrument semantics changed. The Application API still cannot fabricate transport
completion, ACK/readback, safe evidence, `OutputLease` or an authority decision.
No GUI, Presentation API, Steel, replacement language, Python client, SDK or dynamic
plugin loader was added. COM5 was not opened and no hardware test was run.

## Verification

The completion gate comprises:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
cargo tree
git diff --check
```

Focused native-component, protocol, projection, provenance, process and M9B.8 fault
acceptance tests are included in those gates. Final command results are recorded in
the M9C completion handoff and must pass before external review.
