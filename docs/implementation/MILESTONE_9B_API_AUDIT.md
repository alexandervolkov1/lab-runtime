# Milestone 9B Application API audit and implementation design

Date: 2026-09-17

```text
Baseline: e37d0d916e50f96253669a953275cd839f98168b
M8: ACCEPTED
M9A: ACCEPTED
M9B: AUTHORIZED
M9B implementation: NOT STARTED
M9C+: NOT AUTHORIZED
```

This document is the read-only M9B.1 audit of the actual version-one local API. It
does not describe work already implemented unless it is explicitly marked
"current". The M9B implementation must preserve the accepted Runtime, safety,
Recorder and managed-component contracts.

## 1. Product boundary and audit method

The public boundary remains:

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The API may expose resources, instruments, signals, measurements, References,
controllers, Recorder state, configuration, reconnect, virtual instruments,
emulators, operations and errors. It must not expose workspace, plot, trace,
button, panel, tab, color, layout, window or other presentation state.

The audit followed the real dispatch path rather than inferring an API from the
roadmap:

```text
loopback TCP reactor
  -> bounded NDJSON decoder
  -> Application query/mutation dispatch
  -> ServiceHost lifecycle or HostCore
  -> lab-core Query/Command
  -> committed snapshot or retained operation outcome
```

The principal source anchors are:

- `apps/lab-runtime/src/wire.rs:18` and `:144` for framing, schema and operation
  admission;
- `apps/lab-runtime/src/server.rs:24` and `:360` for connection isolation,
  mailboxes and owner scheduling;
- `apps/lab-runtime/src/application.rs:456`, `:526`, `:956` and `:1416` for public
  dispatch and DTO construction;
- `apps/lab-runtime/src/sessions.rs:12` and `:24` for deduplicated operations;
- `apps/lab-runtime/src/events.rs:22` and `:245` for transient replay and frozen
  current-state records;
- `apps/lab-runtime/src/service.rs:965` for deployment/reconnect/model lifecycle;
- `crates/lab-core/src/runtime.rs:60` and `:368` for the authoritative Command and
  Query surfaces.

Babashka code was inspected only as evidence of current wire behavior. It is not a
v0.1 client requirement and must not shape the final API around Clojure-specific
conveniences.

## 2. Executive assessment

The current API has a strong bounded protocol foundation, but it is not yet a
complete v0.1 Application API.

Already sound and worth preserving:

- loopback-only TCP and strict version-one NDJSON;
- finite request, JSON depth/value/string and response-frame bounds;
- one Runtime owner, nonblocking network reactor and disconnect isolation;
- process/session request correlation with retained idempotent mutation outcomes;
- accepted versus terminal operation states;
- pure committed queries with no hidden instrument I/O;
- a frozen snapshot plus replay-cursor recovery pattern;
- bounded event replay with explicit gap detection;
- Reference retune and native PID lifecycle through Core commands;
- Recorder start/stop/status and asynchronous bounded archive reads;
- staged deployment apply and explicit resource reconnect;
- strict rejection of raw transport, direct OutputAuthority and fabricated evidence
  operations.

The main gaps are semantic completeness and consistent projection, not absence of an
API framework:

- discovery and hello capabilities are partly hard-coded to the demo profile;
- resources, signal series, acquisition cadence/state, configured values, virtual
  identity and operation applicability are not discoverable coherently;
- latest and live sample DTOs omit different important fields;
- Core's bounded recent signal window has no public query;
- resource/configuration/Recorder lifecycle is absent from the replay stream;
- archive continuation is connection-local and cannot survive reconnect;
- disconnecting an accepted `history_read` can leave its session operation forever
  nonterminal;
- error codes collapse materially different failures and have no structured detail;
- an oversized otherwise-valid response can propagate an encoder error out of the
  owner loop rather than failing only that exchange;
- configuration operations only reread the pinned startup file and do not expose
  current revision/properties as a coherent public model;
- no external-emulator publication endpoint exists;
- `restart_models` combines two different lifecycle concepts;
- several capabilities describe implementation/profile facts rather than actual
  target-specific semantic support.

`Application API`, `Recorder contract` and `SQLite schema` must remain three
different contracts. Public history may be backed by Recorder, but must not expose
SQL tables as the control protocol.

## 3. Current protocol and ownership model

### 3.1 Wire and connection behavior

The current request envelope is:

```json
{"v":1,"msg_id":"...","op":"...","args":{},"request_id":{"scope":"...","seq":"1"}}
```

`request_id` is required for operations and forbidden for pure queries. IDs and all
u64 values that can exceed JSON's exact integer range use canonical decimal strings.
The decoder rejects duplicate keys, unknown request/argument fields, invalid UTF-8,
nonfinite numbers, excess nesting and excess lexical values. Every outgoing object
uses the same 16 KiB frame cap.

The listener is bound to `127.0.0.1`. One fixed reactor thread owns sockets; the
Runtime owner never blocks on network reads or writes. Malformed input closes only
the offending connection. A nonreading or event-flooded client is detached; it does
not block acquisition, native control, safety or Recorder progress.

One deficiency is that `unknown_operation` is rejected in the wire decoder and
therefore closes an otherwise well-formed connection. For forward-compatible local
clients, schema/framing violations may remain connection-fatal, but an unknown
operation should be a bounded nonfatal Application error.

### 3.2 Sessions and operations

`hello` creates or resumes a boot-local scope. Mutations must use consecutive
sequence numbers. Equal replay returns the retained outcome without execution;
changed payload is `request_id_conflict`; an evicted old outcome is honestly
`outcome_unknown`. Disconnect does not cancel ordinary admitted Runtime work.

Current mutation replies are:

```text
accepted -> completed(result)
         -> failed(code[, result])
```

There is no false claim that acceptance is completion. This contract should remain.
The terms `accepted`, `completed` and `failed` are sound; progress-specific detail
may be added without inventing a second operation mechanism.

The current history-specific disconnect path violates the otherwise sound rule:
`Application::detach()` removes a pending history job and later cancels it, but does
not complete its retained session record. That accepted request remains pending and
can consume bounded session capacity indefinitely. A completed history operation can
also retain a page token whose connection-owned page is deleted on disconnect. M9B
must give both cases an honest terminal/restartable outcome.

### 3.3 Snapshot and event recovery

The existing recovery idea is correct:

```text
freeze current facts at cursor C
page the frozen snapshot
subscribe after C
apply retained events in order
resnapshot on event_gap or instance_changed
```

The current snapshot freezes signal, controller, Reference, output and component
facts, then appends resource records. The event log does not track resources,
configuration, Recorder state or the instrument catalog. Consequently a client can
obtain an initial resource record but cannot replay later resource changes. The
snapshot must become a coherent complete public-state barrier and every mutable fact
required after that barrier must either have a replay event or be explicitly queried
again during resynchronization.

## 4. Exact current public operation inventory

Classification terms:

- **keep**: sound name and semantics; fields may be completed;
- **normalize**: useful operation whose DTO/error/capability projection needs work;
- **incomplete**: required domain surface is missing;
- **transitional**: active pre-v0.1 baggage to remove in M9C, not a final release
  contract;
- **replace**: current public semantic grouping should not become the v0.1 contract.

### 4.1 Queries

| Public operation | Current request fields | Current result | Authority behind it | Classification |
|---|---|---|---|---|
| `hello` | optional `scope` | `boot_id,v,scope,next_seq,state,capabilities,operations,limits,event_oldest,event_latest` | `SessionStore`, Host feature presence | keep + normalize capabilities/version/limits |
| `discover` | none | `instruments[{id,name}],controllers,references,components[{id,kind,implementation}],outputs`; three lists are profile-shaped | `Query::Discover` plus profile constants | incomplete; must be real paged discovery |
| `describe` | `instrument` | `id,name,parameters[]`; each parameter has `id,name,unit,value_spec,access,role,write_effect,signal` | `Query::DescribeInstrument` | keep |
| `latest` | `signal{instrument,parameter}` | null or `value,quality,observed_at,unit` | `Query::GetLatestSignal` | keep + complete failure/freshness/identity metadata |
| `controller` | `controller` | `controller,state,revision,last_tick,latest_output,config{input,output,reference,pid,timing,ema}` | controller/config queries | keep + normalize status/failure/capabilities |
| `reference` | `reference` | `reference,kind,value,revision,unit` and ramp `target,rate,last_at` | `Query::Reference` | keep |
| `component` | `component` | `component,instrument,kind,implementation,generation,revision,state,good_steps,pending,diagnostics` | `Query::Component` plus host catalog | keep; implementation metadata is explicit and legitimate |
| `output` | `actuator{instrument,parameter}` | state/owner/instance/epoch/lease/fault/safe/pending/in-flight/request/sent/ACK/readback/outcome | `Query::Output` | normalize as read-only status; never client-owned authority |
| `runtime_snapshot` | none | `snapshot,cursor,records,count,next_index,expires_at` | `EventLog` plus resource/catalog projection | keep + make complete/coherent at all admitted bounds |
| `snapshot_page` | `snapshot,index` | same frozen page shape | Application cache | keep |
| `snapshot_release` | `snapshot` | released boolean | Application cache | keep |
| `subscribe` | `after{boot_id,seq}`, `filter{kinds[],targets[]}` | `subscription,accepted_cursor` | `EventLog` | keep + add missing semantic kinds and explicit delivery policy |
| `unsubscribe` | `subscription` | removed boolean | Application connection state | keep |
| `operation_status` | `request_id{scope,seq}` | `state` plus terminal `result` or `code` | `SessionStore` | keep + structured error/result shape |
| `recording_status` | none | state/policy/archive/run/interval/watermark/coverage/failure plus queue and WAL fields | Host Recorder state | keep + separate semantic status from optional bounded diagnostics |
| `history_page` | `page_token` | runs page, or raw measurement rows plus watermark/coverage/cursor/loss | Recorder worker result cache | normalize for reconnect-safe continuation |
| `history_release` | `page_token` | released boolean | Application cache | keep if retained pages remain |

There is no public query for a resource by ID, configured instrument state, recent
in-memory signal window, configuration revision/candidate, acquisition schedule or
virtual model state.

### 4.2 Mutations and asynchronous work

| Public operation | Current request fields | Current terminal result | Authority behind it | Classification |
|---|---|---|---|---|
| `reference_retune` | reference, expected revision, target, rate | new revision/current/target/rate/time | `Command::RetuneRampReference` | keep; ramp-specific |
| `controller_configure_pid` | `controller,expected_revision,pid{kp,ki,kd,output_min,output_max}` | controller snapshot | `Command::ConfigureControllerPid` | keep; full binding/timing reconfiguration still missing |
| `controller_start` | controller | controller snapshot | `Command::StartController` | keep |
| `controller_pause` | controller | controller snapshot after safe path | `Command::PauseController` | keep |
| `controller_resume` | controller | controller snapshot | `Command::ResumeController` | keep |
| `recording_start` | label | database/run/interval identity after durable start | Host Recorder lifecycle | keep |
| `recording_stop` | `run_id{boot_id,run_no}` | database/run/interval identity after durable stop | Host Recorder lifecycle | keep |
| `experiment_annotate` | bounded `name,data` | `record_seq,durability`; currently `pending` | Recorder ingress | keep, but result must state durability precisely |
| `history_read` | runs: `mode,database_id,max_records,cursor`; measurements additionally `boot_id,run_id,signal,from_ns,to_ns` | `page_token,durability=checkpoint_frozen` after worker result | Recorder query worker | keep as bounded work; fix disconnect/cursor semantics |
| `stage_configuration` | none | candidate ID, base revision, debug-formatted effects | reload pinned startup path + `DeploymentLifecycle::stage` | normalize; expose typed effects/current revision |
| `apply_configuration` | candidate ID, expected revision | committed revision | deployment lifecycle/safe barrier | keep + structured failures |
| `reload_configuration` | none | committed revision | reread/stage/apply pinned startup path | keep as explicit convenience, document source |
| `reload_managed_sources` | none | reloaded boolean | text-backed implementation adapter lifecycle | transitional; source-artifact-specific, remove with Lua in M9C |
| `restart_models` | none | aggregate model count/max generation | all managed components plus thermal plants | replace with targeted component and virtual-model lifecycle operations |
| `reconnect_resource` | resource, expected binding generation | resource/new generation or bounded failure diagnostic | ServiceHost explicit reconnect | keep + expose resource status and typed errors |
| `runtime_shutdown` | none | safe/cleanup/Recorder terminal evidence | ServiceHost shutdown lifecycle | keep |

No public operation currently reconfigures a Fixed Reference, replaces a complete
Reference policy, resets an acknowledged failed controller, exposes generic
configuration-only properties, publishes an explicitly external emulated
measurement or requests a target-specific deterministic virtual fault.

## 5. Domain audit

### 5.1 Protocol foundation

Current strengths are strict framing, exact v1 selection, duplicate-key rejection,
canonical IDs, bounded parsing, output-frame enforcement, session deduplication and
per-client failure isolation.

Required M9B corrections:

1. Advertise `protocol_version`, `application_api_version`, server boot identity and
   structured capability versions separately. `v:1` remains the envelope major
   version; incompatible major versions remain rejected.
2. Generate operations/capabilities from actual composition and target support, not
   the virtual-demo constants in `Application::handle()`.
3. Make unknown but well-formed operation names nonfatal to the connection.
4. Define one structured error envelope with category, stable code, optional bounded
   details, retry/resync hints and current/expected revisions where applicable.
5. Convert outgoing oversize/encoding failure into a bounded per-exchange error or
   detach of only that client. It must never unwind the service owner loop.
6. Advertise all relevant bounds and retention durations, not only clients/scopes/
   frame bytes.
7. Preserve query purity and accepted/terminal operation correlation.

### 5.2 Discovery

Current `discover` is not deployment-generic: controller 1, Reference 1 and output
instrument 1 are hard-coded; resources and signal series are absent. `describe`
correctly exposes instrument parameter semantics and should remain the detailed
instrument descriptor query.

Final discovery must be derived from committed owner state and trusted deployment
composition. It must cover resources, instruments, signals, References, controllers,
components, outputs and Recorder capability. Each entry identifies virtual versus
physical semantics, current generation/revision where meaningful and supported
operations. No raw serial handle or OS object is exposed.

Because the admitted maxima cannot fit a single 16 KiB response, `discover` becomes
a bounded paged query with a domain selector, stable ordering, catalog revision and
opaque/validated continuation. `describe` remains an instrument-by-ID detail query;
this avoids duplicating all parameter descriptors into discovery pages.

### 5.3 Latest and current state

`latest` is pure and correctly returns `null` before the first attempt. Its field
named `observed_at` currently contains `Sample::at()` (publication/attempt time), not
`Sample::freshness_at()`. It omits the unavailable failure reason and source
freshness time; events include freshness but still omit failure. The two projections
must use one sample DTO:

```text
signal
value or null
quality
failure or null
published_at (boot-local monotonic identity)
observed_at/freshness_at
unit
generation/revision/state_revision where the source defines them
```

Generation fields must be truthful rather than fabricated for sources that have no
replacement generation. Add `instrument_state` backed by
`Query::GetInstrumentState`, including configured values separately from
observations. Add semantic current-state queries/projections for resources,
acquisition, References, controllers, components, Recorder, configuration and
virtual models. Current state comes from the Runtime/Host owner, never SQLite.

### 5.4 History

There are two distinct current-history concepts:

- Core retains an in-memory signal window of 1..=4096 attempts, but the wire does
  not expose it;
- Recorder exposes durable run discovery and raw measurements through bounded
  worker jobs.

Add a `signal_window` query for recent owner state, capped to a public page of 128
attempts with ordering and truncation metadata. It is not durable and does not claim
coverage.

Keep Recorder-backed `history_read` as archival work. Normalize its measurement DTO
with the latest/event unit and timestamp vocabulary while retaining record sequence,
quality, failure, generation/revision/lineage, watermark, coverage and gap metadata.
The archive identity and run identity remain explicit. A page must be filled to both
the row bound and byte bound rather than failing merely because 128 individually
valid rows exceed 8 KiB.

Continuation must not depend on a deduplicated-array index or one TCP connection.
Use a bounded validated cursor tied to database identity, run, signal, time range,
frozen watermark and last `(published_at, record_seq)` key. A reconnecting client can
resubmit it in a new accepted operation. Empty ranges remain successful empty pages.

### 5.5 Live subscriptions and events

The current one-subscription-per-connection model is sufficient for v0.1 if filters
can combine all needed kinds. The 1,024-event ring, explicit event gap, progress
frames, 32-record scan and client detach on bounded output pressure are sound.

M9B must:

- use the same DTOs for query, snapshot and event representations;
- add resource, acquisition, Recorder and configuration/lifecycle event kinds;
- include subscription identity on event/progress/error frames;
- return complete cursor bounds on asynchronous `event_gap` too;
- document at-most-once socket delivery, replay-after-cursor semantics and possible
  duplicates after reconnect;
- retain no server-side subscriber state after disconnect;
- make overflow detach/gap behavior observable without blocking the owner.

No per-event ACK protocol is required. The applied cursor is client-owned; replay is
from the bounded server ring.

### 5.6 Reference API

Current ramp query and optimistic `reference_retune` are correct. Discovery is
hard-coded and there is no public route to Core's complete checked
`ReconfigureReference` capability.

Add a full `reference_configure` operation with expected revision and a complete
bounded `fixed` or `ramp` candidate. Units must match declared dependencies and all
finite/range validation remains Runtime-owned. Retain `reference_retune` as the
continuous ramp-specific operation; it is not merely a naming alias.

### 5.7 Controller/PID API

The API exposes complete current native configuration and PID-only optimistic update,
plus start/pause/resume. These transitions correctly remain behind Runtime commands
and OutputAuthority.

Required additions/clarifications:

- dynamic discovery and target-specific lifecycle capabilities;
- bounded failure reason/status, input/reference/output bindings and dependency
  identities;
- a checked complete controller configuration operation only for changes the Core
  already safely supports;
- deliberate failed-controller recovery backed by
  `Command::ResetFailedController`, with acknowledged preconditions;
- explicit semantics that `pause` is the safe non-running state.

Core has no distinct `Stopped` state or stop command. M9B must not advertise a fake
`controller_stop` alias. If a separate permanent stop is later proven necessary, it
requires a domain lifecycle change and focused safety review; it is not needed to
make the present start/pause/resume contract honest.

The client never owns a lease, epoch or safe-evidence transition. A read-only output
status may expose evidence for diagnosis, but no generic output mutation is added.

### 5.8 Recorder API

Start/stop are correctly asynchronous and terminal only after the durability barrier.
Status exposes run/interval identity, coverage/failure and useful health. History
runs on the Recorder worker, not the Runtime owner.

The public semantic status should retain:

- enabled policy and lifecycle state;
- database/archive identity, current run and interval;
- durable watermark/coverage/gap state;
- first bounded failure and clean/sealed/closed state;
- whether required recording currently permits experiment progress.

Raw queue credits, WAL byte counts and checkpoint counters are bounded diagnostics,
not the primary Recorder semantic contract. Place them in an optional `diagnostics`
object/capability. Never expose a SQLite path as an authority token or require SQL
knowledge for live control. `experiment_annotate` must distinguish ingress accepted
from durable committed; its current immediate result says `durability: pending` and
must not be mistaken for a durable terminal fact.

### 5.9 Resources, transports and reconnect

The current frozen snapshot has useful bounded resource state: idle/in-flight/
recovering/offline, queue length, active transaction, generation and latest outcome.
There is no direct query or discovery record and resource changes are not replayed.

Add `resource` status and resource discovery with semantic type, physical/virtual
classification, read-only/output capability, bound instruments, state, binding
generation and supported operations. Do not expose COM handles or allow port
enumeration through this API.

Keep `reconnect_resource(resource, expected_binding_generation)`. Preserve one public
operation, no blind reconnect retry, old-worker retirement, actual-ready-before-Core-
rebind and generation fencing. Map conflict, transient/terminal transport failure,
timeout, recorder-unavailable and invalid target distinctly. The existing bounded
reconnect diagnostic can remain optional result detail.

### 5.10 Configuration

Current public stage/reload always rereads the trusted startup path. The API cannot
query active revision, source identity, staged candidate expiry or structured
properties. Staged effects are emitted with Rust `Debug` spellings.

M9B should expose:

- `configuration_status`: active revision, deployment identity/hash, source kind,
  optional staged candidate identity/base revision/expiry and typed effects;
- generic read-only property descriptors derived from existing parameter descriptors
  and deployment metadata;
- a checked configuration-only property operation for existing targets where the
  domain already supports safe live mutation;
- existing `stage_configuration`, `apply_configuration` and
  `reload_configuration`, explicitly documented as trusted-file lifecycle operations.

No arbitrary filesystem path or TOML text is accepted over the wire in v0.1. A
property edit must use the same validation, candidate, revision and safe-barrier
lifecycle. Output-affecting parameters are never admitted by the generic property
operation. Persistence expectations must be explicit: a live property change is not
silently claimed to have rewritten the deployment file.

### 5.11 Virtual instruments and external emulator boundary

Current `VirtualInstrument` and `ThermalPlantInstrument` are trusted native models
scheduled by the host. `restart_models` can restart thermal plants and managed
components together. There is no wire operation that publishes an external emulated
sample, and no configured external-emulator instrument kind.

The smallest honest v0.1 external-emulator design is:

1. Add an explicit trusted deployment kind for an externally driven emulated
   instrument. Its descriptor, allowed signals, units/ranges, history bound and
   generation are validated before activation.
2. Add `emulator_publish` for exactly one declared emulated signal per operation.
   Fields are instrument, parameter, expected generation, scalar value or a bounded
   virtual-unavailable reason, and an optional bounded producer sequence for caller
   deduplication. Runtime assigns authoritative publication time; supplied wall time
   is never control time.
3. Add target-specific `virtual_fault_inject` only for explicitly virtual targets and
   allowlisted deterministic failure modes. It cannot target physical instruments.
4. Add `virtual_model_restart` for one native model under expected generation.
   Managed component restart remains a separate target-specific operation.

Strict request schemas admit no ACK, readback, transport result, output-safe evidence
or raw physical quality claim. The host verifies the target's trusted virtual kind
before creating a sample. An emulator's `good` means a virtual observation only;
provenance records that origin. External emulator work enters a bounded owner mailbox
and is lower priority than required acquisition/control/safety/Recorder work.

### 5.12 Operations and errors

Current operation correlation and outcome retention are valuable. The error mapping
is too coarse: many Core errors become `domain_rejected`, and most lifecycle errors
become `invalid_configuration`. Error strings contain no bounded semantic detail.

The final error object is:

```json
{
  "type": "error",
  "code": "revision_conflict",
  "category": "conflict",
  "message": "bounded stable explanation",
  "accepted": false,
  "retryable": false,
  "resync_required": false,
  "details": {"expected_revision":"4","current_revision":"5"}
}
```

Only allowlisted bounded detail fields are permitted. Never echo raw frames,
filesystem errors, OS strings or unbounded diagnostics.

Required categories and representative stable codes:

| Category | Representative codes |
|---|---|
| invalid request | `invalid_args`, `invalid_id`, `invalid_cursor`, `frame_too_large` |
| unsupported | `unknown_operation`, `unsupported_for_target`, `recorder_disabled` |
| not found | `unknown_resource`, `unknown_instrument`, `unknown_signal`, `unknown_controller`, `unknown_reference`, `unknown_component` |
| conflict | `revision_conflict`, `generation_conflict`, `request_id_conflict`, `scope_in_use` |
| unavailable | `input_unavailable`, `transport_unavailable`, `recording_unavailable`, `component_unavailable` |
| timeout | `operation_timeout`, `history_timeout`, `transport_timeout` |
| capacity | `busy`, `scope_capacity`, `subscription_capacity`, `snapshot_capacity`, `history_capacity` |
| resynchronization | `instance_changed`, `event_gap`, `snapshot_expired`, `outcome_unknown` |
| safety/lifecycle | `invalid_state`, `safe_barrier_failed`, `output_rejected`, `shutdown_in_progress` |
| internal | `internal_error`; bounded correlation only, no hidden success |

Before acceptance, errors have `accepted:false`. After acceptance, the operation
terminal frame carries the same structured error under `state:"failed"`; clients do
not infer retry safety from a transport disconnect.

### 5.13 Reconnect and resynchronization

Within one Runtime boot a client can resume its scope and query retained operation
outcomes. Across a process restart the old scope and event cursor correctly become
`instance_changed`; authoritative state is reconstructed, not resurrected from the
client.

The final algorithm is:

```text
connect
-> hello (negotiate v1, create/resume scope)
-> reconcile every uncertain request_id with operation_status
-> page discovery at a catalog/configuration revision
-> obtain complete runtime_snapshot at cursor C
-> subscribe after C
-> apply ordered events/progress
-> use Recorder history with reconnect-safe cursor for missed durable measurements
-> on event_gap/instance_changed: discard transient projection and repeat
```

On a new boot, operation outcomes may be unknowable while durable semantic results
remain discoverable in current state or Recorder history. The API must say
`outcome_unknown`, never "not executed". Client reconnect does not pause or restart
acquisition, control, Recorder, resources or models.

## 6. Proposed final v0.1 Application API surface

The following is the target semantic surface. Existing names are retained unless
there is a concrete semantic reason to change them.

### Protocol and recovery

- `hello`
- `operation_status`
- `runtime_snapshot`, `snapshot_page`, `snapshot_release`
- `subscribe`, `unsubscribe`
- `runtime_shutdown`

### Discovery and current state

- paged `discover`
- `describe` (instrument descriptor)
- `instrument_state`
- `resource`
- `latest`
- `signal_window`
- `reference`
- `controller`
- `component`
- `output` as read-only Runtime-owned status
- `configuration_status`
- `recording_status`
- `virtual_model_status`

### Reference and native control

- `reference_retune`
- `reference_configure`
- `controller_configure_pid`
- checked complete `controller_configure` where current Core supports it
- `controller_start`, `controller_pause`, `controller_resume`
- `controller_reset_failure`

No generic manual output write or client-owned OutputAuthority operation is added.

### Recorder and history

- `recording_start`, `recording_stop`
- `experiment_annotate`
- `history_read`, `history_page`, `history_release`

### Configuration and resources

- `stage_configuration`, `apply_configuration`, `reload_configuration`
- bounded configuration-only `property_configure`
- `reconnect_resource`

### Managed and virtual lifecycle

- targeted `component_restart`
- targeted `virtual_model_restart`
- `emulator_publish`
- target-specific `virtual_fault_inject`

`restart_models` is retired after migration because it combines independent managed
and virtual branches. `reload_managed_sources` remains only as a temporary
text-adapter operation until M9C removes Lua; it is not part of the final v0.1 API.

## 7. Capability and version model

`hello` should return a structured, actual-composition view:

```text
protocol: {major: 1}
application_api: {version: "0.1"}
server: {boot_id, state}
session: {scope, next_seq, retention}
capabilities:
  discovery/history/subscriptions/recorder/configuration/reconnect/emulator...
limits: complete bounded protocol limits
event_window: {oldest, latest}
```

Capabilities are stable semantic IDs with versions and optional limits, not Rust type
names or backend names. `recorder_sqlite_v1` becomes a public `recorder_history.v1`
capability; SQLite format identity belongs to archive metadata/documentation.
`history_raw_paged_v1` becomes `measurement_history.v1`. `native_controller` and
`ramp_reference` become discoverable target kinds/capabilities rather than global
claims that object 1 exists. No public Lua or Babashka name currently exists; none is
added.

## 8. Explicit bounds

Existing bounds remain unless a row explicitly requires an M9B correction.

| Surface | Current bound | Required v0.1 rule |
|---|---:|---|
| inbound/outbound NDJSON frame | 16,384 bytes including LF | keep; oversize affects only the exchange/client |
| JSON nesting / lexical values / string | 16 / 1,024 / 512 bytes | keep and advertise |
| clients / owner-reactor mailbox | 8 / 64 | keep |
| per-client requests / replies / events | 8 / 8 / 16 | keep; full queue detaches client |
| hello, partial-frame, blocked-write deadline | 2 s absolute | keep and advertise as transport policy |
| scopes | 16 | keep |
| pending operations | 8 per scope, 64 total | keep; no disconnect leak |
| terminal outcomes | 32 per scope, 256 total, 600 s | keep; advertise epistemic eviction |
| detached idle scope | 1,800 s | keep |
| retained terminal result | 4,096 bytes | keep |
| event ring / event record | 1,024 / 4,096 bytes | keep |
| event pump | scan 32, offer 4 | keep; owner remains bounded |
| subscriptions | 1 per connection; 8 kinds; 16 targets | keep one aggregate subscription; expand allowlisted kinds without raising counts |
| network sweep | 8 KiB, at most 4 frames/read pass | keep |
| snapshot | currently 1/client, 256 KiB, 4 KiB/record, 8 KiB/page, 5 s | retain 1/client, 4 KiB/record, 8 KiB/page and TTL; raise/derive total to cover every admitted public fact (approximately 5 MiB worst-case under current domain maxima), then test exact maximum |
| discovery page | absent | at most 64 entries and at most 8 KiB encoded; stable ordering/revision |
| recent signal window | absent publicly; Core 1..=4,096 | return at most 128 attempts per query and report truncation/capacity |
| history jobs | 8 Recorder jobs; one pending/page per client | keep; every accepted job reaches terminal state |
| history selection | measurements 1..=128 rows; runs 1..=32 | keep row maxima and additionally fill to <=8 KiB page |
| history worker deadline | 2 s | keep absolute; timeout is terminal, not silent retry |
| history page / cursor retention | 5 s / 30 s, connection-local | retained page may stay 5 s; continuation becomes reconnect-safe and filter/watermark-bound |
| recording label | 128 UTF-8 bytes | keep |
| annotation | name 64; data 2 KiB, depth 4, 64 nodes | keep and advertise |
| property value | Core scalar bounds; text max 1,024 bytes | reuse descriptor bound; no arbitrary object graph |
| emulator publication | absent | one signal attempt per operation; scalar descriptor bounds; at most 8 pending emulator operations globally and one pending per declared instrument; overload returns `busy` without blocking owner |
| error message/detail | fixed messages today | message <=256 bytes; allowlisted detail <=2 KiB and within 16 KiB frame |

The snapshot figure is a design ceiling to verify from the exact maximum number of
public facts. Implementation must use a checked derived constant and a whole-system
memory test; it must not simply allocate an unbounded vector or reject a valid
maximum deployment because the old demo-sized 256 KiB cap was exceeded.

## 9. Required implementation sequence

Dependencies in the source suggest this sequence.

### M9B.2 — protocol, operation and error foundation

1. Introduce typed public DTO builders shared by query/snapshot/event paths.
2. Add structured error categories/details and preserve stable current codes where
   they are already precise.
3. Make unknown operations nonfatal and outgoing encode/size failure client-scoped.
4. Expand hello versions/capabilities/limits from actual host composition.
5. Fix history disconnect terminalization and unusable retained page outcomes before
   adding more asynchronous work.

### M9B.3 — discovery, current measurements, history and subscriptions

1. Replace hard-coded discovery with paged owner-derived catalogs.
2. Add `instrument_state`, `resource`, acquisition state and `signal_window`.
3. Unify sample/unit/time/failure/generation DTOs.
4. Complete the frozen snapshot and event kinds for all resynchronizable state.
5. Make archive cursors reconnect-safe and byte-aware.

### M9B.4 — Reference and controllers

1. Add complete Reference configuration under revision fencing.
2. Complete controller discovery/status and safe supported configuration.
3. Add deliberate failed-controller reset.
4. Prove no operation bypasses OutputAuthority or rearms implicitly.

### M9B.5 — Recorder public contract

1. Separate semantic Recorder status from optional bounded storage diagnostics.
2. Clarify annotation ingress versus durable completion.
3. Normalize run/interval/archive identity and public history terminology.
4. Add Recorder state events needed for resynchronization.

### M9B.6 — resources, configuration and reconnect

1. Add configuration status/revision/candidate projection and typed diff effects.
2. Add bounded generic configuration-only property mutation through stage/apply
   validation.
3. Complete resource discovery/status/events and typed reconnect failures.
4. Retain the accepted M8 reconnect and no-rearm invariants unchanged.

### M9B.7 — virtual and emulator API

1. Add explicit configured external-emulator identity and Core/Host admission.
2. Add bounded `emulator_publish` and allowlisted virtual fault injection.
3. Split target-specific virtual-model and component restart semantics.
4. Prove physical targets and evidence fields are structurally unreachable.

### M9B.8 — reconnect, backpressure and adversarial acceptance

Exercise maximum catalogs/snapshots, slow/nonreading clients, event gaps, scope and
operation eviction, process restart, history reconnect, malformed input, oversized
valid results, emulator flood, required Recorder failure and finite shutdown. Prove
required native work progresses throughout.

### M9B.9 — final API documentation and external review

Freeze the v0.1 operation/DTO/error/capability reference, update active coordination
and write the M9B implementation report. Run full debug/release/fmt/Clippy/rustdoc and
raw protocol/integration acceptance. Stop for explicit M9B external review; do not
begin M9C automatically.

## 10. Required tests

At minimum, M9B implementation needs deterministic tests for:

1. every advertised operation being accepted by the wire schema and every
   composition-dependent capability being truthful;
2. unknown operations returning a nonfatal structured error;
3. malformed/oversized input and oversized output affecting only one client;
4. maximum admitted discovery and snapshot state paging without owner failure;
5. discovery containing no hard-coded object IDs and correctly classifying physical,
   virtual, emulated and managed instruments;
6. latest/window/snapshot/event using identical sample quality, failure, unit, time
   and generation semantics;
7. empty, unavailable, gap, ordering and truncation behavior for recent and durable
   history;
8. history cursor continuation after TCP reconnect and explicit rejection after
   archive/filter/watermark mismatch;
9. disconnect during pending history producing a retained terminal outcome and
   releasing all job/session capacity;
10. slow subscriber detachment, explicit gap/resnapshot and no acquisition/control/
    Recorder starvation;
11. Reference revision conflict and full fixed/ramp validation;
12. controller configuration/lifecycle/reset with no client-created authority or
    automatic rearm;
13. Recorder start/stop/failure/coverage/run/interval semantics and annotation
    durability wording;
14. configuration candidate expiry/conflict/safe barrier/property validation with no
    partial apply or file-persistence claim;
15. resource offline/reconnect/generation/error events while preserving all M8
    ordering and bounded-open invariants;
16. emulator publication only to declared emulated signals, with unit/range/
    generation/dedup/flood checks;
17. emulator requests being unable to express physical ACK/readback/transport/safe
    evidence and unable to target physical instruments;
18. client disappearance leaving acquisition, native control, Recorder and Runtime
    lifecycle unchanged;
19. clean finite shutdown from every API pending state;
20. no public presentation terms or Lua/Babashka-specific operation names in the
    final advertised v0.1 surface.

Existing protocol, session deduplication, client isolation, subscription recovery,
operation outcome, Recorder history, configuration reload, reconnect and hardware-
independent M8 tests remain regression gates.

## 11. Naming and baggage decisions

There is no active public `lua_*` or Babashka-specific operation. The remaining names
requiring deliberate treatment are:

| Name | Decision | Reason |
|---|---|---|
| `reload_managed_sources` | transitional; remove in M9C | truthful only for text-backed adapter artifacts; not a final native-only product concept |
| `managed_source` capability | normalize as a component kind/capability or remove if no active source component remains | currently ambiguous between a virtual source component and executable source text |
| `recorder_sqlite_v1` | rename semantic capability | backend format is not the Application API contract |
| `history_raw_paged_v1` | rename semantic capability | `raw` describes storage/query implementation rather than measurement semantics |
| `restart_models` | replace | crosses managed-component and native virtual-model branches globally |
| `output` | keep as query, document read-only status | DTO exposes Runtime-owned evidence; no client authority is conferred |
| `get`/`query` prefixes | do not churn | current nouns and explicit mutation verbs are already understandable |

No Presentation API or M9C removal work is part of these changes.

## 12. Out of scope

M9B does not include:

- GUI, Presentation API or server-owned presentation state;
- workspace, plot, trace, button, panel, tab, color, layout or window concepts;
- a first-party client, SDK, Babashka/Python product work or scripting environment;
- Steel;
- Lua removal or historical provenance/archive rewriting;
- raw serial handles, port enumeration or generic physical writes;
- SQLite schema as the live API;
- diagnostic logging redesign except bounded API error/health surfaces directly
  required here;
- broad Core/module cleanup, packaging or release audit;
- COM5 or any hardware acceptance.

## 13. Audit conclusion

The existing API is a credible bounded foundation, not a throwaway prototype. M9B
should extend and normalize its single Application boundary rather than create a new
server or duplicate domain lifecycle.

```text
M9B: AUTHORIZED
M9B implementation: NOT STARTED
M9C+: NOT AUTHORIZED
```
