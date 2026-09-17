# lab-runtime — current product brief

## Product goal

`lab-runtime` v0.1 is a polished headless laboratory automation Runtime written in
Rust. It owns authoritative experiment state and provides native instruments,
periodic acquisition, measurement signals and bounded history, native control,
central output safety, durable recording, virtual/emulated instruments and one
documented local Application API.

The release includes:

- `lab-core` and `lab-runtime`;
- native Rust instruments and managed components;
- periodic instrument polling and measurement publication;
- latest values and bounded/paged history;
- Reference and native PID/controller lifecycle;
- OutputAuthority and output-safety enforcement;
- Recorder/SQLite experiment history and provenance;
- configuration, resource and reconnect lifecycle;
- virtual instruments and emulator support;
- a complete local Application API;
- bounded diagnostic logging;
- professional English documentation and Windows packaging.

The release does not include a GUI, Presentation API, plotting/workspace concepts,
Steel, Lua, Babashka or Python clients, a first-party frontend, a client SDK, a
bundled plotting application or a user-facing scripting environment.

v0.1 does not claim physical actuator qualification or generic hardware safety
certification.

## Fundamental boundary

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The local Application API exposes laboratory and Runtime concepts. It does not expose
workspace, plot, trace, button, panel, tab, color, layout or window concepts. Any
client presentation is outside `lab-runtime`.

The Rust Runtime is the sole authoritative mutable experiment owner. Client lifetime
is not experiment lifetime. Queries return committed snapshots without hidden
physical refresh. Commands and Operations perform mutation or work through explicit
Runtime-owned lifecycle rules.

## Runtime priority model

Periodic native acquisition is a primary Runtime responsibility:

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

This is an architectural scheduling priority, not a Windows thread-priority claim.
Slow, stalled or disconnected managed components and API clients must not prevent
required acquisition, native control, safety or Recorder progress.

## Managed components

The implementation-neutral managed-component path remains:

```text
Runtime
  ↓ Invocation
ComponentExecutor
  ↓ ComponentCompletion / ComponentResult
Runtime validation
  ↓
committed managed state and signal
```

The contract includes `Invocation`, `ComponentResult`, `ComponentCompletion`,
`ComponentExecutor`, `PlainData` and generation/revision/failure semantics. M9A added
the bounded native `native.moving_mean.v1` reference implementation and is externally
accepted.

M9C removed the active Lua implementation, executable-text artifacts, Lua
configuration/API vocabulary and obsolete first-party client. Historical milestone
reports, immutable archives and historical provenance such as `managed_lua_source`
remain evidence and are not rewritten.

## Instruments, acquisition and signals

Runtime owns transport scheduling and periodic instrument polling. The trusted
boundary remains:

```text
Transport
  ↓
Protocol
  ↓
Instrument semantics
```

Signals carry typed values, engineering units, quality, generation/revision where
semantically required and explicit time. Latest values, live events and bounded
history are distinct query surfaces. New low-level protocols, complex drivers and
reliability-sensitive paths remain native Rust.

## Reference, control and output safety

Current native control includes independent Reference, EMA processing, PID,
controller lifecycle and finite output leases. Controllers never write hardware
directly:

```text
Controller
  ↓
OutputProposal
  ↓
OutputAuthority
  ↓
trusted operation mapping
  ↓
Transport
```

Keep requested, authorized, first possible output byte, acknowledged, readback and
physical effect distinct. ACK is not readback. Timeout after possible transmission
does not prove no physical effect. There is no blind retry after ambiguous output and
no automatic rearm after safe transition, reload or reconnect.

## Virtual instruments and emulation

`VirtualInstrument` remains the deterministic/fault-injection virtual source.
`ThermalPlantInstrument` remains the built-in stateful thermal emulator/reference
model. M9B adds validated Application API operations around this native foundation.

Only explicitly declared virtual/emulated instruments accept virtual publication.
An emulator or client can never fabricate a physical observation, ACK, readback,
transport completion or safe-output evidence.

## Recorder and diagnostics

```text
Recorder / SQLite
= durable scientific + experiment audit history

Diagnostic logs
= bounded troubleshooting information
```

Recorder stores semantic experiment facts: measurements, quality/status transitions,
run and interval boundaries, controller and component lifecycle, configuration
revisions, resource/reconnect lifecycle, operations, gaps, provenance and other
structured experiment-relevant events. Required recording retains its accepted
fail-closed behavior.

Routine worker, TCP and debug messages do not become unbounded Recorder content.
Diagnostic logging has separate bounds, levels, rotation/retention and collection
guidance. A semantically important diagnostic may also be a structured durable
Runtime event.

The SQLite archive is documented before release, including every active table,
important columns and relationships, run/interval and measurement models, operations,
object snapshots, provenance, component source/build identities, gaps, sealing,
shutdown state, timestamps, invariants, content-addressed deduplication, migration
policy and example read-only SQL.

```text
Application API != Recorder contract != SQLite schema
```

SQLite is a documented archival format, not the live control interface.

## Application API

M9B completes one coherent local semantic API for:

- discovery of resources, instruments, signals, capabilities and properties;
- latest measurements, quality, units, acquisition state, live events and bounded
  history;
- Reference and controller/PID configuration, lifecycle and status;
- Recorder start/stop/status and public history/query surfaces;
- resource status, reconnect and validated configuration operations;
- virtual instruments, emulator publication and deterministic fault injection;
- coherent operations, structured errors, versioning, bounds, backpressure,
  reconnect/resynchronization and malformed-input behavior.

Wire DTOs are adapters and do not become the Core domain model. M9B contains no
Presentation API, frontend, client implementation or bundled scripting environment.

## Current release path

```text
M8   ACCEPTED — deployment, Windows COM and physical read-only evidence
M9A  ACCEPTED — neutral managed components and native Rust execution
M9B  ACCEPTED — coherent bounded Application API
M9C  READY_FOR_EXTERNAL_REVIEW — Lua and obsolete client baggage removed
M10  NOT AUTHORIZED — Core cleanup and studyability
M11  NOT AUTHORIZED — Runtime/Recorder/logging/API hardening
M12  NOT AUTHORIZED — documentation, packaging and final release audit
v0.1.0
```

The roadmap ends at v0.1.0. Milestones do not cross review gates automatically.

## v0.1 identity

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
