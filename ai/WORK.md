# Current work — developer-preview phase transition

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: ACCEPTED
M11: ACCEPTED

Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE

Current phase: Developer Preview Preparation
Developer-preview documentation/packaging execution: NOT STARTED
M12 final documentation/release audit: NOT AUTHORIZED
```

## Accepted core state

External review accepted M11 at
`31a39cf02e7d58a42d731368f56164226566c5e8`. The accepted v0.1 Runtime functionality
is implemented and known developer-preview-blocking correctness, safety and
durability defects are closed.

"Functionally complete" does not mean production certification or exhaustive
physical qualification. Work remaining before v0.1 includes preview/reference
material, practical integration validation, full release documentation and
packaging.

The M9D evidence archive remains immutable:
`examples/metakon-513-m9d-write-smoke.sqlite`, SHA-256
`14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32`.
It proves the accepted software/transport ACK and register-6 readback sequence, not
physical heater effect.

## M11 accepted guarantees

- Configured transport failures terminate finitely. Malformed, CRC-failing,
  truncated, silent and disconnected responses cannot create false Good
  observations. Generation fencing and explicit reconnect semantics remain.
- Dead managed-worker capacity terminalizes truthfully, does not remain Busy forever,
  stays scoped and is not automatically respawned without bound.
- Ambiguous started physical writes fail closed without blind retry. ACK, readback
  and physical effect remain distinct. Reconnect and fresh Good input do not rearm;
  stale lease, epoch and generation are fenced. Recovery requires explicit
  `reset_failed` and the accepted lifecycle transition.
- Required Recorder failure remains fail-closed; BestEffort failure does not
  incorrectly stop unrelated native work. Admission is distinct from durability,
  clean close seals truthfully, process-kill archives remain visibly incomplete, and
  the SQLite schema is unchanged.
- Diagnostic logging is bounded, lossy and non-authoritative: INFO default,
  `%LOCALAPPDATA%\lab-runtime\logs`, four retained 4 MiB files (16 MiB), queue 1,024
  and maximum entry 8 KiB. Logging failure never becomes experiment authority.
- The Application boundary remains eight clients with bounded request/reply/event
  queues and bounded NDJSON/JSON complexity. Malformed and slow clients are isolated;
  subscription gaps/resynchronization are explicit; client lifetime remains separate
  from experiment lifetime; native progress is preserved under pressure. The public
  contract remains 42 operations and 25 capabilities with unchanged protocol and
  error semantics.
- Shutdown remains finite and truthful. Unresolved physical ambiguity is never
  presented as proven physical safety.

The authoritative engineering evidence remains in:

```text
docs/implementation/MILESTONE_11_REPORT.md
docs/implementation/MILESTONE_11_FAILURE_MATRIX.md
```

## Current phase scope

Developer Preview Preparation will prepare a compact developer-facing reference and
preview package for the accepted headless core:

1. active-doc cleanup;
2. concise architecture/concepts guide;
3. compact but complete Application API reference;
4. Recorder/SQLite archive reference;
5. safety/failure/recovery cheat sheet based on
   `MILESTONE_11_FAILURE_MATRIX.md`;
6. configuration and instrument/component extension notes;
7. preview-sufficient getting-started/build/run instructions;
8. preview packaging/build.

This transition does not authorize starting those deliverables automatically. A
separate task must define and authorize the next slice. Do not begin final polished
M12 documentation.

## Preserved boundaries and later practical validation

```text
Runtime owns experiment semantics.
Client owns presentation semantics.
```

Do not add GUI/presentation concepts to the Application API. Do not begin Arduino,
Clojure, Clay or WebSocket implementation during this transition.

The planned practical learning/integration phase remains:

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

Arduino remains an external real instrument/emulator and practical integration
exercise, not a prerequisite for M11 acceptance. WebSocket is not required for the
first Clojure/Clay workflow. If evaluated later, it may only be an optional transport
adapter over the same Application sessions, operations and DTOs.

## Residual limitations

These limitations are explicit and are not current developer-preview blockers:

- no multi-day unattended soak;
- no physical disk-full qualification;
- no real power-loss qualification;
- no exhaustive USB/driver fault injection;
- no Arduino hardware fault-injection evidence yet;
- no hard-real-time guarantee;
- M9D did not prove physical heater effect;
- no remote/network-security qualification;
- release-quality documentation is not complete.

## Scope guard

Do not begin developer-preview documentation or packaging without a separately
authorized task. Do not begin Arduino, Clojure, Clay, WebSocket or final M12 work.
Milestone/review gates never advance automatically.
