# Current work — M12.2 transport-neutral server seam

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M12.2 implementation
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration phase: NOT STARTED
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: AUTHORIZED
```

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

## Authorized work

M12.2 may extract the transport-neutral JSON codec and the smallest concrete shared
connection/delivery seam while preserving the complete TCP/NDJSON contract. It must
retain one serialized `Application`, one `SessionStore`, all accepted bounds,
ordering, deadlines, fairness, shutdown and client-isolation behavior.

M12.2 must not add WebSocket, HTTP Upgrade, Origin/WebSocket configuration, an async
runtime, public Application semantics, capacity, scripting, presentation concepts,
or changes to Runtime, Recorder, controller, output-authority or physical-safety
semantics. Stop for external review after M12.2; do not begin M12.3 automatically.
