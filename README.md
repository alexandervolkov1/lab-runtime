# lab-runtime

`lab-runtime` is a long-running Rust laboratory automation Runtime with explicit
ownership, bounded execution, central output safety, durable recording and a public
local Application API.

## Current status

M1-M7 are complete. M7 Recorder/SQLite is externally accepted.

M8 software-side deployment/Windows-COM/reconnect work is complete and the project is:

```text
STATUS: READY_FOR_HARDWARE_RERUN
```

The latest reported software checkpoint includes 427 named debug workspace tests,
focused reconnect/COM/Recorder suites and real Babashka A/B process acceptance.

Real M8 hardware acceptance is not complete until the prepared reconnect path succeeds
on the actual Metakon bench and the final release/rustdoc gates pass.

Do not start M9 before external M8 acceptance.

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
components may later implement the same contract and replace useful Lua
model/filter/transform implementations.

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
M8  final physical acceptance
M9  unified Application/emulator/presentation/control/properties API
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
