# Agent rules

`lab-runtime` is an architecture-first, headless laboratory automation Runtime.

Current coordination lives in:

```text
ai/PROJECT_BRIEF.md
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
ai/RELEASE_PLAN_TO_V0_1.md
ai/M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md
ai/M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md
ai/M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md
ai/M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md
ai/M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md
ai/M12_CONSOLIDATED_EXTERNAL_REVIEW.md
ai/M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md
ai/M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md
ai/M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md
ai/M14_2_MINIMAL_WORKBENCH_CLIENT.md
ai/M14_3_WORKBENCH_MODEL_PRESENTATION.md
ai/M14_4_MINIMAL_GUI.md
ai/M14_5_OPERATOR_CONTROLS.md
ai/M14_6_RECOVERY_FAULT_AUDIT.md
ai/M14_6B1_RECOVERY_STATUS_UI.md
```

`ai/WORK.md` is the only detailed current authorization. Archived material is
historical context and never overrides it.

## Current phase

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
v0.1.0-preview.1: PUBLISHED
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: ACCEPTED
M12.4 transport parity/fault acceptance: ACCEPTED
M12.5 browser/ClojureScript smoke acceptance: ACCEPTED
M12: ACCEPTED
M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2 Steel: BLOCKED / NOT AUTHORIZED
M14.1 Workbench architecture audit: ACCEPTED
M14.2 minimal native Application client: ACCEPTED
M14.3 WorkbenchModel + PresentationDocument: ACCEPTED
M14.4 minimal eframe/egui GUI: ACCEPTED
M14.5 operator controls + properties/config: ACCEPTED
M14.6A recovery/fault audit: ACCEPTED
M14.6B1 recovery UI + one-shot operation_status: ACCEPTED
M14.6B2A quarantine projection / restart classification: AUTHORIZED
M14.6B2B Exact Retry UI: NOT AUTHORIZED
M14.6B3 explicit-disconnect versus bounded fault reattach: NOT AUTHORIZED
M14.6B4 real-process recovery/fault acceptance: NOT AUTHORIZED
Current phase: M14.6B2A quarantine projection / restart classification
STATUS: M14_6B2A_AUTHORIZED
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.
The accepted M12.5 implementation/evidence commit is
`f291edf35a7805245ce19ea088b9ee899d57f30e`.
The accepted M12 consolidated review-ready commit is
`bb159f674f2cca1c50f903528b268cb070858604`.
The accepted M13.1 audit commit is
`81e303de18021c49c526b12bb1f77f8ea75ae2d9`.
The accepted M13 dependency-safety resolution commit is
`7a150cd8e15d990e00ad62b5c9d9b66c401b7086`.
The accepted M14.1 architecture-audit commit is
`6340aee32b563000bc6397a52aecd85ee945da1c`.
The accepted M14.2 implementation commit is
`bdedf9455305f693a9537f698403c3bbb3840c51`.
The accepted M14.3 implementation commit is
`f01567ba2165b24b9551e3b0acf5b100b0169f32`.
The accepted M14.4 implementation commit is
`116aba631fe47ea24412dd3d4b50e9b00eafc8df`.
The accepted M14.5 implementation commit is
`ab097ed5207ea426cbbd48611015da12ce534a43`.
The accepted M14.6A audit commit is
`4e18801930939404a8e856521b86c55c20d71dc9`.
The accepted M14.6B1 implementation commit is
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`.
Only M14.6B2A is authorized; M14.6B2B–B4 remain unauthorized.

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
