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
Current phase: Developer Preview Preparation
```

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
- Transport: local bounded TCP/NDJSON.
- Recorder: unchanged SQLite schema with provenance, gaps and lifecycle sealing.
- Diagnostics: INFO default, four retained 4 MiB files, 1,024-entry lossy queue,
  8 KiB entry bound, default Windows path `%LOCALAPPDATA%\lab-runtime\logs`.

## Current preparation scope

The next separately authorized work will create a compact architecture/concepts
guide, Application API reference, Recorder/SQLite reference, safety/failure cheat
sheet, extension notes, getting-started/build/run material and preview package.

Final polished release documentation is a later gate.

## Residual limitations

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification;
- release-quality documentation is incomplete.
