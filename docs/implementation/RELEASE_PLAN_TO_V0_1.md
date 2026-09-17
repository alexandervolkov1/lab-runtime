# lab-runtime — release plan to v0.1.0

## Status

Completed and accepted baseline:

```text
M1-M8
```

Current phase:

```text
M8: ACCEPTED
M9A: IMPLEMENTED
STATUS: READY_FOR_M9A_EXTERNAL_REVIEW
Current phase: external M9A review
```

M9B remains NOT AUTHORIZED until M9A receives explicit external acceptance. M10 and
M11 remain future milestones.

This plan is product direction, not automatic authorization to cross review gates.

## v0.1 product definition

v0.1.0 is a usable laboratory automation Runtime with:

- long-running Rust owner;
- declarative deployment configuration;
- read-only real Windows COM/Metakon integration;
- Recorder/SQLite durable history;
- native Reference/PID/controller lifecycle;
- local public Application API;
- optional Babashka;
- validated virtual/emulator API;
- server-owned presentation/workspace model;
- separate Rust GUI client;
- contextual control panels and Properties;
- clear documentation/tutorials;
- Windows packaging.

v0.1 does not claim physical actuator qualification.

## Cross-cutting invariants

Preserve through release:

```text
Runtime is the sole authoritative mutable experiment owner
Query = committed snapshot / no hidden I/O
Command/Operation = mutation or work

client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
Babashka lifetime != native controller lifetime

central OutputAuthority
ACK != readback != physical effect
final authority recheck before first possible output byte
no blind retry after ambiguous output

finite leases/recovery
reload != rearm
safe transition != rearm

bounded queues/histories/workers
monotonic time for control/safety
wall time for human history/display

Recorder is Runtime-owned
disk/network/GUI/scripts do not block required native safety progress

lab-core remains OS/GUI/storage/scripting-runtime independent
```

## Managed-component direction

The language-neutral managed-component execution contract remains part of the product
architecture.

It retains `Invocation`, `ComponentResult`, `ComponentCompletion`,
`ComponentExecutor`, `PlainData` and their generation/revision/failure semantics.

Existing M5 Lua is frozen for v0.1.

M5 Lua is not currently known to violate the safety boundary. Its sandbox denies raw
transport, OutputAuthority, physical ACK/readback/safe evidence and general Runtime
mutability. M9A moved Lua-specific implementation material behind the explicit
`lua.v1` adapter and neutralized the common definition, public lifecycle and new
provenance vocabulary.

No persistent Lua workspace or Lua application/scenario API is required for v0.1.

M9A added a first-class native Rust implementation path through the same contract and
proved the bounded `native.moving_mean.v1` transform. Historical SQLite evidence was
not rewritten merely to rename old Lua provenance.

Optional complete Lua removal is post-v0.1 and requires replacement coverage.

## M8 — final deployment/hardware gate

M8 has already implemented the software contract for:

- strict `runtime.toml`;
- frozen deployment artifacts/provenance;
- staged/classified safe lifecycle activation;
- managed reload/restart operations;
- read-only Windows COM;
- resource recovery/reconnect;
- Metakon compatibility/read acquisition;
- Required Recorder integration.

Completed M8 gate evidence:

1. successful real initial acquisition;
2. finite real device-off transition to Offline;
3. exactly one explicit reconnect;
4. bounded transient `Disconnected` reopen handling;
5. actual-open/settings Ready before Core rebind;
6. generation 1 -> 2 fencing;
7. `channel_type = 3` compatibility probe;
8. durable reconnect lifecycle before ordinary generation-2 acquisition;
9. sustained generation-2 Good acquisition;
10. Babashka A/B independence;
11. harmless live-safe configuration revision without unnecessary rebind;
12. clean shutdown and complete Recorder history;
13. complete debug/release/fmt/Clippy/rustdoc/Babashka/client/demo gates.

External M8 review accepted this gate. No further M8 hardware rerun is required, and
COM5 is not part of current work. Historical M8 reports and evidence remain
immutable.

## M9A — neutral managed components and native Rust execution

### Goal

Neutralize Lua-specific leakage from the common managed-component boundary and make
native Rust a first-class executor path without weakening the established lifecycle
or safety contract.

Completed implementation:

- neutral common definition, capability, lifecycle and provenance vocabulary for new
  work;
- retained `Invocation`, `ComponentResult`, `ComponentCompletion`,
  `ComponentExecutor`, `PlainData` and generation/revision/failure semantics;
- native Rust registration and execution through the same validation, unit,
  bounded-state, bounded-execution, failure and replacement/reload rules;
- bounded native `MovingMean` proved through the contract;
- M5 Lua remains frozen except for bug/regression, safety/security and documentation
  corrections;
- historical SQLite evidence remains immutable.

M9A software gates are complete and the milestone awaits external review. M9B remains
blocked until M9A receives explicit external acceptance.

## M9B — unified Application / emulator / presentation API

### Goal

Make the public semantic boundary complete enough for both GUI and scripting.

### Application API

Normalize/extend the language-neutral API required for:

- discovery/descriptors;
- snapshots/subscriptions;
- bounded history;
- Reference;
- controller/PID;
- recording;
- resources/reconnect;
- configuration candidate/stage/apply;
- virtual/emulator operations;
- presentation/workspace;
- control panels/properties.

Do not duplicate semantics in each adapter.

Provide Babashka wrappers over the same semantic API rather than implementing
Babashka-specific domain behavior.

### Virtual/emulator API

Retain the existing native virtual foundation:

- `VirtualInstrument` remains the simple deterministic/fault-injection virtual
  source;
- `ThermalPlantInstrument` remains the built-in stateful native thermal
  emulator/reference model.

Add a validated virtual-instrument Application API around that foundation for
user-authored emulator behavior. Do not remove, replace or rewrite the existing
native instruments.

Required properties:

- explicit virtual identity;
- typed value/unit/quality;
- bounded input/publication;
- generation/revision/lifecycle fencing;
- explicit stale/failure behavior;
- no physical evidence fabrication;
- Babashka usability.

Babashka now, and future Steel later, use the same semantic API. Steel is an
Application API client, not an M5 replacement VM. This path remains separate from
the frozen M5 Lua managed-component implementation.

### Presentation/workspace

Server-owned semantic model:

- Workspace;
- PlotPanel;
- Trace;
- ControlPanel;
- LogPanel;
- presentation properties.

At minimum traces carry:

- signal identity;
- label;
- color;
- visibility;
- order;
- panel assignment.

Plot panels carry:

- time window;
- follow/live;
- Y policy;
- sizing/presentation configuration.

Both GUI and Babashka manipulate the same model.

### Control-panel API

Provide semantic controls for:

- signal/value readout;
- Reference;
- PID configuration/status;
- controller start/pause/resume;
- recording;
- resource/instrument status;
- reconnect;
- selected validated configuration fields.

No arbitrary GUI callback execution from scripts.

### Properties API

Expose structured deployment editing through:

```text
candidate edit
-> validation
-> stage
-> classified effects
-> safe apply
```

The GUI must not bypass Runtime lifecycle.

### M9B non-goals

- GUI implementation;
- persistent Lua workspace;
- Lua application API;
- Steel implementation;
- scenario menu;
- physical actuator qualification.

## M10 — GUI

Build a separate eframe/egui/egui_plot client using the public Application API.

### Plotting acceptance

Preserve/adapt the successful donor UX:

- multiple panels;
- multiple traces per panel;
- labels/colors/visibility;
- ordering;
- signal-to-panel assignment;
- local-time formatting;
- pan/zoom;
- follow/live;
- configurable time window;
- auto/manual Y;
- double-click autoscale;
- legend;
- plot sizing;
- bounded/downsampled point preparation;
- signal/series sidebar.

### Application UI acceptance

Provide:

- contextual control panels;
- Reference editor/display;
- PID settings/status;
- controller start/pause/resume;
- recording;
- resource/instrument status;
- reconnect;
- bounded/filterable logs;
- separate Properties window.

No scenario menu for v0.1.

No embedded Lua/Steel editor or REPL for v0.1.

GUI closure/reconnect must not own Runtime experiment lifetime.

## M11 — hardening, docs and release candidate

### Hardening

- integration/fault regressions;
- reconnect/reload/restart;
- GUI reconnect;
- history reopen;
- boundedness audit;
- shutdown paths;
- clean-machine Windows smoke.

### Documentation target

Create a coherent release set so users do not need milestone archaeology:

- README and quick start;
- architecture/ownership guide;
- configuration/deployment guide;
- Metakon/physical-read guide;
- signals/Reference/PID guide;
- Recorder/history guide;
- GUI guide;
- Babashka/Application API guide;
- virtual/emulator guide based on `VirtualInstrument`,
  `ThermalPlantInstrument`, the Application API emulator path and a Babashka example;
- strong native Rust managed-component guide based on real repository code;
- native-extension developer guide;
- end-to-end tutorials.

The native managed-component guide teaches when to use a normal native
instrument/model versus a managed component, `ComponentManifest`/identity,
`Invocation`, PlainData configuration/state, `ComponentResult`,
`ComponentExecutor`, validation, units, bounded state/execution, failure propagation,
generation/replacement/reload, registration, tests and adding one small native Rust
model/filter/transform.

Do not publish a Lua tutorial, Lua fallback workflow, Lua application API guide,
`experiment.lua`/`model.lua`/`filter.lua` tutorial or Lua example-script package.
`MILESTONE_5_DESIGN.md` and `MILESTONE_5_REPORT.md` remain historical engineering
evidence.

### Packaging

- Windows package;
- examples;
- version metadata;
- SHA-256;
- clean install/run test;
- final external review.

Then release:

```text
v0.1.0
```

## Model/test economy

Use SOL_HIGH for substantial implementation.

Use a cheaper capable model for mechanical documentation, fixtures, packaging and
routine UI plumbing.

Use ASTRA_HIGH only when implementation exposes a real unresolved architecture,
safety, lifecycle or trust-boundary contradiction.

Do not impose a mandatory Astra design phase on every milestone.

## Post-v0.1 direction

Not part of the first-release commitment:

1. learn/exercise the Application API with Babashka procedures and emulators;
2. evaluate embedded Steel as an in-process client of the SAME Application API;
3. migrate useful remaining M5 Lua implementations after native replacement coverage;
4. optionally remove `lab-lua` after replacement coverage;
5. richer controllers, instruments and recipes.

## v0.1 definition of done

v0.1 is done when:

- M8 real read-only hardware gate is accepted;
- Application/emulator/presentation/control/properties API is usable;
- GUI reaches the required plot/control UX;
- Runtime ownership/safety/Recorder invariants remain green;
- Babashka remains optional and useful;
- documentation/tutorials are sufficient without historical milestone reports;
- Windows package is reproducible and checksummed;
- final external release review is accepted.
