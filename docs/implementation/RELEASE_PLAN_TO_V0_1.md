# lab-runtime — release plan to v0.1.0

## Status

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
post-M9C real-device smoke passed. M9D software integration is complete, but its
real-device write acceptance has not run. The read-only transport shutdown blocker
was corrected and verified on COM5; no physical write occurred. M10 and later work
are not authorized.
The roadmap ends at v0.1.0.

## v0.1 product definition

v0.1 is a polished headless laboratory automation Runtime containing:

- `lab-core` and `lab-runtime`;
- native Rust instruments and managed components;
- periodic instrument polling/acquisition;
- measurement signals/series, latest values and bounded history;
- Reference and native controllers/PID;
- OutputAuthority and output safety;
- Recorder/SQLite durable experiment history and provenance;
- configuration, reconnect and resource lifecycle;
- virtual instruments and emulator support;
- one complete local Application API;
- bounded diagnostic logging;
- extensive professional English documentation;
- Windows release packaging.

The release does not contain a GUI, Presentation API, Runtime-owned plotting or
workspace state, Steel, Lua, Babashka/Python clients, a first-party frontend, client
SDK, bundled plotting application or user-facing scripting environment.

v0.1 does not claim physical actuator qualification.

## Cross-cutting boundaries

```text
Runtime owns experiment semantics.
Clients own presentation semantics.

Query = committed snapshot / no hidden physical I/O
Command / Operation = mutation or work

Application API != Recorder contract != SQLite schema
```

The Application API exposes laboratory and Runtime concepts. It never exposes
workspace, plot, trace, button, panel, tab, color, layout or window concepts.

Preserve central OutputAuthority, distinct ACK/readback/physical-effect states,
authority recheck before first possible output byte, no blind retry after ambiguous
output, finite leases, generation fencing, no automatic rearm, bounded
queues/histories/workers and monotonic scheduling/control time.

Required transport, polling, measurement, native control, OutputAuthority, Recorder
and lifecycle/safety work has architectural priority over managed components and API
clients. Slow or disconnected extension/client work must not block required Runtime
progress. This is not a Windows thread-priority-class requirement.

## Recorder and diagnostic logging

```text
Recorder / SQLite
= durable scientific + experiment audit history

Diagnostic logs
= bounded troubleshooting information
```

Recorder contains semantic experiment facts: measurements, quality/status changes,
run/interval boundaries, controller and component lifecycle, configuration revisions,
resource/reconnect lifecycle, operations, gaps, provenance and other structured
experiment-relevant events. Routine worker/TCP/debug messages do not become
unbounded Recorder content.

Diagnostics are a separate bounded/rotated mechanism. Semantically important
diagnostics may also be represented as structured durable Runtime events.

## Accepted milestones

### M8 — deployment, Windows COM and real read-only acceptance

M8 is externally accepted. It established strict/frozen deployment configuration,
safe lifecycle activation, read-only Windows COM/Metakon acquisition, Required
Recorder integration and bounded explicit reconnect with transient `Disconnected`
open handling.

Its preserved evidence proves real initial acquisition, device-off to Offline,
exactly one reconnect, generation fencing, compatibility probe ordering, durable
lifecycle before resumed acquisition, sustained generation-2 acquisition, zero
outputs/gaps and clean shutdown. No further M8 hardware run is required.

### M9A — neutral managed components and native Rust execution

M9A is externally accepted. It retained one neutral `Invocation`,
`ComponentExecutor`, `ComponentCompletion`/`ComponentResult`, Runtime validation and
generation/revision lifecycle for both adapter-backed and native implementations.

It added bounded native `MovingMean`, scoped source reload, exact component-to-source
provenance and actual executable identity for built-in components. Historical SQLite
archives and provenance remain immutable.

## M9B — Complete and stabilize Application API

Goal: expose complete useful Runtime semantics through one coherent bounded local
Application API.

Completed M9B slices:

- M9B.1 audited the actual public API and established the bounded implementation
  sequence;
- M9B.2 established explicit protocol/Application version identities, one operation
  and capability source, bounded structured public errors, deterministic malformed
  and unknown-operation handling, and correlated exactly-one-terminal operation
  responses;
- M9B.3 completed structured discovery, authoritative current measurements, bounded
  transient/durable history, live subscriptions and explicit gap recovery;
- M9B.4 completed Reference current/mutation and native controller/PID current,
  configuration, lifecycle and event surfaces above the existing OutputAuthority
  boundary;
- M9B.5 completed semantic Recorder status, start/stop, run/interval identity,
  failure and lifecycle-event surfaces without exposing SQLite implementation state.
- M9B.6 completed resource status/discovery, generic typed property metadata and
  validated live mutation, deployment candidate status and semantic reconnect state.
- M9B.7 completed explicit virtual-only external measurement publication, ordinary
  signal/history/Recorder integration and targeted native virtual-model restart.
- M9B.8 completed reconnect/resynchronization, adversarial bounded-pressure and
  multi-client isolation acceptance.

M9B.9 consolidated the complete Application API contract and external-review
checklist. External review accepted M9B at
`d228d697c01a4b77d55333655129f95ee784c07c`.

Required coverage:

- discovery of resources, instruments, signals/series, capabilities, properties and
  implementation-neutral status;
- latest measurements, quality, units, generation/revision where needed, periodic
  acquisition state, live subscriptions/events and bounded/paged history;
- Reference lifecycle/configuration and controller/PID configuration, lifecycle,
  status and validated results;
- Recorder start/stop/status, current run/interval state, durable failure/status and
  appropriate public history/query surfaces;
- Runtime/resource status, validated configuration operations, reconnect, safe
  reload/rebind and explicit revision/conflict behavior;
- virtual instruments, emulator publication/model steps and deterministic fault
  injection with typed/lifecycle validation;
- coherent names, structured errors, capabilities, versioning, request/response
  bounds, backpressure, disconnect/reconnect, resynchronization, malformed input,
  resource exhaustion and finite shutdown.

Virtual/emulator operations can never fabricate physical ACK, physical readback,
transport completion or safe-output evidence.

M9B has no Presentation API, frontend/client implementation or bundled scripting
environment. Acceptance uses Rust, raw-protocol and integration tests.

## M9C — Remove Lua and obsolete client baggage — ACCEPTED

M9C removed active product Lua:

- `lab-lua`, `mlua`, `lua.v1` and the Lua runtime/sandbox;
- Lua source/configuration paths and Lua-only active fixtures;
- active Lua-specific API vocabulary, including `reload_managed_sources` and
  `managed_source_reload`;
- Lua user documentation.

Preserve historical milestone reports, immutable SQLite archives and historical
provenance such as `managed_lua_source`; retain inspection compatibility only where
genuinely required.

Active first-party client baggage was removed: the Babashka client/workflow and its
client-only process acceptance. Required coverage remains in implementation-neutral
Rust/protocol/process integration tests.

The transitional `runtime_snapshot`, `snapshot_page` and `snapshot_release` family
was removed because accepted domain-specific discovery/current surfaces supersede it.

Keep the neutral managed-component types and executor architecture, BuiltIn/native
implementation path, bounded `PlainData`, generation/revision fencing,
`native.moving_mean.v1` and neutral provenance. Active product cleanup must not
rewrite historical M5 documentation, Recorder schemas, immutable archives or
historical provenance names required to understand old evidence.

The active component model becomes:

```text
Application API
      ↓
authoritative Runtime
      ├── native instruments
      ├── native managed components
      ├── native controllers / OutputAuthority
      ├── Recorder
      └── virtual instruments / emulator API
```

M9C adds no replacement scripting language, GUI, Presentation API, client SDK or
post-v0.1 roadmap. It is simplification, not substitution.

The supplementary post-M9C real-device smoke confirmed the accepted Metakon read,
reconnect/generation, Application API and Recorder paths with zero outputs/gaps and
clean shutdown. The apparent first failure was a harness interpretation defect: the
intentional generation-2 Unavailable/Transport rebind baseline was not an ordinary
poll result. No production or test change was required.

## M9D — Physical Metakon output integration — HARDWARE ACCEPTANCE BLOCKED

The software path preserves controller proposals, OutputAuthority, final authority
recheck, bounded Metakon WRITE, strict ACK and distinct register-6 readback. Debug
and release workspace tests plus warning-denied lint/docs pass. The read-only
preflight shutdown defect was corrected under the existing finite bound and a real
COM5 rerun closed with zero unfinished transports. Real-device acceptance remains
blocked pending a separately authorized physical write run; no output command was
sent.

## M10 — Core cleanup and studyability — NOT AUTHORIZED

Refactor only under test coverage, preserving behavior and safety:

- split oversized files by responsibility;
- preserve the single Runtime owner;
- make API -> Runtime -> domain flow easy to trace;
- make periodic acquisition and signal/series lifecycle explicit;
- make transport/instrument, native control, OutputAuthority, Recorder and native
  component extension boundaries obvious;
- remove stale naming, dead compatibility and unused abstractions;
- keep public/private boundaries deliberate.

Developers must be able to find quickly how instruments, polling, measurement
series, controllers, output authorization, Recorder submission, API dispatch and
native managed extensions work. Do not create speculative frameworks.

M10 must preserve authoritative Runtime ownership, periodic acquisition, the M9B
Application API, native managed components, OutputAuthority/controller safety,
Recorder semantics/provenance, virtual/emulator behavior, bounds/backpressure,
reconnect/generation fencing and all accepted hardware-facing behavior. It must not
add features, presentation or scripting; change API semantics or the Metakon
protocol; redesign safety; or change Recorder schema merely for readability.

## M11 — Runtime / Recorder / logging / API hardening

### Acquisition

Audit monotonic scheduling, cadence/drift, bounded queues, slow transport behavior,
starvation resistance, finite shutdown and priority of required native work.

### Control and safety

Audit controller lifecycle, leases, generation/epoch fencing, OutputAuthority,
ambiguous writes, retry behavior and rearm behavior.

### Recorder

Audit complete durable history, provenance integrity, sealing/gaps, bounded ingestion,
failure paths, crash/clean-close expectations and SQLite integrity behavior.

### Diagnostic logging

Implement/document a separate bounded diagnostics mechanism: destination, levels,
size bounds, rotation/retention, startup/shutdown and user collection workflow.

### Application API

Harden names, errors, capabilities/versioning, response bounds, backpressure,
disconnect/reconnect/resync, malformed input, resource exhaustion and shutdown. Add
fault/adversarial tests where useful.

## M12 — Documentation, packaging and final release audit

Documentation is a release gate and is written in professional technical English.

### Architecture and concepts

Document overview/ownership, Runtime/domain separation, instruments, signals/series,
periodic acquisition, Query/Command/Operation, time, transport, Reference,
controllers/PID, OutputAuthority, Recorder/provenance, native managed components,
virtual/emulated instruments, Application API and diagnostic logging.

### Tutorials

Provide step-by-step tutorials for build/run, instrument configuration, polling and
series, latest values, history, protocol subscriptions, Reference, PID with a virtual
plant, Recorder, SQLite and provenance inspection, virtual/emulator configuration,
adding native instruments/components, diagnostics and common failures.

Raw protocol examples are allowed. Do not create or ship a first-party client SDK for
tutorials.

### Reference documentation

Document the Application API, protocol/versioning, operations, errors, capabilities,
configuration schema, instrument definitions, native component contract, diagnostics,
shutdown/recovery and supported platform assumptions.

The Recorder SQLite documentation must cover:

- database purpose and schema version;
- every active table and important column;
- primary/foreign relationships;
- run/interval model and measurements;
- operations/events and object snapshots;
- provenance and component implementation/source/build identities;
- gaps, sealing/completeness and shutdown state;
- timestamps/time semantics and invariants;
- content-addressed data/deduplication;
- migration/version policy;
- example read-only SQL queries.

SQLite is a documented archival format, not the live control interface.

### Rust documentation

Document meaningful public ownership, invariants, bounds, failure and safety seams;
avoid restating Rust syntax.

### Packaging and release audit

Prepare a minimal Windows bundle:

```text
lab-runtime.exe
example configuration
instrument definitions
README/documentation entry point
LICENSE if applicable
checksums
```

Do not ship Lua, Babashka/Python clients, GUI, Steel, development fixtures or
unrelated historical evidence archives. Perform clean Windows release acceptance and
final external release review.

## v0.1 definition of done

v0.1 is done when M8 and M9A remain accepted; M9B-M12 are explicitly reviewed and
accepted; Runtime ownership/safety/Recorder/API invariants are green; the SQLite
archive and local API are fully documented; bounded diagnostics are operational; and
the reproducible checksummed Windows package passes final review.

```text
lab-runtime v0.1

A polished headless laboratory automation runtime with:
- periodic instrument acquisition,
- measurement signals and history,
- native control and output safety,
- durable recording and provenance,
- native Rust extensibility,
- virtual/emulated instruments,
- a documented local Application API,
- bounded diagnostic logging,
- comprehensive English documentation.
```

The release roadmap ends at v0.1.0.
