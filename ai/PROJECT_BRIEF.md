# Current product brief

## Product

`lab-runtime` v0.1 is a headless Rust laboratory automation Runtime. It owns
authoritative experiment state and provides periodic acquisition, measurements,
native control, output safety, durable recording, virtual instruments, native managed
components and one local Application API.

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The Runtime contains no GUI or Presentation API. The workspace contains the accepted
external native `lab-workbench` client/model/observational GUI, but no scripting
runtime or dynamic plugin system.

## Accepted state

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
M14.6B1 recovery UI + one-shot operation_status: AUTHORIZED
M14.6B2 exact retry / quarantine reconciliation: NOT AUTHORIZED
M14.6B3 explicit-disconnect versus bounded fault reattach: NOT AUTHORIZED
M14.6B4 real-process recovery/fault acceptance: NOT AUTHORIZED
Current phase: M14.6B1 recovery UI + one-shot operation_status
STATUS: M14_6B1_AUTHORIZED
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

Functionally complete means accepted v0.1 Runtime behavior is implemented and known
preview-blocking correctness, safety and durability defects are closed. Remaining
work is preview/reference material, practical integration validation, full release
documentation and packaging. This status is not production certification or
exhaustive physical qualification.

## Core invariants

- Runtime is the sole authoritative mutable experiment owner.
- Required native acquisition, safety, control and Recorder work cannot be blocked
  indefinitely by clients, managed components or storage I/O.
- Physical writes require `OutputAuthority`, finite authority and a final recheck.
- ACK, readback and physical effect are distinct; ambiguous writes are not blindly
  retried; reconnect and fresh input do not auto-rearm.
- Recorder admission is distinct from durable commit. Required Recorder failure is
  fail-closed; BestEffort failure remains scoped.
- All long-lived queues, histories, sessions and workers are bounded.
- Diagnostic logging is bounded, lossy and non-authoritative.
- `lab-core` is independent of storage, OS serial, wire, deployment and presentation.

## Accepted public/storage surface

- Application registry: 42 operations and 25 capabilities.
- Public errors: the accepted 12-category taxonomy.
- Transports: local bounded TCP/NDJSON and optional loopback WebSocket/JSON through
  one shared eight-client capacity pool.
- Recorder: unchanged SQLite schema with provenance, gaps and lifecycle sealing.
- Diagnostics: INFO default, four retained 4 MiB files, 1,024-entry lossy queue,
  8 KiB entry bound, default Windows path `%LOCALAPPDATA%\lab-runtime\logs`.

## Current preparation scope

The compact architecture/concepts, complete Application API, Recorder/SQLite,
safety/failure, extension, and getting-started references are complete. A
reproducible Windows x86_64 developer-preview package is ready locally and
`v0.1.0-preview.1` is published as a GitHub pre-release from source commit
`70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e`.

The accepted read-only M12.1 audit established the source-derived path for adding a
second loopback WebSocket/JSON transport to the same Application instance. M12.2
implemented the behavior-preserving transport-neutral JSON and bounded delivery
seams. M12.3 added the first bounded loopback WebSocket/JSON adapter through that
same seam, with exact browser Origin policy and no duplicated Application semantics.
Its review evidence is `M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`.
M12.4 parity, migration, bounded-fault and shutdown evidence is recorded in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` and is accepted. M12.5 real-browser
ClojureScript smoke evidence is recorded in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is accepted. The consolidated source,
bounds, security, parity and browser review is recorded in
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is accepted. The accepted read-only M13.1
external Steel host architecture/dependency audit is recorded in
`M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. Steel dependency safety
resolution is accepted in `M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`. No acceptable
current Steel release or upstream commit was found;
M13.2 remains blocked and unauthorized. The read-only Workbench architecture audit
is accepted in `M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. Workbench remains an
external Application client. Its minimal native M14.2 client is accepted in
`M14_2_MINIMAL_WORKBENCH_CLIENT.md`; its M14.3 client-owned model/persistence is
accepted after focused model-state and recovery-projection remediation in
`M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The minimal native GUI is complete in
`M14_4_MINIMAL_GUI.md` with its accepted overflow/live-continuity remediation. M14.5
operator controls and property/configuration workflows are accepted in
`M14_5_OPERATOR_CONTROLS.md` at implementation commit
`ab097ed5207ea426cbbd48611015da12ce534a43`. The recovery/fault audit in
`M14_6_RECOVERY_FAULT_AUDIT.md` is accepted at commit
`4e18801930939404a8e856521b86c55c20d71dc9`. Only M14.6B1 recovery status UI and
one-shot `operation_status` are authorized; M14.6B2–B4 are not authorized, and any
future Steel integration remains a deferred optional Workbench subsystem.

Final polished release documentation is a later gate.

## Residual limitations

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification;
- release-quality documentation is incomplete.
