# AI handoff

## Current state

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
Preview reference/package work: NOT STARTED
Final release documentation/audit: NOT AUTHORIZED
```

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

## Next authorized direction

Developer Preview Preparation will cover:

1. concise architecture/concepts;
2. compact complete Application API reference;
3. Recorder/SQLite archive reference;
4. safety/failure/recovery cheat sheet derived from the M11 matrix;
5. configuration and instrument/component extension notes;
6. preview-sufficient getting-started/build/run instructions;
7. preview packaging/build.

Do not begin these deliverables without a separate task. Do not begin final polished
release documentation automatically.

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
evidence. WebSocket is optional future transport over the same Application semantics
and is not required for the first Clojure/Clay workflow.

## Residual limitations

No multi-day unattended soak, physical disk-full, real power-loss, exhaustive
USB/driver or Arduino fault-injection, hard-real-time, physical-heater-effect or
remote-security qualification is claimed. Final release-quality documentation is not
complete.
