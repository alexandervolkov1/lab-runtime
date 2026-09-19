# Agent rules

`lab-runtime` is an architecture-first, headless laboratory automation Runtime.

Current coordination lives in:

```text
ai/PROJECT_BRIEF.md
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
ai/RELEASE_PLAN_TO_V0_1.md
```

`ai/WORK.md` is the only detailed current authorization. Archived material is
historical context and never overrides it.

## Current phase

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
```

Functionally complete means the accepted Runtime functionality is implemented and no
known preview-blocking correctness, safety or durability defect remains. It does not
mean production certification or exhaustive physical qualification.

## Ownership and scheduling

The Rust Runtime is the sole authoritative mutable experiment owner. Preserve:

```text
client lifetime != experiment lifetime
Query = committed snapshot
Command / Operation = mutation or work
```

Runtime owns controller lifecycle, transport scheduling, Recorder lifecycle and
output safety. Required acquisition, safety, control and Recorder progress takes
architectural priority over managed components and API clients. This is not an OS
thread-priority claim.

## Application and presentation

Keep one language-neutral local Application API for laboratory semantics. Runtime
owns experiment semantics; clients own presentation semantics. Do not add workspace,
plot, trace, panel, layout, window or other presentation concepts to Runtime.

Only explicitly virtual/emulated instruments may accept virtual publication. A
client must never fabricate physical observations, ACK, verified readback, safe
evidence or transport completion.

## Output safety

No adapter or extension may bypass central `OutputAuthority`. Keep distinct:

```text
requested
authorized
send started / first possible byte
ACK
readback
physical effect
```

Re-check authority immediately before the first possible output byte. Timeout after
send start is ambiguous and must not trigger blind retry. Leases remain finite;
reconnect, reload and safe transition never imply automatic rearm.

## Configuration, time and failure

Deployment candidates follow read, parse, validate, cross-reference, safety
validation, stage and Runtime-owned apply. Invalid candidates do not partially mutate
active state. Safety-sensitive changes require a safe barrier.

Use monotonic time for scheduling, freshness, control, leases and deadlines. Use
wall-clock time only for human-facing recording/export. Scope ordinary failures;
safety-critical failures fail closed and ambiguity is never reported as success.

## Recorder and diagnostics

Recorder is Runtime-owned, bounded and independent of client lifetime. Required
recording failure follows the accepted fail-closed contract.

```text
Application API != Recorder contract != SQLite schema
Recorder = durable scientific/audit history
Diagnostic log = bounded best-effort troubleshooting
```

Logging failure must not alter experiment semantics.

## Managed components and dependencies

Managed components execute through the bounded, language-neutral invocation/result
contract. Active implementations are trusted compile-time Rust. Do not restore Lua,
add scripting, dynamic plugins or let components acquire transport/output authority.

`lab-core` remains independent of OS serial APIs, SQLite, wire encoding, deployment
filesystem, scripting and presentation. Avoid speculative frameworks and crate
proliferation.

## Concurrency and boundedness

Every long-running collection, queue and worker has an explicit bound and lifecycle.
For any new worker document its mutable-state owner, blocking behavior, capacity,
overflow, shutdown and failure behavior. Runtime required work must not wait on disk,
network clients or managed components indefinitely.

## Repository discipline

The read-only `com_port_reader` donor may exist at `D:\rust\com_port_reader` or
`D:\rust_projects\com_port_reader`. Never modify it or make it a workspace dependency.

Keep code comments and Rust documentation in English. Document non-obvious ownership,
lifecycle, time, safety, failure and bounds. Preserve warning-denied builds and
missing-docs enforcement.

Use one logical change per commit. Do not modify unrelated user files and never cross
a milestone/review gate automatically.
