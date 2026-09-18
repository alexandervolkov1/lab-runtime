# Milestone 10 implementation report

## Status

```text
M10: AUTHORIZED
M10.1: COMPLETE
M10.2: COMPLETE
M10.3: COMPLETE
M10.4: COMPLETE
M10.5: COMPLETE
M10.6: NOT STARTED
M11+: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

M10.2 implementation baseline:

```text
77f14b6ad256fb626cd3aa3d041d7ba5a2d81976
```

M10.4 implementation baseline:

```text
99676df6bb8cc119e55143376debd3fb4a1a4da2
```

M10.5 implementation baseline:

```text
cbb8d562f7fb74705cc4711d51a8d993c151f0fc
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

At M10.3 completion, M10.4 Host/Service/Runtime organization had not started.
Recorder storage internals, native component registration, systematic
visibility/test organization and all HIGH-risk ownership, scheduling,
OutputAuthority, transport, protocol and schema changes remained deferred.

## M10.4 — Runtime / Host / Service orchestration organization

M10.4 reorganized implementation files only. The `Runtime`, `HostCore`,
`ServiceHost` and `SchedulePlan` structs retain the same fields and lifetimes. No
manager object, second state owner, queue, worker or new progression phase was
introduced. Tests required no semantic updates.

Before this slice, `runtime.rs` combined the command/query boundary, managed
components, native controller progression and all physical transaction correlation
in one 4,794-line file. `host.rs` combined composition, deployment application,
Recorder admission, managed orchestration, output/resource lifecycle and the native
scheduler in 3,937 lines. `service.rs` combined startup, configuration lifecycle,
reconnect and finite shutdown in 2,507 lines. The ownership boundaries were correct,
but navigation crossed long unrelated sections.

### Final Runtime module map

```text
crates/lab-core/src/runtime.rs
    public Command/Query/result types
    the one authoritative Runtime struct and all of its fields
    construction, semantic recording gate and resource registration
    shared private lookup helpers and Runtime-local regression tests

crates/lab-core/src/runtime/dispatch.rs
    synchronous Command mutation and Query projection routing

crates/lab-core/src/runtime/controllers.rs
    Reference/controller validation, lifecycle, PID progression and proposals

crates/lab-core/src/runtime/managed_components.rs
    component validation, invocation, bounded completion and authoritative commit

crates/lab-core/src/runtime/physical_io.rs
    Metakon transaction correlation, OutputAuthority/ResourceExecutor progression,
    ACK/readback settlement, acquisition commit and transport retirement
```

All child modules contain `impl Runtime` blocks. Their `pub(super)` seams are visible
only within the private `runtime` module tree and exist solely where the physical
split requires cross-responsibility calls. OutputAuthority and resource executors
remain private fields of the same Runtime.

### Final Host module map

```text
apps/lab-runtime/src/host.rs
    Clock/Periodic/SchedulePlan and HostCore field ownership
    trusted composition and profile construction

apps/lab-runtime/src/host/configuration.rs
    configured probes, resource rebind and validated deployment application

apps/lab-runtime/src/host/recording.rs
    Recorder lifecycle, semantic-fact admission, history and configuration fences

apps/lab-runtime/src/host/components.rs
    managed executor composition, native components and virtual/emulator projection

apps/lab-runtime/src/host/lifecycle.rs
    Runtime command/event facade, resource/output evidence and host shutdown barrier

apps/lab-runtime/src/host/scheduler.rs
    the one bounded safety-first owner turn
```

`HostCore` still owns exactly one `Runtime`, one schedule, one EventLog, Recorder
admission/lifecycle state and deployment-facing catalogs. The modules add no
stateful manager and do not move experiment authority out of Runtime.

### Final Service module map

```text
apps/lab-runtime/src/service.rs
    ServiceOptions and lifecycle DTOs
    the one ServiceHost struct
    startup/composition, process-facing facade and owner accessors

apps/lab-runtime/src/service/configuration.rs
    staged application and durable lifecycle-operation coordination

apps/lab-runtime/src/service/reconnect.rs
    explicit retirement/open/probe/rebind/generation-fenced release

apps/lab-runtime/src/service/shutdown.rs
    finite safety, transport and Recorder terminal progression
```

`ServiceHost` still owns the process clock/listener, one `HostCore`, deployment
lifecycle, reconnect candidate/diagnostic and finite shutdown clocks. In particular,
the M9D correction remains explicit in `service/shutdown.rs`: an ordinary
`TransportShutdown::Pending` is non-terminal and receives bounded subsequent owner
turns under the unchanged deadline.

### Preserved scheduler order

`host/scheduler.rs` now makes one turn readable without crossing configuration or
Recorder implementation sections. The order is unchanged:

```text
poll Recorder receipts
→ due safety slot
   → required-recording deadline and output watchdogs
   → controller fail-safe checks and pending safe dispatch
   → one bounded transport/recovery/completion turn
   → managed expiry and at most two completions
→ shutdown transport retirement, when safe
→ stop/configuration-quiesce early return
→ virtual thermal plants
→ configured Metakon read admission
→ References
→ controllers on a new distinct Good observation
→ periodic managed sources
→ observation-driven managed transforms
→ admit semantic recording facts and poll Recorder receipts
```

Every lower-priority phase retains the same early yield when safety is due. Periods,
next-due calculations, skipped-deadline accounting and actual clock reads are
unchanged.

### Main execution traces

Physical measurement:

```text
ServiceHost::startup
→ HostCore::configured_with_transports
→ host/scheduler.rs::HostCore::service_inner
→ Runtime::command(Command::QueueMetakonRead) in runtime/dispatch.rs
→ Metakon READ transaction on ResourceExecutor
→ runtime/physical_io.rs::poll_transports
→ handle_transport_event / decode_read / apply_metakon_value
→ Runtime SignalBuffer commit + semantic recording fact
→ Host EventLog/current/history/controller/Recorder observation
```

Physical controller output:

```text
host/scheduler.rs controller phase
→ Runtime::command(Command::TickController)
→ runtime/controllers.rs::tick_controller / calculate_controller_update
→ OutputProposal
→ runtime/physical_io.rs::deliver_physical
→ OutputAuthority admission
→ queue_metakon_output / ResourceExecutor
→ final authority and binding-generation recheck at first possible byte
→ Metakon WRITE → strict ACK
→ separate register readback
→ handle_output_readback / settle_controller_after_physical_readback
```

The physical-I/O module documents and preserves that requested, authorized,
send-started, ACK, readback and physical effect are distinct. An ambiguous started
write is not blindly retried.

Reconnect:

```text
service/reconnect.rs::reconnect_resource
→ recorded lifecycle reservation
→ HostCore configured-resource quiesce and old executor retirement
→ bounded ComTransport replacement/open under the original deadline
→ HostCore rebind and generation advance
→ resource-scoped compatibility probe
→ durable lifecycle completion
→ HostCore acquisition release
```

Shutdown:

```text
service/shutdown.rs::request_shutdown
→ host/lifecycle.rs::begin_shutdown
→ managed admission fence + controller pause/safe obligations
→ host/scheduler.rs safety/transport turns
→ ResourceExecutor/COM retirement, including later Pending observations
→ host/recording.rs Recorder stop/flush/seal
→ ServiceHost terminal status only after finite cleanup evidence or deadline
```

### Ownership and behavior freeze

Runtime remains the sole authoritative mutable experiment owner. HostCore retains
orchestration state only; ServiceHost retains process/deployment/reconnect/shutdown
state only; Application remains the accepted external projection/control boundary.
Recorder still owns durable machinery, not experiment authority.

The refactor did not modify the 42-operation registry, 25-capability composition,
public errors or bounds; no public DTO or protocol version changed. It did not modify
SQLite schema or Recorder semantics, configuration/instrument definitions, serial or
Metakon codecs, OutputAuthority transitions, transaction retry policy, scheduler
cadence/order, reconnect generations or hardware settings. The accepted M9D archive
and hash remain applicable; COM5 was not opened and no hardware test was performed.

Focused scheduler/acquisition, configured physical read/output, Core controller,
OutputAuthority, M9D physical-output, reconnect, configuration, shutdown, managed
component and Recorder-boundary suites passed during extraction. Final gates:

```text
cargo fmt --all -- --check                              PASS
cargo test --workspace                                  PASS
cargo test --workspace --release                        PASS
cargo clippy --workspace --all-targets -- -D warnings   PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
                                                         PASS
exact 42-operation / 25-capability registry regression   PASS
M9B.8 fault acceptance                                   PASS
M9D physical-output software suite                       PASS
reconnect/serial and shutdown suites                     PASS
native managed-component regressions                     PASS
Recorder integration regressions                         PASS
git diff --check                                         PASS
```

### Deferred work

M10.5 Recorder internal organization has not started. Native component/instrument
extension simplification, systematic visibility/rustdoc/test organization and all
HIGH-risk ownership, scheduler, OutputAuthority, transport, protocol and schema
changes remain deferred. M11 remains unauthorized.

## M10.5 — Recorder / SQLite physical organization

M10.5 reorganized the existing Recorder implementation without changing its state
owners, queues, SQL, schema, transactions or public projections. Before this slice,
`recorder.rs` mixed the semantic contract with open/recovery, schema declarations,
write transactions, history reads and provenance SQL in 3,533 lines, while
`recorder/worker.rs` mixed the host-side lifecycle, ingress accounting, history-job
mailboxes, fault controls and the storage loop in 2,336 lines.

### Final Recorder module tree

```text
crates/lab-core/src/recording.rs
    authoritative semantic RecordingFact outbox and Required-policy gate

apps/lab-runtime/src/recorder.rs
    semantic Recorder contract and public module index
apps/lab-runtime/src/recorder/history.rs
    durable-history filters, cursors, rows and bounded pages
apps/lab-runtime/src/recorder/provenance.rs
    semantic content/build provenance entries and object snapshots
apps/lab-runtime/src/recorder/time.rs
    monotonic-to-wall-clock recording anchors

apps/lab-runtime/src/recorder/worker.rs
    one RecorderWorker owner, finite channel protocol and worker construction
apps/lab-runtime/src/recorder/worker/lifecycle.rs
    activation, start/stop/finish, receipts, sealing state and failure propagation
apps/lab-runtime/src/recorder/worker/ingress.rs
    causal-group, operation and annotation admission/credit
apps/lab-runtime/src/recorder/worker/history.rs
    bounded history/run job admission, result mailboxes and cancellation
apps/lab-runtime/src/recorder/worker/storage_loop.rs
    exclusive SQLite command execution and durability receipts
apps/lab-runtime/src/recorder/worker/fault_injection.rs
    WriterBarrier and its canonical mutex/Condvar regression oracle
```

`RecorderWorker` remains the single host-side owner of lifecycle projections,
reserved record identities, ingress charge, history-job credit and the worker
channel. Only the existing storage thread owns `SqliteStore` and may block on SQL.
No stateful manager, backend trait, queue, capacity or thread was added.

### Final SQLite module tree

```text
apps/lab-runtime/src/recorder/sqlite/mod.rs
    SqliteStore state, settings, health and storage error boundary
apps/lab-runtime/src/recorder/sqlite/open.rs
    open, compatibility validation, recovery and storage budget checks
apps/lab-runtime/src/recorder/sqlite/schema.rs
    the unchanged version-one DDL and schema identity checks
apps/lab-runtime/src/recorder/sqlite/write.rs
    run/interval lifecycle, fact transactions, gaps, checkpoints and seals
apps/lab-runtime/src/recorder/sqlite/history.rs
    bounded indexed run and measurement history reads
apps/lab-runtime/src/recorder/sqlite/provenance.rs
    immutable activation/content/object-snapshot transactions
apps/lab-runtime/src/recorder/sqlite/encoding.rs
    checked SQLite identity, scalar and JSON encoding/decoding helpers
```

The split uses the existing `SqliteStore` directly; it does not introduce an ORM,
repository abstraction, generic persistence trait or alternate backend. Table and
column spellings, PRAGMAs, SQL statements, transaction boundaries and the schema
version are unchanged.

### Frozen lifecycle and boundedness

An absent Host Recorder remains `unconfigured`. An opened worker starts `Idle`;
`request_start*` moves the owner projection to `Starting` until the durable start,
boundary and provenance receipt confirms `Recording`. Ordinary admission is then
bounded by the unchanged four causal groups, 1,024 records and 4 MiB accounted
bytes, with the same per-group limit and reserved lifecycle slots. Capacity is
reserved before try-send and released only by a durability receipt.

`request_stop*` closes ordinary admission by entering `Stopping`, queues the stop
behind the accepted FIFO prefix, and reaches `Idle` only after the interval and run
are sealed and that transaction is committed. A successful stop still does not
destroy the writer or claim a terminal boot seal. `request_finish*` separately
commits the boot seal and closes the SQLite connection before `Closed` becomes
visible. `Failed` remains sticky and Required recording continues to fail closed.

Boot, run, interval and record identities retain their checked meanings. The owner
continues to expose only the committed receipted prefix; a post-commit process loss
can still be recovered from SQLite without fabricating an owner receipt. Gaps retain
their first-known-loss identity, reason and coverage meaning, and neither an
unsealed tail nor a failed close is reported as complete.

### Representative paths

Measurement recording:

```text
Runtime semantic RecordingFact (lab-core/recording.rs)
→ host/recording.rs admission boundary
→ recorder/worker/ingress.rs finite causal-group reservation
→ recorder/worker/storage_loop.rs exclusive worker dispatch
→ recorder/sqlite/write.rs one SQLite fact transaction + checkpoint
→ lifecycle receipt releases the owner-side charge
```

Recorder start and stop:

```text
Application recorder handler (recorder_api.rs / application/recorder.rs)
→ host/recording.rs orchestration
→ recorder/worker/lifecycle.rs start or stop barrier
→ recorder/worker/storage_loop.rs FIFO execution
→ recorder/sqlite/write.rs durable boundary/seal transaction
→ lifecycle receipt and unchanged public RecordingStatus projection
```

Durable history:

```text
Application history_read
→ bounded operation/session job
→ host/recording.rs
→ recorder/worker/history.rs eight-slot admission/cancellation
→ recorder/worker/storage_loop.rs
→ recorder/sqlite/history.rs indexed frozen-checkpoint page
→ bounded raw page and existing Application projection/cursor
```

Provenance:

```text
frozen activation/configuration identity
→ recorder/provenance.rs semantic entries and object snapshots
→ recorder/worker/lifecycle.rs activation command
→ recorder/worker/storage_loop.rs
→ recorder/sqlite/provenance.rs content-addressed transaction
```

The public `recorder_api.rs` facade and M10.4 `host/recording.rs` orchestration were
not turned into persistence modules: neither contains table knowledge or SQL.
Recent Runtime history remains distinct from durable Recorder history.

### Historical compatibility and test isolation

The active native provenance kind `managed_component_source` and historical
`managed_lua_source` reader acceptance remain together in
`recorder/sqlite/provenance.rs`. Historical operation facts, including removed
source-reload/model-restart vocabulary already present in archives, remain opaque
durable facts rather than active API operations. No archive, migration or evidence
file was rewritten.

`WriterBarrier` moved intact to `worker/fault_injection.rs`. Its `held` and `reached`
predicate remains protected by one mutex and Condvar; predicate mutation occurs
under that mutex and waiting uses the canonical predicate loop. The storage worker
is the only participant that may wait there; Runtime ownership and production
scheduling do not depend on this fault seam. The boundary lost-wake oracle passed
repeated execution after the move.

### Behavior freeze and verification

The 42-operation registry, 25 capabilities, public error/bounds/status DTOs,
history cursors and response shapes are unchanged. Schema DDL, provenance hashes,
WAL/SHM behavior, durability confirmation, causal-group ingress, Required policy,
run/interval identity and clean-close semantics are unchanged. M9D physical output,
Metakon and hardware configuration code was not modified.

Every `recorder*.rs` integration binary and `com_recorder_shutdown` passed after the
move, including startup, start/stop, Required policy, history/cursors, provenance,
process reopen, backpressure, held-writer native progress, SQLite integrity,
shutdown and WriterBarrier tests. Final gates:

```text
cargo fmt --all -- --check                              PASS
cargo test --workspace                                  PASS
cargo test --workspace --release                        PASS
cargo clippy --workspace --all-targets -- -D warnings   PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
                                                         PASS
exact 42-operation / 25-capability registry regression   PASS
M9B.8 fault acceptance                                   PASS
M9D physical-output software suite                       PASS
all Recorder integration binaries                        PASS
WriterBarrier lost-wake regression                       PASS
historical provenance compatibility                      PASS
git diff --check                                         PASS
```

COM5 was not opened and no hardware test was performed. Accepted M8, post-M9C and
M9D evidence remains unchanged.

### Deferred work

M10.6 native component/instrument extension clarity has not started. Systematic
visibility/rustdoc/test organization and every HIGH-risk change to ownership,
scheduling, Recorder policy, schema, history protocol, OutputAuthority, transport
or public API remain deferred. M11 remains unauthorized.
