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
STATUS: M7_IMPLEMENTATION_IN_PROGRESS

Current model:
SOL_HIGH

Completed work:
M7 Recorder/SQLite architecture and tests-first D1–D18 acceptance contract.

Implementation contract:
docs/implementation/MILESTONE_7_DESIGN.md

M7 production implementation is in progress. The incoming uncommitted M7
changes preceded the explicit SOL_HIGH switch and have been audited against
D1-D18; remaining acceptance work is recorded in the M7 report.

External review result:
M6 accepted as the implementation baseline for the first-release sequence.

Authorized work:
M7 tests-first implementation against MILESTONE_7_DESIGN.md only.

Model gate:
ASTRA_HIGH -> SOL_HIGH crossed by the user's explicit instruction.

Do not start:
M8 or later milestones.

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
M7_IMPLEMENTATION_IN_PROGRESS

Current model: SOL_HIGH. M7 design is complete and accepted as the implementation
contract. Continue M7 work against D1-D18, recording any
acceptance tests added after their production code honestly in the M7 report.
M8 is not authorized. Set READY_FOR_EXTERNAL_REVIEW only after all M7 checks pass.

M7 implementation pause checkpoint — 2026-09-16

The user requested an interruption after finishing the current logical commit
and recording completed and unfinished work. The latest Recorder acceptance
commit is 53880ca: an actual blocked-writer server process exits unsuccessfully
after its finite flush watchdog; a fresh SQLite reopen reports the old run's
unknown tail. Its named debug/release tests, formatting and focused clippy pass.

The design, incoming production audit and substantial Runtime-owned Recorder,
Required/BestEffort, SQLite and bounded-history coverage are complete or in
place. D1-D17 still have edge acceptance cases listed in ai/WORK.md and the
in-progress MILESTONE_7_REPORT.md. The final D18 latest-head verification and
version/limit inventory are not complete. This pause remains inside SOL_HIGH
with STATUS: M7_IMPLEMENTATION_IN_PROGRESS. It is not an external-review request
or a model handoff. Resume M7 only when the user asks; M8 is not authorized.
