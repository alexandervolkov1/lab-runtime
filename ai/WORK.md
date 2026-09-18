# Current work — M11.1 hardening and failure-model audit

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: ACCEPTED
M11: AUTHORIZED
M11.1 hardening and failure-model audit: AUTHORIZED — NOT STARTED
Current phase: M11 — runtime / Recorder / logging / API hardening
M12+: NOT AUTHORIZED
```

M9D software integration, physical acceptance and external review are complete. Its
accepted production path is:

```text
native controller
→ OutputProposal
→ OutputAuthority
→ bounded ResourceExecutor
→ final authority/generation recheck
→ Metakon WRITE reg06
→ strict ACK
→ separate reg06 readback
```

The clean acceptance archive is
`examples/metakon-513-m9d-write-smoke.sqlite`, SHA-256
`14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32`.
It proves `0 → ACK → readback 0`, `+10 → ACK → readback 10`, `0 → ACK →
readback 0` and clean shutdown. The load/heater was physically disconnected, so
physical heater effect was intentionally not tested. Existing M8, post-M9C and M9D
archives remain immutable.

## M10 accepted guarantees

M10 is externally accepted. It made Runtime, HostCore and ServiceHost ownership and
progression easier to navigate without changing owners; organized the Application
API by semantic domain; physically separated the Recorder semantic contract from
SQLite implementation; clarified acquisition, output-safety and extension paths;
and tightened internal visibility and architectural rustdoc.

The accepted guarantees include:

- Runtime remains the sole authoritative mutable experiment owner;
- HostCore composes bounded scheduling and adapter/Recorder orchestration, while
  ServiceHost owns process/deployment/reconnect/shutdown progression;
- the public Application contract remains exactly 42 operations and 25 capabilities;
- public protocol, error, bounds/backpressure and reconnect semantics are unchanged;
- Recorder schema, durability, provenance and historical compatibility are unchanged;
- scheduler ordering and all M9D OutputAuthority/Metakon behavior are unchanged;
- native-component registration is centralized, explicit and compile-time;
- ordinary instrument properties and signals need no generic Application, Recorder,
  history, subscription or SQLite changes;
- an Arduino-like instrument is confined to instrument-specific configuration,
  protocol, composition, scheduling, optional authority-gated output and tests.

## Authorized next task

M11 is authorized, but implementation does not start immediately. The first task is
`M11.1 — hardening and failure-model audit`. It must inspect the accepted post-M10
codebase and produce, before any production change:

- an exact fault matrix for acquisition, control/safety, Recorder/storage and the
  Application/process boundary;
- the immediate Runtime action, controller/output response, retry/no-retry rule,
  Recorder/API result and recovery/rearm rule for each important failure;
- a classification of relevant guarantees as `GUARANTEED`, `BEST EFFORT` or
  `NOT GUARANTEED`;
- an inventory of missing or weak fault/adversarial tests;
- the diagnostic-logging gap and a bounded logging design covering levels,
  destination, size, rotation/retention, startup/shutdown, failures and collection;
- a small, ordered M11 implementation sequence with explicit risk and verification.

The audit must cover periodic acquisition, monotonic cadence/drift, bounded queues,
slow or hung transport, reconnect, starvation resistance and finite shutdown;
controller timing, stale/unavailable input, leases, epoch/generation fencing,
OutputAuthority, ambiguous WRITE, ACK/readback mismatch, safe transition and rearm;
Recorder ingress, durable failure, gaps/sealing, provenance, history pressure,
SQLite locking/failure/integrity and clean/crash-close expectations; and malformed or
slow clients, capacity pressure, reconnect/resync, subscription overflow, history or
emulator pressure, multi-client isolation and finite server shutdown.

Recorder remains durable scientific/audit history. Diagnostic logging is a separate
bounded troubleshooting mechanism; routine worker, TCP and debug messages must not
become unbounded Recorder facts.

M11 is hardening, not feature development. Do not add GUI/Presentation, scripting,
a Clojure client, an Arduino production instrument, new public convenience
operations, new Recorder schema features, final tutorials or polished M12 release
documentation. The planned educational Arduino furnace is outside M11 unless it is
separately authorized as test infrastructure.

M11.1 has not started. Do not begin the audit or make production/test changes
automatically; wait for the separate M11.1 task. M12 and later milestones remain
unauthorized.
