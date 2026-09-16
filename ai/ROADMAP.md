Product roadmap

ai/HANDOFF.md contains the current review/model gate.

ai/WORK.md contains only the currently authorized model task.

The persistent detailed path to the first stable release is:

docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
Completed
M1  domain foundation
M2  central OutputAuthority
M3  bounded transport + narrow Metakon + data-driven definitions
M4  native control pipeline + corrected Warming/renewal
M5  bounded disposable Lua components
M6  autonomous headless Runtime + local API + real Babashka process
M7  Runtime-owned Recorder/SQLite + durable provenance + bounded raw history

M6 was verified at the supplied checkpoint with 189 named Rust tests plus real Babashka A/B acceptance and has now passed the external release-planning review.

Current gate

M7 Recorder/SQLite design and the tests-first D1–D18 acceptance contract are
complete in docs/implementation/MILESTONE_7_DESIGN.md.

M7 is externally accepted at f3ff456, including D1-D18 and the recorded 345-test
debug/release gates. Its reported limitations remain accepted limitations.

STATUS: M8_HARDWARE_ACCEPTANCE_PENDING
Current model: SOL_HIGH
Current phase: corrected Metakon 513 read-only hardware acceptance pending.
M8 software implementation and the 388-test debug/release completion gates are
complete at `0a42d73`; details are in `MILESTONE_8_REPORT.md`.
Do not start M9.

An actual COM5 bench passed compatibility, strict frame decoding, Required
recording and clean shutdown, but exposed a deployment-specific decimal scale
error. The reviewed tests-first corrections are complete at `867f985`: the
actual thermocouple profile is explicit, relative deployment paths are stable,
and the first SQLite archive is preserved. Corrected readings and the physical
disconnect/reconnect gate remain pending; M8 is incomplete and M9 is not
authorized.

First stable release sequence
M7
    Runtime-owned Recorder
    SQLite
    durable provenance
    bounded history query

M8
    runtime.toml deployment configuration
    validation/staging/reload
    managed script reload
    virtual model/emulator restart
    Windows COM adapter
    real Metakon read-only smoke

M9
    optional persistent Lua workspace/fallback scripts
    presentation/workspace model
    Babashka presentation operations

M10
    separate Rust egui/eframe GUI client
    reuse/adapt successful v1 plot behavior
    series sidebar
    logs
    control panels
    minimal reload controls

M11 / release candidate
    integration/fault hardening
    repository cleanup
    test-file naming cleanup
    final architecture/user/developer documentation
    tutorials
    Windows packaging
    SHA256
    release review

v0.1.0

Each milestone requires:

ASTRA_HIGH design
    ↓
manual model switch
    ↓
SOL_HIGH tests-first implementation
    ↓
external review

No model crosses a gate automatically.

Release character

v0.1.0 is a usable and extensible laboratory Runtime, not full historical
com_port_reader feature parity.

Full On/Off/Furnace/filter parity, richer recipes, Arduino and other extensions
continue after release in an educational explain -> code -> test -> commit workflow.

Babashka is an optional external client and bb.exe is not a Runtime dependency.

Lua has two distinct roles:

managed disposable bounded components
+
small optional persistent local workspace for fallback experiment scripting

Neither role owns physical safety.

The GUI is a separate client and must work through the same public Runtime API.
