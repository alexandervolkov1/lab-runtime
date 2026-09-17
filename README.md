# lab-runtime

`lab-runtime` is a long-running, headless Rust laboratory automation Runtime with
explicit ownership, periodic acquisition, native control, central output safety,
durable recording and a local Application API.

## Current status

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: READY_FOR_EXTERNAL_REVIEW
Current phase: M9C external review gate
M10+: NOT AUTHORIZED
```

M8 physical read-only acceptance and M9A neutral/native managed-component execution
are externally accepted. No additional M8 hardware run is required, and COM5 is not
part of current work.

M9B's coherent local Application API is externally accepted. M9C removed active Lua,
the obsolete Babashka client and transitional source-reload/snapshot API without
adding a replacement scripting language, client SDK or presentation surface.

## Product boundary

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The Runtime API covers resources, instruments, signals, measurements, bounded
history, Reference, native controllers/PID, Recorder, configuration/reconnect and
virtual/emulator operations. Workspace, plot, trace, button, panel, tab, color,
layout and window concepts are client concerns and are not Runtime API concepts.

Periodic native acquisition, controller deadlines, OutputAuthority, Recorder and
required lifecycle/safety work take architectural priority over managed components
and API clients. This is not an operating-system thread-priority claim.

## v0.1 product

The release contains:

- `lab-core` and `lab-runtime`;
- native Rust instruments and managed components;
- periodic polling, measurement signals, latest values and bounded history;
- Reference, native PID/controllers and OutputAuthority;
- Recorder/SQLite durable history and provenance;
- configuration, resource and reconnect lifecycle;
- virtual instruments and emulator support;
- a documented local Application API;
- bounded diagnostic logging;
- professional English documentation and Windows packaging.

The release does not contain a GUI, Presentation API, Steel, Lua, Babashka or Python
client, first-party frontend, client SDK, bundled plotting application or user-facing
scripting environment.

Historical milestone reports and immutable Recorder evidence still describe Lua and
Babashka where necessary to explain earlier acceptance. They are not active product
dependencies or supported v0.1 workflows.

## Recorder and diagnostics

Recorder/SQLite is durable scientific and experiment audit history. Diagnostic logs
are separate bounded troubleshooting information. SQLite is a documented archival
format, not the live control interface:

```text
Application API != Recorder contract != SQLite schema
```

## Release path

```text
M9B  complete and stabilize Application API
M9C  remove Lua and obsolete client baggage
M10  Core cleanup and studyability
M11  Runtime / Recorder / logging / API hardening
M12  documentation, packaging and final release audit
v0.1.0
```

The roadmap ends at v0.1.0.

## Project guidance

Read:

```text
AGENTS.md
PROJECT_BRIEF.md
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
docs/implementation/RELEASE_PLAN_TO_V0_1.md
```

`ai/WORK.md` is the only detailed currently authorized implementation task.
Historical milestone design/report files remain engineering evidence.

## Common verification

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Use the active work authorization for milestone-specific gates.
