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
may later implement the same `Invocation -> ComponentResult` execution semantics.

## Current: M8 final physical acceptance

M8 already provides:

- strict `runtime.toml`;
- immutable/frozen deployment artifacts;
- validated staging and safe lifecycle activation;
- distinct config/script/model/reconnect operations;
- read-only Windows COM adapter;
- real Metakon read path;
- Required Recorder integration;
- bounded reconnect diagnostics and corrected recovery/rebind logic.

Current status:

```text
READY_FOR_HARDWARE_RERUN
```

Remaining:

- successful real reconnect through generation 2;
- Babashka independence during that physical run;
- one harmless live-safe change;
- clean final shutdown/history evidence;
- final release/rustdoc and regression gates;
- external M8 review.

M9 is not authorized before external M8 acceptance.

## M9 — unified Application, emulator and presentation API

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

### Virtual/emulator API

Add a validated virtual/emulator path so user-written models can drive only explicitly
declared virtual instruments.

The API must preserve:

- typed values/units/quality;
- generation/revision fencing;
- bounded publication/work;
- explicit model lifecycle;
- stale/failure behavior;
- no physical-evidence fabrication.

Babashka must be able to write useful instrument/emulator scripts through this API.

### Managed components

Do not expand M5 Lua.

Keep the language-neutral managed-component contract.

Native Rust managed components are allowed by the architecture and may later replace
Lua implementations one-for-one, but that migration is not required for v0.1.

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

### M9 non-goals

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
- frozen M5 Lua component guide and limitations;
- developer/native-extension guide;
- end-to-end tutorials.

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
3. add native Rust managed-component implementations for useful M5 Lua models/filters;
4. after replacement coverage exists, optionally remove `lab-lua`;
5. richer controllers, instruments and recipes.

Steel is an option, not a v0.1 requirement.

## Model economy

Default:

- SOL_HIGH for substantial implementation;
- cheaper model for mechanical docs/fixtures/packaging/routine UI plumbing;
- ASTRA_HIGH only for a genuine unresolved architecture/safety/lifecycle/trust-boundary contradiction.

No automatic Astra -> Sol ceremony for every milestone.
