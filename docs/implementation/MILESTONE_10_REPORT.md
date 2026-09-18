# Milestone 10 implementation report

## Status

```text
M10: AUTHORIZED
M10.1: COMPLETE
M10.2: COMPLETE
M10.3: NOT STARTED
M11+: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

M10.2 implementation baseline:

```text
77f14b6ad256fb626cd3aa3d041d7ba5a2d81976
```

## M10.2 — terminology, archaeology and architecture indexes

M10.2 is a behavior-preserving navigation/readability slice derived from the M10.1
audit. It changes no Application operation, wire DTO, protocol result, scheduler
order, controller algorithm, OutputAuthority transition, Metakon frame, Recorder
schema or SQLite query. It does not start the M10.3 Application-module split.

### Internal terminology changes

The following internal names now describe their active meaning:

| Previous name | Current name | Reason |
|---|---|---|
| `FrozenSnapshot` | `FrozenProjection` | It is a connection-local paged API projection, not the removed M9C aggregate snapshot API |
| `Application::snapshots` | `Application::projections` | Distinguishes delivery state from authoritative Runtime snapshots |
| `expire_snapshots_at` | `expire_projections_at` | Describes the delivery objects actually expired |
| `EventLog::snapshot_records` | `EventLog::projection_records` | The records seed discovery projection pages, not a public snapshot operation |
| `FrozenDeployment::runtime_overrides` | `property_overlays` | The retained entries are specifically bounded Application property overlays |
| `runtime_override_count` | `property_overlay_count` | Matches the internal stored concept; the accepted wire field remains `runtime_overrides` |
| `HostCore::model_generations` | `virtual_model_generations` | Separates native virtual thermal models from generic instruments and managed components |
| `restart_configured_models` | `restart_configured_virtual_models` | Avoids collision with the removed broad `restart_models` operation |
| `safe_needed` | `safe_obligation_pending` | States that safe state remains an obligation, not permission to send |
| `safe_resend_blocked` | `ambiguous_safe_resend_blocked` | States why another safe WRITE is prohibited |

The accepted public operation names, response keys and errors are unchanged. In
particular, `virtual_models_restart`, `runtime_overrides`, `snapshot_expired` and
`snapshot_capacity` retain their accepted wire spelling.

The property-overlay maximum and staged-candidate retention now have named owners:
`configuration::MAX_PROPERTY_OVERLAYS` and
`deployment::CANDIDATE_LIFETIME`. Protocol limit projection reads those constants;
the advertised numeric values remain 32 and 30 seconds.

### Archaeological classification

The active tree was searched for Lua, Babashka, source/script configuration,
source-reload, old model-restart and removed snapshot-family vocabulary.

| Finding | Classification | M10.2 result |
|---|---|---|
| Recorder operation facts `reload_managed_sources` and `restart_models` | required historical archive compatibility | retained with an explicit compatibility comment |
| Recorder provenance kinds `managed_component_source` and `managed_lua_source` | required historical archive compatibility | retained with exact spelling/bounds so old content-addressed evidence remains readable |
| SQLite `object_snapshots` and `source_hash` | required historical schema compatibility | retained unchanged; schema and field names were not modernized |
| Lua/source provenance tests | required regression oracle | retained and labelled as historical archive compatibility, not active execution |
| removed-operation and removed-capability rejection tests | required regression oracle | retained; they continue to prove M9C surface removal |
| `ComponentKind::Source` and source-component tests | active generic behavior | retained; a neutral observation source is not a script |
| domain/controller/Recorder snapshot structs | active generic behavior | retained; they are ordinary committed state values, not the removed Application operation family |
| fake transport `Script`/`ScriptedTransport` test names | active test behavior | retained; “script” means deterministic I/O instructions, not a scripting runtime |
| active Lua/Babashka loaders, DTOs, configuration fields or dependencies | safe to delete, but absent | no code existed to delete |
| stale “source reload” and Lua-callback comments | safe to delete | replaced with active deployment/extension terminology |

No executable compatibility branch was deleted: every remaining branch either reads
accepted historical archives or guards a real removed-surface invariant. No
transitional source DTO, Lua runtime helper, Babashka fixture or unused snapshot
operation type remains in production code.

### Source architecture indexes

Concise rustdoc/indexes were added at the existing ownership seams:

- `lab-core` crate root: authoritative Runtime ownership and domain-module map;
- Core `runtime`: physical acquisition and physical output navigation;
- Core `output` and private authority: proposal/evidence stages, safe obligation
  versus resend permission and ambiguous-write non-retry;
- Core `transport`: resource conflict owner versus byte-moving adapter;
- Core `recording`: semantic fact contract versus Application and SQLite;
- Core `managed`: neutral invocation/result contract and active generic Source meaning;
- `lab-runtime` crate root: Runtime/Host/Service/Application/Recorder ownership map;
- `host`: exact bounded scheduler priority and acquisition/output entry points;
- `service`: process/deployment/reconnect/shutdown ownership;
- `application`: registry/decode/dispatch path and frozen projection meaning;
- `recorder` and its worker: facts, ingress, lifecycle, history, provenance,
  SQLite, gaps, sealing and time projection;
- `managed_executor`: the concrete `native.moving_mean.v1` path through
  `Invocation`, executor, `ComponentResult` and Runtime validation;
- `configuration` and `deployment`: persistent source, process-local overlays,
  staged candidate and serialized apply ownership.

The indexes deliberately point to existing types/modules instead of duplicating the
implementation or becoming M12 user documentation.

### Visibility and safety boundaries

No broad visibility pass was performed. The audit found no M10.2 deletion that made
an item mechanically private, so visibility is unchanged apart from exposing two
crate-only bound constants to sibling modules.

`OutputAuthority`, output intents, authorization callbacks and transport events
remain crate-private. The Windows COM module remains crate-private. No raw write,
register write or authority mutation was added to the Application, managed-component
or ordinary instrument surface.

### Behavior freeze

Focused regressions passed for:

- the exact 42-operation/capability registry inventory;
- removed M9C operation rejection;
- M9B.8 reconnect, replay, pressure and client-isolation fault acceptance;
- `native.moving_mean.v1` and the neutral managed-component contract;
- M9D physical WRITE/ACK/separate-readback, mismatch, timeout and ambiguous-safe
  non-retry semantics;
- configured physical output through the normal authority path;
- frozen projections, subscriptions and durable history;
- historical Recorder Lua/source provenance validation and native provenance.

Full gates:

```text
cargo fmt --all -- --check                              PASS
cargo test --workspace                                  PASS
cargo test --workspace --release                        PASS
cargo clippy --workspace --all-targets -- -D warnings   PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
                                                         PASS
git diff --check                                         PASS
```

COM5 was not opened and no hardware test was performed. Accepted SQLite evidence,
instrument definitions and deployment files were not modified.

### Deferred work

M10.3 and later audit stages remain separate work. This slice does not split
Application handlers, Runtime/Host/Service files, Recorder storage modules or native
component registration. It does not perform the systematic M10.7 visibility/test
organization pass. No HIGH-risk ownership, scheduler, OutputAuthority, time, schema,
protocol or plugin redesign was started.
