# AI handoff

## Current state

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M14.6B2B1 Exact Retry core / evidence lifecycle
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration phase: NOT STARTED
Final release documentation/audit: NOT AUTHORIZED
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
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: AUTHORIZED
M14.6B2B2 Exact Retry GUI: NOT AUTHORIZED
M14.6B3 explicit-disconnect versus bounded fault reattach: NOT AUTHORIZED
M14.6B4 real-process recovery/fault acceptance: NOT AUTHORIZED
STATUS: M14_6B2B1_AUTHORIZED
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

External review accepted M11 at
`31a39cf02e7d58a42d731368f56164226566c5e8`. The accepted v0.1 core has no known
developer-preview-blocking correctness, safety or durability defect. This is not a
production-certification claim.

## Accepted boundaries

- Runtime remains the sole authoritative mutable experiment owner.
- Host/Service orchestrate bounded external work; Application is a semantic
  projection; Recorder and diagnostics are not experiment owners.
- Public Application behavior remains 42 operations, 25 capabilities and the
  accepted 12-category error taxonomy.
- Output ambiguity fails closed without blind retry. ACK, readback and physical
  effect remain distinct. Reconnect or fresh input does not rearm; recovery uses the
  accepted explicit controller lifecycle.
- Required Recorder remains fail-closed, admission is not durability, clean archives
  seal truthfully and killed-process archives remain visibly incomplete.
- Diagnostics are bounded, lossy and observational. Malformed/slow clients are
  isolated and native Runtime progress remains prioritized.
- Shutdown is finite and truthful; unresolved physical ambiguity is not presented as
  proof of physical safety.

## Evidence

Accepted M9D evidence remains at
`examples/metakon-513-m9d-write-smoke.sqlite`, SHA-256:

```text
14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32
```

It proves the accepted software/transport ACK and register-6 readback sequence with
the load disconnected, not physical heater effect.

Selected internal rationale and evidence is under `ai/archive/`. Git history holds
the superseded milestone and migration documents removed from the current tree.

## Current public reference

The compact developer-preview reference is now public under `docs/` and covers:

1. concise architecture/concepts;
2. compact complete Application API reference;
3. Recorder/SQLite archive reference;
4. safety/failure/recovery cheat sheet derived from the M11 matrix;
5. configuration and instrument/component extension notes;
6. preview-sufficient getting-started/build/run instructions;

The README is the landing page. Public claims were checked against the accepted
source/tests: 42 operations, 25 capabilities, 12 public error categories, protocol
and bounds, schema v1, logging bounds, and controller lifecycle.

## Current local artifact

The Windows x86_64 developer-preview package is reproducibly built by
`scripts/package-developer-preview.ps1`. Generated `dist/` artifacts are ignored by
Git and are not published automatically.

The first preview is published at
`https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.1`.
Its immutable release tag points to packaged source commit
`70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e`; later coordination-only commits do
not move that tag.

## Accepted M12 and M13.1 work

External review accepted `M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md` as the
source-derived current transport/Application boundary, exact bounds and ownership,
browser security policy, recommended synchronous/nonblocking implementation
direction, milestone slicing, and future acceptance matrix.

The audit found no architectural contradiction. WebSocket can join the current
serialized server through one shared delivery boundary and the existing single
`Application`. TCP + WebSocket must retain one global eight-client budget. The
recommended local browser endpoint remains loopback-only and requires exact Origin
allowlisting before Upgrade.

M12.2 is implemented and recorded in
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`. Shared Application JSON now sits below
NDJSON framing, and one concrete coordinator owns checked connection IDs, the global
eight-client pool, bounded per-client delivery, fair owner request scheduling and
exact detach lifecycle.

M12.3 added one optional loopback WebSocket/JSON adapter through that same
coordinator and delivery path. It uses the same bounded Application JSON codec,
serialized `Application`, `SessionStore`, connection-ID space and global eight-client
budget. Exact path, numeric Host, Origin allowlist and required subprotocol checks
occur before Upgrade; text/message/frame/handshake/write state and close progression
are finite. Implementation and verification evidence is in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.3 is accepted and M12.4 transport
parity/fault acceptance is complete in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` and accepted. M12.5 real-browser
ClojureScript smoke evidence is in `M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is
accepted. The consolidated source, bounds, security, parity and browser review is
recorded in `M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is accepted. M13.1 is the
accepted read-only external Steel host architecture/dependency audit, recorded in
`M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. It establishes an external
one-shot host over TCP/NDJSON. The current read-only Steel dependency safety
resolution is complete in
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md` and found no acceptable current release
or upstream commit. M13.2 remains blocked and unauthorized. The read-only Workbench
architecture audit is accepted in `M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The
minimal native Application client is accepted in
`M14_2_MINIMAL_WORKBENCH_CLIENT.md`; the M14.3 client-owned model and persistence
implementation is accepted after focused model-state and recovery-projection
remediation in `M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The completed minimal
eframe/egui GUI and its dependency, ownership, rebuild, live-plot and native Windows
smoke evidence and focused overflow/live-continuity remediation are in
`M14_4_MINIMAL_GUI.md` and accepted. The typed M14.5 operator controls,
property/configuration workflows, real Runtime acceptance, and native GUI smoke are
accepted in `M14_5_OPERATOR_CONTROLS.md` at implementation commit
`ab097ed5207ea426cbbd48611015da12ce534a43`. The recovery/fault audit in
`M14_6_RECOVERY_FAULT_AUDIT.md` is accepted at commit
`4e18801930939404a8e856521b86c55c20d71dc9`. Only the renderer-neutral recovery
status UI and one-shot `operation_status` slice M14.6B1 is accepted. M14.6B2A
quarantine projection/restart classification is accepted at implementation commit
`f945d910cfeabbe8552634c53d95016e79a6eafa`. Only M14.6B2B1 Exact Retry core and
evidence lifecycle is authorized; Exact Retry GUI, automatic fault reattach,
consolidated process acceptance, practical integration, and final polished release
documentation remain gated.

## Later practical validation

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

Arduino remains an external practical integration exercise, not retroactive M11
evidence. The published `v0.1.0-preview.1` artifact remains TCP/NDJSON-only. The
accepted post-preview M12.3 implementation adds an optional WebSocket adapter for
the same Application sessions, operations and DTOs.

## Residual limitations

No multi-day unattended soak, physical disk-full, real power-loss, exhaustive
USB/driver or Arduino fault-injection, hard-real-time, physical-heater-effect or
remote-security qualification is claimed. Final release-quality documentation is not
complete.
