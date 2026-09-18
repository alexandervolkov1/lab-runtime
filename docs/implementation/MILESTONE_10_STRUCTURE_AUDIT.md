# Milestone 10.1 — structural and studyability audit

## Status and authority

```text
M10: AUTHORIZED
M10.1: COMPLETE
M10 implementation: NOT STARTED
M11+: NOT AUTHORIZED
```

Audit date: 2026-09-18 (Europe/Moscow).

Accepted baseline:

```text
ae1c72d0f9adfe90677ff81ca8cadf8fc37bc201
```

Accepted M9D evidence remains:

```text
examples/metakon-513-m9d-write-smoke.sqlite
SHA-256 14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32

0 → ACK → readback 0
+10 → ACK → readback 10
0 → ACK → readback 0
clean shutdown
```

The load/heater was physically disconnected, so the evidence intentionally makes no
physical-heater-effect claim.

This is a design audit of the actual post-M9D tree. Historical milestone reports
were used as context; the current source is authoritative. No production code,
tests, Cargo/configuration files, instrument definitions or SQLite evidence were
changed. COM5 was not opened and no hardware test was performed.

The central finding is that the architecture is sounder than its physical layout.
The accepted ownership and safety boundaries are present, but the main study paths
cross a few very large files whose `impl` blocks combine composition, scheduling,
protocol projection, persistence and lifecycle concerns. M10 should expose those
existing boundaries without moving authoritative state.

The non-negotiable invariant remains:

```text
Runtime = sole authoritative mutable experiment owner
```

## 1. Current repository map

The workspace has two crates. `lab-core` is an OS-, storage-, wire- and
presentation-independent synchronous domain crate. `lab-runtime` composes Core with
deployment files, Windows COM, SQLite, the local NDJSON Application API and process
lifecycle.

### `crates/lab-core`

| Module | Category | Responsibility and major owner | Main callers/dependencies | Coherence |
|---|---|---|---|---|
| `runtime.rs` | domain/safety and orchestration | `Runtime`, the sole mutable experiment owner; commands, queries, instruments, controllers, output authorities, resource executors, managed-component commit and Recorder fact outbox | `HostCore`; all other Core domain modules | Architecturally correct owner, physically over-mixed |
| `output.rs`, `output/authority.rs` | domain/safety | Output value types and private `OutputAuthority`; leases, epochs, proposal admission, safe obligation and evidence states | `Runtime`; `transport` intent correlation | Cohesive and deliberately encapsulated |
| `transport.rs` | transport | `ByteTransport` and one `ResourceExecutor` per conflicting byte resource; bounded transaction queues, recovery and shutdown | `Runtime`; implemented by `ComTransport` | Cohesive; output hooks correctly crate-private |
| `metakon.rs` | instrument/protocol adapter | Metakon address/value types and pure READ/WRITE/ACK codecs | `Runtime`; codec tests | Cohesive |
| `instrument.rs` | instrument/domain | Validated data-instrument definitions and Metakon runtime binding state | `Runtime`, host definition loader | Cohesive |
| `virtual_instrument.rs`, `plant.rs` | domain/instrument | Deterministic virtual source and stateful thermal reference model | `Runtime` | Cohesive |
| `control.rs`, `processing.rs`, `reference.rs` | domain/control | Controller state, PID/EMA and fixed/ramp reference semantics | `Runtime` | Cohesive |
| `managed.rs` | domain/extension contract | Language-neutral `Invocation`, `ComponentResult`, `ComponentCompletion`, `ComponentExecutor`, plain data and component state | `Runtime`, native executor | Cohesive |
| `recording.rs` | Recorder semantic boundary | Bounded semantic `RecordingFact` outbox and required-recording gate | `Runtime`, `HostCore` | Cohesive and storage-independent |
| `model.rs`, `signal.rs` | domain primitives | Identifiers, descriptors, values, units, samples and bounded signal history | all Core modules; re-exported by `lib.rs` | Cohesive |

`lab-core/Cargo.toml` has no dependencies. It does not depend on SQLite,
`serialport`, Windows, NDJSON, deployment files, scripting or presentation code.

### `apps/lab-runtime`

| Module | Category | Responsibility and major owner | Main callers/dependencies | Coherence |
|---|---|---|---|---|
| `host.rs` | orchestration | `HostCore`: owns `Runtime`, `SchedulePlan`, event observation, Recorder ingress/lifecycle and configured catalogs | `ServiceHost`, Application projections/tests | Correct owner boundary, too many physical responsibilities |
| `service.rs` | orchestration/configuration/deployment | `ServiceHost`: process clock/listener, startup composition, configuration lifecycle, reconnect and finite shutdown | server/main/Application | Mixed; startup, reconnect and shutdown obscure the service facade |
| `application.rs` | Application API | `Application`: sessions, frozen delivery projections, subscriptions, asynchronous recording/history/shutdown work, query/mutation dispatch and projections | server; protocol/wire; `ServiceHost` | Should split by semantic domain |
| `protocol.rs` | Application API | Accepted 42-operation registry, capabilities/limits and stable 12-category public error mapping | wire/Application/tests | Good |
| `wire.rs` | Application API transport | Bounded NDJSON framing, top-level request decode and nested-argument shape validation | server/Application | Acceptable; semantic DTO parsing is split with `application.rs` |
| `sessions.rs` | Application API | finite client-independent scopes, deduplication and retained operation results | `Application` | Good |
| `events.rs` | Application API/events | owner-local current facts and bounded 1,024-event replay ring | `HostCore`, `Application` | Acceptable; projection imports point back into `application.rs` |
| `server.rs` | transport/API delivery | bounded nonblocking loopback TCP reactor and per-client queues | main; Application | Cohesive |
| `recorder.rs` | Recorder/persistence | semantic Recorder DTOs plus `SqliteStore`, schema, writes, history and provenance | worker, host, Application history | Should split |
| `recorder/worker.rs` | Recorder/persistence | bounded ingress, disk worker, receipts, history jobs, lifecycle and fault injection | `HostCore`, Application history/tests | Mostly cohesive but test seams and several protocols are mixed |
| `recorder/time.rs` | Recorder/time | monotonic-to-wall-clock `TimeAnchor` | Recorder worker/store | Good |
| `recorder_api.rs` | Application projection | stable Recorder status projection | Application | Good |
| `configuration.rs` | configuration/deployment | parse, freeze, validate, cross-reference, property overlay and deployment DTOs | startup, deployment lifecycle, configuration API | Large but substantially cohesive |
| `deployment.rs` | configuration/deployment | staged candidate, diff effects and safe apply contract | `ServiceHost`, `HostCore` apply port | Good |
| `configuration_api.rs` | Application projection | resource and property metadata/projections | Application/ServiceHost | Acceptable |
| `definition.rs` | configuration/instrument adapter | bounded JSON instrument-definition parsing into Core definitions | host composition | Good |
| `serial.rs` | transport | crate-private nonblocking `ComTransport`; one bounded worker owns blocking serial device/OS handle | ServiceHost startup/reconnect; `ResourceExecutor` | Cohesive, although test code is partly embedded |
| `managed_executor.rs` | extension execution | fixed two-worker `ManagedExecutor`, native implementation registry, moving-mean metadata/validation/implementation | `HostCore`, `Runtime` | Executor is coherent; registry and implementation should be separated |
| `measurements.rs` | Application projection | stable signal/current/history JSON helpers | Application/events | Good |
| `build_identity.rs` | provenance | cached native binary identity | host provenance | Good |
| `main.rs` | process entry | CLI/startup/server handoff | ServiceHost/server | Good |

The desired dependency direction is intact:

```text
lab-core
   ↑
lab-runtime orchestration/adapters
```

There is no questionable reverse dependency. The main issue is organization within
`lab-runtime`, not inversion between crates.

## 2. Largest production modules

LOC below is physical line count and intentionally only a navigation signal.

| File | Approx. LOC | Responsibilities | Assessment and useful boundary | Split risk |
|---|---:|---|---|---|
| `crates/lab-core/src/runtime.rs` | 4,769 | complete command/query surface, all instrument families, managed lifecycle, controller path, output/transport coordination and recording facts | One state owner is justified; one file is not. Keep `Runtime` and its fields together, split private `impl` sections into managed, control, physical I/O, recording and query/command routing | MEDIUM |
| `apps/lab-runtime/src/host.rs` | 3,924 | deployment composition, schedules, event observation, configuration apply, emulator/model actions, Recorder admission/lifecycle, service turn | Unrelated orchestration concerns are mixed. Split `impl HostCore` by composition, scheduler, Recorder, configuration and projections without creating another owner | MEDIUM |
| `apps/lab-runtime/src/recorder.rs` | 3,514 | semantic records/history/provenance plus SQLite store, schema and all SQL | Size is not justified by cohesion. Separate semantic model, SQLite store/schema, writes and history queries | MEDIUM |
| `apps/lab-runtime/src/application.rs` | 2,619 | state, all query handlers, all mutation decode/dispatch, history/subscriptions and JSON projections | The accepted API is semantically grouped but not physically grouped. Split by query domain, mutation domain, delivery state and projections | MEDIUM |
| `apps/lab-runtime/src/service.rs` | 2,503 | startup, listener/service facade, configuration stage/apply, reconnect state machine and shutdown | Reconnect and startup dominate navigation. Split private `impl` files while retaining one `ServiceHost` | MEDIUM |
| `apps/lab-runtime/src/recorder/worker.rs` | 2,331 | ingress accounting, lifecycle messages, receipts, history jobs, writer loop and `WriterBarrier` | Core worker protocol is related; fault injection and history-job plumbing deserve focused modules | MEDIUM |
| `apps/lab-runtime/src/configuration.rs` | 1,540 | DTOs, parsing/freezing, validation, diff and property overlays | Large but reasonably cohesive. Prefer named sections or private submodules only after higher-value splits | LOW/MEDIUM |
| `apps/lab-runtime/src/serial.rs` | 1,299 | settings, adapter state, worker protocol, Windows device and inline tests | Production part is coherent. Move tests/test device support; keep adapter/worker contract adjacent | LOW |
| `apps/lab-runtime/src/protocol.rs` | 934 | registry, capabilities, limits, public error taxonomy and tests | Cohesive; do not split merely for LOC | LOW |
| `crates/lab-core/src/output/authority.rs` | 882 | safety-critical authority state machine and tests | Cohesive. Improve state-transition documentation; do not fragment the machine | LOW for docs, HIGH for behavioral edits |
| `crates/lab-core/src/transport.rs` | 821 | byte port, executor state machine, transactions and tests | Cohesive; preserve adjacency of queue/recovery/dispatch | LOW |
| `apps/lab-runtime/src/server.rs` | 784 | loopback reactor and bounded delivery | Cohesive | LOW |
| `apps/lab-runtime/src/sessions.rs` | 627 | session/dedup state and tests | Cohesive | LOW |
| `crates/lab-core/src/recording.rs` | 614 | semantic facts and required gate | Cohesive | LOW |

No M10 criterion should impose a maximum file length. The proposed splits follow
responsibility and navigation seams, not a numeric threshold.

## 3. Periodic physical acquisition trace

The current Metakon measurement path is:

1. `configuration::load_runtime_toml` / `parse_runtime_toml` produces a validated
   `FrozenDeployment`; `InstrumentDto::Metakon` refers to an artifact parsed by
   `definition::parse_definition` into Core `DataInstrumentDefinition`.
2. `ServiceHost::startup` converts resource configuration to `ComSettings` and
   creates crate-private `ComTransport` adapters. The blocking worker, not the owner
   lane, owns the Windows COM handle.
3. `HostCore::configured_with_transports` registers each `ByteTransport` in
   `Runtime`, registers the Metakon instrument/binding and constructs a
   `MetakonReadSchedule` in `SchedulePlan`.
4. `HostCore::service_inner` tests the periodic slot and sends
   `Command::QueueMetakonRead { instrument, parameter, at, queue_ttl, timeout }`.
5. `Runtime::command` resolves the binding/operation, calls
   `metakon::encode_read`, and calls `ResourceExecutor::enqueue_read`.
6. The higher-priority safety cadence sends `Command::PollComponents`.
   `Runtime::poll_components` first calls `service_safety`, which calls
   `poll_transports`; each executor receives one bounded progress opportunity.
7. `ResourceExecutor` drives `ByteTransport::try_write`/`try_read`. `ComTransport`
   exchanges bounded messages with its sole serial worker and understands bytes,
   not Metakon semantics.
8. A `TransportEvent::ReadTerminal` returns to
   `Runtime::handle_transport_event`. `metakon::decode_read` validates the exact
   expected response; `Runtime::apply_metakon_value` scales and validates it and
   commits a `Sample` into the instrument's `SignalBuffer`.
9. `Runtime::record_pending_sample_at` emits a semantic `RecordingFact` to the
   bounded Core outbox. Failed reads publish unavailable quality rather than a
   fabricated physical observation.
10. `Query::GetLatestSignal` and `Query::GetSignalWindow` expose committed current
    and transient history; `EventLog::observe` creates bounded live events.
11. On its own periodic slot, `HostCore::service_inner` reads the latest distinct
    Good input and sends `Command::TickController`.
12. `HostCore::admit_recording_facts` transfers facts to the nonblocking
    `RecorderWorker`; `SqliteStore` writes them on the disk worker.

A new reader currently needs roughly eight production files to reconstruct one
measurement: `configuration.rs`, `definition.rs`, `service.rs`, `host.rs`,
`runtime.rs`, `transport.rs`, `serial.rs` and `metakon.rs`, then `events.rs` or the
Recorder modules to follow publication. The number of legitimate layers is not the
problem; the path lacks one architectural index and its coordinating steps are
buried inside two giant `impl` blocks.

M10 should add a module-level acquisition walkthrough and place composition,
scheduling and physical read handling in named private modules. It must not collapse
codec, transport and Runtime commit into one adapter or move sample authority out of
`Runtime`.

## 4. Physical output and safety trace

The accepted M9D path is implemented as follows:

1. `HostCore::service_inner` selects a new Good measurement and invokes
   `Command::TickController`.
2. `Runtime::tick_controller` and `calculate_controller_update` advance the native
   EMA/PID and create an `OutputProposal` carrying the finite lease and proposal
   expiry.
3. `Runtime::deliver_physical` records the requested attempt, asks the private
   `OutputAuthority` to admit the proposal and respects the required-recording gate.
4. `Runtime::queue_metakon_output` calls
   `OutputAuthority::reserve_transport`, freezing actuator instance, lease, epoch,
   value, expiry, binding generation and mapping revision. It encodes a signed-I8
   Metakon WRITE to register 6 and enqueues it in the bounded `ResourceExecutor`.
   Safe-zero uses the executor's reserved safe slot and the same path.
5. In `Runtime::poll_transports`, `ResourceExecutor::poll_authorized` asks for
   `AuthorizationStep::Validate` at byte offset zero. Runtime rechecks current
   resource binding/generation/revision and calls
   `OutputAuthority::validate_transport` immediately before the adapter can accept
   the first possible byte.
6. Only after `ByteTransport::try_write` accepts a positive byte count does the
   executor report `AuthorizationStep::Started`. Runtime then calls
   `OutputAuthority::begin_transport` and records authorized/send-started evidence.
7. `Runtime::handle_transport_event` accepts success only when
   `metakon::decode_ack` validates the strict five-byte ACK. It calls
   `acknowledge_transport`, then creates a separate, non-retryable Metakon READ of
   register 6 and stores `PendingOutputReadback`.
8. `Runtime::handle_output_readback` decodes signed I8, scales it and calls
   `complete_transport_readback`. Only an exact match is `ReadbackVerified`; a
   mismatch is failed and an unavailable readback is ambiguous. Native lease renewal
   occurs only after matching readback.
9. Pause, failure, expiry or authority loss reaches `request_safe` and
   `service_safety`; physical safe-zero is sent through the same authority-gated
   transport and ACK/readback sequence.

The requested, reserved/authorized, send-started, acknowledged, readback and
physical-effect concepts are represented distinctly. No code claims physical effect.
The executor never retries output transactions after send-start; timeout is not
treated as proof that nothing was written.

Safety ownership is clearest in `output/authority.rs` and at the crate-private
`AuthorizationStep` callback. It is less discoverable because transaction
coordination spans the last 900 lines of `runtime.rs`. M10 should create a focused
private physical-I/O `impl Runtime` module and add a state/evidence table linking the
methods above. Protocol codec and byte movement are already sufficiently separate.

## 5. Native managed-component trace

For `native.moving_mean.v1` the current path is:

```text
configuration::ManagedComponentDto
→ HostCore::stage_configured_component / ComponentDefinition
→ managed_executor registry
→ Runtime::InvokeComponent / Invocation
→ ManagedExecutor implementing ComponentExecutor
→ ComponentCompletion(ComponentResult)
→ Runtime::poll_components validation
→ Runtime managed SignalBuffer commit
→ generic Query/EventLog/Application/Recorder provenance paths
```

`managed.rs` defines the neutral contract. `managed_executor.rs` currently contains
the implementation identifier, property metadata, registry matches, configuration
validation, worker pool and moving-mean algorithm. `configuration.rs` repeats
implementation-specific validation and `configured_window`; `host.rs` repeats
implementation-specific manifest construction and warmup selection.

Today an ordinary native component requires edits to:

- `apps/lab-runtime/src/managed_executor.rs` for identity, validation, metadata,
  registration and execution;
- `apps/lab-runtime/src/configuration.rs` for implementation allowlisting and
  implementation-specific TOML validation;
- `apps/lab-runtime/src/host.rs` for manifest/kind construction;
- focused Core/Runtime, configuration, provenance and API tests as applicable.

Generic Application or Recorder dispatch does not require a new branch once the
component produces a normal descriptor/signal/fact. That is already a good property.
The accidental ceremony is the same implementation identifier and validation logic
matched in three files.

M10 should introduce a compile-time native registration descriptor that owns
identity, metadata, configuration-to-definition validation and runner selection,
then keep the bounded worker pool generic. The intended touch set becomes
implementation, registration/metadata and tests. This is static Rust registration,
not a dynamic plugin framework.

## 6. Adding another physical instrument

An ordinary new Rust physical instrument currently touches:

- identity/model: a new `InstrumentDto` variant, Core instrument state and Runtime
  registration/query branches;
- protocol: a pure codec beside `metakon.rs`;
- transport: normally no new transport if it uses `ByteTransport`; a genuinely new
  medium needs its own adapter implementing that trait;
- scheduling: a new schedule collection and `service_inner` branch in `host.rs`;
- signals: descriptor validation, `SignalBuffer` ownership and Runtime commit;
- output, if any: explicit `OutputAuthority` binding, final-check intent correlation,
  protocol ACK/readback semantics and safe profile;
- configuration: DTO, validation, artifact/config composition and property metadata;
- generic discovery/API: descriptor and generation helpers should suffice, but today
  `HostCore::instrument_kind`, `signal_generation` and configuration projections
  contain family-specific matches;
- tests: codec, scripted transport, configuration, scheduler, API discovery and
  safety/fault tests.

Codec, configuration and scheduler work is essential. Parallel Runtime maps for
virtual, thermal and Metakon instruments, parallel `SchedulePlan` vectors and several
family matches in host projection are accidental coupling. Recorder facts and the
accepted Application operations are already generic and should not be edited merely
to expose standard signals/properties.

M10 should first centralize instrument-family composition/projection helpers and
document an instrument adapter checklist. Replacing Runtime's maps with trait objects
or moving instrument ownership behind another authority would be a HIGH-risk
redesign and is deferred.

## 7. Representative Application request paths

Every request follows:

```text
server NDJSON frame
→ wire::decode_frame / WireRequest
→ protocol::operation registry check
→ Application::handle
→ query handler or typed_mutation
→ dispatch / explicit asynchronous handler
→ ServiceHost or HostCore
→ Runtime/domain or Recorder/deployment owner
→ projection
→ server response and EventLog/subscription delivery
```

Representative details:

| Operation | Semantic path after decode |
|---|---|
| `measurements_current` | `Application::handle_query` → `Query::Discover` → per-signal `Query::GetLatestSignal` → `measurements::current_json` → bounded frozen projection/page |
| `reference_configure` | `typed_mutation` → session admission → `dispatch` → existing reference query for preserved unit → `Command::ReconfigureReference` through `HostCore::command_with_cause` → Runtime → `reference_json` and event observation |
| `controller_start` | `typed_mutation` → `Mutation::Start` → `dispatch` → `Command::StartController` → Runtime validation/authority lifecycle → controller projection/event |
| `recording_start` | `typed_mutation` → session admission → special `Application::handle_mutation` branch → `HostCore::start_recording` → bounded `RecorderWorker` request → pending completion polling → terminal operation/result |
| `property_configure` | typed value/target decode → `ServiceHost::configure_property` → clone active `FrozenDeployment` with property overlay → `DeploymentLifecycle` stage/apply → `HostCore` safe/live apply port → property projection |
| `emulator_publish` | `typed_mutation` → `HostCore::publish_emulator_measurement` → `Command::PublishVirtualMeasurement` → Runtime declaration/generation checks → normal signal/event/fact paths |
| `reconnect_resource` | `typed_mutation` → `ServiceHost::reconnect_resource` → Recorder lifecycle reservation → Runtime executor retirement → bounded replacement `ComTransport` → generation-fenced rebind/probe → lifecycle result/event |

The semantic pipeline is correct, but `application.rs` makes operations hard to find:
`handle_query`, `handle_mutation`, `typed_mutation`, `dispatch` and projection helpers
are four distant navigation points for one operation.

## 8. Application API physical organization

| Area | Classification | Finding |
|---|---|---|
| 42-operation registry | GOOD | One allowlisted `OPERATIONS` table is the contract index |
| capabilities and operation metadata | GOOD | Stable and centrally projected |
| protocol limits | ACCEPTABLE | Useful aggregate, but some values duplicate literals owned by other modules |
| wire DTO validation | ACCEPTABLE | Framing is coherent; nested validation and typed semantic decode are separated between `wire.rs` and `application.rs` |
| query handlers | SHOULD SPLIT | One large match mixes discovery, current data, control, configuration, history and Recorder queries |
| mutation handlers/dispatch | SHOULD SPLIT | Admission, async lifecycle special cases and Core command construction are interleaved |
| projections | SHOULD SPLIT | Domain projections live at the tail of `application.rs`; `events.rs` imports them back from the dispatcher module |
| error mapping | ACCEPTABLE | Stable taxonomy is preserved, but lifecycle/domain/public conversions are spread across Application, Service and protocol |
| sessions/deduplication | GOOD | `sessions.rs` is a coherent finite owner |
| history jobs/cursors | SHOULD SPLIT | Correctly bounded but state and polling dominate `Application` navigation |
| subscriptions/events | ACCEPTABLE | Separate bounded EventLog exists; delivery coordination belongs to Application |
| `FrozenSnapshot` naming | SHOULD RENAME | It is a paged immutable API projection, not the removed M9C snapshot API |
| obsolete public operations | DEAD/ABSENT | `runtime_snapshot`, page/release, source reload and `restart_models` are absent; negative regression tests are useful |

The split must preserve operation names, arguments/results, capability inventories,
bounds, event order, error codes, deduplication and response timing.

## 9. Runtime, HostCore and ServiceHost responsibilities

`Runtime` owns all authoritative experiment mutation: instrument samples, references,
controllers, output authorities, resource executors, component state and semantic
Recorder facts. Its synchronous API takes explicit monotonic time. It contains no OS
listener, filesystem deployment parsing, SQLite or presentation state.

`HostCore` owns one `Runtime` and the orchestration needed to drive it: periodic
schedule, consumed-sample cursors, event observation, configured identity catalogs,
Recorder worker/admission state, deployment provenance, model generations, probe
state and configuration/reconnect quiescence. These are coordination owners or
read-only catalogs, not competing domain authorities.

`ServiceHost` owns the process-facing boundary: monotonic system clock, loopback
listener, boot identity, finite shutdown clocks, declarative deployment lifecycle,
configuration path and the resource reconnect candidate/diagnostic state.

Each `ResourceExecutor`, stored inside `Runtime`, owns one adapter, transaction queue,
active/recovery state and transport generation. `ComTransport` owns bounded channel
state; its worker thread exclusively owns the blocking OS serial handle.

The names are only moderately explanatory. `HostCore` sounds more authoritative
than `Runtime` despite being its orchestrator, and `ServiceHost` hides substantial
deployment/reconnect responsibility. M10 should first add a three-level ownership
glossary and split implementation files. An internal rename such as `HostCore` to
`RuntimeHost` may be considered only after the split and is MEDIUM risk due to broad
test churn. Moving any authoritative mutable state between these types is HIGH risk
and out of scope.

## 10. Scheduler and acquisition organization

`SchedulePlan` contains safety, thermal plant, Metakon read, reference, controller,
managed source and managed transform schedules. `Periodic::take` owns next-due and
skipped-deadline arithmetic.

The actual bounded order in `HostCore::service` / `service_inner` is:

```text
poll Recorder receipts
→ safety cadence
   → Runtime::PollComponents
      → Runtime::service_safety
         → controller/lease/freshness checks
         → safe work
         → transport completions/dispatch
      → managed completions/timeouts
→ stopping transport retirement, if applicable
→ configuration quiescence fence
→ thermal plant refresh
→ Metakon read admission
→ reference evaluation
→ controller ticks
→ periodic managed sources
→ observation-driven managed transforms
→ admit Recorder facts
→ poll Recorder receipts again
```

Before every lower-priority loop, the host checks whether safety became due and
returns so the next bounded turn services it. This priority is correct but is only
understandable after reading a long method plus `Runtime::poll_components` and
`service_safety`. M10 should add this order as rustdoc and split each work class into
small private `service_*_due` methods that retain identical order and early-return
points. Scheduler semantics must not be redesigned.

## 11. Recorder audit

The Recorder layers are conceptually:

```text
Runtime
→ recording::FactOutbox / RequiredGate (semantic facts)
→ HostCore admission, gaps and lifecycle policy
→ RecorderWorker bounded ingress and receipts
→ worker_loop
→ SqliteStore
→ fixed SQLite schema
```

Durable history follows a separate read path:

```text
Application
→ HostCore history request
→ RecorderWorker bounded history job
→ SqliteStore history query
→ semantic HistoryPage/RunsPage
→ Application cursor/page delivery
```

Current conceptual responsibilities are located as follows:

- semantic facts and required gate: Core `recording.rs`;
- lifecycle/admission/gap/sealing coordination: large sections of `host.rs` and
  `recorder/worker.rs`;
- bounded ingress, receipts and history job mailboxes: `recorder/worker.rs`;
- writer thread and shutdown: `recorder/worker.rs`;
- record/history/provenance DTOs, storage health and budgets: top of `recorder.rs`;
- SQLite schema, SQL writes, queries, provenance validation and sealing:
  `SqliteStore` in the remaining `recorder.rs`;
- monotonic/wall-clock bridge: `recorder/time.rs`;
- fault injection: public `WriterBarrier` embedded in `worker.rs` because external
  integration tests need the seam.

SQLite does not contaminate Core Runtime semantics, which is the important boundary.
Physical layout nevertheless makes the app-side semantic contract look SQLite-owned.
M10 should split `recorder.rs` into semantic model, store/schema, write lifecycle,
history and provenance modules, and split HostCore's Recorder orchestration into a
private `host/recording.rs` `impl` block. Schema text, SQL, transaction boundaries,
gap/seal meaning and history ordering must remain byte-for-byte/semantically
unchanged.

## 12. Configuration and deployment audit

`FrozenDeployment` contains the exact loaded TOML/artifact bundle plus an
`EffectiveDeployment`. Parsing and validation are side-effect free. `DeploymentLifecycle`
owns the active immutable bundle, its revision and exactly one staged candidate.
`ServiceHost` owns that lifecycle; `HostCore` implements the live/safe apply port.

Application property edits do not mutate Runtime directly. They create a clone of
the active deployment with a bounded process-local property overlay, revalidate it,
stage it and apply it through the same lifecycle. Diff effects select live-safe,
reinitialize, restart or safe-barrier behavior. Provenance is prepared from the exact
frozen activation.

The persistent/effective distinction exists but is not immediately obvious from
names:

```text
persistent deployment source of truth
  = exact FrozenDeployment source/artifacts

effective active configuration
  = parsed DTO plus bounded runtime property overlays

ephemeral Runtime state
  = samples, generations, controller/component state, schedules
```

M10 should document this at the type boundary and prefer explicit internal names
such as `runtime_property_overlays` over generic `runtime_overrides`. Broad DTO/type
renames are lower priority than the orchestration split.

Lua removal left no active configuration path. One `host.rs` comment still says
startup and “source reload” call component staging; that wording is dead and safe to
remove. `restart_configured_models` is not the removed public `restart_models`
operation: it is the active private thermal-model lifecycle and must remain.

## 13. Transport and protocol audit

The separation is currently strong:

```text
ComTransport / ByteTransport
    moves bounded bytes and owns the blocking OS worker

ResourceExecutor
    serializes transactions, deadlines, recovery and shutdown

metakon codec
    understands frames, exact ACK and typed register values

Runtime
    correlates reads/writes and owns authoritative commit

OutputAuthority
    grants permission and validates lease/epoch/safety state
```

`serial` is crate-private. `OutputAuthority`, `OutputIntent`,
`AuthorizationStep` and `TransportEvent` are crate-private. `ResourceExecutor`'s
ordinary output enqueue/final-check path is not an Application or managed-component
surface. A raw physical write bypass is therefore not part of ordinary production
API.

M10 should not widen any of these items. Rustdoc should say explicitly that
`ByteTransport` can move bytes but cannot authorize physical output, and that
`ResourceExecutor` serializes work but cannot independently decide authority.

## 14. OutputAuthority studyability

The private machine correctly separates:

- permission: state, owner, finite `OutputLease`, epoch and instance;
- pending normal proposal: value and expiry;
- immutable transport reservation: `OutputIntent` plus binding/mapping generations;
- send-started dispatch and subsequent evidence;
- safe obligation: `safe_needed`;
- permission to attempt another safe write: blocked by `safe_resend_blocked` after
  an already-started safe write becomes uncertain.

The crucial rule is:

```text
safe_needed = true
does not imply
another safe WRITE is currently authorized
```

`transport_uncertain` requests/retains safe obligation. If the uncertain dispatch
was itself safe, it also sets `safe_resend_blocked` and fault-latches the authority.
`reserve_transport` then rejects another safe write. Only a fresh authority
instance/rebind or a future explicit reconciliation design can resolve that state.
Normal output remains prohibited.

The implementation comment states this, but a reader must infer the full transition
table from methods. M10 should add module rustdoc with normal and safe transaction
state tables, define “obligation” versus “permission”, and cross-reference the
Runtime transport callbacks. Internal names such as `safe_obligation_pending` and
`ambiguous_safe_resend_blocked` would improve clarity, but should be a focused,
test-backed rename only. Removing either boolean or merging states is a HIGH-risk
behavioral redesign and is deferred.

## 15. Time model

Monotonic time is represented primarily by explicit `Duration` since owner/boot
origin in Core and Host:

- `Periodic`, `HostCore::last_now` and command `at`: scheduling/order;
- `Sample::at`, input ages and controller `last_tick`: freshness/PID;
- reference state: ramp progress;
- output lease/proposal/queue expiry: authority;
- `ResourceExecutor`: queue/transaction/recovery deadlines;
- component invocation times/deadlines;
- reconnect/shutdown bounds use `Instant` at the OS-facing service/worker boundary.

Wall clock is confined to Recorder time projection. `recorder::TimeAnchor` ties a
monotonic point to `SystemTime`; SQLite stores wall estimates/human UTC fields while
control continues to use monotonic time.

The semantics are correct. Generic local names such as `at`, `now` and `deadline`
are usually clear inside their subsystem, but cross-layer helpers such as
Application's `nanos` obscure the model. M10 should add one architectural time-model
rustdoc section and use names such as `monotonic_at` or `monotonic_ns_json` at
boundaries. Introducing pervasive new time newtypes would cause broad, HIGH-risk
churn with no semantic benefit and is deferred.

## 16. Mutable-state ownership map

| State | Authoritative mutable owner | Copies/projections and lifetime notes |
|---|---|---|
| experiment instruments/samples | `Runtime` | API/EventLog/Recorder receive owned projections or semantic facts |
| references/controllers/PID | `Runtime` | `HostCore` holds only schedule and consumed-sample cursors |
| OutputAuthority | one private entry in `Runtime::outputs` | snapshots are read-only; no adapter owns permission |
| transport transaction state | one `ResourceExecutor` per Runtime resource | `HostCore` keeps resource IDs/closed set only |
| COM OS handle/blocking I/O | serial worker thread | `ComTransport` owns mailbox state and finite worker handle |
| Recorder semantic fact outbox | `Runtime` | `HostCore` drains it under admission policy |
| Recorder ingress/lifecycle receipts | `RecorderWorker` on owner lane | SQLite mutation belongs solely to disk worker/`SqliteStore` |
| SQLite connection/schema/run rows | Recorder disk worker | Application sees semantic history pages, never connection state |
| Application sessions/dedup records | `SessionStore` inside `Application` | independent of TCP connection lifetime |
| subscriptions/projection pages/history cursors | `Application` | bounded delivery state, not experiment authority |
| live event replay/current facts | `EventLog` inside `HostCore` | derived from committed Runtime snapshots |
| persistent/staged configuration | `DeploymentLifecycle` inside `ServiceHost` | Runtime holds applied state; `HostCore` catalogs/schedules mirror active deployment for orchestration |
| reconnect candidate/diagnostic | `ServiceHost` | candidate transport is quarantined until fenced replacement |
| managed worker jobs | `ManagedExecutor` fixed slots | component committed state remains in Runtime; workers return plain results only |

No competing authoritative experiment cache was found. `HostCore::consumed`, model
generations, configured catalogs, EventLog facts and Application frozen projections
are convenience/fencing/delivery state with clear consumers, although their rustdoc
should say so. They should not be removed simply because they duplicate read-only
identity or snapshots.

## 17. Dead compatibility and archaeological code

| Occurrence | Classification | Action |
|---|---|---|
| Recorder accepts operation facts named `reload_managed_sources` and `restart_models` | required historical archive compatibility | Keep; document compatibility scope |
| Recorder accepts provenance `managed_lua_source` and `managed_component_source` | required historical archive compatibility | Keep exact names so old SQLite archives remain readable |
| provenance tests containing `lua.v1` and `managed_lua_source` | useful regression oracle | Keep and label as historical archive compatibility |
| protocol/configuration tests rejecting Lua, Babashka and removed operations | useful regression oracle | Keep; group as M9C removal regression |
| `HostCore::stage_configured_component` comment saying “source reload” | safe to delete/rename | Replace with active startup/reconfiguration wording in M10.2 |
| private `restart_configured_models` | required accepted behavior | Keep; it restarts configured native thermal models and is not the removed public operation |
| Application `FrozenSnapshot` / `snapshots` | useful behavior, obsolete/misleading name | Rename to frozen projection/delivery page; do not restore removed snapshot operations |
| active Babashka helpers/code | absent | No deletion needed |
| active Lua/source execution types | absent | No deletion needed; `ComponentKind::Source` is neutral and still valid |

No historical milestone report or SQLite artifact should be rewritten to modernize
old vocabulary.

## 18. Internal naming findings

High-value internal naming improvements are:

- `FrozenSnapshot` / `snapshots` → `FrozenProjection` / `projections`;
- `expire_snapshots_at` → a projection/delivery-specific name while preserving the
  public protocol's accepted `snapshot_expired` error if that string is contractual;
- component “source reload” comment → configured component staging/replacement;
- `runtime_overrides` → `runtime_property_overlays`;
- document that `model` means the native thermal virtual model, while `instrument`
  is the generic identity exposed through descriptors;
- document that a `resource` is the serialized byte-conflict domain and a
  `transport` is its byte adapter/executor machinery;
- document `Runtime` versus `HostCore` versus `ServiceHost` before considering type
  renames;
- reserve `RecordingFact` for authoritative semantic input, `OperationRecord` for
  admitted API operation history, and `EventLog` records for transient delivery.

Accepted Application operation names and public error strings must not be renamed
for aesthetic consistency.

## 19. Public/private visibility

Must remain public across the crate boundary:

- Core domain commands, queries, identifiers, descriptors and snapshots used by the
  app crate;
- `ByteTransport` and `ResourceExecutor` needed by app composition and scripted
  transport tests;
- neutral managed-component contract types;
- stable library surfaces intentionally exercised by integration tests, unless those
  tests are moved behind an internal test-support facade first.

Correctly crate-private/private today:

- `OutputAuthority`, `OutputIntent`, final authorization steps and transport events;
- the entire `serial` module outside `lab-runtime`;
- configuration DTO internals and property mutation representation;
- Runtime instrument instances and signal buffers.

Candidates for `pub(crate)` or private after test reorganization:

- app orchestration implementation details in `deployment`, `events`, `sessions`,
  projection helpers and worker fault seams that are public mainly for integration
  tests;
- `WriterBarrier` and `WorkerBarrier` through a clearly named test-support export;
- concrete SQLite helper types not required by the public Application contract.

Visibility tightening must be usage-driven (`rg` plus compiler), staged after module
splits and covered by integration tests. M10 must not make raw output enqueue,
serial-device access or authority mutation more visible.

## 20. Bounds organization

Bounds are generally owned by the correct subsystem: signal history in `signal`,
transport queue/duration in `transport`, managed limits in `managed`/executor,
Recorder limits in worker/store, event ring in `events`, sessions in `sessions`, wire
frame size in `wire` and protocol delivery limits in `protocol`.

The main issue is duplicated literal projection. `protocol::limits_json` repeats
values such as projection/page sizes, history jobs, emulator targets and property
overlays that are enforced elsewhere; `Application`, `HostCore` and configuration
also contain local numeric limits. M10 should:

1. name each enforced bound in its owning module;
2. have protocol metadata read those exported crate-level constants where practical;
3. add tests that advertised and enforced values agree;
4. keep algorithm-specific bounds, such as moving-mean window or controller timing,
   with that implementation/domain.

Do not create a global `constants.rs` dumping ground.

## 21. Error organization

The layers are:

```text
Core Error
├── controller/output/component domain errors
├── TransportError
└── configuration/validation-style domain failures

lab-runtime
├── CodecError → Core Error at Runtime correlation
├── SerialError → startup/reconnect lifecycle result
├── StorageError → Recorder status/fail-closed policy
├── ConfigurationError / StageError / ApplyError
├── LifecycleOperationError
├── SessionError
└── wire decode errors before owner admission

Application mapping
→ internal domain code
→ protocol::PublicError
→ accepted 12-category public taxonomy
```

The public taxonomy is centralized and must remain unchanged. Internal conversions
are harder to follow because lifecycle mapping lives in `application.rs` while some
classification happens in `service.rs` and `protocol.rs`. The Application split
should create one focused internal error-mapping module with exhaustive tests against
all 12 public categories; it must not merge ambiguity into success or expose raw OS,
SQLite or filesystem errors.

## 22. Test architecture

Current coverage groups are strong:

- Core domain/unit/integration: milestone runtime, controller configuration,
  managed contract, transport, OutputAuthority and recording facts;
- Runtime orchestration: scheduler, startup/shutdown, configuration apply/reload,
  reconnect and physical scripted transport;
- Recorder: SQLite/schema, transactions, provenance, time, history, operations,
  backpressure, isolation, reopen and shutdown;
- protocol/API: registry, owner queries, control, measurements, history,
  subscriptions, deduplication and client isolation;
- safety/fault: M9B.8 fault acceptance, M9D physical output simulation,
  required-recording, ambiguous write/readback and reconnect ordering;
- historical regression: rejection of removed Lua/snapshot operations and historical
  Recorder provenance readability;
- hardware evidence helpers/artifacts: examples and milestone reports, which M10
  must not edit or execute.

Several test files are very large: `recorder_history_api.rs` (~1,302 LOC),
`milestone4_runtime.rs` (~1,282), `recorder_provenance.rs` (~1,261),
`recorder_sqlite.rs` (~1,179), `milestone5_managed.rs` (~1,008),
`recorder_facts.rs` (~969), `configured_physical.rs` (~939) and
`recorder_shutdown.rs` (~891). Size often reflects valuable scenario matrices, but
purpose is obscured by historical milestone names and repeated setup.

At least twenty Recorder integration files repeat `temporary_database`; several API
files repeat frame/request/ask helpers; multiple orchestration tests define nearly
identical manual clocks and scripted transports. `WriterBarrier` is a necessary,
well-used synchronization/fault seam but currently appears as ordinary Recorder
production API. Inline host reconnect ordering tests already demonstrate a useful
focused submodule split.

M10 should introduce narrowly scoped test-support modules for temporary archives,
API request fixtures, manual clocks and scripted transports, then split large files
by safety purpose without deleting cases. Historical tests should be named/commented
as regression oracles. Test reorganization must not weaken assertions, timing/fault
coverage or release-mode execution.

## 23. Priority rustdoc targets

M10 should add architectural rustdoc specifically to:

- `Runtime`: sole ownership, command/query semantics and its internal subsystem map;
- `HostCore` and `ServiceHost`: three-level ownership/lifetime glossary;
- `SchedulePlan` and `HostCore::service`: exact work priority and early-return rule;
- `OutputAuthority`: transition/evidence table, safe obligation versus resend
  permission and lease/epoch/instance meanings;
- `ResourceExecutor` and `ByteTransport`: byte ownership, bounds, recovery, shutdown
  and inability to grant output authority;
- Runtime physical read/write handlers: final-check timing, send-started, ACK and
  distinct readback;
- Core `recording` plus `RecorderWorker`/`SqliteStore`: semantic contract, bounded
  ingress and storage boundary;
- `Application`: service/session/delivery model and operation-domain index;
- `ComponentExecutor`/native registry: worker isolation, validation and commit owner;
- `TimeAnchor` and Core command time: monotonic versus wall clock;
- reconnect fencing: binding generation, transport generation and mapping revision.

Trivial getters do not need expanded documentation.

## 24. Dependency direction conclusion

The intended dependency direction is verified. `lab-core` has no external crate
dependency and contains no `rusqlite`, serial/Windows, serde/NDJSON, filesystem
deployment, scripting or UI code. `lab-runtime` depends on Core and supplies all OS,
storage, serialization and process adapters. No M10 stage should add a dependency to
Core merely to make app-side organization convenient.

## 25. Studyability walkthrough

| New developer question | Current difficulty | Why |
|---|---|---|
| Where is authoritative Runtime state? | EASY | `Runtime` has explicit sole-owner rustdoc and private fields in one struct |
| How is a physical measurement produced? | HARD | Correct layers span configuration, service, host scheduler, Runtime, executor, COM and codec without one index |
| How does PID reach physical output? | HARD | Safety steps are precise but split between scheduler, distant Runtime sections, authority and executor |
| Why can an ambiguous WRITE not be retried? | MODERATE | Strong comments/state exist in authority/transport, but full safe ambiguity rule lacks a transition table |
| How is data written to SQLite? | HARD | Runtime fact outbox, Host admission, worker protocol and a 3,500-line store file must all be reconstructed |
| How does an API request reach Runtime? | MODERATE/HARD | Registry is easy; decode, admission, dispatch and projection for one operation are far apart in `application.rs` |
| How do I add a native component? | MODERATE | Neutral executor contract is clear, but registration/validation/manifest knowledge is duplicated in three files |
| How do I add a new instrument? | HARD | Family-specific maps, schedules, configuration and projections create more touch points than the generic API/Recorder model requires |

The highest-value M10 outcome is to turn the four HARD paths into documented,
physically discoverable paths without changing their owners or behavior.

## 26. Proposed M10 implementation plan

### M10.2 — archaeological cleanup, terminology and architecture indexes

Risk: **LOW**.

Files/modules:

- `apps/lab-runtime/src/application.rs`;
- `apps/lab-runtime/src/host.rs`;
- `apps/lab-runtime/src/configuration.rs`;
- module rustdoc in Core `runtime.rs`, `transport.rs`, `output/authority.rs` and app
  `host.rs`, `service.rs`, Recorder and Application modules;
- focused tests whose internal names must follow a rename.

Work:

- rename `FrozenSnapshot` delivery state to projection terminology;
- remove the dead “source reload” comment and clarify model/instrument,
  resource/transport, fact/event/record and configuration overlay vocabulary;
- add ownership, acquisition, scheduler, output evidence and time-model indexes;
- label historical Lua/source compatibility paths and regression tests explicitly;
- inventory and name subsystem constants without behavioral changes.

Must not change protocol strings, 42 operations, public error taxonomy, historical
Recorder identifiers, bounds or time calculations.

Required regression: workspace debug tests, protocol inventory, snapshot-removal
regressions, subscriptions/projections, M9D physical-output tests and Recorder
historical provenance tests.

### M10.3 — Application API physical organization

Risk: **MEDIUM**.

Files/modules:

- `application.rs` into `application/{mod,queries,mutations,dispatch,delivery,history,projections,errors}.rs`;
- `events.rs`, `measurements.rs`, `configuration_api.rs`, `recorder_api.rs`;
- `wire.rs`, `protocol.rs`, `sessions.rs` only where imports/owned constants move;
- Application/API integration tests.

Work:

- preserve one `Application` owner while separating query domains, typed mutation
  decode, semantic dispatch, async history/recording delivery and projections;
- remove `events`' reverse dependency on dispatcher-local projection helpers;
- centralize internal-to-public error mapping and advertised/enforced bound tests;
- provide an operation-to-handler index by semantic domain.

Must not change wire DTOs, operation metadata, admission/deduplication, page tokens,
event order, history cursor semantics, backpressure or public results/errors.

Required regression: all API/protocol/session/history/subscription/client-isolation
tests, M9B.8 fault acceptance, debug and release workspace suites.

### M10.4 — Runtime/Host/Service orchestration organization

Risk: **MEDIUM**.

Files/modules:

- Core `runtime.rs` into private Runtime `impl` modules for commands/queries,
  managed work, control, physical I/O and recording coordination;
- app `host.rs` into `host/{composition,scheduler,recording,configuration,lifecycle}.rs`;
- app `service.rs` into `service/{startup,configuration,reconnect,shutdown}.rs`;
- existing host/reconnect/scheduler/shutdown tests.

Work:

- keep the `Runtime`, `HostCore`, `ServiceHost` and `SchedulePlan` structs and every
  field under the same owner;
- extract methods by responsibility and make scheduler order readable as named
  private steps with identical order/short-circuit points;
- put physical read/output correlation in a discoverable private Runtime module;
- isolate reconnect and Recorder orchestration from ordinary service navigation.

Must not change state ownership, command/query semantics, scheduler ordering/cadence,
deadline arithmetic, generation fences, transport polling count, output state
transitions, reconnect or shutdown behavior.

Required regression: all Core milestone/controller/managed/transport/recording tests,
host scheduler, configured physical/read/output, lifecycle/reconnect, Runtime and
COM/Recorder shutdown, fault acceptance, debug and release suites.

### M10.5 — Recorder contract and SQLite organization

Risk: **MEDIUM**.

Files/modules:

- `recorder.rs` into `recorder/{mod,model,provenance,sqlite,schema,history}.rs`;
- `recorder/worker.rs`, `recorder/time.rs` and a clearly scoped test-support seam;
- `host/recording.rs`, `recorder_api.rs`, Application history imports;
- all Recorder integration tests.

Work:

- make semantic records/history/provenance types visibly independent from
  `SqliteStore`;
- isolate schema/SQL, writer lifecycle and durable queries without rewriting SQL;
- separate worker ingress/receipts from fault-injection helpers;
- consolidate repeated temporary-database and polling fixtures.

Must not change schema bytes/version, SQL meaning/order, transaction boundaries,
provenance validation, time projection, gaps, sealing/completeness, fail-closed
policy, history cursors or ingress accounting.

Required regression: every Recorder test in debug and release, historical Lua
provenance readability, required-recording/fault tests, WAL/reopen/process/shutdown
tests and schema/hash comparison against generated baseline fixtures. Historical
accepted SQLite evidence remains read-only.

### M10.6 — native component and instrument extension clarity

Risk: **MEDIUM**.

Files/modules:

- `managed_executor.rs` into executor and static native registration/implementation
  modules;
- `configuration.rs`, `configuration_api.rs`, `host/composition`;
- Core instrument/Runtime registration helpers and `SchedulePlan` composition
  helpers where purely mechanical;
- native component, configuration, discovery and physical scripted tests.

Work:

- give a static native component registration one owner for identity, metadata,
  validation, manifest construction and runner;
- make ordinary component touch points implementation, registration/metadata and
  tests;
- centralize instrument-family composition/projection helpers so generic standard
  signals need no Recorder/Application dispatcher surgery;
- document the physical instrument checklist and essential safety hooks.

Must not introduce dynamic plugins, scripting, new instruments/features, generic raw
write APIs, new queues/workers or changes to managed validation/commit semantics.

Required regression: M9A native managed suites, component backpressure/failure,
configuration/property/provenance, discovery/current/history/subscription,
configured physical and all M9D safety tests.

### M10.7 — visibility and test organization

Risk: **LOW/MEDIUM**.

Files/modules:

- crate `lib.rs` files and subsystem module declarations;
- app test-support modules and the largest Core/app test files;
- transport, Recorder, configuration and projection internals identified by compiler
  usage.

Work:

- reduce visibility only after callers/tests have focused facades;
- keep raw output/serial/SQLite implementation surfaces narrow;
- consolidate manual clock, framed request, temporary archive and scripted transport
  fixtures;
- split very large tests by semantic/safety purpose and label historical regression
  oracles.

Must not remove tests, weaken assertions, reduce fault coverage or widen production
test seams. No public Application behavior changes.

Required regression: full debug/release workspaces, warning-denied Clippy/docs and a
coverage/test-name inventory proving no accepted regression oracle disappeared.

### M10.8 — structural acceptance and external review

Risk: **LOW** (verification/review only).

Required gates:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Also run focused M9A managed, M9B API/fault, M9D physical-output simulation,
configuration/reconnect, Recorder failure/reopen/shutdown and historical compatibility
suites. Compare the 42-operation/capability/error/limit inventory and SQLite schema
against the accepted baseline. Verify accepted evidence hashes without opening COM5
or performing hardware tests. Submit for external review; do not cross into M11.

### Explicitly deferred HIGH-risk changes

The following are not M10 implementation work:

- moving authoritative state out of `Runtime` or creating multiple experiment owners;
- replacing concrete instrument maps with a plugin/trait-object ownership framework;
- scheduler redesign, concurrency changes or different work ordering;
- OutputAuthority state/behavior simplification or ambiguity reconciliation design;
- new transport/protocol semantics or raw output surfaces;
- pervasive time newtypes that change public/domain signatures;
- SQLite schema/migration redesign;
- public Application contract changes;
- product features, GUI, scripting, logging/M11 or tutorials/M12.

## 27. M10 acceptance criteria

M10 is complete only when all of the following are true:

1. `Runtime` is visibly and documentably the sole authoritative mutable experiment
   owner.
2. A reader can follow physical acquisition from deployment through COM, codec,
   Runtime commit, controller and Recorder from one architectural index.
3. A reader can follow physical output through proposal, authority, final check,
   send-started, ACK, separate readback and safe-zero, including ambiguous safe-write
   non-retry semantics.
4. All 42 Application operations are physically discoverable by semantic domain
   without changing their public contract.
5. The Recorder semantic contract, bounded worker and SQLite store/schema are visibly
   separate.
6. An ordinary native component has a documented small touch set: implementation,
   registration/metadata and tests.
7. An ordinary instrument can expose standard descriptors/signals/properties without
   unrelated Application dispatcher or Recorder storage edits.
8. Runtime ownership, scheduler order, OutputAuthority, ResourceExecutor,
   transport worker, Recorder, Application sessions, component executor and time
   model have useful architectural rustdoc.
9. Safe dead post-Lua/client terminology/code is removed while historical archive
   compatibility and regression oracles remain.
10. Public, crate-private and private boundaries are deliberate; raw physical output
    authority is no wider than at the accepted M9D baseline.
11. Important bounds have subsystem owners and advertised/enforced values agree.
12. Internal error flow is discoverable while the accepted 12-category public API
    taxonomy is unchanged.
13. No accepted Application, controller, managed-component, acquisition, emulator,
    Recorder, reconnect, backpressure or client-isolation behavior changes.
14. No SQLite schema/provenance/history semantics change.
15. No Metakon codec, transport, ACK/readback, output-authority or hardware-facing
    behavior changes.
16. Debug, release, focused safety/fault, warning-denied Clippy and rustdoc suites are
    green.
17. Accepted M8, post-M9C and M9D evidence hashes remain byte-identical.

There is deliberately no maximum-file-length acceptance criterion.

## Final M10.1 conclusion

M10 should be a sequence of small structural extractions, focused internal renames,
rustdoc and test-fixture improvements. The first implementation slice must be
separately authorized. This audit does not authorize automatic refactoring.
