# lab-runtime

`lab-runtime` is a long-running Rust laboratory automation Runtime with explicit
ownership, bounded execution, central output safety, durable recording and a public
local Application API.

## Current status

M1-M7 are complete. M7 Recorder/SQLite is externally accepted.

M8 deployment/Windows-COM/reconnect work, final physical acceptance and final
software gates are complete. The project is:

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
```

The completed gate includes the successful real Metakon reconnect path, complete
Recorder evidence and full debug/release/fmt/Clippy/rustdoc/Babashka/client/demo
verification. No further COM5 or hardware rerun is authorized or required.

Do not start M9A before external M8 acceptance.

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
- `lab-lua`: existing bounded disposable M5 Lua component executor;
- `lab-runtime`: host/application composition and adapters.

M5 Lua remains present but frozen for v0.1.

The managed-component execution boundary itself is language-neutral. Native Rust
components become a first-class path through the same contract in M9A. M5 Lua is not
currently known to violate the safety boundary; the planned cleanup concerns
Lua-specific naming leaked into otherwise common interfaces.

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
M8  external acceptance review
M9A neutral managed-component boundary + native Rust component path
M9B unified Application/emulator/presentation/control/properties API + Babashka wrappers
M10 GUI
M11 hardening + docs + packaging
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
