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
M16.1–M16.5: ACCEPTED
M16.6: AUTHORIZED
M16.7: NOT AUTHORIZED
Current phase: M16.6 faults, bounds, provenance, recovery acceptance
STATUS: M16_6_FAULT_BOUNDS_PROVENANCE_RECOVERY_AUTHORIZED
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
The accepted M15.1 documentation/productization audit commit is
`8cd89e2b58ba11248c2ce2532c165270a9c60797`. M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. The accepted M15.3 architecture +
Application API reference implementation commit is
`48383185309fc6810dff54b9656067280a535171`. The accepted M15.4 Workbench user guide
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15 is paused
without consolidated acceptance; M15.5–M15.8 are deferred and unauthorized until M16
consolidated acceptance. The read-only M16.1 audit is accepted in
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`. The accepted M16.2
read-only simple-device implementation commit is
`aac377470d7142005ad5e0d098fddf6c16734db8`. The accepted M16.3 writable
simple-device implementation commit is `df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`.
The accepted M16.4 Application provisioning implementation commit is
`92847a046c4ef2e4e69d24ea51fc56e28e42af38`. External re-review accepted the original
implementation plus the pending-apply property fence, existing `operation_failed`
mapping for internal owner failures, and phase-aware absolute apply deadline with
quarantine based on actual `send_started && !safe_confirmed` evidence. M16.5 generic
integration acceptance is externally accepted at
`799e7f969393b960ae54409f90842ff2ab1b84cf`. M16.6 fault/bounds/provenance/recovery
acceptance is authorized; M16.7 remains unauthorized.

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

- Application registry: 43 operations and 26 capabilities.
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
`4e18801930939404a8e856521b86c55c20d71dc9`. M14.6B1 recovery status UI and
one-shot `operation_status` are accepted at
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`. M14.6B2A quarantine projection
and restart classification are accepted at implementation commit
`f945d910cfeabbe8552634c53d95016e79a6eafa`. The accepted M14.6B2B1 implementation
commit is `21e4a6f623e39c102109703b22a5266a17f5e0e0`. The accepted M14.6B2B2
implementation commit is `eafd73adf43a8a566336bfe1a14370066fe06c5c`. The accepted M14.6B3
implementation commit is `0319b1d1a7917903c1cf0aa6e4852c149543f7fd`. The accepted M14.6B4
implementation commit is `801a4b559d82a903e9232cc08f5e7d27d714b1d5`. M14 consolidated
acceptance is granted. M15.1 is accepted at audit commit
`8cd89e2b58ba11248c2ce2532c165270a9c60797`; M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. The accepted M15.3 architecture +
Application API reference implementation commit is
`48383185309fc6810dff54b9656067280a535171`. The accepted M15.4 Workbench user guide
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15 is paused
without consolidated acceptance. M15.5–M15.8 are deferred and unauthorized until M16
consolidated acceptance. The read-only M16.1 audit is accepted in
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`; M16.2 is accepted at
`aac377470d7142005ad5e0d098fddf6c16734db8`; M16.3 is accepted at
`df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`; M16.4 is accepted at
`92847a046c4ef2e4e69d24ea51fc56e28e42af38`; M16.5 generic integration acceptance is
externally accepted at `799e7f969393b960ae54409f90842ff2ab1b84cf`; M16.6 is authorized
for the frozen fault/bounds/provenance/recovery acceptance scope; and M16.7 remains
unauthorized.

M16 targets bounded declarative simple devices that become ordinary Runtime
Instrument, Signal, and Actuator entities after validation, without device-specific
operational API or special Workbench/Recorder/history/controller paths. Writable
parameters remain behind existing Runtime OutputAuthority and resource-generation
fences; no public raw byte/command authority is permitted.

```text
persistent deployment configuration != current Runtime state != experiment procedure
```

Deployment describes the installed laboratory and reproducible safe baseline; a
declarative definition maps a bounded physical protocol to typed parameters; the
Application API authoritatively changes execution-time state/configuration; a future
external procedure acts on semantic entities and does not know transport/protocol or
OutputAuthority internals.

Final polished release documentation is a later gate.

## Residual limitations

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification;
- release-quality documentation is incomplete.


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
