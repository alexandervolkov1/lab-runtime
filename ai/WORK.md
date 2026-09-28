# Current work — M13 Steel dependency safety resolution

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M13 Steel dependency safety resolution
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
M13 dependency safety resolution: READY FOR EXTERNAL REVIEW
M13.2: BLOCKED ON STEEL DEPENDENCY SAFETY
M13.2: NOT AUTHORIZED
M14: NOT AUTHORIZED
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
M13 dependency safety resolution: READY FOR EXTERNAL REVIEW
M13.2: BLOCKED ON STEEL DEPENDENCY SAFETY
M13.2: NOT AUTHORIZED
M14: NOT AUTHORIZED
```

M13.1 is accepted in `M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. The external
one-shot TCP/NDJSON host architecture is closed. The current slice is a read-only
investigation of current upstream Steel and `mattwparas/steel-imbl` to determine
whether an upstream-owned release or exact commit removes RUSTSEC-2026-0255 and the
abandoned `im-rc` chain without a lab-runtime-owned fork.

The investigation is complete in
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`. Released `steel-core 0.8.3` and current
upstream master retain the mandatory `im-rc -> sized-chunks` memory-safety blocker.
The optional `steel-imbl` path adds separate `imbl-sized-chunks` and `bitmaps`
memory-safety advisories and does not remove the old graph. No acceptable candidate
was found.

No Cargo file or production source changed. Steel was not added to the workspace,
`apps/lab-steel` was not created, and M13.2/M14 remain unauthorized.

```text
STATUS: M13_DEPENDENCY_RESOLUTION_READY_FOR_EXTERNAL_REVIEW
```
