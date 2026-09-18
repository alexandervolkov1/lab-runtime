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
M9D: ACCEPTED
M10: ACCEPTED
M10.1: COMPLETE
M10.2: COMPLETE
M10.3: COMPLETE
M10.4: COMPLETE
M10.5: COMPLETE
M10.6: COMPLETE
M10.7: COMPLETE
M10.8: COMPLETE
M11: AUTHORIZED
M11.1: COMPLETE
M11.2: COMPLETE
M11.3: COMPLETE
M11.4: COMPLETE
M11.5: COMPLETE
M11.6: COMPLETE
M11.7: NOT STARTED
Current phase: M11 — runtime / Recorder / logging / API hardening
M12+: NOT AUTHORIZED
```

M9B and M9C implementation and external review are accepted. The supplementary
post-M9C real-device smoke passed. M9D software integration, hardware acceptance and
external review are complete. Final real-device acceptance proved production
startup zero, one authority-gated +10 percent controller command, strict ACK,
separate matching register-6 readback, normal pause to verified zero, Recorder
sealing and clean shutdown. M10.1 mapped the actual post-M9D codebase. M10.2 added
behavior-preserving internal terminology and source architecture indexes while
retaining historical Recorder compatibility. M10.3 organized the Application API
by semantic domain while preserving the accepted registry and wire contract. M10.4
organized Runtime, Host and Service orchestration without changing authoritative
ownership, scheduler order or lifecycle behavior. M10.5 organized the Recorder
semantic contract, bounded worker and SQLite implementation without changing
schema, lifecycle or durability. M10.6 centralized static native-component
registration and configured instrument composition without changing public behavior.

M11.1-M11.4 consolidated the failure model and hardened acquisition/managed-worker,
controller/output recovery, and Recorder/SQLite crash behavior. M11.5 added bounded
best-effort diagnostics separate from Recorder: a 1,024-record lossy queue, 8 KiB
record cap and four 4 MiB files with stderr fallback and finite shutdown.

M11.6 consolidated the accepted bounded Application/process pressure model. The
real TCP path now has explicit malformed/oversized and eight-client churn oracles;
the earlier one-off nonreader timeout was traced to a test admission race and fixed
with an owner-channel predicate. Public API, capacities and production semantics are
unchanged. No M11.7 work has started.

## M10.6 completion

`managed_executor/registry.rs` is now the one explicit compile-time implementation
selection/composition point. Moving-mean logic and metadata live together, and a
real `cfg(test)` second registration proves generic discovery, property, current,
recent-history and subscription surfaces without new Application/Recorder/SQLite
branches. `host/instruments.rs` is the focused static composition match for declared
instrument kinds; standard signals continue through generic Runtime projections.

Core remains implementation-neutral. No plugin framework, physical protocol,
Application operation, schema, scheduler, OutputAuthority or Metakon behavior was
added or changed. At M10.6 close, M10.7 had not started.

## M10.7 completion

Wire-facing measurement/configuration/Recorder projection modules and native
component registration metadata are now crate-private. The unused managed-executor
barrier/construction path was removed; the active Recorder `WriterBarrier` remains
as a documented integration-test fault seam. Runtime, HostCore, ServiceHost,
Application, OutputAuthority, ResourceExecutor and RecorderWorker now have concise
ownership/lifecycle navigation rustdoc. The integration-test map and comments make
the M9D shutdown/ambiguous-write, WriterBarrier and history-disconnect regression
oracles explicit.

The Application registry, protocol, bounds, schema, scheduling and hardware-facing
semantics are unchanged.

## M10 external-review correction

The first M10 external review found one blocker: `configuration_api` enumerated
`InstrumentDto` variants to build ordinary property records. The correction adds a
neutral `InstrumentPropertySource` contract and typed metadata in the configuration
layer. `InstrumentDto` owns its variant-specific property declaration;
`configuration_api` now performs only generic JSON projection and has no production
reference to `InstrumentDto`.

Exact VirtualMeasurement, ThermalPlant and Metakon property DTOs are frozen by a
focused regression. A second test-only property source proves the real generic
projection seam. Existing property mutation/revision fencing, the 42-operation /
25-capability registry and all accepted safety/storage behavior remain unchanged.
The focused external re-review accepted M10. Ordinary instrument properties and
signals no longer require generic Application branches, and an Arduino-like adapter
is confined to instrument-specific configuration, protocol, composition,
scheduling, optional authority-gated output and tests.

## M11 authorization

M11 is authorized as the final major technical hardening milestone before the
developer-preview/reference phase. M11.1-M11.6 are complete. M11.7 has not started,
and M12 and later work remain unauthorized.

## M10.5 completion

Recorder semantic history/provenance types now live apart from SQLite. The one
`RecorderWorker` remains the host-side lifecycle/credit owner; its lifecycle,
ingress, bounded history jobs, exclusive storage loop and fault injection are
physically discoverable modules. `SqliteStore` remains the sole direct SQLite
implementation and is organized into open/recovery, unchanged schema, writes,
history, provenance and encoding.

All Recorder integration binaries, process-reopen, Required/failure, held-writer,
shutdown and canonical `WriterBarrier` lost-wake regressions pass. The 42-operation
API, schema, transactions, run/interval/provenance semantics and historical
`managed_lua_source`/`managed_component_source` compatibility are unchanged. M10.6
was still gated at M10.5 close.

## M10.4 completion

The authoritative structs and all mutable fields remain unchanged. Core Runtime
command/query routing, controller progression, managed-component commit and physical
I/O correlation now have focused private implementation modules. `HostCore` remains
the single Runtime host owner while composition, configuration/rebind, Recorder
admission, components, lifecycle and scheduler implementations are physically
discoverable. `ServiceHost` still owns startup/process lifecycle; configuration,
reconnect and finite shutdown progression now have focused implementation modules.

The safety-first service order, reconnect generation fence and M9D shutdown handling
of non-terminal `TransportShutdown::Pending` are unchanged.

## M10.3 completion

`application.rs` now owns only bounded Application state, hello/session admission,
top-level routing, mutation lifecycle and shutdown correlation. Focused modules own
discovery, References, controllers/output projections, virtual/emulator operations,
delivery/subscriptions, typed mutation routing, common error mapping and asynchronous
Recorder/history delivery. Existing `measurements.rs`, `recorder_api.rs` and
`configuration_api.rs` now contain their corresponding API semantics.

`protocol::OPERATIONS` remains the sole authoritative 42-operation registry. Its
regression now freezes exact operation names, query/mutation kinds, availability,
argument allowlists and capability witness mapping.

## M10.2 completion

M10.2 renamed only misleading internal delivery/configuration/virtual-model and
OutputAuthority state names. It retained the exact 42-operation Application API,
wire vocabulary, SQLite schema and historical `managed_lua_source`,
`managed_component_source`, `reload_managed_sources` and `restart_models` archive
readability. Crate/module rustdoc now indexes Runtime ownership, periodic acquisition,
physical output, Application dispatch, Recorder/SQLite, native components,
transport/protocol and configuration/deployment.

Debug/release workspaces, warning-denied Clippy/rustdoc and focused M9B.8, M9D,
native-component, registry/projection and historical provenance suites pass. COM5
was not opened and no hardware test was performed. See
`docs/implementation/MILESTONE_10_REPORT.md`.

## M9D accepted physical-output architecture

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

Runtime remains the sole authoritative mutable owner. Physical writes require
OutputAuthority, and raw transport write bypass is not part of the ordinary
production surface. Finite leases, epoch/generation fencing and the final authority
recheck before the first possible output byte remain mandatory. ACK, readback and
physical effect are distinct. Timeout after send-started is not proof that no write
occurred, so there is no blind retry. An ambiguous started safe WRITE remains
latched and non-retriable while its safe obligation stays recorded; unresolved
ambiguity prohibits normal output. Safe transition does not automatically rearm the
controller.

## M9D external review record

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
evidence. The corrected harness passed offline validation and reached the real output
query, but rejected a valid settled snapshot because `requested` was null and ACK
and readback shared one owner timestamp. Cleanup performed another verified zero and
clean shutdown. No controller or nonzero output ran; the second sealed archive is
also diagnostic only. See `docs/implementation/MILESTONE_9D_REPORT.md`.

The final harness asserted terminal semantic state instead of transient request
storage and accepted non-decreasing owner time while still requiring distinct ACK
and readback fields/stages. Its complete normal and injected-mismatch paths passed
offline. The one final COM5 run then observed four Good 29.0 degC samples, verified
startup zero, issued exactly one authority-gated +10 percent nonzero command with
matching ACK/readback, paused to verified zero, sealed Recorder run/interval 1 with
complete coverage and zero gaps, and shut down with zero unfinished transports. The
full process issued four register-6 writes (startup zero, +10, pause zero, shutdown
zero), with one nonzero, zero blind retries, zero ambiguous writes and zero
mismatches. The load was disconnected; no physical-effect claim is made.

The first external review blocker is remediated. An ambiguous already-started safe
WRITE now retains the safety obligation but latches resend admission closed, reports
fault-latched/unconfirmed state and cannot rearm or emit a second WRITE. A failure
before first byte remains eligible for later delivery. No automatic ambiguous-write
reconciliation was added; current behavior is deliberately fail-closed. The exact
partial-prefix regression keeps the physical WRITE count at one through repeated
safety turns.

The reported debug failures were fixture synchronization defects, not one shared
production fault. Controller start now refreshes due native work before its input-age
check; the process-reopen harness waits on the exact SQLite barrier predicate and
preserves child status/stderr. Additional latent Recorder/measurement fixture timing
assumptions exposed while establishing the required two consecutive workspace runs
were also corrected without changing product semantics. Both debug workspace runs,
release, Clippy, warning-denied rustdoc and focused safety/fault/shutdown suites pass.
The successful hardware evidence remains valid because it exercised no ambiguous or
mismatching write, and no further COM5 run was performed.

Clean M9D acceptance evidence:

```text
examples/metakon-513-m9d-write-smoke.sqlite
SHA-256 14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32
size 221184 bytes
WAL/SHM absent; SQLite integrity check ok
```

The accepted hardware sequence was `0 → ACK → readback 0`, `+10 → ACK → readback
10`, `0 → ACK → readback 0`, followed by clean shutdown. The load/heater was
physically disconnected, so physical heater effect was intentionally not tested.

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
preflight and bounded output acceptance described above. The final output readback
was zero and COM5 was released.

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
coverage; historical Recorder provenance compatibility remains. M10 improved
structure and studyability without changing accepted semantics and is externally
accepted. M11 is authorized with M11.1 audit first; M12 remains blocked. The roadmap
ends at v0.1.0.
