# Current work — M14 consolidated acceptance

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: no later implementation phase authorized
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration phase: NOT STARTED
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
STATUS: M14_CONSOLIDATED_ACCEPTED
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

## Completed preparation step

The repository documentation surface was cleaned before preview-reference writing:

- keep only current developer-facing documentation outside `ai/`;
- keep active coordination concise and current;
- archive only safety rationale and engineering evidence useful for future regression
  investigation;
- delete superseded milestone, POC, migration and obsolete-technology documents;
- preserve accepted artifacts and fix references after moves.

The cleanup changed documentation and repository organization only. It did not modify
production code, tests, Cargo/configuration/schema, API or Runtime/Recorder semantics.

## Completed reference step

The compact public reference now covers architecture/concepts, the complete
Application API, Recorder/SQLite schema and archive semantics, safety/failure
behavior, extension paths, and preview-sufficient build/run instructions.

## Completed packaging step

`scripts/package-developer-preview.ps1` builds the locked release workspace and
creates an allowlisted Windows x86_64 directory, ZIP, SHA-256 sidecar, build
provenance, package manifest and third-party license inventory under ignored
`dist/`. The starter configuration is virtual-only and opens no COM resource.

GitHub pre-release `v0.1.0-preview.1` was built from and tagged at
`70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e`. The tag is immutable; the current
coordination HEAD may be later because this publication record is docs-only.

## Completed M12.1 audit

The read-only source audit is recorded in
`M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md`. It establishes:

- the actual TCP/reactor/Application/ServiceHost boundary;
- current connection, session, dedup, subscription, replay and history ownership;
- exact network/Application bounds and a single global client budget;
- the required split between shared Application JSON and NDJSON framing;
- cross-transport scope reattachment semantics;
- a loopback-only browser Origin policy;
- a synchronous/nonblocking WebSocket library direction without adding a
  dependency;
- M12.2-M12.5 review slices and the future acceptance matrix.

No production Rust, tests, Cargo/dependencies, configuration schema, Application
operation/DTO, Runtime, Recorder, SQLite or safety semantics changed.

## Completed M12.2 implementation

M12.2 extracted the transport-neutral Application JSON codec and the smallest
concrete shared connection/delivery seam while preserving the complete TCP/NDJSON
contract. It retains one serialized `Application`, one `SessionStore`, the global
eight-client capacity, and all accepted ordering, deadlines, fairness, shutdown and
client-isolation behavior. The implementation and verification evidence is
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`.

M12.2 added no WebSocket, HTTP Upgrade, Origin/WebSocket configuration, async
runtime, public Application semantics, capacity, scripting, presentation concepts,
or changes to Runtime, Recorder, controller, output-authority or physical-safety
semantics.

## Completed M12.3 implementation

M12.3 adds one optional IPv4-loopback WebSocket/JSON listener through the accepted
M12.2 coordinator, shared Application JSON codec, bounded delivery path, serialized
`Application`, and `SessionStore`. TCP plus WebSocket retains one eight-client pool.

The endpoint enforces exact path, numeric Host, Origin allowlist and required
subprotocol checks before Upgrade. Handshake/message/frame/write state, text-only
Application messages, ping/pong/close progression, readiness and failure isolation
are all explicitly bounded. It adds no async runtime, TLS, authentication framework,
Application operations/DTOs/errors, UI/scripting semantics, or second
coordinator/Application/session store.

Implementation, dependency review, exact bounds and verification evidence are in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.3 is accepted. M12.4 parity, reconnect,
backpressure and fault acceptance is complete in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` and accepted. M12.5 real-browser
ClojureScript smoke evidence is recorded in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is accepted. The consolidated source,
bounds, security, parity and browser review is complete in
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and accepted. No production Rust changed
during the consolidation.

## Current authorization

```text
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
```

M13.1 is accepted in `M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. Its external
one-shot TCP/NDJSON host architecture is historical accepted evidence. The later
dependency-safety investigation found no acceptable current Steel candidate, so
M13.2 remains blocked while Workbench work proceeds independently.

The accepted dependency investigation is
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`. M13.2 remains blocked. Steel is a
deferred optional Workbench scripting candidate and must not be added while its
dependency gate remains unresolved.

The accepted read-only M14.1 audit is
`M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The accepted private bounded TCP/NDJSON
Application client and its evidence are in `M14_2_MINIMAL_WORKBENCH_CLIENT.md`. It
adds no GUI, PresentationDocument or Steel dependency/source and changes no
Runtime/Application semantics. The client-owned Workbench model, presentation model
and bounded persistence are accepted after focused model-state and
recovery-projection remediation in `M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The
minimal native GUI and focused overflow/live-continuity remediation are accepted in
`M14_4_MINIMAL_GUI.md`. The typed operator controls, property/configuration workflows,
real Runtime acceptance, and native GUI smoke are accepted in
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
acceptance is granted; no later implementation phase is authorized.

The M14.6B3 worker-owned episode, explicit-disconnect boundary, non-replay proof,
overflow unification, bounds, and verification evidence are recorded in
`M14_6B3_BOUNDED_FAULT_REATTACH.md`.

The executable 30-row recovery/fault matrix, A-J scenarios, process/fault-injection
methodology, and final evidence are recorded in `M14_6B4_RECOVERY_FAULT_ACCEPTANCE.md`.


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
