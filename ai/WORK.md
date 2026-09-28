# Current work — M12.3 bounded WebSocket/JSON transport

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M12.3 external review
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration phase: NOT STARTED
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: READY FOR EXTERNAL REVIEW
M12.4 transport parity/fault acceptance: NOT AUTHORIZED
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.

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
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.3 is ready for external review. M12.4 is
not authorized.
