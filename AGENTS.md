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

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: ACCEPTED
M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
M12 final documentation/release audit: NOT AUTHORIZED
```

Final physical M8 acceptance and the supplementary post-M9C read smoke remain
accepted. M9D software integration, hardware acceptance and external review are
complete. Final real-device acceptance proved startup safe-zero, one authority-gated
+10 percent controller output, distinct ACK and register-6 readback, normal pause to
verified zero, Recorder sealing and clean finite shutdown. The heater/load remained
physically disconnected, so no physical-effect claim is made.

M9A-M10 implementation and external review have succeeded. M11 external review also
accepted the acquisition/transport, managed-worker, controller/output,
Recorder/SQLite, bounded-diagnostics and Application/process hardening work. The
accepted v0.1 Runtime functionality is implemented and no known preview-blocking
correctness, safety or durability defect remains. "Functionally complete" does not
mean production certification or exhaustive physical qualification: preview/reference
material, practical integration validation, full release documentation and packaging
remain. M12 final polished documentation and release audit remain behind an explicit
authorization gate.

## Donor repository

The read-only `com_port_reader` donor may exist at either:

```text
D:\rust\com_port_reader
D:\rust_projects\com_port_reader
```

Check both before reporting it unavailable.

Never modify the donor.
Never make it a workspace dependency.

Use it only as a source of verified behavior, algorithms, protocol knowledge, tests
and migration evidence.

## Core ownership

The Rust Runtime is the sole authoritative mutable experiment owner.

Preserve:

```text
client lifetime != experiment lifetime

Query = committed snapshot
Command / Operation = mutation or work

Runtime owns controller lifecycle
Runtime owns transport scheduling
Runtime owns Recorder lifecycle
Runtime owns output safety
```

Runtime owns experiment semantics. Clients own presentation semantics.

A slow client, managed component or disk writer must not block required native
acquisition, safety, control or Recorder progress.

Required Runtime work has scheduling priority over non-authoritative extension and
external work. This is an architectural priority, not a Windows thread-priority
class:

```text
authoritative / required Runtime work
├── transport scheduling
├── periodic instrument polling
├── measurement publication
├── native controller deadlines
├── OutputAuthority
├── Recorder ingestion
└── required lifecycle/safety work

non-authoritative extension/external work
├── managed components
└── API clients
```

## Application boundary

Keep one coherent language-neutral local Application API for laboratory and Runtime
semantics. Adapters may use different encodings, but must not duplicate or contradict
domain semantics.

The Application API must not expose presentation concepts such as workspace, plot,
trace, button, panel, tab, color, layout or window. Any client presentation belongs
outside `lab-runtime`.

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
generation/revision/failure semantics
```

A managed component is Runtime-invoked computation:

```text
Runtime -> component -> validated result
```

M5 Lua is historical implementation context. M9C removed its active crate, runtime,
configuration and public API while preserving the neutral managed-component contract
and historical evidence. The active execution path is compile-time native Rust.

Do not rename historical SQLite evidence merely to neutralize old Lua provenance.

Do not restore Lua or add another scripting language, dynamic plugin loader,
REPL/editor or broader M5 scope.

M9A added the first-class `native.moving_mean.v1` path through the same bounded
contract and is externally accepted.

## Virtual/emulator boundary

The existing `VirtualInstrument` deterministic/fault-injection source and
`ThermalPlantInstrument` stateful native thermal emulator/reference model remain
useful native infrastructure. Do not remove or replace them.

M9B added a validated Application API around this native virtual-instrument foundation.
This does not mean rewriting the existing native instruments or moving emulator
behavior into a scripting layer.

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

Any property/configuration edit through the Application API must use the same
validation/stage/apply lifecycle rather than mutating live state directly.

## Recorder

Recorder is Runtime-owned and independent of client lifetime.

Runtime producers do not block on disk I/O.

Recorder queues, batches and histories are bounded.

Required recording failure must follow the accepted fail-closed contract.

Durable history and transient event replay remain separate concepts.

Recorder/SQLite is durable scientific and experiment audit history. Diagnostic logs
are separate bounded troubleshooting information. Routine worker, TCP and debug
messages must not become unbounded Recorder content. Semantically important
diagnostics may be represented as structured durable Runtime events.

Before release, document the active SQLite schema, relationships, time semantics,
provenance, gaps, sealing/completeness, migration/version policy and example
read-only queries. Keep these contracts distinct:

```text
Application API != Recorder contract != SQLite schema
```

## Presentation boundary

v0.1 is headless. It contains no GUI, Presentation API, plotting/workspace concepts,
first-party frontend or client SDK. Runtime must not depend on presentation toolkits.
Clients own all presentation semantics outside `lab-runtime`.

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
client disconnect
one unavailable measurement
one invalid configuration candidate
ordinary managed-component failure
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

The Runtime owner must not wait on network clients, managed components or disk I/O in
a way that prevents required acquisition, control, safety or Recorder service.

## Core dependency discipline

`lab-core` remains OS-, presentation-, storage- and scripting-runtime-independent.

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
