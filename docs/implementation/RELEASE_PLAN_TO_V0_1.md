# lab-runtime — release plan to v0.1.0

## Status

Completed and accepted baseline:

```text
M1-M7
```

Current phase:

```text
M8 final real-hardware acceptance
STATUS: READY_FOR_HARDWARE_RERUN
```

M9 has not started.

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

Existing M5 Lua is frozen for v0.1.

No persistent Lua workspace or Lua application/scenario API is required for v0.1.

Native Rust components may later implement the same managed-component contract and
replace useful Lua model/filter/transform implementations one-for-one.

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

Remaining release gate:

1. successful real initial acquisition;
2. finite real device-off recovery to Offline;
3. one explicit real reconnect;
4. actual-open Ready before generation-2 rebind;
5. channel-type compatibility probe;
6. durable reconnect lifecycle;
7. generation-2 acquisition only after release;
8. Babashka A/B independence;
9. harmless live-safe configuration change;
10. clean shutdown/history evidence;
11. final debug/release/rustdoc/fmt/clippy/Babashka gates;
12. external M8 review.

Do not start M9 before this gate is accepted.

## M9 — unified Application / emulator / presentation API

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

### Virtual/emulator API

Move user-authored emulator behavior to a validated virtual-instrument interface.

Required properties:

- explicit virtual identity;
- typed value/unit/quality;
- bounded input/publication;
- generation/revision/lifecycle fencing;
- explicit stale/failure behavior;
- no physical evidence fabrication;
- Babashka usability.

This path is separate from the frozen M5 Lua managed-component implementation.

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

### M9 non-goals

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
- virtual/emulator guide;
- frozen M5 Lua component guide;
- native managed-component/developer guide;
- end-to-end tutorials.

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
3. add native Rust implementations of useful managed components;
4. migrate away from M5 Lua component implementations;
5. optionally remove `lab-lua` after replacement coverage;
6. richer controllers, instruments and recipes.

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
