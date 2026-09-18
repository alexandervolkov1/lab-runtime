# Current work — M10 core cleanup and studyability

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M10: AUTHORIZED
Current phase: M10 — core cleanup and studyability
M11+: NOT AUTHORIZED
```

M10 is the only authorized implementation milestone. It is a structural/readability
milestone for the nearly final v0.1 codebase, not a product-semantics milestone. Do
not begin M11, M12 or post-v0.1 work without an explicit review decision.

## Goal

Make the codebase exceptionally easy to understand and study while preserving all
accepted behavior, safety properties and externally visible contracts.

M10 may:

- split oversized modules by clear responsibility;
- clarify module and ownership boundaries;
- improve names and architectural rustdoc;
- remove dead compatibility code;
- reduce unnecessary indirection;
- make important Runtime, API, acquisition, component, safety and Recorder paths
  easier to trace.

Do not create speculative frameworks merely to improve the directory tree. Prefer
small reviewable structural slices under existing tests.

## Required preservation

Every M10 change must preserve:

- the Runtime as sole authoritative mutable experiment owner;
- periodic acquisition and measurement semantics;
- the Application API contract accepted in M9B;
- neutral native managed-component execution and `native.moving_mean.v1`;
- OutputAuthority, controller lifecycle and all output-safety rules;
- Recorder behavior, schema, provenance and client independence;
- virtual-instrument and emulator behavior and the physical/virtual boundary;
- all documented bounds, overflow policy and backpressure;
- reconnect ordering, generation/revision/epoch fencing and stale-result rejection;
- accepted Windows COM and Metakon hardware-facing behavior.

Required Runtime work remains architecturally prior to managed components and API
clients. Structural cleanup must not introduce blocking, unbounded queues or new
mutable owners.

## Prohibited scope

M10 must not:

- add product features or change Application API semantics;
- add GUI, Presentation API, client SDK or first-party client;
- restore Lua or add any scripting/runtime/plugin replacement;
- change Recorder schema merely for readability;
- redesign controller or output safety;
- change the Metakon protocol or accepted serial/reconnect semantics;
- weaken bounds, backpressure, shutdown or failure behavior;
- begin M11 hardening, M12 packaging/documentation release work or post-v0.1 work.

Additional hardware testing is not required for ordinary structural work. If a later
M10 change unexpectedly touches hardware-facing semantics, stop and obtain an
explicit hardware-test decision rather than opening COM5 automatically.

## Accepted starting point

M9C removed active Lua, executable-text configuration, source reload, the
transitional snapshot family and the Babashka client while preserving the neutral
managed-component architecture and implementation-neutral process coverage. The
active Application API contains 42 operations and 25 capabilities.

The supplementary post-M9C read-only Metakon smoke passed. Its report is
`docs/implementation/POST_M9C_HARDWARE_SMOKE.md`; its clean archive is:

```text
examples/metakon-513-post-m9c-smoke.sqlite
SHA-256 098f2fdc31805cbcdfe46d04055a205c9f980fcf6955ccf8ede6043cbc00a9fc
```

It confirmed generation-1 acquisition, one explicit generation 1 -> 2 reconnect,
the expected generation-2 rebind baseline, resumed ordinary Good acquisition,
Application API current/history/subscription, sealed complete Recorder evidence,
zero outputs, zero gaps and clean shutdown. No production or test change was needed.
Historical M8 evidence remains immutable.

## Verification and review boundary

Use verification proportional to each structural slice, including focused tests for
the moved responsibility and the normal workspace gates before review. Treat any
behavioral change, contract ambiguity, safety change or hardware-facing change as a
scope boundary rather than folding it into cleanup.

M10 does not cross its external review gate automatically. M11 and later milestones
remain unauthorized.
