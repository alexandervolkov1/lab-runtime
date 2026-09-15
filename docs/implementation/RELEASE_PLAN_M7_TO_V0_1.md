# lab-runtime — release plan from M7 to v0.1.0

## Status and authority

M1–M6 are complete.

The reviewed implementation baseline before this plan is:

```text
M1  domain foundation
M2  central OutputAuthority
M3  bounded transport + narrow Metakon + data-driven definitions
M4  native thermal loop + EMA + Reference + PID
M5  bounded disposable Lua model/transform components
M6  autonomous headless Runtime + bounded local API + actual Babashka process
```

M6 is accepted as the implementation baseline for the release sequence below.

The first stable release target is:

```text
v0.1.0
```

This document is the persistent implementation roadmap from M6 to that release.

It does not authorize a model to cross a model/review gate automatically.

---

# 1. Product definition of v0.1.0

The first stable release must be a usable laboratory-automation platform, not merely an architecture proof.

At v0.1.0 a user must be able to:

```text
start lab-runtime as a long-running server

load a declarative deployment configuration

connect with the Rust GUI client

optionally connect with Babashka

run without Babashka installed

optionally run a local Lua experiment script

communicate with a real Windows COM instrument

read a real Metakon conservatively

record durable experiment history to SQLite

inspect live signals

plot live and recent recorded data

configure and operate the existing native PID/Reference path

disconnect and reconnect GUI/Babashka without stopping Runtime-owned control

reload supported configuration safely

reload Lua component scripts safely

restart a virtual model/emulator safely

configure GUI workspaces/panels/traces from GUI or Babashka

shut down with honest safe/output/recorder evidence
```

The Runtime remains the authoritative owner.

GUI, Babashka and Lua workspace are clients/adapters.

---

# 2. Explicit first-release non-goals

v0.1.0 is not full `com_port_reader` feature parity.

Do not delay the first stable release for:

```text
all historical filters

On/Off controller parity

full Furnace controller parity

full historical Lua app API

rich Babashka workflow/recipe DSL

full historical Metakon register coverage

historical v1 SQLite import

physical actuator-write certification

remote Internet exposure

TLS/authentication

dynamic native Rust plugin ABI

WASM

24/72-hour certification soak

GUI designer framework

arbitrary user-created widget code

full persistent embedded Lua application API
```

These are later learning/product increments.

The release architecture must make them straightforward to add.

---

# 3. Model roles

Use two model roles deliberately.

## ASTRA_HIGH

Astra owns:

```text
architecture
safety/lifecycle semantics
persistence semantics
configuration/reload semantics
trust boundaries
presentation model
GUI/server boundary
release architecture review
tests-first acceptance contracts
```

Astra is design-only unless a later user instruction explicitly says otherwise.

## SOL_HIGH

Sol owns:

```text
tests-first implementation
bounded concurrency
storage adapter implementation
serial adapter implementation
protocol integration
Lua bindings
presentation API
GUI implementation
documentation of actual code
release tooling
```

Sol implements the accepted Astra contract.

Sol may make ordinary local implementation choices.

Sol must not silently redesign a high-level contract.

---

# 4. Mandatory handoff rule

Every milestone follows:

```text
ASTRA_HIGH
    design + acceptance contract
    ↓
MODEL HANDOFF
    ↓
SOL_HIGH
    tests-first implementation + report
    ↓
READY_FOR_EXTERNAL_REVIEW
    ↓
external review
```

Neither model crosses the gate automatically.

If implementation exposes a genuine contradiction involving:

```text
OutputAuthority
physical write ambiguity
Recorder-required safety
configuration atomicity
Lua authority
Runtime ownership
persistent identity
presentation/domain separation
GUI bypass of server API
```

write:

```text
STATUS: WAITING_FOR_REVIEW
```

in `ai/HANDOFF.md` and STOP.

---

# 5. Cross-cutting invariants through release

Preserve all accepted M1–M6 invariants.

In particular:

```text
one authoritative mutable Runtime owner

Query = committed snapshot only

Command/Operation = mutation or work

ACK != readback

network receipt != operation completion

client lifetime != experiment lifetime

GUI lifetime != experiment lifetime

Babashka lifetime != native controller lifetime

Lua lifetime != safety lifetime

finite native leases remain finite

only trusted Runtime-native progress may renew native-controller authority

no raw client/Lua output bypass

no physical evidence supplied by a client

no unbounded queue/history/worker collection

monotonic time controls freshness/scheduling

wall time is recording/presentation metadata

safe transition does not imply automatic rearm
```

`lab-core` remains independent of:

```text
egui
SQLite
Windows serial implementation
Lua VM implementation
network protocol encoding
```

unless a concrete already-reviewed architectural change explicitly says otherwise.

---

# M7 — Runtime-owned Recorder + SQLite

## 6. M7 purpose

M7 turns transient Runtime activity into durable experiment history.

The Recorder belongs to Runtime.

It does not belong to:

```text
GUI
Babashka
Lua
one TCP connection
one subscription
```

SQLite is the first storage adapter.

The domain must not become SQLite-shaped.

---

# 7. M7 data model

Astra must design a language-neutral recording contract before SQLite implementation.

Record enough information to reconstruct and inspect what happened.

At minimum:

```text
runtime boot identity

experiment/run identity

configuration activation identity/version

wall-clock start/end

monotonic elapsed time

instrument/parameter stable identity

measurement value

unit identity

sample quality

observation time

publication time

source/component generation

configuration revision

controller lifecycle transitions

Reference changes

PID configuration changes

output requested

output authorized

output send started

output ACK

output readback

output failed

output ambiguous

safe transition/evidence

Runtime operation accepted/completed/failed/outcome_unknown where meaningful

Lua component load/reload/failure

external structured annotations

Runtime errors

Recorder gaps/degraded state

shutdown result
```

Do not collapse:

```text
requested
sent
ACK
readback
physical effect
```

into one nullable field.

---

# 8. Run versus Runtime process

Do not make one database session mean “the process existed”.

Define separately:

```text
Runtime boot
Experiment run
Configuration activation
Recording interval
```

A Runtime process may contain multiple recording runs.

A process restart must not silently continue active output authority.

A new Runtime boot receives a new process identity.

---

# 9. Recorder queue and backpressure

Runtime producers never perform blocking disk I/O.

Use a bounded recorder ingress.

Bounds must cover:

```text
record count
estimated/actual bytes
pending batch
SQLite transaction batch
shutdown flush
```

A slow disk must not create unlimited RAM growth.

Define explicit overload behavior.

Do not silently lose required records.

---

# 10. Recording policy

Support at least:

```text
BestEffort
Required
```

### BestEffort

Recorder failure:

```text
marks recording degraded/failed
publishes diagnostics
does not automatically claim durable history
may allow observation-only Runtime work to continue
```

### Required

For a run configured as requiring durable recording:

```text
Recorder failure
    ↓
stop/revoke affected control production
    ↓
Rust-owned safe transition
    ↓
fault/degraded state
```

Do not allow required physical control to continue indefinitely after durable recording has failed.

The exact affected scope must be designed explicitly.

---

# 11. SQLite adapter

Prefer a simple single-writer SQLite adapter.

Astra should evaluate and Sol should implement a small schema with:

```text
schema_version

runtime_boots
runs
configurations
instruments/parameter snapshots or equivalent provenance
measurements
operations/actions
controller_events
reference_events
output_events/evidence
runtime_events/logs/annotations
gaps/failures
```

Exact tables are a design decision.

Do not copy the v1 schema literally.

Useful v1 ideas may be adapted:

```text
single writer
transactional measurement batching
WAL where appropriate
ordered finalization
action completion semantics
```

Document the selected durability policy.

Do not claim WAL/NORMAL is a power-loss guarantee.

---

# 12. Configuration/script provenance

Every recording run must know which configuration actually ran.

Store sufficient provenance for:

```text
runtime.toml committed version/content hash or bounded canonical snapshot
instrument definitions used
Lua managed-component source/version
Lua workspace script identity when applicable
controller/Reference configuration
safety profile identity
```

Recording a mutable pathname alone is insufficient provenance.

---

# 13. Wall time

M7 introduces wall-clock timestamps for human history.

Do not use wall time for:

```text
PID dt
Reference progress
freshness
leases
event ordering
```

Store both where useful:

```text
monotonic elapsed time
wall-clock timestamp
```

Define one boot/run mapping clearly.

---

# 14. Minimal history query

The external API should gain a bounded history query sufficient for GUI reconnect and inspection.

Do not expose arbitrary SQL.

Conceptually support:

```text
signal
time range
maximum records/points
page/cursor
```

A request must have hard limits.

For large plot ranges provide either:

```text
bounded paged raw observations
```

and/or a clearly defined:

```text
min/max envelope downsampling query
```

Do not return an unlimited experiment in one frame.

This API is read-only.

---

# 15. M7 failure tests

Astra must define stable acceptance IDs.

At minimum prove:

```text
D1  measurement durability and reopen

D2  Good/Unavailable quality preserved

D3  unit/generation/revision provenance preserved

D4  requested/sent/ACK/readback remain distinct

D5  action accepted before terminal outcome is represented honestly

D6  bounded producer queue

D7  slow writer cannot block native PID/safety

D8  BestEffort recorder failure becomes visible

D9  Required recorder failure revokes/faults/safes affected control

D10 SQLite transaction failure does not fabricate durability

D11 final flush/shutdown success

D12 failed flush reported honestly

D13 process restart creates new boot/run identity

D14 history query bounds

D15 history paging has no silent gap/duplication

D16 configuration/script provenance

D17 wall-clock jump cannot affect controller/reference behavior

D18 existing M1–M6 gates remain green
```

---

# 16. M7 Sol implementation order

Sol should implement:

```text
1. language-neutral recorder contracts and failing tests
2. bounded recorder service/port
3. SQLite adapter/schema
4. Runtime integration
5. required/best-effort policies
6. history query
7. shutdown/failure paths
8. report and verification
```

Create:

```text
docs/implementation/MILESTONE_7_DESIGN.md
docs/implementation/MILESTONE_7_REPORT.md
```

After implementation:

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
```

STOP before M8.

---

# M8 — Deployment configuration, reload and real Windows I/O

## 17. M8 purpose

M8 turns the fixed M6 virtual profile into a deployable laboratory server.

It introduces:

```text
runtime.toml
validated deployment profiles
safe reload
real Windows serial adapter
real Metakon read-only integration
```

---

# 18. Declarative Runtime configuration

Use a human-readable declarative configuration.

Preferred first format:

```text
TOML
```

Main file:

```text
runtime.toml
```

Conceptually it may contain:

```text
runtime identity/display settings

local server settings

Recorder/SQLite path and policy

serial resources

instrument instances

instrument definition paths

polling periods

managed Lua component declarations

References

controllers

safe profiles

presentation workspace path

logging limits
```

Instrument data definitions that are already strict JSON do not need to be rewritten merely for format purity.

---

# 19. Configuration is data, not executable setup

Parsing configuration must not:

```text
open COM
start a controller
write an actuator
run arbitrary Lua
modify the active Runtime
```

Required flow:

```text
read candidate
    ↓
parse
    ↓
schema validation
    ↓
cross-reference validation
    ↓
safety validation
    ↓
build staged candidate/diff
    ↓
only then apply through Runtime lifecycle
```

Invalid candidate:

```text
old committed configuration keeps running
candidate rejected
```

---

# 20. Configuration reload

Add a real Runtime operation:

```text
reload configuration
```

It must classify differences.

At minimum:

```text
presentation-only/live-safe

ordinary live configuration

requires component reinitialize

requires transport rebind

requires controller pause/re-warm

requires output safe barrier

unsupported live change / restart required
```

Safety-sensitive change must never be applied piecemeal while an affected output remains armed.

For changes requiring a safe barrier:

```text
identify dependencies
    ↓
stop affected proposal production
    ↓
revoke/drive safe
    ↓
prove required safe evidence
    ↓
apply candidate
    ↓
reinitialize/re-warm
```

Never auto-rearm afterward.

---

# 21. Script reload

Expose a separate operation:

```text
reload managed scripts
```

For existing M5 disposable components retain:

```text
stage candidate
validate
bounded init
new generation
reset state
re-warm
late old result rejected
```

Invalid script replacement keeps the previous committed generation where the accepted M5 contract permits that.

Dependent controllers are never silently rearmed.

---

# 22. Virtual model/emulator restart

Expose a separate operation:

```text
restart virtual model/emulator
```

Required semantics:

```text
find dependents
stop affected controller production
revoke/safe affected outputs
new model generation
invalidate stale observations
initialize new model
re-warm dependencies
remain stopped/Ready/Paused until deliberate restart
```

Rule:

```text
reload != rearm
```

---

# 23. Real Windows serial adapter

Implement concrete Windows COM support outside `lab-core`.

Support the practical settings already used by v1:

```text
port
baud rate
data bits
parity
stop bits
flow control
read/write timeout policy
```

Provide bounded open/read/write behavior compatible with the existing M3 `ByteTransport` contract.

Do not create a second serial ownership model.

Rust Runtime remains the resource owner.

---

# 24. Port discovery and status

Expose enough diagnostics for clients to inspect:

```text
configured resource
bound COM port
online/offline/recovering
current generation
last error
```

Optional port enumeration may be host-side.

Enumeration does not automatically rebind a running resource.

---

# 25. Real Metakon first hardware gate

The first physical acceptance remains deliberately conservative.

Required:

```text
connect real Metakon over COM
read temperature/status
record observations
disconnect cable/device
observe explicit failure/recovery
reconnect
resume measurements
inspect results from Babashka/API
inspect SQLite
clean shutdown
```

Do not require physical actuator writes for v0.1.0 acceptance.

Do not claim physical output safety certification.

The existing output architecture remains available for later deliberate hardware testing.

---

# 26. M8 acceptance

Astra assigns stable IDs, covering at least:

```text
C1  valid runtime.toml loads deterministically
C2  invalid candidate causes zero active mutation
C3  unknown/duplicate keys rejected
C4  broken references rejected before activation
C5  config reload live-safe change
C6  safe-barrier reload
C7  failed safe barrier leaves honest old/faulted state
C8  no reload auto-rearm
C9  managed script valid reload
C10 managed script invalid reload preserves allowed old generation
C11 emulator restart generation fencing
C12 Windows COM open/configure/close
C13 partial/timeout behavior maps to existing M3 semantics
C14 disconnect/reconnect recovery
C15 resource rebind fences stale work
C16 real Metakon read-only smoke
C17 physical read recorded to SQLite
C18 GUI/Babashka absence does not affect acquisition
C19 clean shutdown with COM/Recorder
C20 all M1–M7 regression gates
```

Sol stops for external review after M8.

---

# M9 — Local Lua fallback + presentation model + richer client API

## 27. M9 purpose

M9 supplies two user-facing extension layers required before GUI release:

```text
minimal standalone Lua experiment environment
+
server-owned semantic presentation model
```

These are separate concepts.

Do not merge their ownership.

---

# 28. Two Lua modes

The project intentionally supports two Lua lifecycles.

## 28.1 Managed Lua components

Already implemented in M5:

```text
fresh disposable VM per job
bounded plain state
model/filter/transform
strict capability boundary
```

Keep this unchanged unless an explicit correction is required.

## 28.2 Lua workspace

Add a separate optional persistent Lua VM for local interactive/script automation.

Purpose:

> A user with only `lab-runtime`, a configuration, scripts and instruments can run a useful experiment without installing Babashka or GUI.

The Lua workspace is convenience/orchestration, not safety authority.

---

# 29. Lua workspace isolation

Run the persistent workspace outside the Runtime owner lane.

A long-running Lua script must not block:

```text
ServiceSafety
native PID
acquisition
transport recovery
Recorder
network clients
```

The workspace may wait/block its own worker.

The Runtime owner never waits on it.

---

# 30. Minimal Lua workspace API

Do not recreate the huge v1 global `app` API.

Expose a deliberately small `lab` surface over the same application/domain services used by other adapters.

Useful first-release capabilities:

```text
discover instruments

describe instrument

get latest value

wait/poll for a condition with explicit bounded/cancellable semantics

set/retune Reference

configure PID while allowed

start/pause/resume controller

read controller/status diagnostics

emit structured log/experiment annotation

start/stop recording run if M7 API exposes it

query operation result

sleep/yield without stopping Runtime
```

Optionally:

```text
reload managed scripts
restart virtual model
```

No raw output lease API.

---

# 31. Lua workspace forbidden capabilities

Do not expose directly:

```text
raw COM handle

ByteTransport

OutputLease

OutputAuthority

hardware evidence completion

arbitrary physical frame bytes

Runtime mutable reference

unbounded callbacks into Rust

hidden rearm
```

For first release, arbitrary Lua network/process/FFI access is not required.

Loading a user-selected script file through the trusted host is sufficient.

---

# 32. Lua script execution mode

Support a simple deployment path conceptually like:

```powershell
lab-runtime --config runtime.toml --script scripts/experiment.lua
```

Exact CLI syntax is an implementation detail.

A script failure:

```text
stops/fails the Lua procedure
publishes diagnostics
does not kill Runtime
does not stop unrelated Runtime-owned PID
does not fabricate safe state
```

---

# 33. Lua REPL

A persistent Lua REPL is not a mandatory first-release GUI feature.

The architecture must permit one later.

Expected later presentation:

```text
Tools
    Lua Workspace...
```

with a separate window containing:

```text
editor
REPL history
output
Run file
Stop script
Reset workspace VM
```

Do not compromise M5 disposable VM isolation to implement this.

---

# 34. Presentation model purpose

GUI layout/presentation becomes explicit server-owned semantic state so that:

```text
GUI
Babashka
future client
```

can manipulate the same workspace.

Presentation state is not safety-critical domain state.

Do not put egui types into `lab-core`.

---

# 35. Presentation entities

Use the smallest clear model around concepts such as:

```text
Workspace

Panel

PlotPanel

Trace

ControlPanel

Control

LogPanel
```

Do not turn each noun automatically into a Rust trait.

---

# 36. Plot presentation state

The model must support at least:

```text
panel stable ID

panel title

panel ordering / semantic layout placement

trace signal identity

trace display label

trace color

trace visibility

trace ordering

plot time window

follow/live mode

X/time formatting policy

Y auto/manual mode

optional Y bounds

legend visibility
```

Client-specific OS window coordinates may remain local to the GUI.

---

# 37. Control-panel presentation state

Support declarative controls built from domain capabilities.

Useful first-release kinds:

```text
signal/value readout

Reference display/editor

PID configuration/status

controller start/pause/resume control

generic configuration parameter editor

instrument/resource status

recording status/control

safe output/status indicator

named Runtime operation button where explicitly allowed
```

A UI control never grants authority.

It only invokes the normal Runtime operation.

---

# 38. Minimal global GUI actions

Do not build a giant toolbar.

The important top-level operational actions are:

```text
Reload configuration

Reload scripts

Restart emulator / virtual model
```

They call the same Runtime operations available to Babashka.

Other operational controls belong in contextual control panels.

---

# 39. Logs presentation

Provide a bounded semantic logs/diagnostics stream suitable for GUI display.

Examples:

```text
resource connect/disconnect

controller lifecycle

Recorder health

Lua failure/reload

configuration reload

output fault/safe

Runtime warning/error
```

GUI-side filtering may support:

```text
severity
subsystem
instrument/controller
text
```

Durable experiment annotations/logs belong to Recorder where appropriate.

---

# 40. Workspace persistence

Presentation persistence is separate from experiment history.

Prefer:

```text
runtime.toml
    human deployment configuration

SQLite
    authoritative experiment history

workspace.json
    server-managed presentation/workspace state
```

The exact workspace format may change after Astra review.

GUI and Babashka do not directly edit the persistence file.

They use presentation operations.

Use atomic replace/write and schema versioning.

Invalid workspace candidate does not destroy the previous committed workspace.

---

# 41. Babashka presentation API

Extend the thin Babashka client.

Conceptual usage:

```clojure
(ui/create-plot! c ...)
(ui/add-trace! c ...)
(ui/update-trace! c ...)
(ui/set-trace-color! c ...)
(ui/set-visible! c ...)
(ui/create-control-panel! c ...)
(ui/add-control! c ...)
```

Exact names are not fixed.

The API should use plain Clojure maps/vectors/keywords.

Do not create a Babashka object framework.

Babashka remains an external optional client.

`bb.exe` is not a Runtime dependency.

---

# 42. Presentation synchronization

An already-running GUI must react to presentation changes made by Babashka.

Required flow:

```text
Babashka presentation command
    ↓
Runtime presentation service commits
    ↓
presentation event
    ↓
GUI updates without restart
```

GUI reconnect obtains:

```text
presentation snapshot
+
domain snapshot
+
cursor
```

and reconstructs its state.

---

# 43. M9 acceptance

At minimum:

```text
P1  managed disposable Lua behavior remains unchanged

P2  Lua workspace runs independently of owner thread

P3  Lua script can discover/query

P4  Lua script can retune/start/pause through normal commands

P5  Lua script cannot access raw OutputAuthority/COM

P6  Lua failure leaves native Runtime alive

P7  reset workspace discards persistent globals

P8  script stop is bounded/cooperative and honest

P9  presentation workspace create/load/save

P10 invalid workspace replacement is atomic

P11 create/remove plot panel

P12 add/remove/reorder trace

P13 color/visibility/label mutation

P14 axis/time/follow settings

P15 create/update control panel

P16 presentation operation cannot bypass domain safety

P17 Babashka changes become live presentation events

P18 GUI-independent presentation persistence

P19 logs projection bounded

P20 all M1–M8 gates green
```

Sol stops for external review.

---

# M10 — Rust egui GUI client

## 44. M10 purpose

Build the first official GUI client.

Preferred stack:

```text
Rust
eframe
egui
egui_plot
```

unless implementation research reveals a concrete blocker.

The GUI is a separate application:

```text
lab-gui.exe
```

The server remains:

```text
lab-runtime.exe
```

Closing GUI does not stop Runtime.

---

# 45. Hard GUI architecture rule

`lab-gui` communicates through the public server API.

It must not:

```text
borrow Runtime

share Arc<Mutex<Runtime>>

invoke private lab-core mutation paths

own serial

own Recorder

own controllers

own safety state
```

If a small shared Rust client crate is justified to avoid coupling GUI to server internals, Astra may design it.

Do not create one merely because another crate looks tidy.

---

# 46. Reuse the successful v1 GUI behavior

Before implementation, inspect the donor GUI read-only and identify exact reusable presentation logic.

The user explicitly considers the following old behavior good and wants it preserved unless a concrete reason exists to change it:

```text
plot rendering behavior

egui_plot interaction

time-axis labels and local time formatting

pan

zoom

follow/live behavior

Y auto/manual behavior

double-click/autoscale behavior where applicable

multiple series per plot

legends

series colors

series visibility

plot sizing

downsampling / plot-point preparation

series sidebar behavior

series display labels

series-to-panel assignment
```

Treat these as user-visible requirements.

Reuse/ADAPT code where boundaries permit.

Do not copy v1 shared-state ownership.

---

# 47. Series sidebar

The first GUI must have a useful series/signal sidebar.

It should support:

```text
discover signals generically

show/hide

display label

color

plot/panel assignment

ordering
```

Later line styles or richer styling may be added without changing Runtime domain semantics.

---

# 48. Plot data source

GUI plotting uses:

```text
live subscriptions
+
bounded local client cache
+
Recorder history query for reconnect/recent history
```

Do not recreate an unlimited `SeriesStore` in Runtime or GUI by accident.

For long ranges use the bounded/downsampled history path.

---

# 49. Logs panel

Retain a useful logs panel.

It should show bounded current diagnostics/events and optionally query durable run information where supported.

It is not the safety authority.

---

# 50. Control panels

Render generic control panels from presentation definitions and Runtime descriptors.

The GUI should support the current release capabilities without hardcoding one furnace screen into core architecture.

Custom specialized views may be added later.

---

# 51. Minimal main toolbar

Do not reproduce a toolbar full of historical buttons.

The main infrastructure actions should prominently include:

```text
Reload configuration

Reload scripts

Restart emulator/model
```

Connection/reconnect status is also visible.

Normal controller/reference/recording controls belong inside context/control panels.

---

# 52. Optional Lua console seam

Do not require a Lua REPL window for v0.1.0.

But do not design GUI in a way that prevents adding later:

```text
Tools -> Lua Workspace
```

as a separate window connected to the M9 persistent Lua workspace.

---

# 53. GUI/Babashka presentation acceptance

Mandatory demonstration:

```text
start lab-runtime

start lab-gui

connect Babashka

from Babashka create plot A

add temperature trace

change trace color

change visibility

create plot B

move another trace to B

create a PID/Reference control panel

GUI updates while already open

close GUI

Runtime/PID remain alive

reopen GUI

workspace restores

domain state restores from Runtime snapshot

recent plot history restores from Recorder

pause controller safely
```

---

# 54. M10 acceptance

At minimum:

```text
G1  GUI process can start/close independently

G2  reconnect after GUI restart

G3  no private Runtime access

G4  generic instrument/signal discovery

G5  plot live signal

G6  multiple traces

G7  time-axis labels match accepted v1 behavior

G8  pan/zoom/follow

G9  Y auto/manual

G10 downsampling bounded

G11 series sidebar

G12 colors/visibility/labels/panel assignment

G13 presentation changes from Babashka appear live

G14 logs panel

G15 control-panel commands use normal server operations

G16 reload config/scripts/emulator buttons use normal server operations

G17 GUI closure does not alter native PID authority

G18 M1–M9 gates remain green
```

Sol stops for external review.

---

# M11 — Release candidate hardening, cleanup and documentation

## 55. M11 purpose

Do not add a new product subsystem here.

Turn the accepted platform into a comprehensible, reproducible first stable release.

---

# 56. Integration run

Run at least one 30–60 minute real-time integration scenario covering:

```text
lab-runtime server

SQLite Recorder

virtual thermal plant

native PID

managed Lua component

optional Lua workspace script

Babashka connect/disconnect/reconnect

GUI connect/close/reconnect

presentation changes

config/script reload

safe emulator restart

bounded queues

clean shutdown
```

Inspect:

```text
memory growth

queue sizes

worker count

thread count

Recorder backlog

event-ring pressure

GUI history cache

shutdown time
```

No unexplained monotonic growth.

---

# 57. Physical Metakon smoke

Run a safe read-only physical smoke on Windows:

```text
real COM port
real Metakon
temperature/status polling
Recorder
GUI display
Babashka observation
disconnect/reconnect
shutdown
```

Record:

```text
device
COM settings
duration
observed failures/reconnect
database result
```

Do not perform a dangerous physical actuator test merely to decorate a release checklist.

---

# 58. Fault-injection suite

Before release exercise:

```text
serial unavailable at startup

serial disconnect during run

corrupted/partial protocol response

Recorder slow/failure

invalid runtime.toml reload

invalid Lua script reload

both managed Lua workers stalled

persistent Lua workspace failure

Babashka killed

GUI killed

slow/nonreading network client

presentation persistence write failure

Runtime shutdown during pending work
```

Native safety/acquisition behavior must remain consistent with existing contracts.

---

# 59. Repository Markdown cleanup

This is now mandatory.

Inventory every tracked Markdown file.

Classify:

```text
KEEP
CONSOLIDATE
ARCHIVE if genuinely useful
DELETE
```

Keep final public documentation concise.

Historical milestone reports may remain under:

```text
docs/implementation/
```

when useful as engineering history.

Delete obsolete duplicate prompts/handoffs/plans after their durable information is consolidated.

Git history already preserves them.

Keep permanently:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
```

---

# 60. Test filename cleanup

Review historical opaque integration-test filenames.

Rename with `git mv`.

Prefer behavioral names:

```text
output_authority.rs
controller_warmup.rs
native_lease_renewal.rs
metakon_transport.rs
lua_sandbox.rs
host_scheduler.rs
request_deduplication.rs
subscription_recovery.rs
recorder_failure.rs
configuration_reload.rs
presentation_workspace.rs
gui_reconnect.rs
```

Use:

```text
step_01_<name>.rs
```

only when files intentionally describe human-readable stages of one tutorial/acceptance sequence.

Tests must never rely on execution order.

Keep renames in separate organization commits from behavioral changes.

---

# 61. Final documentation set

The release should have a clear small public documentation set approximately:

```text
README.md

docs/
    architecture.md
    runtime.md
    instruments.md
    signals.md
    control.md
    output_safety.md
    configuration.md
    lua.md
    babashka.md
    recording.md
    gui.md
    tutorial.md

    developer/
        runtime_owner.md
        add_instrument.md
        add_protocol.md
        add_native_filter.md
        add_native_controller.md
        add_lua_component.md
        client_protocol.md

    implementation/
        historical milestone design/report documents as intentionally retained
```

Exact split may be adjusted to avoid tiny redundant documents.

---

# 62. Documentation quality target

The user intends to study the project after release.

Documentation is part of release acceptance.

A reader should be able to answer:

```text
Who owns Runtime state?

How does one measurement reach a controller?

How does one output reach hardware?

Why can a stale output not execute?

What does ACK mean versus readback?

How does native lease renewal work?

What happens when Babashka disconnects?

How is Lua isolated?

What is the difference between managed Lua and Lua workspace?

How is configuration reloaded safely?

How does Recorder failure affect control?

How does GUI recover after restart?

How do I add a new Rust instrument?

How do I add a new controller?

How do I write an experiment in Babashka?

How do I run a small experiment with only Lua?
```

If documentation cannot answer these without reading milestone archaeology, documentation is not finished.

---

# 63. Required tutorials

## Tutorial A — Virtual furnace from zero

Show:

```text
start Runtime with example config

start GUI

connect Babashka

discover instruments

inspect descriptors

read temperature

configure Ramp

configure PID

start controller

plot temperature/reference/output

start recording

disconnect Babashka

show PID continues

reconnect Babashka

close/reopen GUI

recover presentation/history

pause controller

verify safe output

inspect recorded history
```

## Tutorial B — Build a dashboard from Babashka

Show:

```text
create plot
add traces
change colors
change visibility
create control panel
observe live GUI update
save/reload workspace
```

## Tutorial C — Lua fallback

Show the minimum no-Babashka workflow:

```text
lab-runtime
runtime.toml
experiment.lua
instrument
```

The script should:

```text
read a signal
change Reference
start/pause a native controller
wait for a condition
add an annotation
finish cleanly
```

## Tutorial D — Add a native Rust component

Use one small educational feature, not another framework.

Demonstrate:

```text
domain type
validation
registration
test
documentation
commit
```

---

# 64. Source documentation

Keep the existing requirement:

```text
all Rust module docs
rustdoc
implementation comments
TODO/FIXME
examples
```

in English.

Comments explain:

```text
ownership
why a boundary exists
time semantics
failure semantics
safety significance
```

not syntax.

Code should remain readable to a Rust learner.

---

# 65. Babashka release packaging policy

Babashka is an optional external client.

Do not bundle `bb.exe` as a Runtime dependency.

Repository and release examples should contain:

```text
clients/babashka/
    bb.edn
    client namespaces
    examples/tutorial scripts
```

Documentation explains installation separately.

The core product works without Babashka installed.

---

# 66. Lua release packaging policy

Lua is embedded through the Rust build and requires no separate user installation.

Include example scripts:

```text
scripts/
    examples/
        experiment.lua
        model.lua
        filter.lua
```

Keep managed-component and workspace examples clearly separated.

---

# 67. Release archive

The Windows release archive should contain approximately:

```text
lab-runtime.exe
lab-gui.exe

config/
    runtime.example.toml
    instrument definitions

scripts/
    examples/

clients/
    babashka source/examples
    # no bb.exe

docs/ or concise offline README material where appropriate

README.md
LICENSE if applicable
```

Do not ship development junk, target directories or historical AI coordination files unless deliberately useful.

---

# 68. Final release verification

Required:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
git diff --check
git status --short
```

Also run:

```text
real Babashka acceptance
Lua fallback acceptance
GUI acceptance
Recorder reopen/history acceptance
configuration reload acceptance
Windows COM/Metakon read-only smoke
30–60 minute integration run
```

Build release binaries.

Verify archive on a clean/representative Windows environment where practical.

---

# 69. Release artifacts

Produce:

```text
lab-runtime-v0.1.0-windows-x86_64.zip
lab-runtime-v0.1.0-windows-x86_64.zip.sha256
```

Record the SHA-256 in release notes.

Create:

```text
docs/RELEASE_CHECKLIST.md
CHANGELOG.md or equivalent concise release notes
```

Tag only after final review.

Suggested final tag:

```text
v0.1.0
```

---

# 70. Final model gate

Astra performs the final architecture/documentation/release audit.

Sol fixes concrete findings and builds the final artifacts.

Then:

```text
STATUS: READY_FOR_V0_1_0_RELEASE_REVIEW
```

STOP.

The external reviewer verifies:

```text
release checklist
final tests
physical read-only smoke evidence
documentation
archive contents
SHA256
clean Git state
```

Only after approval should the user create/push the release tag and publish the GitHub release.

---

# 71. Post-release development mode

After v0.1.0 stop large autonomous milestone development by default.

Move to an educational feature loop:

```text
explain
    ↓
inspect existing architecture
    ↓
user writes or reviews a small piece
    ↓
REPL/test
    ↓
discuss
    ↓
commit
```

Good post-release learning/features include:

```text
On/Off controller

remaining native filters

Furnace controller

fuller Metakon functionality

richer Babashka recipes

optional Lua Workspace GUI window

more sophisticated presentation controls

Arduino instrument/protocol

experimental controllers/models

physical output acceptance with an explicit safe bench plan
```

A successful post-release architecture should let these features be added without redesigning Runtime, Recorder, OutputAuthority, server or GUI ownership.

---

# Definition of done for v0.1.0

The first stable release is ready when all of the following are true:

```text
autonomous Runtime works

server/API works

Babashka external client works but is optional

Recorder + SQLite work durably

bounded history query works

runtime.toml deployment works

safe configuration reload works

managed Lua reload works

virtual emulator restart works safely

Windows COM adapter works

real Metakon read-only smoke passes

persistent Lua fallback script can run without Babashka

presentation model works

Babashka can manipulate plots/panels/colors/control panels

separate egui GUI works entirely through public API

successful v1 plot/series UX is preserved where specified

logs panel works

GUI can close/reconnect without stopping Runtime

30–60 minute integration run is stable

repository/documentation cleanup is complete

architecture and developer documentation are understandable

virtual-furnace tutorial is reproducible

Lua fallback tutorial is reproducible

Babashka dashboard tutorial is reproducible

release archive and SHA256 are reproducible

no milestone claims physical actuator certification that was not actually tested
```
