# Milestone 10 implementation report

## Status

```text
M10: AUTHORIZED
M10.1: COMPLETE
M10.2: COMPLETE
M10.3: COMPLETE
M10.4: NOT STARTED
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

## M10.3 — Application API physical organization

M10.3 started from a 2,632-line `application.rs` containing connection/session
ownership, every query domain, all mutation DTO decoding, Runtime/Service dispatch,
Recorder/history completion, subscriptions and most public projections. The
centralized registry in `protocol.rs` was already the correct source of public
metadata and remains authoritative.

### Final module map

```text
protocol.rs                         authoritative 42-operation metadata
wire.rs                             bounded NDJSON decode and shape validation
application.rs                      Application owner/facade and lifecycle routing
application/common.rs               shared parsing, responses and error mapping
application/operations.rs           typed mutation router and operation facts
application/delivery.rs             frozen projections and subscriptions
application/discovery.rs            discovery/describe/component projections
application/references.rs           Reference query/configure/retune
application/controllers.rs          controller query/config/lifecycle and output view
application/virtuals.rs             emulator publication and virtual-model restart
application/recording.rs            async Recorder/history operation delivery
application/projections.rs          shared controller/Reference/output event DTOs
measurements.rs                     current/recent measurement handlers and DTOs
recorder_api.rs                     Recorder DTO validation/status/history projection
configuration_api.rs                resource/configuration handlers and projections
events.rs                           bounded semantic fact/replay owner
sessions.rs                         admission, deduplication and retained outcomes
```

`application.rs` is now 546 lines and retains the one `Application` struct, bounded
connection/session/delivery fields, hello, detach/expiry, shutdown completion,
request routing and common mutation lifecycle. It no longer implements domain
projections, domain DTO parsing or Core command construction.

The domain modules do not own Runtime state, sessions or event queues. Typed domain
results return to the common Application lifecycle, which records terminal state
before delivery and maps errors through the unchanged public taxonomy.

### Dependency direction

```text
protocol registry + wire validation
               ↓
Application session/lifecycle facade
               ↓
focused semantic handler
               ↓
ServiceHost / HostCore / authoritative Runtime
```

The operation registry is not duplicated into per-module metadata lists. The small
routers select handlers by an already validated operation name or typed `Mutation`.
Event ownership stays in `events.rs`; delivery retains only a cursor/filter/token.
Recent Runtime measurement history remains separate from durable Recorder history.
Recorder API projection remains separate from its worker and SQLite implementation.

### Representative request traces

```text
measurements_current
→ protocol::OPERATIONS
→ Application::handle / handle_query_routed
→ measurements::current_records
→ Runtime Query::Discover + Query::GetLatestSignal
→ measurements::current_json
→ delivery::begin_projection

controller_start
→ protocol::OPERATIONS
→ operations::decode
→ controllers::decode
→ SessionStore admission
→ operations::dispatch
→ controllers::dispatch
→ Runtime Command::StartController
→ common terminal projection

recording_start
→ protocol::OPERATIONS
→ recorder_api::decode_mutation
→ SessionStore admission
→ recording::handle_recorder_mutation
→ HostCore Recorder lifecycle
→ recording::poll_recording terminal delivery

emulator_publish
→ protocol::OPERATIONS
→ virtuals::decode
→ SessionStore admission
→ virtuals::dispatch
→ HostCore::publish_emulator_measurement
→ authoritative Runtime signal commit
```

### Contract freeze

The registry regression now freezes all 42 operation names, query/mutation classes,
composition availability, argument allowlists and exact capability-to-witness
mapping. Protocol identity/version, wire DTOs, error taxonomy/messages, bounds,
session/deduplication, page/cursor/subscription behavior and reconnect semantics are
unchanged. No Core Runtime, scheduler, transport, OutputAuthority, Metakon, Recorder
schema/SQL or configuration-deployment behavior was modified.

Focused API, discovery, measurement, subscription, Reference/controller, Recorder,
configuration/resource, emulator and M9B.8 fault suites passed during extraction.
The process-level loopback acceptance and M9D physical-output software suite also
passed. Final gates:

```text
cargo fmt --all -- --check                              PASS
cargo test --workspace                                  PASS
cargo test --workspace --release                        PASS
cargo clippy --workspace --all-targets -- -D warnings   PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
                                                         PASS
exact registry/capability/argument contract              PASS
M9B.8 fault acceptance                                   PASS
process-level API acceptance                             PASS
M9D physical-output software suites                      PASS
git diff --check                                         PASS
```

COM5 was not opened and no hardware test was performed. Accepted SQLite evidence,
instrument definitions, Cargo/configuration files and SQLite schema were not
modified.

### Deferred work

M10.4 Host/Service/Runtime organization has not started. Recorder storage internals,
native component registration, systematic visibility/test organization and all
HIGH-risk ownership, scheduling, OutputAuthority, transport, protocol and schema
changes remain deferred.
