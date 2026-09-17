# AI handoff

## Current state

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: AUTHORIZED
M9B.1 API audit: COMPLETE
M9B.2 protocol / error / operation foundation: COMPLETE
M9B.3: COMPLETE
M9B.4: NOT STARTED
M9C+: NOT AUTHORIZED
Current phase: M9B — complete and stabilize Application API
```

M9B is the only authorized implementation milestone. Do not begin M9C, cleanup,
hardening, packaging or release work automatically.

## Accepted baselines

M7 Recorder/SQLite is externally accepted at `f3ff456`. M8 implementation, final
read-only hardware acceptance and final software gates are accepted. M9A neutral
managed-component architecture, native Rust execution and the three focused review
corrections are externally accepted at
`923dd78345c319d07f4937802fd636eebbb8264e`.

M9A provides one implementation-neutral `Invocation -> ComponentExecutor ->
ComponentCompletion/ComponentResult -> Runtime validation -> commit` lifecycle.
Native `native.moving_mean.v1` and the temporary Lua adapter use that same bounded
path. Text-backed component provenance is content-addressed; built-in provenance
identifies the actual Runtime executable bytes.

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
further M8 hardware rerun is required. COM5 is not part of current work.

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

M9B.3 is complete. Stop until a new instruction authorizes M9B.4. The
remaining M9B domains are discovery, measurements/history/subscriptions,
Reference/control, Recorder, resources/configuration/reconnect, virtual/emulator
behavior and full reconnect/resynchronization.

M9B has no Presentation API, frontend/client implementation or bundled scripting
environment. Testing uses Rust, protocol and integration tests.

After explicit M9B acceptance, M9C may remove active Lua and obsolete first-party
client baggage while preserving historical evidence. M10-M12 remain blocked. The
roadmap ends at v0.1.0.
