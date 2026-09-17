# lab-runtime — current product brief

## Goal

`lab-runtime` is a long-running Rust laboratory automation Runtime.

It owns authoritative experiment state and combines:

- physical and virtual instruments;
- acquisition;
- signal processing;
- native References/controllers;
- central output safety;
- durable recording/history;
- a language-neutral Application API;
- external interactive automation;
- a separate GUI client.

The architecture should remain understandable enough to study and small enough for
real laboratory workloads.

## Runtime ownership

The Rust Runtime is the single authoritative mutable experiment owner.

Clients and adapters do not own critical experiment state.

```text
GUI
Babashka
future embedded scripting
tests
        ↓
language-neutral Application API
        ↓
Runtime
```

Client lifetime is not experiment lifetime.

Queries return committed snapshots and do not perform hidden physical refresh.

Commands/Operations perform mutation or work.

## First stable release: v0.1.0

v0.1.0 should provide:

- long-running headless Runtime;
- declarative deployment configuration;
- validated/staged lifecycle changes;
- real read-only Windows COM support;
- conservative Metakon 513 read path;
- Runtime-owned Recorder + SQLite;
- bounded raw history queries;
- native Reference/PID/controller lifecycle;
- public local Application API;
- optional Babashka client;
- server-owned semantic presentation/workspace state;
- separate Rust eframe/egui/egui_plot GUI;
- contextual control panels;
- Properties/configuration editing through the normal lifecycle;
- clear user/developer documentation and tutorials;
- Windows packaging and checksums.

v0.1.0 does not claim physical actuator qualification.

## Managed components

The language-neutral managed-component boundary is useful independent of Lua.

Conceptually:

```text
Runtime
  ↓ Invocation
managed component
  ↓ ComponentResult
Runtime validates/commits
```

The current Core execution seam includes language-neutral data and executor concepts
such as `Invocation`, `ComponentResult`, `ComponentCompletion`, `ComponentExecutor`
and `PlainData`, together with generation, revision and failure semantics.

### Current Lua role

M5 implemented bounded disposable Lua model/filter/transform components.

For v0.1 this subsystem is FROZEN.

M5 Lua is not currently known to violate the safety boundary. Its existing sandbox
denies raw transport, OutputAuthority, physical ACK/readback/safe evidence and
general Runtime mutability.

M9A removed the Lua-specific leakage from common definitions, public capability and
lifecycle names, trusted composition helpers and new provenance. Historical
`managed_lua_source` SQLite evidence remains unchanged; new activations use neutral
managed-component provenance.

Allowed changes:

- bug/regression fixes;
- safety/security fixes;
- documentation corrections.

Not planned before v0.1:

- persistent Lua application workspace;
- Lua scenario/orchestration API;
- Lua GUI API;
- Lua REPL/editor;
- broader M5 component features.

### Native Rust components

Native Rust components now have a first-class implementation path through the same
managed-component execution contract. The reference implementation is the bounded
`native.moving_mean.v1` transform; frozen Lua uses `lua.v1` through the same executor.

This gives a simple future migration:

```text
M5 Lua model/filter/transform
        ↓
equivalent native Rust component
        ↓
remove the Lua implementation after replacement coverage exists
```

Recompilation is acceptable for trusted internal algorithms and models.

A future complete `lab-lua` removal remains a separate post-release decision.

## User-facing automation

User-facing experiment automation should use one semantic Application API.

Babashka is the current external interactive/orchestration client.

A future embedded scripting language may expose the same Application semantics through
an in-process adapter.

The intended post-v0.1 candidate is Steel, but Steel is not part of the v0.1
commitment and is not currently authorized.

No scripting language receives raw Runtime mutability, OutputAuthority or physical
evidence capabilities.

## Virtual instruments and emulators

The repository already has two useful native virtual foundations:

- `VirtualInstrument`, the simple deterministic/fault-injection virtual source;
- `ThermalPlantInstrument`, the built-in stateful native thermal
  emulator/reference model.

They remain in the architecture and are not scheduled for removal or replacement.
M9B adds a validated virtual-instrument/emulator part of the Application API around
this foundation rather than rewriting it.

This enables the same emulator procedure to be expressed from Babashka now and from
a future embedded language later.

Only explicitly declared virtual/emulated instruments may accept virtual model
publication.

A script/client may never manufacture facts for a physical instrument or fabricate
ACK/readback/safe/transport evidence.

## Instruments and transports

The trusted architecture keeps these concerns separate where useful:

```text
Transport
  ↓
Protocol
  ↓
Instrument semantics
```

Runtime owns physical transport resources and scheduling.

Data-driven instrument definitions may describe known protocol primitives.

New low-level protocols, complex drivers and reliability-sensitive paths remain
native Rust.

## Signals, References and controllers

Signals carry typed values, unit metadata, quality and explicit time.

Current native control includes:

- independent Reference;
- EMA processing;
- native PID;
- controller lifecycle;
- bounded native output leases.

Controllers do not write hardware directly.

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

## Output safety

Rust owns physical output authority.

Keep separate:

```text
requested
authorized
send started
ACK
readback
physical effect
```

ACK is never silently promoted to readback.

A timeout after possible output transmission does not prove that nothing happened.

No client, GUI, Lua component or future Steel script may fabricate physical evidence.

## Recorder

Recorder is Runtime-owned.

SQLite is a storage adapter, not the domain model.

Recording and history survive client disconnects.

Disk I/O must not block native safety/control progress.

Required recording follows the accepted fail-closed behavior.

## Application API

The Application API should be the semantic boundary used by GUI, Babashka and future
embedded scripting.

It should cover, as required by product work:

- discover/describe;
- current snapshots;
- subscriptions/events;
- bounded history;
- Reference operations;
- PID/controller lifecycle/configuration;
- recording;
- reconnect/resource status;
- configuration stage/apply/reload;
- virtual/emulator operations;
- presentation/workspace operations;
- control-panel operations;
- properties/configuration editing.

Wire DTOs are adapters and must not become the Core domain model.

## Presentation and GUI

Presentation is server-owned semantic state, separate from safety-critical state.

The target presentation model supports:

- workspaces;
- plot panels;
- traces;
- labels;
- colors;
- visibility;
- ordering;
- panel assignment;
- time windows;
- follow/live;
- axis policy;
- control panels;
- logs.

The GUI is a separate Rust client using the public Application API.

Required donor plotting behavior to preserve/adapt:

- multiple plot panels;
- multiple traces per panel;
- local-time/time-axis formatting;
- pan/zoom;
- follow/live;
- auto/manual Y;
- double-click autoscale;
- legend;
- trace color/visibility/labels;
- ordering/panel assignment;
- configurable time window;
- plot sizing;
- downsampling;
- signal/series sidebar.

Contextual controls should include:

- live signal/value display;
- Reference editor/display;
- PID configuration/status;
- controller start/pause/resume;
- recording start/stop/status;
- resource/instrument status;
- reconnect;
- logs;
- Properties.

No scenario menu is required for v0.1.

Properties edits must produce structured validated candidates and use the normal
validate/stage/apply lifecycle.

## External API and Babashka

The first API is local-first and not an Internet security claim.

Babashka is optional and not a Runtime startup dependency.

Babashka is appropriate for:

- REPL-driven exploration;
- experiment procedures;
- supervisory orchestration;
- emulator scripts;
- Application API learning;
- presentation/workspace manipulation.

Disconnecting Babashka does not stop Runtime-owned native acquisition/control unless
an explicit accepted safety policy requires it.

## Current release path

```text
M8  ACCEPTED
M9A READY_FOR_M9A_EXTERNAL_REVIEW — neutral boundary + native Rust path implemented
M9B NOT AUTHORIZED until M9A acceptance
M10 future GUI milestone
M11 future hardening, documentation, tutorials and packaging milestone
v0.1.0
```

Current phase: external M9A review. The completed M8 gate requires no further
hardware rerun; COM5 is not part of current work.

The v0.1 documentation teaches native Rust managed components and the native
virtual/emulator foundation plus Application API and Babashka use. It does not teach
a Lua application workflow. `MILESTONE_5_DESIGN.md` and `MILESTONE_5_REPORT.md`
remain historical engineering evidence.

Post-v0.1 work may include embedded Steel over the same Application API, optional
later Lua removal, additional controllers and richer instruments.
