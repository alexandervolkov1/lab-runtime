# lab-runtime

`lab-runtime` is a long-running, headless Rust laboratory automation Runtime with
explicit ownership, periodic acquisition, native control, central output safety,
durable recording and a local Application API.

## Current status

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: READY_FOR_EXTERNAL_REVIEW
M9C: NOT AUTHORIZED
Current phase: M9B external review gate
```

M8 physical read-only acceptance and M9A neutral/native managed-component execution
are externally accepted. No additional M8 hardware run is required, and COM5 is not
part of current work.

M9B implementation and internal gates are complete and await external review. The
coherent local Application API exposes laboratory and Runtime semantics without
presentation state, a frontend, client SDK or scripting environment. M9C is not
authorized.

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

The current workspace still contains temporary Lua and Babashka-era implementation
and acceptance material. M9C removes active Lua and obsolete first-party client
baggage after M9B acceptance while preserving historical reports and immutable
evidence.

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
