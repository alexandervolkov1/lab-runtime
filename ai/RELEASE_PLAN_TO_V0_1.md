# Release plan to v0.1.0

## Current status

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
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: AUTHORIZED
M14.6B2B2 Exact Retry GUI: NOT AUTHORIZED
M14.6B3 explicit-disconnect versus bounded fault reattach: NOT AUTHORIZED
M14.6B4 real-process recovery/fault acceptance: NOT AUTHORIZED
Current phase: M14.6B2B1 Exact Retry core / evidence lifecycle
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

The consolidated M12 source, bounds, security, parity and real-browser evidence is
recorded in `M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is accepted. The read-only
M13.1 external Steel host architecture/dependency audit is accepted in
`M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. Steel dependency safety
resolution is complete in `M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md` and found no
acceptable current dependency candidate. M13.2 remains blocked and unauthorized.
The read-only M14.1 Workbench architecture audit is accepted in
`M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The M14.2 minimal native Application client
is accepted in `M14_2_MINIMAL_WORKBENCH_CLIENT.md`; the client-owned model and
persistence in `M14_3_WORKBENCH_MODEL_PRESENTATION.md` are also accepted. The minimal
eframe/egui GUI and focused remediation are accepted in `M14_4_MINIMAL_GUI.md`.
M14.5 operator controls and property/configuration workflows are accepted in
`M14_5_OPERATOR_CONTROLS.md` at implementation commit
`ab097ed5207ea426cbbd48611015da12ce534a43`. The recovery/fault audit in
`M14_6_RECOVERY_FAULT_AUDIT.md` is accepted at commit
`4e18801930939404a8e856521b86c55c20d71dc9`. M14.6B1 recovery status UI and
one-shot `operation_status` are accepted at
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`. M14.6B2A quarantine projection
and restart classification are accepted at implementation commit
`f945d910cfeabbe8552634c53d95016e79a6eafa`. M14.6B2B1 is authorized;
M14.6B2B2–B4 remain unauthorized.

Functionally complete means the accepted Runtime behavior is implemented and no
known preview-blocking correctness, safety or durability defect remains. It does not
mean production certification, exhaustive hardware qualification or final release
readiness.

## Completed preview preparation

The preview package needs concise, current developer material:

- architecture/concepts;
- complete Application API semantics and bounds;
- Recorder/SQLite schema, provenance, gaps, sealing and read-only inspection;
- safety/failure/recovery guarantees and evidence limits;
- configuration and instrument/component extension paths;
- build, run and getting-started workflow;
- bounded diagnostics collection;
- reproducible preview packaging.

This preparation did not change the accepted 42-operation / 25-capability API,
SQLite schema, scheduler, output safety or Runtime ownership.

## Current path to v0.1.0

```text
v0.1.0-preview.1
    ↓
M12 WebSocket transport
    ↓
M14 GUI / Workbench / client-owned PresentationDocument
    ↓
future optional Steel-in-Workbench scripting, after dependency acceptance
    ↓
thermal-plant-uno end-to-end integration validation
    ↓
final documentation / packaging / review
    ↓
v0.1.0
```

M12 adds a bounded loopback WebSocket/JSON adapter to the same Application API,
session state and global client budget as TCP/NDJSON. It must not create transport-
specific operations, DTOs, errors, history or experiment semantics.

The original separate external Steel-host implementation is deferred while its
dependency graph is blocked. A later accepted Steel implementation should preferably
be an optional Workbench subsystem using the same external Application client
boundary. Steel crash or script termination must not destroy authoritative
experiment state:

```text
script lifetime != experiment lifetime
```

M14 keeps GUI/Workbench behavior in a separate client. A future
`PresentationDocument` is client-owned and transport/language-neutral; it is not a
Runtime experiment-semantic DTO.

Across M12-M14:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.
```

No UI or scripting semantics may enter Runtime core.

## End-to-end integration validation

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

This validates the completed transport, external scripting-client and presentation
boundaries in practice. It is not retroactive evidence for M8-M11 acceptance and
does not change physical-evidence limits.

## Final release gate

Before v0.1.0, complete the polished reference/tutorial set, supported-platform and
recovery guidance, archive documentation, licensing/checksum review, clean Windows
build/package acceptance and external release review.

The final package should contain the executables, example deployment material,
instrument definitions, developer documentation, applicable license files and
checksums. It must not bundle historical AI reports, development fixtures or
obsolete Lua/Babashka clients. GUI/Workbench packaging, if provided, remains a
separate client artifact and does not enter Runtime core.

## Residual qualification limits

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification.

These limits are explicit and do not currently block developer preview.
