# Product roadmap to v0.1.0

`ai/HANDOFF.md` records current state. `ai/WORK.md` is the only detailed current
implementation authorization. This roadmap ends at v0.1.0.

## Status

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: READY_FOR_EXTERNAL_REVIEW
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3: COMPLETE
M9B.4: COMPLETE
M9B.5: COMPLETE
M9B.6: COMPLETE
M9B.7: COMPLETE
M9B.8: COMPLETE
M9B.9: COMPLETE
M9C: NOT AUTHORIZED
Current phase: M9B external review gate
```

Completed milestones M1-M9A established the domain, OutputAuthority, bounded
transport, native control, Recorder/SQLite, deployment/Windows COM/reconnect,
implementation-neutral managed components and native Rust managed execution.

## Product boundary through release

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

v0.1 is headless. The Runtime and Application API contain no Presentation API or
workspace, plot, trace, button, panel, tab, color, layout or window concepts. The
release contains no first-party frontend, client SDK or user-facing scripting
environment.

Required Runtime work has architectural scheduling priority over managed components
and API clients:

```text
authoritative / required Runtime work
├── transport scheduling
├── periodic instrument polling
├── measurement publication
├── native controller deadlines
├── OutputAuthority
├── Recorder ingestion
└── required lifecycle/safety work

non-authoritative extension/external work
├── managed components
└── API clients
```

This does not prescribe Windows thread-priority classes.

## M9B — Complete and stabilize Application API

Goal: expose complete useful Runtime semantics through one coherent bounded local
Application API.

M9B.1 audited the actual API. M9B.2 completed the shared protocol identity,
composition-aware operation/capability registry, bounded public error taxonomy and
operation lifecycle response foundation. M9B.3 completed discovery, current/recent
measurements, durable-history delivery cleanup, and live subscription semantics.
M9B.4 completed bounded Reference mutation/current state and native controller/PID
configuration, status, lifecycle and event projections without exposing output
authority.
M9B.5 completed semantic Recorder current state, start/stop durability boundaries,
stable run/interval identity, failure projection and bounded lifecycle events while
keeping SQLite below the Application boundary.
M9B.6 completed semantic resources, generic validated property metadata/mutation,
typed deployment stage/apply state and generation-fenced reconnect projections.
M9B.7 completed explicit virtual-only external measurement publication and separated
native virtual-model restart from managed-component lifecycle.
M9B.8 completed adversarial reconnect/resynchronization, bounded-pressure and
multi-client fault acceptance, including the final connection-local history-cursor
bound.
M9B.9 consolidated the source-derived operation/capability inventory, identity and
lifecycle contracts, error taxonomy, authoritative limits, transition list and
external-review checklist. M9B now awaits external review and is not self-accepted.

### Discovery

- resources and transports;
- instruments;
- signals/series;
- capabilities;
- properties;
- implementation-neutral status.

### Measurements

- latest values, quality and units;
- generation/revision where semantically needed;
- periodic acquisition state;
- live subscriptions/events;
- bounded and paged history.

### References and control

- Reference lifecycle/configuration;
- controller/PID configuration;
- supported start/pause/resume/stop operations;
- controller status;
- validated operation results.

### Recorder

- start/stop/status;
- current run/interval state;
- durable failure/coverage status;
- public bounded history/query surfaces.

### Resources, transports and configuration

- Runtime/resource status;
- validated configuration operations;
- reconnect;
- safe reload/rebind where applicable;
- explicit revision/conflict behavior.

### Virtual and emulator API

- explicitly declared virtual instruments;
- emulator publication endpoints/model steps;
- deterministic fault injection where appropriate;
- typed values/units/quality and generation/lifecycle fencing;
- strict separation from physical evidence.

An emulator can never fabricate physical ACK, physical readback, transport
completion or safe-output evidence.

### Protocol/API quality

- coherent operation naming and structured errors;
- capability discovery and protocol/API versioning;
- bounded request/response sizes;
- subscription backpressure;
- disconnect/reconnect and resynchronization;
- malformed/untrusted input and resource-exhaustion behavior;
- finite shutdown.

M9B contains no Presentation API, frontend/client implementation or bundled scripting
environment. Testing uses Rust, raw-protocol and integration tests.

## M9C — Remove Lua and obsolete client baggage

After explicit M9B acceptance, remove active Lua product code and dependencies where
no longer required:

- `lab-lua`;
- `mlua`;
- `lua.v1`;
- Lua runtime/sandbox;
- active Lua configuration surface;
- Lua fixtures and active API vocabulary;
- Lua user documentation.

Preserve historical milestone reports, immutable SQLite archives and historical
provenance such as `managed_lua_source`. Keep compatibility for historical evidence
inspection only when genuinely necessary.

Remove active first-party client baggage from the release product:

- Babashka client and user workflow;
- client SDK material;
- bundled external-client examples.

Replace required acceptance coverage with implementation-neutral Rust/protocol
integration tests.

Final active component model:

```text
Application API
      ↓
Runtime
      ↓
native instruments
native managed components
native control
Recorder
```

## M10 — Core cleanup and studyability

Goal: make the repository unusually easy to understand and study while preserving
behavior and safety. Refactor only under test coverage.

Targets:

- split oversized implementation files by clear responsibility;
- preserve one authoritative Runtime ownership model;
- make API request -> Runtime operation -> domain behavior easy to trace;
- make periodic acquisition and signal/series lifecycle explicit;
- make transport/instrument, controller, OutputAuthority and Recorder boundaries
  obvious;
- make native component extension points obvious;
- eliminate stale naming, dead compatibility code and unused abstractions;
- keep public/private boundaries deliberate.

A developer should be able to answer quickly:

```text
How is an instrument added?
How does periodic polling work?
How is a measurement series created?
How is latest/history state represented?
How does a controller consume data?
How is an output authorized?
How is data submitted to Recorder?
How does an API request reach Runtime?
How is a native managed component added?
```

Do not create speculative frameworks merely to improve the directory tree.

## M11 — Runtime / Recorder / logging / API hardening

Perform a systematic release-hardening pass.

### Acquisition

- deterministic monotonic scheduling and polling cadence;
- drift correction;
- bounded queues;
- slow transport and starvation behavior;
- finite shutdown;
- priority of required native work over extension/client work.

### Control and safety

- controller lifecycle;
- finite leases and generation/epoch fencing;
- OutputAuthority;
- ambiguous-write handling;
- no blind retry or automatic rearm.

### Recorder

- complete durable experiment history and provenance integrity;
- run/interval sealing and gaps;
- bounded ingestion and failure handling;
- crash/clean-close expectations;
- SQLite integrity behavior.

### Diagnostic logging

Implement and document a separate bounded diagnostics mechanism with explicit
destination, levels, size bounds, rotation/retention, startup/shutdown behavior and
user collection guidance.

```text
Recorder / SQLite
= durable scientific + experiment audit history

Diagnostic logs
= bounded troubleshooting information
```

Routine verbose implementation messages do not become unbounded Recorder content.
Semantically important diagnostics may also be structured durable Runtime events.

### Application API

Harden operation naming, errors, capabilities/versioning, response bounds,
subscription backpressure, disconnect/reconnect/resync, malformed input, resource
exhaustion and shutdown. Add fault/adversarial tests where useful.

## M12 — Documentation, packaging and final release audit

Documentation is a release gate. All active release documentation is clear,
professional technical English.

### Architecture and concepts

Document project/architecture overview, ownership, Runtime/domain separation,
instruments, signals/series, periodic acquisition, Query/Command/Operation, time,
transport, Reference, PID/controllers, OutputAuthority, Recorder/provenance, native
managed components, virtual/emulated instruments, Application API and diagnostics.

### Tutorials

Provide complete step-by-step tutorials to:

1. build the project;
2. run `lab-runtime`;
3. configure and understand an instrument;
4. understand polling and measurement series;
5. inspect latest measurements through the raw protocol/API;
6. query history;
7. subscribe to live data at protocol level;
8. configure/use Reference;
9. configure/use PID with a virtual plant;
10. start/stop Recorder;
11. inspect the SQLite archive;
12. inspect provenance;
13. configure/use a virtual instrument/emulator;
14. add a native Rust instrument;
15. add a native Rust managed component;
16. understand diagnostic logs;
17. diagnose common failures.

Tutorials may use raw protocol interaction. Do not create a first-party client SDK
for tutorials.

### Reference documentation

Document the Application API, protocol/versioning, operations, errors, capabilities,
configuration schema, instrument definitions, native component contract, logging,
shutdown/recovery guarantees and supported platform assumptions.

Fully document the Recorder SQLite archival format:

- purpose and schema version;
- every active table and important column;
- primary and foreign relationships;
- run/interval and measurement models;
- operations/events and object snapshots;
- provenance and component implementation/source/build identities;
- gaps, sealing/completeness and shutdown state;
- timestamp/time semantics and invariants;
- content-addressed deduplication;
- migration/version policy;
- example read-only SQL queries.

```text
Application API != Recorder contract != SQLite schema
```

SQLite is a documented archival format, not the live control interface.

### Rust documentation

Provide useful rustdoc for public APIs and important ownership, invariant, bound,
failure and safety seams. Avoid obvious restatements of Rust syntax.

### Packaging

Prepare a minimal Windows release bundle:

```text
lab-runtime.exe
example configuration
instrument definitions
README/documentation entry point
LICENSE if applicable
checksums
```

Do not ship Lua, Babashka/Python clients, GUI, Steel, development-only fixtures or
unrelated historical evidence archives. Perform clean Windows release acceptance.

## v0.1.0

```text
lab-runtime v0.1

A polished headless laboratory automation runtime with:
- periodic instrument acquisition,
- measurement signals and history,
- native control and output safety,
- durable recording and provenance,
- native Rust extensibility,
- virtual/emulated instruments,
- a documented local Application API,
- bounded diagnostic logging,
- comprehensive English documentation.
```

The roadmap ends here.
