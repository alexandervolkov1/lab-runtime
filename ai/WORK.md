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
M11.1 hardening and failure-model audit: COMPLETE
M11 implementation: NOT STARTED
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

## Completed audit and next gate

M11.1 inspected the accepted post-M10 codebase and recorded the normative fault
model, guarantee levels, bounds, missing coverage, diagnostic-logging gap and
ordered implementation sequence in
`docs/implementation/MILESTONE_11_HARDENING_AUDIT.md`.

The audit found that accepted acquisition, control/OutputAuthority, Recorder and
Application failure behavior is already strongly bounded and regression-tested.
The highest-priority remaining work is a bounded diagnostic log, a combined
physical loss/ambiguity-reconnect-no-rearm oracle, explicit managed-worker death
handling, targeted configured transport/SQLite/process pressure coverage and
bounded soak acceptance.

M11 implementation has not started. The next possible slice is M11.2, but it is not
authorized by this audit task and must wait for a separate instruction. M12 and
later milestones remain unauthorized.

The completed audit covers:

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

It covers periodic acquisition, monotonic cadence/drift, bounded queues,
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

Do not begin M11.2 or make production/test changes automatically. M12 and later
milestones remain unauthorized.
