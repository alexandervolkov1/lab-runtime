# lab-runtime

`lab-runtime` is a long-running, headless Rust laboratory automation Runtime with
explicit ownership, periodic acquisition, native control, central output safety,
durable recording and a local Application API.

## Current status

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: ACCEPTED
M11: AUTHORIZED
Current phase: M11 — runtime / Recorder / logging / API hardening
M12+: NOT AUTHORIZED
```

M8 physical read-only acceptance and M9A neutral/native managed-component execution
are externally accepted. The supplementary post-M9C real-device smoke also passed;
its historical evidence remains immutable.

M9B's coherent local Application API is externally accepted. M9C removed active Lua,
the obsolete Babashka client and transitional source-reload/snapshot API without
adding a replacement scripting language, client SDK or presentation surface. M9D
connects physical Metakon output through the existing OutputAuthority architecture.
Its software gates and bounded real-device acceptance pass: startup safe-zero, one
authority-gated +10 percent controller command, strict ACK, distinct matching
register-6 readback, normal pause to zero, Recorder sealing and clean shutdown. The
load was disconnected, so this does not claim physical heater effect. External
review accepted M9D. M10.1 completed the post-M9D structural audit; M10.2 completed
behavior-preserving terminology, archaeology and source architecture indexes. M10.3
organized the Application API by semantic domain without changing the accepted
42-operation contract. M10.4 organized Runtime, Host and Service orchestration into
focused implementation modules without changing ownership or behavior. M10.5
separated the Recorder semantic contract, bounded worker concerns and SQLite
implementation without changing schema or durability. M10.6 centralized native
component registration and clarified the static physical-instrument composition
path without adding plugins, protocols or API semantics. M10.7 tightened internal
visibility, architectural rustdoc and regression-test navigation without changing
public behavior. The focused external re-review accepted M10 after neutral
instrument property metadata removed concrete-instrument knowledge from the generic
Application projection. M11 is authorized; its first task is the audit-only M11.1
hardening and failure-model audit before implementation begins.

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
M9D  physical Metakon output integration (accepted)
M10  Core cleanup and studyability (accepted)
M11  Runtime / Recorder / logging / API hardening (authorized; audit first)
M12  documentation, packaging and final release audit (not authorized)
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
