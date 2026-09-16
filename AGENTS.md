# Permanent coordination files

The permanent coordination files are:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
```

Read all three before each new phase.

`ai/HANDOFF.md` is the only file used for external reviewer communication.

`ai/WORK.md` contains the current phase instruction and is replaced at a phase handoff.

`ai/ROADMAP.md` contains the persistent product roadmap.

Legacy root-level model instruction/handoff files are temporary compatibility
material and will be removed during the mandatory pre-release Markdown cleanup.

# Agent Instructions

This repository is an architecture-first greenfield project.

## Current phase

The current phase is:

```text
ASTRA_HIGH — M8 design only; complete and stopped at implementation handoff.
```

M1–M7 are complete and M7 has passed external implementation review.

The persistent path from M7 to v0.1.0 is defined in:

```text
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
```

Read before acting:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
```

`ai/WORK.md` is the authoritative instruction for the currently authorized phase.

Current model: ASTRA_HIGH.

Authorized work: M8 design only, now complete in
`docs/implementation/MILESTONE_8_DESIGN.md`. M7 at `f3ff456` is externally
accepted; do not restart its implementation or request its old model switch.

STATUS: APPROVED_FOR_ASTRA_HIGH_M8_DESIGN
M8 implementation: not yet authorized. Await the user's explicit switch.
The next (not yet crossed) M8 model gate is:

```text
ASTRA_HIGH -> SOL_HIGH
```

After explicit authorization, SOL_HIGH implements M8 tests-first against its
design and C1-C20. Real Windows COM/Metakon read-only C16/C17/C19 acceptance is
mandatory; fake transport cannot replace missing hardware. Software completion
alone leaves M8 incomplete at that hardware gate. Do not begin M9.

If a genuine architectural contradiction is discovered, record the precise issue in
`ai/HANDOFF.md`, set:

```text
STATUS: WAITING_FOR_REVIEW
```

and stop.

Do not silently cross a model or milestone gate.

## Milestone discipline

Implement only concepts required by the current milestone.

A concept in the target architecture is not a reason to implement it ahead of time.

Completed milestones and historical plans remain useful engineering context, but
they do not override the current authorization in:

```text
ai/HANDOFF.md
ai/WORK.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
```

Do not interpret legacy root-level model instruction files as current authorization.

## Source of truth

Read:

```text
PROJECT_BRIEF.md
```

before doing architectural work.

The current architecture and implementation contracts are also defined by the
relevant documents under:

```text
docs/architecture/
docs/implementation/
docs/migration/
docs/adr/
```

Use the current milestone design and report as the immediate implementation
contract when one exists.

## Relationship to v1

The existing `com_port_reader` project is a separate stable v1 implementation.

Do not copy its architecture automatically.

The donor repository is:

```text
D:\rust\com_port_reader
```

and must remain read-only.

It is a source of:

```text
working behavior
tested algorithms
protocol knowledge
validation knowledge
user-interface requirements
test cases
real product requirements
```

It is not a workspace dependency.

Architecture and migration analysis in `docs/architecture` and `docs/migration`
form the target baseline, not permission to copy the donor structure.

When reusing donor knowledge, preserve useful behavior and test intent while
adapting ownership, safety, lifecycle and concurrency semantics to `lab-runtime`.

Do not modify the donor repository.

## Design priorities

In order:

1. safety;
2. conceptual clarity;
3. extensibility;
4. interactive development;
5. reliability for long-running experiments;
6. testability;
7. simplicity;
8. performance.

Do not optimize for hypothetical extreme throughput.

Typical workloads are small compared with general data-acquisition systems.

Prefer an implementation that is easy to reason about and study over one that is
more generic merely for future possibilities.

## Architecture style

Prefer:

* explicit domain boundaries;
* dependency inversion where it solves a real problem;
* data-driven capabilities;
* descriptors and introspection;
* composition;
* clear state ownership;
* clear lifecycle semantics;
* bounded queues and histories;
* explicit failure behavior;
* explicit time semantics;
* small understandable modules.

Avoid:

* speculative abstractions;
* large generic frameworks;
* trait proliferation;
* unnecessary micro-crates;
* dependency on a specific GUI;
* dependency on one scripting language;
* Rust ABI plugin systems without a demonstrated need;
* unbounded queues or histories;
* hidden blocking on the Runtime owner;
* hidden I/O inside Queries;
* duplicated application semantics in different adapters;
* clever code that obscures ownership, timing or safety.

## Runtime ownership

The Rust Runtime owns authoritative experiment state.

GUI, Babashka, Lua and network connections are clients or adapters.

They must not accidentally become owners of critical state.

Preserve these principles:

```text
client lifetime != experiment lifetime

GUI lifetime != experiment lifetime

Babashka lifetime != native controller lifetime

Lua lifetime != safety lifetime

Query = committed snapshot

Command / Operation = mutation or work

Runtime owns controller lifecycle

Runtime owns transport scheduling

Runtime owns Recorder lifecycle

Runtime owns output safety
```

Slow clients, scripts or GUI work must not block native safety-critical progress.

## Extension philosophy

The system supports several extension levels:

* native Rust;
* data-driven definitions;
* embedded Lua;
* external Babashka/Clojure orchestration;
* possibly compiled or isolated extensions later if a real need appears.

These levels must share one coherent domain model rather than becoming independent
parallel applications.

### Native Rust

Native Rust remains first-class.

Recompilation is acceptable for:

```text
performance-critical code
high-rate processing
new low-level protocols
hardware drivers
safety-sensitive components
reliability-sensitive controllers
```

Do not introduce a dynamic native plugin ABI merely to avoid recompilation.

### Data-driven extensions

Use declarative definitions when Rust already knows the required trusted protocol
primitives and semantics.

Do not turn configuration formats into programming languages.

### Lua

Lua is embedded and optional.

Two different Lua roles are intentionally allowed.

#### Managed Lua components

Existing bounded managed components use disposable Lua VMs and explicit bounded
plain state.

They are suitable for:

```text
models
filters
transforms
small bounded extension callbacks
```

They must preserve:

```text
generation fencing
bounded state
bounded execution
failure isolation
no raw transport ownership
no safety authority
```

#### Persistent Lua workspace

A later release milestone may add a separate persistent Lua workspace for local
fallback experiment scripting.

Its purpose is:

> Allow useful experiment automation with only `lab-runtime`, configuration,
> scripts and instruments, without requiring Babashka or GUI.

It must remain outside the Runtime owner lane.

It must not receive:

```text
raw COM handles
ByteTransport ownership
OutputLease
OutputAuthority
physical-evidence completion
mutable Runtime references
raw actuator bytes
```

Do not recreate the entire historical v1 global Lua `app` API.

### Babashka

Babashka is an optional external orchestration environment.

It is not a Runtime dependency.

`bb.exe` must not be required for normal Runtime startup or deployment.

Babashka uses the public external API and is appropriate for:

```text
interactive REPL work
experiment procedures
supervisory orchestration
dynamic composition
external automation
presentation/workspace manipulation
```

A disconnected Babashka client must not stop a Runtime-owned native controller
unless an explicit safety policy requires that behavior.

## Safety

No extension mechanism may bypass the central output-safety/arbitration layer.

Physical output control remains authoritative in Rust.

Preserve the distinction between:

```text
requested
authorized
send started
ACK
readback
physical effect
```

An ACK is not readback.

A timeout does not prove that a physical write did not occur.

A client, GUI or Lua script must never be able to fabricate:

```text
ACK
ReadbackVerified
safe evidence
transport completion
```

Finite native controller leases remain finite.

Only trusted Runtime-native progress may use private native renewal semantics.

External/manual ownership must not inherit native renewal merely because an
identifier resembles a controller ID.

Safe transition never means automatic rearm.

Reload never means automatic rearm.

## Configuration philosophy

Deployment configuration must be declarative.

The target first-release configuration is expected to use a human-readable
configuration such as:

```text
runtime.toml
```

Parsing configuration must not perform experiment side effects.

Configuration loading should follow:

```text
read candidate
    ↓
parse
    ↓
validate
    ↓
cross-reference validation
    ↓
safety validation
    ↓
stage
    ↓
apply through Runtime lifecycle
```

An invalid candidate must not partially mutate the active Runtime.

Safety-sensitive configuration changes require an explicit safe barrier before
commit.

The following concepts are distinct:

```text
compiled Rust capabilities
runtime deployment configuration
instrument definition
instrument instance configuration
safety configuration
Lua extension code
Babashka procedure
presentation workspace
durable experiment history
```

Do not merge them merely because one file format could technically contain all of
them.

## Reload semantics

The first release plan includes explicit operations for:

```text
Reload configuration
Reload managed scripts
Restart virtual emulator/model
```

They are Runtime operations, not GUI-only behavior.

A GUI button or Babashka helper invokes the same application operation.

General rule:

```text
reload != rearm
```

A reload/restart must preserve generation/revision fencing and explicitly handle
dependent controllers and outputs.

## Recorder and persistence

Recorder is Runtime-owned.

It is independent of GUI, Babashka and connection lifetime.

Durable history and transient client-event replay are different concerns.

Do not turn the M6 semantic event ring directly into a persistence schema.

SQLite is the planned first storage adapter, not the domain model.

Persistence code must remain outside `lab-core`.

Runtime producers must not block on disk I/O.

Recorder queues and batches must be bounded with explicit failure/overflow policy.

Where recording is configured as required for control, Recorder failure must lead
to the reviewed safe/fault behavior rather than silently continuing indefinitely.

## GUI and presentation

GUI is a client.

The target first official GUI is expected to use:

```text
Rust
eframe
egui
egui_plot
```

unless a later design review finds a concrete blocker.

The GUI must communicate through the public Runtime server API.

It must not obtain a private mutable Runtime reference merely for convenience.

Presentation state is separate from safety-critical domain state.

The release plan includes a semantic presentation/workspace model supporting:

```text
workspaces
plot panels
traces
colors
visibility
labels
panel assignment
axis settings
control panels
logs panel
```

Both GUI and Babashka should be able to manipulate this presentation model.

### v1 GUI behavior worth preserving

The user considers the following v1 behavior successful and it should be
REUSED/ADAPTED where practical:

```text
plot rendering behavior
egui_plot interaction
time-axis labels
pan
zoom
follow/live behavior
Y auto/manual behavior
multiple series
legends
series colors
series visibility
plot sizing
downsampling / plot-point preparation
series sidebar
series display labels
series-to-panel assignment
logs panel
```

Preserve user-visible behavior where useful.

Do not preserve old shared-state ownership merely because the rendering code came
from v1.

### Main GUI actions

Avoid a toolbar full of global application buttons.

The important infrastructure actions are expected to be:

```text
Reload configuration
Reload scripts
Restart emulator/model
```

Normal Reference/controller/recording controls belong in contextual control
panels.

A future Lua workspace/REPL should preferably live in a separate optional window,
not dominate the main GUI.

## External API

The current external API is local-first.

The first release is not an Internet-facing service.

Do not silently broaden loopback/local assumptions into a remote-security claim.

Future remote clients may be supported later through a separately reviewed
security/permission model.

Wire DTOs are adapters.

They are not the Core domain model and not the Recorder persistence schema.

## Commands, Queries and Events

Preserve one coherent application boundary across adapters.

Conceptually:

```text
GUI
Babashka
Lua workspace
tests
       ↓
Commands / Queries / Events
       ↓
Runtime
```

Adapters may differ in encoding or convenience helpers.

They must not implement conflicting domain semantics.

Queries do not perform hidden physical refresh or mutation.

Physical read/refresh work is an explicit Command/Operation where applicable.

## Boundedness

Every long-running structure must have an explicit bound or lifecycle.

This includes, where applicable:

```text
signal windows
transport queues
Recorder queues
client request queues
event rings
operation retention
snapshots
Lua state
Lua workers
presentation state
GUI client caches
history queries
diagnostics/log buffers
```

Do not use:

```text
Vec forever
unbounded channel
unbounded lazy event sequence
```

in long-running production paths.

Document overflow/eviction behavior.

## Time

Use monotonic time for:

```text
scheduler cadence
freshness
PID dt
Reference progress
leases
deadlines
ordering
```

Wall-clock time is for:

```text
human-readable recording
display
export
```

A wall-clock adjustment must not change control behavior.

When Recorder stores both, document their relationship explicitly.

## Failure behavior

Ordinary subsystem failure should be scoped where possible.

Do not globally kill Runtime for:

```text
ordinary Lua error
one malformed client
GUI disconnect
Babashka disconnect
one unavailable measurement
one invalid configuration candidate
presentation persistence failure
```

unless the current safety/persistence contract explicitly requires escalation.

Safety-critical failure must fail closed.

Do not hide ambiguity by reporting success.

## Git

Use:

```text
1 logical architectural change = 1 commit
```

Keep documentation/baseline commits separate from implementation commits.

Use small logical commits.

Do not combine unrelated cleanup with behavioral changes.

Preserve unrelated user files.

In particular, preserve:

```text
ai/project_snapshot.txt
```

if it exists outside the authorized task.

Do not commit automatically unless the current user instructions permit commits.

Never modify the donor repository.

## Tests

Use descriptive behavioral names.

Tests must be independent and must not rely on execution order.

Prefer names such as:

```text
host_scheduler.rs
request_deduplication.rs
subscription_recovery.rs
recorder_failure.rs
configuration_reload.rs
presentation_workspace.rs
gui_reconnect.rs
```

Use `step_01_...` naming only for intentionally human-readable scenario/tutorial
stages, never as an execution-order dependency.

Do not mass-rename historical tests during unrelated implementation work.

The mandatory pre-release cleanup may perform `git mv` test renames in separate
rename-only commits.

## Documentation and teaching quality

The user intends to study the implementation after the first stable release.

Source readability and documentation are first-class requirements.

All Rust:

```text
//! module documentation
/// rustdoc
// implementation comments
TODO
FIXME
rustdoc examples
```

must be written in English.

Every important public API must have meaningful rustdoc.

Document non-obvious:

```text
ownership
state transitions
mutation behavior
time semantics
failure semantics
safety significance
boundedness
shutdown behavior
```

For important private helpers, add comments when they explain an invariant or
architectural reason.

Prefer comments that explain why.

Good:

```rust
// Re-check the current authority immediately before the first possible byte so
// a proposal revoked while queued cannot reach the actuator.
```

Bad:

```rust
// Check authority.
```

Keep missing-docs enforcement active.

Prefer clear intermediate variables and explicit control flow over clever compressed
code when that makes the architecture easier to learn.

## Algorithms

For important algorithms such as:

```text
PID
EMA
Ramp Reference
CRC
thermal model
downsampling
Recorder batching
scheduler deadline advancement
```

document:

```text
the algorithm/equation
state carried between calls
time assumptions
validation-before-commit behavior
failure behavior
donor provenance where reused
```

Do not merely copy old implementation code.

## Concurrency

Whenever introducing a thread, worker or channel, document:

```text
who owns mutable state
who may block
queue capacity
overflow policy
shutdown behavior
failure behavior
why the concurrency boundary exists
```

The Runtime owner must not wait on:

```text
network socket I/O
GUI
Babashka
Lua execution
Recorder disk I/O
```

in a way that prevents required safety service.

## Crate/module discipline

Avoid crate proliferation.

Create a new crate only when it establishes a useful dependency or deployment
boundary.

Do not create one crate per architectural noun.

`lab-core` should remain small and OS/presentation/storage independent.

Host adapters and executable composition belong around Core.

## Mandatory pre-release cleanup

Before v0.1.0 perform a deliberate repository documentation cleanup.

Classify tracked Markdown files as:

```text
KEEP
CONSOLIDATE
ARCHIVE
DELETE
```

Keep the permanent coordination files:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
```

Consolidate obsolete duplicate roadmaps/prompts/handoffs.

Git history already preserves removed historical instructions.

Do not perform this cleanup early merely to make the repository look tidy.

## Release documentation target

The first stable release must include clear user and developer documentation.

The release is not complete if understanding the architecture requires reading all
historical milestone reports.

The public documentation should explain at least:

```text
architecture
Runtime ownership
instruments
signals
controllers
OutputAuthority
configuration
Lua
Babashka
recording
GUI
tutorial workflow
how to add native extensions
```

The final release plan is defined in:

```text
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
```

## Post-release direction

After v0.1.0, default development should move away from large autonomous agent
milestones and toward a smaller educational workflow:

```text
explain
    ↓
inspect
    ↓
implement a small feature
    ↓
REPL/test
    ↓
discuss
    ↓
commit
```

Likely post-release subjects include:

```text
On/Off controller
remaining filters
Furnace controller
richer Metakon support
Babashka recipes
Lua workspace GUI
Arduino driver/protocol
experimental controllers/models
physical output acceptance
```

The architecture should allow these without redesigning the Runtime ownership,
server, Recorder or OutputAuthority boundaries.

## Language

Conversation and architecture-review output may be in Russian during design and
review phases.

All source documentation, Rust comments, TODO/FIXME text and documentation
examples intended to live in the repository must be in English unless a user-facing
localized document explicitly requires another language.
