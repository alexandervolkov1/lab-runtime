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
STATUS: APPROVED_FOR_ASTRA_HIGH_M7_DESIGN

Completed model:
SOL_HIGH

Completed work:
M6 implementation and H1–H28 verification

External review result:
M6 accepted as the implementation baseline for the first-release sequence.

Next model:
ASTRA_HIGH

Authorized next work:
M7 Recorder/SQLite design only

Do not begin:
M7 production implementation
M8+
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

Current review authorization
APPROVED_FOR_ASTRA_HIGH_M7_DESIGN

Astra designs M7 only and then requests ASTRA_HIGH -> SOL_HIGH.

Do not design M8 yet.

Do not implement production M7 code under Astra.
