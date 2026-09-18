# AI handoff

## Current state

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3: COMPLETE
M9B.4: COMPLETE
M9B.5: COMPLETE
M9B.6: COMPLETE
M9B.7: COMPLETE
M9B.8: COMPLETE
M9B.9: COMPLETE
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: HARDWARE ACCEPTANCE BLOCKED
M10: NOT AUTHORIZED
Current phase: M9D — physical Metakon output integration
M11+: NOT AUTHORIZED
```

M9B and M9C implementation and external review are accepted. The supplementary
post-M9C real-device smoke passed. M9D software integration and the read-only
shutdown correction are committed. The authorized write acceptance stopped on a
harness defect after production startup safe-zero and before controller start or any
nonzero proposal. Do not retry a physical write or begin M10 without a new explicit
decision.

## M9D blocked hardware gate

The authority-gated WRITE/ACK/separate-readback path passed debug/release workspace
tests, clippy and warning-denied rustdoc. The failed preflight exposed a
`ServiceHost` terminalization defect, not a second transport owner: it froze the
first nonblocking COM retirement `Pending` result while Recorder was already
flushed. Commit `b6e1840` waits for the later retirement observation under the same
finite deadline. A corrected read-only COM5 run proved channel type 3, four Good
temperature samples, zero unfinished transports, clean shutdown and exit 0. No
nonzero Metakon WRITE was attempted. A later acceptance attempt completed one
startup safe-zero WRITE/ACK/readback, then its PowerShell harness failed before the
first API query. Its unsealed WAL archive is failure evidence, not M9D acceptance
evidence. See `docs/implementation/MILESTONE_9D_REPORT.md`.

## Accepted baselines

M7 Recorder/SQLite is externally accepted at `f3ff456`. M8 implementation, final
read-only hardware acceptance and final software gates are accepted. M9A neutral
managed-component architecture, native Rust execution and the three focused review
corrections are externally accepted at
`923dd78345c319d07f4937802fd636eebbb8264e`.

M9A provides one implementation-neutral `Invocation -> ComponentExecutor ->
ComponentCompletion/ComponentResult -> Runtime validation -> commit` lifecycle.
Native `native.moving_mean.v1` uses that bounded path. M9C removed the temporary Lua
adapter and executable-text artifacts; BuiltIn provenance identifies the actual
Runtime executable bytes.

## Preserved hardware evidence

Do not modify, reuse or reopen these archives through Runtime:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

The successful archive proves real generation-1 acquisition, device-off transition,
one explicit reconnect, bounded transient open retry, generation-2 compatibility and
acquisition, durable lifecycle ordering, zero outputs/gaps and clean shutdown. No
further M8 hardware rerun is required. M9D used COM5 only for the bounded read-only
preflight described above and stopped before any output write.

The supplementary post-M9C read-only smoke is recorded in
`docs/implementation/POST_M9C_HARDWARE_SMOKE.md`. Its clean acceptance archive is:

```text
examples/metakon-513-post-m9c-smoke.sqlite
SHA-256 098f2fdc31805cbcdfe46d04055a205c9f980fcf6955ccf8ede6043cbc00a9fc
```

It proves the real Metakon read path, channel type 3, Good acquisition before and
after one explicit generation 1 -> 2 reconnect, Application API
current/history/subscription, sealed complete Recorder run/interval, zero outputs,
zero gaps and clean shutdown. The initial failed smoke was a harness interpretation
defect: it mistook the intentional generation-2 Unavailable/Transport rebind
baseline for an ordinary acquisition result. Existing M8 evidence, production code
and tests establish the same ordering; no production or test change was required.

## Final v0.1 direction

v0.1 is a polished headless laboratory automation Runtime. It ships `lab-core`,
`lab-runtime`, native instruments and managed components, periodic acquisition,
measurement signals/history, Reference, native control, OutputAuthority, Recorder,
configuration/resource/reconnect lifecycle, virtual/emulated instruments, one local
Application API, bounded diagnostics, English documentation and Windows packaging.

It does not ship a GUI, Presentation API, Runtime-owned plotting/workspace state,
Steel, Lua, Babashka/Python clients, a frontend, client SDK or user-facing scripting
environment.

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

Required transport, polling, measurement, controller, OutputAuthority, Recorder and
lifecycle/safety work has architectural priority over managed components and API
clients. Slow extension or client work must not block required Runtime progress.

Recorder/SQLite is durable scientific and experiment audit history. Diagnostic logs
are separate bounded troubleshooting information. The Application API, Recorder
contract and SQLite schema are distinct contracts.

## M9B implementation progress

M9B.2 now provides explicit `lab-runtime.application` protocol identity/versioning,
one operation registry shared by framing and hello, structured composition-aware
capabilities, bounded typed public errors and normalized accepted/terminal operation
responses. Unknown operations are nonfatal; malformed input remains reactor-scoped;
oversized outgoing results become correlated client errors instead of stopping the
service. Existing session/dedup/disconnect ownership remains intact.

M9B.6 is complete. Resources, configuration status, generic property metadata and
validated live property mutation now use the existing deployment/revision lifecycle;
reconnect remains the accepted generation-fenced M8 operation.

The M9B.6 optimized release gate also passes. Its two observed optimized-only
failures were test timing defects: a yield-count loop raced the serial worker's
legitimate retry park, and one lifecycle test injected a future 1 ms timestamp ahead
of the service clock. Test synchronization/time input was corrected without changing
production serial, lifecycle or Application API behavior.

M9B.7 is complete. Explicitly configured external-publication virtual signals accept
bounded Good/Unavailable observations through the ordinary Runtime measurement,
event, recent-history, controller and Recorder paths. Native thermal model restart
is now the distinct `virtual_models_restart` operation; the mixed public
`restart_models` name is retired.

M9B.8 acceptance is complete. Reconnect reconstruction, operation replay,
slow-client gap recovery, subscription churn, emulator/Recorder pressure, malformed
traffic and process-level client isolation passed. One connection-local durable
history defect was fixed: continuation tokens are now capped at eight, one per
connection, and released on disconnect.

M9B.9 is complete. The final report now contains the exact 46-operation and
27-capability inventories, identity/current/history/subscription/mutation contracts,
structured error taxonomy, authoritative bounds, authority and extensibility
invariants, transitional M9C inputs and the external-review checklist. External
review accepted M9B at `d228d697c01a4b77d55333655129f95ee784c07c`.

M9B has no Presentation API, frontend/client implementation or bundled scripting
environment. Testing uses Rust, protocol and integration tests.

M9C removed active Lua, executable-text configuration, source reload, the transitional
snapshot family and the Babashka client. The final registry has 42 operations and 25
capabilities. Neutral Rust/process tests retain managed-component and client-isolation
coverage; historical Recorder provenance compatibility remains. M10 may now improve
structure and studyability without changing accepted semantics. M11-M12 remain
blocked. The roadmap ends at v0.1.0.
