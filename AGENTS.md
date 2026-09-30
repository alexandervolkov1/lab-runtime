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
ai/M14_6B2A_RECOVERY_QUARANTINE.md
ai/M14_6B2B1_EXACT_RETRY_CORE.md
ai/M14_6B2B2_EXACT_RETRY_GUI.md
ai/M14_6B3_BOUNDED_FAULT_REATTACH.md
ai/M14_6B4_RECOVERY_FAULT_ACCEPTANCE.md
ai/M15_1_DOCUMENTATION_PRODUCTIZATION_AUDIT.md
ai/M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md
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
M13.2 Steel:
BLOCKED ON STEEL DEPENDENCY SAFETY
NOT AUTHORIZED
M14.1 Workbench architecture audit: ACCEPTED
M14.2 bounded native Application client: ACCEPTED
M14.3 WorkbenchModel + PresentationDocument: ACCEPTED
M14.4 minimal native GUI: ACCEPTED
M14.5 operator controls + properties/config: ACCEPTED
M14.6A recovery/fault audit: ACCEPTED
M14.6B1 recovery UI + one-shot operation_status: ACCEPTED
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: ACCEPTED
M14.6B2B2 Exact Retry GUI: ACCEPTED
M14.6B3 bounded fault reattach: ACCEPTED
M14.6B4 consolidated recovery/fault acceptance: ACCEPTED
M14.6 recovery/reconnect/fault acceptance: ACCEPTED
M14 consolidated acceptance: ACCEPTED
M15.1 documentation/productization audit: ACCEPTED
M15.2 README + getting started: ACCEPTED
M15.3 architecture + Application API reference: ACCEPTED
M15.4 Workbench user guide: ACCEPTED
M15.5–M15.8:
DEFERRED UNTIL M16 CONSOLIDATED ACCEPTANCE
NOT AUTHORIZED
M16.1 declarative simple-device architecture/API audit:
ACCEPTED
M16.2 read-only simple-device vertical slice:
ACCEPTED
M16.3 writable simple-device actuator + ACK/readback vertical slice:
ACCEPTED
M16.4 Application simple-device provisioning:
ACCEPTED
M16.5 generic system integration acceptance:
ACCEPTED
M16.6–M16.7: NOT AUTHORIZED
Current phase: M16.5 generic system integration acceptance
STATUS: M16_5_GENERIC_INTEGRATION_ACCEPTED
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
The accepted M14.6B2A implementation commit is
`f945d910cfeabbe8552634c53d95016e79a6eafa`. The accepted M14.6B2B1 implementation
commit is `21e4a6f623e39c102109703b22a5266a17f5e0e0`. The accepted M14.6B2B2
implementation commit is `eafd73adf43a8a566336bfe1a14370066fe06c5c`. The accepted M14.6B3
implementation commit is `0319b1d1a7917903c1cf0aa6e4852c149543f7fd`. The accepted M14.6B4
implementation commit is `801a4b559d82a903e9232cc08f5e7d27d714b1d5`. M14 consolidated
acceptance is granted. The accepted M15.1 documentation/productization audit commit is
`8cd89e2b58ba11248c2ce2532c165270a9c60797`. M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. The accepted M15.3 architecture +
Application API reference implementation commit is
`48383185309fc6810dff54b9656067280a535171`. The accepted M15.4 Workbench user guide
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15 is paused
after M15.4; consolidated M15 acceptance is not claimed. M15.5–M15.8 are deferred and
unauthorized until M16 consolidated acceptance. The read-only M16.1 declarative
simple-device architecture/API audit is accepted in
`ai/M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`. The accepted M16.2
read-only simple-device implementation commit is
`aac377470d7142005ad5e0d098fddf6c16734db8`. The accepted M16.3 writable
simple-device implementation commit is `df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`.
The accepted M16.4 Application provisioning implementation commit is
`92847a046c4ef2e4e69d24ea51fc56e28e42af38`. External re-review accepted the original
implementation plus the pending-apply property fence, existing `operation_failed`
mapping for internal owner failures, and phase-aware absolute apply deadline with
quarantine based on actual `send_started && !safe_confirmed` evidence. M16.5 generic
integration acceptance is externally accepted; M16.6–M16.7 remain unauthorized.

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


## Accepted M14 consolidated invariants

Runtime owns experiment semantics and Workbench owns presentation semantics.
Workbench/GUI/future-script lifetime is not Runtime or experiment lifetime; Workbench
failure, clean close, or OS termination does not stop Runtime, controllers, Recorder,
or the experiment.

One private bounded Rust Application client owns the socket, framing, hello/session/
scope, message IDs, mutation sequence, subscription/cursor, recovery-journal
interaction, and bounded retained-scope reconnect. No second Application owner was
introduced. One WorkbenchModel and one PresentationDocument remain client-owned and
never become Runtime experiment authority.

Check Status and Exact Retry are explicit/manual; mutation, status, and Exact Retry
are never replayed automatically. Exact bounded worker-owned recovery evidence and
quarantined old-boot/scope evidence remain visible. Explicit Disconnect has no
automatic reconnect and uses the accepted out-of-band/coalesced command-admission
fence. Unexpected continuity loss gets one retained-scope episode with one absolute
3 s deadline, connect timeout no greater than `min(2 s, remaining)`, and retry
spacing of at least 10 ms.

Fresh requires the complete authoritative rebuild barrier. Hello, process liveness,
socket reconnect, an old event cursor, or mutation terminal outcome alone does not
prove Fresh.

The frozen 30-row M14.6A matrix and A-J scenarios pass. Process evidence comprises
eight real-Runtime acceptances and one self-spawn Workbench plus scripted Application
peer acceptance. Native Glow OS-termination evidence preserves Runtime, workspace,
journal, and Reference authority; controller identity `1`, state `ready`, revision
`1` are unchanged, and Recorder state `idle` with null `active_run`/`run_id` is
unchanged. Recorder evidence intentionally proves the weaker authoritative idle
continuity invariant, not active-run survival.

Accepted verification: Workbench `160 passed; 9 ignored; 0 failed` three times;
workspace debug and release each `680 passed; 11 ignored; 0 failed`; real Runtime
`8 passed; 0 failed`; self-spawn/scripted `1 passed; 0 failed`; A-J and native
Glow PASS; recorded focused fault/race repetitions 10/10; fmt, Clippy with warnings
denied, and diff-check PASS.

M13.2 remains blocked on Steel dependency safety and is not authorized. Discard/
Forget, a later milestone, and automatic release are not authorized.
