# lab-runtime

`lab-runtime` is a long-running Rust laboratory automation Runtime with explicit
ownership, bounded execution, central output safety, durable recording and a public
local Application API.

## Current status

M1-M7 are complete. M7 Recorder/SQLite is externally accepted.

M8 deployment/Windows-COM/reconnect work, final physical acceptance and final
software gates are accepted. The project is:

```text
M8: ACCEPTED
M9A: IMPLEMENTED
STATUS: READY_FOR_M9A_EXTERNAL_REVIEW
Current phase: external M9A review
```

The completed gate includes the successful real Metakon reconnect path, complete
Recorder evidence and full debug/release/fmt/Clippy/rustdoc/Babashka/client/demo
verification. No further M8 hardware rerun is required, and COM5 is not part of
current work.

M9B remains NOT AUTHORIZED until M9A receives explicit external acceptance. M10 and
M11 remain future milestones.

## Active project guidance

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

Historical milestone design/report files remain engineering evidence and are read on
demand.

## Workspace

The Rust workspace currently contains:

- `lab-core`: OS/GUI/storage/scripting-runtime-independent domain and safety logic;
- `lab-lua`: frozen bounded disposable Lua implementation adapter;
- `lab-runtime`: host/application composition and adapters.

M5 Lua remains present but frozen for v0.1.

The managed-component execution boundary is language-neutral. M9A added explicit
implementation identity/artifacts, one common bounded executor registry and the
native `MovingMean` transform through the same `Invocation`/`ComponentResult`
lifecycle. Frozen M5 Lua remains behind the `lua.v1` adapter.

The existing native `VirtualInstrument` and `ThermalPlantInstrument` remain. M9B
adds a validated emulator Application API around that foundation rather than
replacing it.

## Application model

Runtime is the authoritative experiment owner.

```text
Babashka
GUI
future embedded scripting
        ↓
Application API
        ↓
Runtime
```

Clients do not own controller, transport, Recorder or output-safety lifetime.

## Current release direction

```text
M8  ACCEPTED
M9A READY_FOR_M9A_EXTERNAL_REVIEW — neutral boundary + native Rust path implemented
M9B NOT AUTHORIZED until M9A acceptance
M10 future GUI milestone
M11 future hardening + docs + packaging milestone
v0.1.0
```

Future Steel is post-v0.1 and, if adopted, should use the same semantic Application
API through an in-process adapter.

## Development verification

Common software checks:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Use the current milestone/work file for any additional rustdoc, Babashka, SQLite or
hardware acceptance commands.

## Hardware warning

The first release hardware gate is conservative and read-only.

v0.1 does not claim physical actuator qualification or generic hardware safety
certification.
