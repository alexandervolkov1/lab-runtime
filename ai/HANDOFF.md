File: ai/HANDOFF.md
AI Handoff

Read before every phase:

ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
Current state
STATUS: APPROVED_FOR_ASTRA_HIGH_M8_DESIGN

Current model:
ASTRA_HIGH

Current phase:
M8 design only; design complete, stopped at the explicit implementation handoff.

Completed work:
M7 Recorder/SQLite architecture, implementation and D1-D18 acceptance contract,
externally accepted by the user. M8 design/lifecycle/C1-C20 contract is frozen.

Implementation contract:
docs/implementation/MILESTONE_8_DESIGN.md (after explicit implementation handoff)

M7 production implementation is complete and audited against D1-D18. The exact
acceptance mapping, red/green history, versions, bounds, limitations and final
latest-head verification are recorded in MILESTONE_7_REPORT.md.

External review result:
M7 accepted as the implementation baseline at f3ff456. The external review found
no architectural contradiction or blocking D1-D18 defect. Accepted evidence
includes the complete M7 report and 345-test debug/release gates. The reported
hardware, physical-output, power-loss, remote-security and long-soak limitations
remain limitations, not new claims of acceptance in those areas.

Authorized work:
M8 design only under ASTRA_HIGH, now complete. M8 implementation is not yet
authorized. Await the user's explicit ASTRA_HIGH -> SOL_HIGH switch; do not
implement production code or tests during this design phase.

Model gate:
The earlier ASTRA_HIGH -> SOL_HIGH gate applied to M7 and was crossed.
The new M8 ASTRA_HIGH -> SOL_HIGH implementation gate is not yet crossed.

Do not start:
M8 implementation before the explicit handoff authorization; M9 or later work.

M8 design completion — 2026-09-16

Recovered clean HEAD f3ff456, containing 53880ca and all later accepted M7 work.
Read the permanent coordination files, project brief, release plan and relevant
accepted M3-M7 contracts/source. The design preserves Runtime ownership, Core's
OS/storage independence and accepted safety/evidence/Required semantics.

The new MILESTONE_8_DESIGN.md fixes strict declarative loading, exact loaded TOML
identity, staged diff/atomic commit, irreversible safe-barrier failure semantics,
distinct configuration/script/model operations, fencing, fixed Recorder ingress
budgets, read-only worker-backed Windows COM, conservative recovery and C1-C20.
Recorder bounds are not increased. Fixed-topology live reload rejects topology,
listener and Recorder policy/path changes as restart-required.

Actual Windows/Metakon read-only C16/C17/C19 evidence remains mandatory during
implementation acceptance. Fake transport cannot replace it. Without hardware or
a justified clean-response boundary, report M8_HARDWARE_ACCEPTANCE_PENDING and
stop; software completion alone does not complete M8 or authorize M9.

Documentation only: no production code, tests or dependencies changed; no COM
opened; no accepted M7 test gate rerun or new hardware evidence claimed. The
donor path was unavailable on this computer and was not modified. No unrelated
cleanup or ai/project_snapshot.txt changes were made.

Design validation passed: exactly 20 distinct C1-C20 matrix rows, resolving local
design links, balanced Markdown fences, no trailing whitespace in all five
changed documents, and git diff --check. Changes are documentation-only and left
uncommitted; the accepted implementation HEAD remains f3ff456.

MODEL HANDOFF

STOP HERE.

Switch:
ASTRA_HIGH -> SOL_HIGH

Resume with:
M8 tests-first implementation against MILESTONE_8_DESIGN.md.

M8 implementation: not yet authorized. Do not start M9.

M7 design checkpoint — 2026-09-15

Inspected baseline: edb436a (main), initially clean working tree.
Baseline verification completed successfully:

git status
git log --oneline -20
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cargo test --workspace -- --list confirmed 189 named Rust tests.
The workspace suite included actual Babashka A/B process acceptance.
Release-profile verification was not repeated in this documentation phase.

The design fixes Runtime ownership, bounded Core fact capture and storage ingress,
separate boot/run/interval/activation identities, immutable provenance, monotonic
and wall-clock mapping, typed output evidence, BestEffort/Required policies,
Required revoke/fault/safe behavior, WAL/FULL single-connection SQLite, versioned
schema/reopen, explicit bounded raw-history operations and finite shutdown/flush.

The history service implements the release plan's bounded paged raw-observation
option; downsampling is not advertised. M6 transient replay remains separate.

Donor inspected read-only at 50d3d1e3de84c650e1aa0ffbf1625044f794d315.
No donor tests were run or donor files changed. The original design checkpoint
added no production code, tests or dependencies. A later continuation produced
the uncommitted M7 implementation now under SOL_HIGH audit; its test sequence
must be reported honestly. No M8 design, GUI or Lua expansion was added.
ai/project_snapshot.txt was absent and was not created or modified.
No commits were made at that design checkpoint. The user subsequently authorized
small logical commits during SOL_HIGH work.

Documentation validation: all 18 distinct D1–D18 acceptance rows are present,
local design links resolve, Markdown fences balance, no trailing whitespace,
and git diff --check passes. Only MILESTONE_7_DESIGN.md and the four authorized
coordination/instruction files changed.

No unresolved architectural contradiction was found at the design checkpoint.
That checkpoint requested a model handoff, not external acceptance. SOL_HIGH must map D1–D18
to actual red/green evidence in MILESTONE_7_REPORT.md, run the completion gates,
then set READY_FOR_EXTERNAL_REVIEW here and STOP before M8.

M6 review conclusion

The supplied M6 checkpoint is accepted.

The reviewed evidence reports:

189 named Rust workspace tests

debug and release workspace tests passing

fmt/clippy checks passing

real Rust host

real Babashka A/B process acceptance

Babashka A killed without Pause

native PID remained autonomous for more than three finite lease lifetimes

Babashka B reconciled the same Runtime process

safe virtual pause and shutdown completed

This external review is based on the supplied repository snapshot/source/report
and recorded verification evidence. The reviewer did not independently execute
Cargo or hardware commands on the user's workstation.

Preserve all accepted M1–M6 contracts.

Release direction approved

The detailed release roadmap is:

docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md

The approved product direction includes:

M7 Recorder + SQLite

M8 declarative runtime configuration, safe reload, real Windows COM and
   conservative physical Metakon read-only smoke

M9 persistent optional Lua fallback workspace plus server-owned presentation model

M10 separate egui GUI client using the public API

M11 release hardening, cleanup, documentation, tutorials and packaging
Product clarifications from the user
Babashka

Babashka is an optional external orchestration client.

Do not bundle or require bb.exe for Runtime operation.

Keep the thin client/examples in the repository/release examples.

Lua

The previous migration recommendation that all persistent embedded application
REPL behavior should simply become Babashka is superseded by the user's explicit
product requirement.

The final product should retain:

M5 disposable bounded Lua VMs
    for model/filter/component execution

plus

a separate small persistent Lua workspace
    for local fallback experiment scripting

The fallback requirement is:

With only lab-runtime, configuration/scripts and instruments, a user should be
able to automate a useful experiment without installing Babashka or GUI.

Do not recreate the entire v1 global Lua application API.

The persistent workspace remains outside the Runtime owner lane and may only use
validated Runtime application/domain operations.

No raw COM, OutputLease, OutputAuthority or physical-evidence bypass.

A GUI Lua REPL/editor is optional after the first release or as a later small
addition. Preserve an easy seam for a separate Tools -> Lua Workspace window.

Configuration

The application must not depend on executable Lua configuration.

Use a declarative deployment configuration, preferably runtime.toml.

Parsing configuration must be side-effect free.

Reload uses validate/stage/safe/apply semantics.

Invalid candidates leave the committed configuration intact.

Support explicit operations for:

Reload configuration
Reload managed scripts
Restart virtual emulator/model

reload != rearm.

Presentation and GUI

The GUI is a separate client.

Server-owned semantic presentation state must be configurable both from GUI and
Babashka.

Support:

workspaces
plot panels
traces
trace labels
colors
visibility
panel assignment/order
time window
follow/live
axis policy
control panels
logs panel

The user explicitly considers the old v1 plot/series UX good.

Before implementing M10, inspect the donor read-only and preserve/adapt where
appropriate:

plot rendering
egui_plot interaction
time-axis labels
pan/zoom
follow/live
Y auto/manual behavior
legend/multiple series
series colors/visibility
plot sizing
downsampling
series sidebar
series labels/panel assignment

Do not preserve old shared-state ownership.

Global GUI buttons should remain minimal.

Prominent infrastructure actions:

Reload configuration
Reload scripts
Restart emulator/model

Normal controller/reference/recording actions belong in contextual control panels.

Documentation and learning

The first stable release is not complete without clear architecture and developer
documentation plus reproducible tutorials.

After v0.1.0 the default development mode should become a smaller educational
workflow rather than large autonomous agent milestones.

Current model gate
APPROVED_FOR_ASTRA_HIGH_M8_DESIGN

Current model: ASTRA_HIGH. M7 is externally accepted. M8 design is complete;
implementation is not yet authorized. Await the explicit handoff above.

Historical M7 implementation pause checkpoint — 2026-09-16

The user requested an interruption after finishing the current logical commit
and recording completed and unfinished work. The latest Recorder acceptance
commit is 53880ca: an actual blocked-writer server process exits unsuccessfully
after its finite flush watchdog; a fresh SQLite reopen reports the old run's
unknown tail. Its named debug/release tests, formatting and focused clippy pass.

The design, incoming production audit and substantial Runtime-owned Recorder,
Required/BestEffort, SQLite and bounded-history coverage are complete or in
place. D1-D17 still have edge acceptance cases listed in ai/WORK.md and the
in-progress MILESTONE_7_REPORT.md. The final D18 latest-head verification and
version/limit inventory were not complete. At that pause the status remained
M7_IMPLEMENTATION_IN_PROGRESS; it was not an external-review request or a model
handoff. The completion record below supersedes that historical state.

M7 implementation completion — 2026-09-16

The pause checkpoint was recovered exactly, including 53880ca, and all recorded
remaining D1-D17 cases were closed. The final report maps D1-D18 to named tests
and records the honest sequence where production preceded coverage or a fixture
expectation was corrected.

Final verification on the latest source HEAD passed:

- 345 named Rust tests in debug and optimized release;
- formatting, warning-free clippy and rustdoc with warnings denied;
- finite demo;
- actual M6 and recording-enabled M7 Babashka process tests;
- `bb test-client`: 8 tests, 13 assertions;
- real recording A-kill/B-reconnect/Pause/Stop/Shutdown/SQLite-reopen history;
- `git diff --check` and a clean tree before these handoff documentation edits.

Resolved versions include Rust/Cargo 1.95.0, Babashka 1.13.220, rusqlite 0.40.2,
libsqlite3-sys 0.38.2, bundled SQLite 3.53.2 and sha2 0.11.0. No donor files,
M8 work or unrelated cleanup were performed. No architectural contradiction was
found. At that checkpoint external review was required before later work; the
M7 acceptance and M8 design authorization above now supersede that old gate.
