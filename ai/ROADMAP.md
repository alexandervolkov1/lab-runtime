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

M6 was verified at the supplied checkpoint with 189 named Rust tests plus real Babashka A/B acceptance and has now passed the external release-planning review.

Current gate

M7 Recorder/SQLite design and the tests-first D1–D18 acceptance contract are
complete in docs/implementation/MILESTONE_7_DESIGN.md.

Current model: SOL_HIGH
Gate: ASTRA_HIGH -> SOL_HIGH crossed by explicit user instruction.
Current work: audit and complete M7 implementation against MILESTONE_7_DESIGN.md.
Do not start: M8.

M7 implementation is in progress; it has not passed the D1-D18 or external
review gates.

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
