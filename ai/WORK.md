# Current work — M9C external review

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: READY_FOR_EXTERNAL_REVIEW
Current phase: M9C external review gate
M10+: NOT AUTHORIZED
```

M9C implementation is complete. No implementation milestone is currently authorized.
Do not begin M10, M11, M12 or post-v0.1 work without an explicit review decision.

## Completed removal

The active v0.1 product no longer contains:

- the `lab-lua` workspace crate or `mlua` dependency;
- executable-text component artifacts, Lua VM/sandbox/source loading or `lua.v1`;
- Lua source configuration and source-reload lifecycle;
- `reload_managed_sources` or `managed_source_reload`;
- the transitional `runtime_snapshot`, `snapshot_page` and `snapshot_release` API;
- the first-party Babashka client, workflow and client-only acceptance tests.

No replacement scripting language, client SDK, GUI or Presentation API was added.

## Active architecture

```text
Application API
      ↓
authoritative Runtime
      ├── native instruments
      ├── native managed components
      ├── native controllers / OutputAuthority
      ├── Recorder
      └── virtual instruments / emulator API
```

The neutral managed-component contract remains: `ComponentDefinition`,
`ComponentImplementationId`, BuiltIn selection, `Invocation`, `ComponentResult`,
`ComponentCompletion`, `ComponentExecutor`, bounded `PlainData` and generation /
revision fencing. `native.moving_mean.v1` is the canonical active implementation.

The Application API contains 42 operations (22 queries and 20 mutations) and 25
capabilities. Discovery/current/domain-specific projections supersede the removed
aggregate snapshot family.

## Historical compatibility

Historical milestone reports and immutable SQLite evidence remain unchanged.
Recorder validation retains the minimal historical provenance vocabulary needed to
interpret old `managed_component_source` / `managed_lua_source` records. New active
recording emits BuiltIn native implementation provenance with Runtime binary identity;
it does not emit executable source artifacts.

```text
active product surface != historical compatibility/evidence
```

## Review boundary

External review should confirm:

- no active Lua runtime/dependency/configuration/API remains;
- no first-party Babashka product/client remains;
- generic native component discovery, properties, current/history/subscriptions and
  Recorder provenance remain intact;
- removed operations are deterministically `unsupported_operation`;
- process/reconnect/client-isolation acceptance is implementation-neutral;
- M8 archives and hardware evidence are unchanged;
- M10 has not started.
