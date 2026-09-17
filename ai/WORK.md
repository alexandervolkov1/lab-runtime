# Current work — M9B Application API completion and stabilization

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: AUTHORIZED
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3: COMPLETE
M9B.4: COMPLETE
M9B.5: COMPLETE
M9B.6: COMPLETE
M9B.7: NOT STARTED
M9C+: NOT AUTHORIZED
Current phase: M9B — complete and stabilize Application API
```

This is the only current implementation authorization. Do not begin M9C, M10, M11
or M12 automatically.

The read-only M9B.1 audit is complete in
`docs/implementation/MILESTONE_9B_API_AUDIT.md`. M9B.2 implemented the common
protocol/error/operation foundation; M9B.3 completed read-side discovery,
measurement/history and subscription semantics; M9B.4 completed Reference and native
controller/PID operations; M9B.5 completed the semantic Recorder status, lifecycle,
event and durable-boundary API; M9B.6 completed resources, configuration/property
and reconnect semantics. These are recorded in
`docs/implementation/MILESTONE_9B_REPORT.md`. Stop before M9B.7; a new instruction
must authorize the next implementation slice.

## Goal

Expose complete useful Runtime semantics through one coherent bounded local
Application API.

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

M9B extends laboratory/runtime semantics only. It must not add workspace, plot,
trace, button, panel, tab, color, layout or window concepts.

## Required Runtime priority

Periodic native acquisition remains primary Runtime work:

```text
authoritative / required Runtime work
├── transport scheduling
├── periodic instrument polling
├── measurement publication
├── native controller deadlines
├── OutputAuthority
├── Recorder ingestion
└── required lifecycle/safety work

non-authoritative extension/external work
├── managed components
└── API clients
```

This is an architectural scheduling priority, not a Windows thread-priority class.
Slow, stalled or disconnected managed components or clients must not prevent required
acquisition, native control, safety or Recorder progress.

## Scope

### Discovery

Provide coherent discovery for:

- resources and transports;
- instruments;
- signals/measurement series;
- capabilities;
- validated properties;
- implementation-neutral status.

### Measurements

Expose:

- latest values;
- quality and engineering units;
- generation/revision where semantically required;
- periodic acquisition state;
- bounded live subscriptions/events;
- bounded and paged history.

Keep query snapshots free of hidden physical I/O.

### References and control

Expose:

- Reference lifecycle and configuration;
- controller/PID configuration;
- supported controller start, pause, resume and stop operations;
- controller status;
- validated operation results and conflicts.

All physical output remains behind Runtime-owned OutputAuthority.

### Recorder

Expose appropriate public operations and queries for:

- start, stop and status;
- current run/interval state;
- durable failure and coverage status;
- bounded history/query surfaces.

Recorder/SQLite remains durable scientific and experiment audit history. It is not a
diagnostic log stream or a substitute for the live Application API.

### Resources, transports and configuration

Expose:

- Runtime/resource status;
- validated configuration operations;
- reconnect;
- safe reload/rebind semantics where applicable;
- explicit configuration revisions and conflict behavior.

Preserve no-blind-retry, no-automatic-rearm and ambiguous-write safety rules.

### Virtual and emulator API

Expose validated operations for:

- explicitly declared virtual instruments;
- emulator publication/model steps;
- deterministic fault injection where appropriate;
- typed value, unit, quality and lifecycle fencing.

An emulator or client must never fabricate:

- a physical instrument observation;
- physical ACK;
- physical readback;
- transport completion;
- safe-output evidence.

### Protocol and API quality

Complete and normalize:

- coherent operation names;
- structured errors;
- capability discovery;
- protocol/API versioning;
- bounded request and response sizes;
- subscriptions and backpressure;
- client disconnect/reconnect;
- resynchronization after gaps or stale cursors;
- malformed/untrusted input behavior;
- resource-exhaustion behavior;
- finite shutdown behavior.

Testing is through Rust, raw-protocol and integration tests. Do not build a first-party
client or SDK merely to exercise the API.

## Existing contracts to preserve

- `lab-core` remains OS-, storage-, presentation- and adapter-independent.
- Runtime remains the sole authoritative mutable experiment owner.
- Query, Command and Operation semantics remain distinct.
- Lua and native implementations continue to use the one accepted neutral managed
  lifecycle until Lua is removed in a later authorized M9C.
- Managed execution retains its fixed worker/mailbox bounds and cannot block owner
  safety progress.
- Generation/revision fencing, scoped component failure and bounded PlainData remain.
- Recorder remains Runtime-owned, bounded and fail-closed when Required.
- Client disconnect does not end Runtime-owned acquisition/control/recording.
- Historical SQLite archives and milestone reports remain immutable.

## Non-goals

Do not implement:

- Presentation API or server-owned presentation state;
- workspace, plot, trace, button, panel, tab, color, layout or window concepts;
- GUI or any first-party frontend;
- client SDK;
- Babashka, Python or other release client work;
- bundled scripting environment;
- Steel;
- Lua removal (M9C);
- broad Core cleanup (M10);
- release-hardening or logging redesign outside what M9B directly requires;
- packaging or final documentation work;
- new physical protocol/hardware acceptance.

## M9B documentation and review gate

Document the final API surface, bounds, ownership, error behavior, versioning,
subscriptions/backpressure, resynchronization and emulator safety boundary. Update
active coordination documents and create an M9B implementation report following the
existing milestone convention.

M9B completes only after focused and full debug/release/fmt/Clippy/rustdoc/protocol
gates pass and explicit external review accepts it. Do not authorize or start M9C
automatically.
