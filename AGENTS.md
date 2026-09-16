# Agent rules

`lab-runtime` is an architecture-first laboratory automation Runtime.

This file contains stable project rules. Current work authorization belongs in
`ai/WORK.md`.

## Active coordination files

Read these before a new implementation slice:

```text
PROJECT_BRIEF.md
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
docs/implementation/RELEASE_PLAN_TO_V0_1.md
```

`ai/WORK.md` is the only detailed currently authorized implementation task.

Milestone, architecture, migration and ADR documents are historical engineering
context. Read them on demand. They do not override the active authorization.

## Current phase

Current status:

```text
READY_FOR_HARDWARE_RERUN
```

Authorized work is only the final read-only M8 hardware acceptance and its final
verification/reporting gate.

Do not start M9 before external M8 acceptance.

## Donor repository

The read-only `com_port_reader` donor may exist at either:

```text
D:\rust\com_port_reader
D:\rust_projects\com_port_reader
```

Check both before reporting it unavailable.

Never modify the donor.
Never make it a workspace dependency.

Use it only as a source of verified behavior, algorithms, protocol knowledge, tests,
GUI requirements and migration evidence.

## Core ownership

The Rust Runtime is the sole authoritative mutable experiment owner.

Preserve:

```text
client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
Babashka lifetime != native controller lifetime

Query = committed snapshot
Command / Operation = mutation or work

Runtime owns controller lifecycle
Runtime owns transport scheduling
Runtime owns Recorder lifecycle
Runtime owns output safety
```

GUI, Babashka and future embedded scripting are clients/adapters.

A slow client, script, GUI operation or disk writer must not block required native
safety/control progress.

## Application boundary

Keep one coherent language-neutral semantic Application API.

Adapters may use different encodings or convenience helpers, but must not duplicate
or contradict domain semantics.

The expected client directions are:

```text
Babashka -> public local API -> Runtime
GUI       -> public local API -> Runtime
future Steel -> in-process Application adapter -> Runtime
```

Future Steel is post-v0.1 work, not current scope.

## Managed-component boundary

The language-neutral managed-component execution contract is useful and remains part
of the architecture.

Core concepts include:

```text
Invocation
ComponentResult
ComponentCompletion
ComponentExecutor
PlainData
```

A managed component is Runtime-invoked computation:

```text
Runtime -> component -> validated result
```

Existing M5 Lua is one executor/implementation of this contract.

For v0.1, M5 Lua is frozen. Only bug/regression, safety/security and documentation
corrections are allowed.

Do not add a persistent Lua workspace, Lua application/scenario API, Lua GUI API,
Lua REPL/editor or broader M5 scope.

Native Rust components may later implement the same managed-component execution
contract. Useful Lua models/filters/transforms may then be replaced one-for-one by
native implementations before an optional later `lab-lua` removal.

Do not perform that migration during M8-M11 unless explicitly authorized.

## Virtual/emulator boundary

User-authored virtual instrument/emulator behavior should use a validated Application
API rather than M5 Lua.

Only explicitly declared virtual/emulated instruments may accept virtual publication
or model steps.

A client/script must never fabricate:

```text
physical instrument observation
ACK
ReadbackVerified
safe evidence
transport completion
```

Physical evidence remains Rust-owned.

## Output safety

No adapter or extension may bypass central OutputAuthority.

Keep these states distinct:

```text
requested
authorized
send started / first possible byte
acknowledged
readback
physical effect
```

ACK is not readback.
Timeout does not prove that a physical write did not occur.

Re-check current authority immediately before the first possible output byte.

After possible partial/ambiguous physical output, do not blindly start a fresh retry.

Finite leases remain finite.
Safe transition never means automatic rearm.
Reload never means automatic rearm.

## Configuration

Deployment configuration is declarative and validated before activation.

Required lifecycle:

```text
read candidate
parse
validate
cross-reference
safety validation
stage
apply through Runtime lifecycle
```

Invalid candidates do not partially mutate active Runtime state.

Safety-sensitive changes require an explicit safe barrier.

GUI Properties editing must use the same validation/stage/apply lifecycle rather than
mutating live state directly.

## Recorder

Recorder is Runtime-owned and independent of GUI/client lifetime.

Runtime producers do not block on disk I/O.

Recorder queues, batches and histories are bounded.

Required recording failure must follow the accepted fail-closed contract.

Durable history and transient event replay remain separate concepts.

## GUI and presentation

GUI is a separate client over the public Application API.

Runtime must not depend on egui.

Server-owned semantic presentation state may be manipulated by GUI or external
clients.

The target GUI uses:

```text
Rust
eframe
egui
egui_plot
```

unless a concrete blocker appears.

Preserve/adapt successful donor plot behavior:

```text
multiple plot panels
multiple traces
trace labels
colors
visibility
ordering
panel assignment
local-time/time-axis formatting
pan/zoom
follow/live
configurable time window
auto/manual Y
double-click autoscale
legend
plot sizing
downsampling
signal/series sidebar
logs
```

Required contextual controls include Reference, PID, controller lifecycle, recording,
resource/instrument status, reconnect and Properties.

No scenario menu is required for v0.1.

## Boundedness

Every long-running collection/queue/worker has an explicit bound or lifecycle.

Do not use unbounded production queues, histories or lazy event streams.

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

Use wall-clock time for:

```text
human-readable recording
display
export
```

Wall-clock changes must not alter control behavior.

## Failure behavior

Scope ordinary failures where possible.

Do not globally kill Runtime merely for:

```text
one malformed client
GUI disconnect
Babashka disconnect
one unavailable measurement
one invalid configuration candidate
ordinary frozen-M5 Lua failure
presentation persistence failure
```

Safety-critical failures fail closed.

Do not hide ambiguity by reporting success.

## Concurrency

Whenever adding a thread/worker/channel, document:

```text
mutable-state owner
who may block
queue capacity
overflow policy
shutdown behavior
failure behavior
why the boundary exists
```

The Runtime owner must not wait on network, GUI, scripts or disk I/O in a way that
prevents required safety service.

## Core dependency discipline

`lab-core` remains OS-, GUI-, storage- and scripting-runtime-independent.

Avoid crate proliferation and speculative framework layers.

Native Rust rebuilds are acceptable for trusted algorithms, models, filters,
protocols, drivers and safety-sensitive code.

## Code and documentation quality

The implementation is intended to be studied.

All Rust module docs, rustdoc, implementation comments, TODO/FIXME text and rustdoc
examples are in English.

Document non-obvious ownership, lifecycle, time, failure, safety and boundedness.

Prefer comments that explain why.

Keep missing-docs enforcement active.

## Git

Use:

```text
1 logical change = 1 commit
```

Keep documentation cleanup separate from behavior changes.

Do not modify unrelated user files.

## Model use

SOL_HIGH is the default implementation model for difficult slices.

Use cheaper models for mechanical documentation, fixture, packaging and routine UI
plumbing where appropriate.

Use ASTRA_HIGH only when a genuine unresolved architecture/safety/lifecycle/trust
boundary contradiction appears. Do not require Astra automatically for every
milestone.

No model crosses a milestone/review gate automatically.
