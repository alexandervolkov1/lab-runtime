# Product roadmap

`ai/HANDOFF.md` contains the current state.
`ai/WORK.md` contains the only currently authorized detailed task.

## Completed

```text
M1  domain foundation
M2  central OutputAuthority
M3  bounded transport + narrow Metakon + data-driven definitions
M4  native control pipeline + Warming + finite native renewal
M5  bounded disposable Lua managed components
M6  autonomous host + local API + real Babashka process
M7  Runtime-owned Recorder/SQLite + durable provenance/history
```

M7 is externally accepted.

M5 Lua remains implemented but is frozen for v0.1.

The language-neutral managed-component contract remains useful. Native Rust components
will implement the same `Invocation -> ComponentResult` execution semantics through
a first-class M9A path. The retained contract includes `Invocation`,
`ComponentResult`, `ComponentCompletion`, `ComponentExecutor`, `PlainData` and its
generation/revision/failure semantics.

M5 Lua is not currently known to violate the safety boundary: its sandbox denies raw
transport, OutputAuthority, physical ACK/readback/safe evidence and general Runtime
mutability. Its narrower pre-release problem is Lua-specific leakage into otherwise
language-neutral interfaces, including `ComponentDefinition.source`, public
`lua_source`/`lua_transform` capabilities, `reload_managed_scripts`, lifecycle
helpers such as `stage_standard_lua`, and `ManagedLuaSource`/`managed_lua_source`
provenance vocabulary. Historical SQLite evidence is never rewritten merely to
rename that provenance.

## Accepted: M8

M8 already provides:

- strict `runtime.toml`;
- immutable/frozen deployment artifacts;
- validated staging and safe lifecycle activation;
- distinct config/script/model/reconnect operations;
- read-only Windows COM adapter;
- real Metakon read path;
- Required Recorder integration;
- bounded reconnect diagnostics and corrected recovery/rebind logic;
- one-worker transient `Disconnected` OS-open grace bounded by the original deadline.

```text
M8: ACCEPTED
```

The final real reconnect, Babashka independence, harmless live-safe change, clean
shutdown/history evidence and complete debug/release/rustdoc regression gates all
passed. A release-only Recorder failure was classified and corrected as a test-only
semantic synchronization defect; production hardware behavior is unchanged.

No further M8 hardware rerun is required. COM5 is not part of current work.

## Current: M9A — neutral managed components and native Rust execution

```text
M9A: AUTHORIZED
Current phase: M9A — neutral managed components and native Rust execution
M9B: NOT AUTHORIZED
```

Purpose: remove Lua-specific leakage from the shared managed-component boundary and
make native Rust a first-class implementation of the retained language-neutral
contract.

Required work:

- neutralize the common interface/capability/lifecycle vocabulary without rewriting
  historical Recorder evidence;
- preserve `Invocation`, `ComponentResult`, `ComponentCompletion`,
  `ComponentExecutor`, `PlainData` and generation/revision/failure behavior;
- add registration and execution for native Rust managed components through the same
  validation, bounded-state, bounded-execution and replacement lifecycle;
- prove at least one small native Rust model/filter/transform through that contract;
- keep M5 Lua frozen except for bug/regression, safety/security and documentation
  corrections.

M9B remains NOT AUTHORIZED until M9A receives explicit external acceptance. M10 and
M11 remain future milestones.

## M9B — unified Application, emulator and presentation API

Purpose: complete one language-neutral semantic API before GUI and before any future
embedded scripting language.

### Application API

Complete/normalize the API required by:

```text
Babashka
GUI
future in-process scripting adapter
tests
```

Do not create language-specific domain semantics.

Provide Babashka wrappers over that same semantic API; wrappers must not become a
second source of application behavior.

### Virtual/emulator API

Retain the existing native virtual infrastructure:

- `VirtualInstrument` remains the simple deterministic/fault-injection virtual
  source;
- `ThermalPlantInstrument` remains the built-in stateful native thermal
  emulator/reference model.

Add a validated virtual/emulator Application API around that foundation so
user-written models can drive only explicitly declared virtual instruments. Do not
remove or replace either existing native implementation.

The API must preserve:

- typed values/units/quality;
- generation/revision fencing;
- bounded publication/work;
- explicit model lifecycle;
- stale/failure behavior;
- no physical-evidence fabrication.

Babashka must be able to write useful instrument/emulator scripts through this API.
Future Steel may later use the same semantic Application API; it is not an M5
replacement VM and is not v0.1 scope.

### Presentation/workspace API

Implement server-owned semantic presentation state:

- workspace;
- plot panel;
- trace;
- label;
- color;
- visibility;
- order;
- panel assignment;
- time window;
- follow/live;
- axis policy;
- logs panel;
- control panels.

Both GUI and Babashka should manipulate the same model.

### Control-panel API

Support semantic controls at least for:

- signal/value display;
- Reference display/editor;
- PID settings/status;
- controller start/pause/resume;
- recording controls/status;
- resource/instrument status;
- reconnect;
- generic validated configuration/property fields where applicable.

Scripts declare semantic controls. They do not receive egui callbacks.

### Properties/configuration API

Expose structured candidate editing sufficient for the GUI Properties window.

Changes use:

```text
edit structured candidate
-> validate
-> stage
-> classify effects
-> safe apply
```

Do not bypass Runtime lifecycle.

### M9B non-goals

- persistent Lua application workspace;
- Lua REPL/editor;
- Steel implementation;
- GUI implementation;
- scenario menu;
- physical actuator qualification.

## M10 — separate GUI client

Implement a Rust:

```text
eframe
egui
egui_plot
```

client over the public Application API.

### Required plotting UX

Preserve/adapt donor behavior:

- multiple plot panels;
- multiple traces per plot;
- labels/colors/visibility;
- trace ordering;
- signal-to-panel assignment;
- UNIX/elapsed data with local-time formatting;
- pan/zoom;
- follow/live;
- configurable time window;
- auto/manual Y;
- double-click autoscale;
- legend;
- plot sizing;
- bounded/downsampled point preparation;
- signal/series sidebar.

### Required application UI

- contextual control panels;
- Reference;
- PID;
- controller lifecycle;
- recording;
- resource/instrument status;
- reconnect;
- bounded/filterable logs;
- separate Properties window.

No scenario menu for v0.1.

No embedded Lua/Steel editor or REPL for v0.1.

## M11 — hardening, documentation and release

### Hardening

- integration/fault regression;
- reconnect/restart/reload behavior;
- GUI disconnect/reconnect;
- Recorder/history reopening;
- boundedness review;
- shutdown paths;
- clean-machine Windows validation.

### Documentation

Produce a coherent first-release set:

- README/quick start;
- architecture guide;
- deployment/configuration guide;
- instrument/Metakon guide;
- control/Reference/PID guide;
- Recorder/history guide;
- GUI guide;
- Babashka/API guide;
- strong native Rust managed-component guide based on real repository code;
- virtual/emulator guide based on `VirtualInstrument`,
  `ThermalPlantInstrument`, the Application API emulator path and a Babashka example;
- developer/native-extension guide;
- end-to-end tutorials.

The native managed-component guide must teach when to use a normal native
instrument/model versus a managed component, `ComponentManifest` and identity,
`Invocation`, PlainData configuration/state, `ComponentResult`,
`ComponentExecutor`, validation, units, bounded state/execution, failure propagation,
generation/replacement/reload, registration, tests and adding one small native Rust
model/filter/transform.

Do not plan a Lua tutorial, fallback workflow, application API guide,
`experiment.lua`/`model.lua`/`filter.lua` tutorial or Lua example-script package.
`MILESTONE_5_DESIGN.md` and `MILESTONE_5_REPORT.md` remain historical engineering
evidence, not v0.1 learning material.

A user should not need to read all historical milestone reports to understand v0.1.

### Packaging

- Windows package;
- examples;
- version metadata;
- SHA-256;
- reproducible release smoke;
- final external review.

Then:

```text
v0.1.0
```

## Post-v0.1 direction

Not authorized now:

1. use Babashka interactively to learn and exercise the public Application API;
2. evaluate/implement Steel as an embedded client of the SAME Application API;
3. after native replacement coverage exists, optionally remove `lab-lua`;
4. richer controllers, instruments and recipes.

Steel is an option, not a v0.1 requirement.

## Model economy

Default:

- SOL_HIGH for substantial implementation;
- cheaper model for mechanical docs/fixtures/packaging/routine UI plumbing;
- ASTRA_HIGH only for a genuine unresolved architecture/safety/lifecycle/trust-boundary contradiction.

No automatic Astra -> Sol ceremony for every milestone.
