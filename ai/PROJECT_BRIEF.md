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

The product contains no GUI, Presentation API, bundled client, scripting runtime or
dynamic plugin system.

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
M12 consolidated external review: READY FOR EXTERNAL REVIEW
M13: NOT AUTHORIZED
Current phase: M12 consolidated external review
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.
The accepted M12.5 implementation/evidence commit is
`f291edf35a7805245ce19ea088b9ee899d57f30e`.

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
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is ready for external review.

Final polished release documentation is a later gate.

## Residual limitations

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification;
- release-quality documentation is incomplete.
