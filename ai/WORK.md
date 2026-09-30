# Current work — M16.2 focused remediation external re-review

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M16.2 focused remediation external re-review
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration architecture: M16.1 ACCEPTED; M16.2 READY FOR EXTERNAL RE-REVIEW
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: ACCEPTED
M12.4 transport parity/fault acceptance: ACCEPTED
M12.5 browser/ClojureScript smoke acceptance: ACCEPTED
M12: ACCEPTED
M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2 Steel:
BLOCKED ON STEEL DEPENDENCY SAFETY
NOT AUTHORIZED
M14.1 Workbench architecture audit: ACCEPTED
M14.2 bounded native Application client: ACCEPTED
M14.3 WorkbenchModel + PresentationDocument: ACCEPTED
M14.4 minimal native GUI: ACCEPTED
M14.5 operator controls + properties/config: ACCEPTED
M14.6A recovery/fault audit: ACCEPTED
M14.6B1 recovery UI + one-shot operation_status: ACCEPTED
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: ACCEPTED
M14.6B2B2 Exact Retry GUI: ACCEPTED
M14.6B3 bounded fault reattach: ACCEPTED
M14.6B4 consolidated recovery/fault acceptance: ACCEPTED
M14.6 recovery/reconnect/fault acceptance: ACCEPTED
M14 consolidated acceptance: ACCEPTED
M15.1 documentation/productization audit: ACCEPTED
M15.2 README + getting started: ACCEPTED
M15.3 architecture + Application API reference: ACCEPTED
M15.4 Workbench user guide: ACCEPTED
M15.5–M15.8:
DEFERRED UNTIL M16 CONSOLIDATED ACCEPTANCE
NOT AUTHORIZED
M16.1 declarative simple-device architecture/API audit:
ACCEPTED
M16.2 read-only simple-device vertical slice:
FOCUSED REMEDIATION COMPLETE / READY FOR EXTERNAL RE-REVIEW
M16.3–M16.7: NOT AUTHORIZED
STATUS: M16_2_FOCUSED_REMEDIATION_READY_FOR_EXTERNAL_RE_REVIEW
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.
The accepted M12.5 implementation/evidence commit is
`f291edf35a7805245ce19ea088b9ee899d57f30e`.
The accepted M12 consolidated review-ready commit is
`bb159f674f2cca1c50f903528b268cb070858604`.
The accepted M13.1 audit commit is
`81e303de18021c49c526b12bb1f77f8ea75ae2d9`.
The accepted M13 dependency-safety resolution commit is
`7a150cd8e15d990e00ad62b5c9d9b66c401b7086`.
The accepted M14.1 architecture-audit commit is
`6340aee32b563000bc6397a52aecd85ee945da1c`.
The accepted M14.2 implementation commit is
`bdedf9455305f693a9537f698403c3bbb3840c51`.
The accepted M14.3 implementation commit is
`f01567ba2165b24b9551e3b0acf5b100b0169f32`.
The accepted M14.4 implementation commit is
`116aba631fe47ea24412dd3d4b50e9b00eafc8df`.
The accepted M14.5 implementation commit is
`ab097ed5207ea426cbbd48611015da12ce534a43`.
The accepted M14.6A audit commit is
`4e18801930939404a8e856521b86c55c20d71dc9`.
The accepted M14.6B1 implementation commit is
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`.
The accepted M15.1 documentation/productization audit commit is
`8cd89e2b58ba11248c2ce2532c165270a9c60797`. M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. The accepted M15.3 architecture +
Application API reference implementation commit is
`48383185309fc6810dff54b9656067280a535171`. The accepted M15.4 Workbench user guide
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15 is paused
without consolidated acceptance. M15.5–M15.8 are deferred and unauthorized until M16
consolidated acceptance. M16.1 is accepted; M16.2 focused remediation is complete
and ready for external re-review; no later M16 slice is authorized.

## Completed preparation step

The repository documentation surface was cleaned before preview-reference writing:

- keep only current developer-facing documentation outside `ai/`;
- keep active coordination concise and current;
- archive only safety rationale and engineering evidence useful for future regression
  investigation;
- delete superseded milestone, POC, migration and obsolete-technology documents;
- preserve accepted artifacts and fix references after moves.

The cleanup changed documentation and repository organization only. It did not modify
production code, tests, Cargo/configuration/schema, API or Runtime/Recorder semantics.

## Completed reference step

The compact public reference now covers architecture/concepts, the complete
Application API, Recorder/SQLite schema and archive semantics, safety/failure
behavior, extension paths, and preview-sufficient build/run instructions.

## Completed packaging step

`scripts/package-developer-preview.ps1` builds the locked release workspace and
creates an allowlisted Windows x86_64 directory, ZIP, SHA-256 sidecar, build
provenance, package manifest and third-party license inventory under ignored
`dist/`. The starter configuration is virtual-only and opens no COM resource.

GitHub pre-release `v0.1.0-preview.1` was built from and tagged at
`70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e`. The tag is immutable; the current
coordination HEAD may be later because this publication record is docs-only.

## Completed M12.1 audit

The read-only source audit is recorded in
`M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md`. It establishes:

- the actual TCP/reactor/Application/ServiceHost boundary;
- current connection, session, dedup, subscription, replay and history ownership;
- exact network/Application bounds and a single global client budget;
- the required split between shared Application JSON and NDJSON framing;
- cross-transport scope reattachment semantics;
- a loopback-only browser Origin policy;
- a synchronous/nonblocking WebSocket library direction without adding a
  dependency;
- M12.2-M12.5 review slices and the future acceptance matrix.

No production Rust, tests, Cargo/dependencies, configuration schema, Application
operation/DTO, Runtime, Recorder, SQLite or safety semantics changed.

## Completed M12.2 implementation

M12.2 extracted the transport-neutral Application JSON codec and the smallest
concrete shared connection/delivery seam while preserving the complete TCP/NDJSON
contract. It retains one serialized `Application`, one `SessionStore`, the global
eight-client capacity, and all accepted ordering, deadlines, fairness, shutdown and
client-isolation behavior. The implementation and verification evidence is
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`.

M12.2 added no WebSocket, HTTP Upgrade, Origin/WebSocket configuration, async
runtime, public Application semantics, capacity, scripting, presentation concepts,
or changes to Runtime, Recorder, controller, output-authority or physical-safety
semantics.

## Completed M12.3 implementation

M12.3 adds one optional IPv4-loopback WebSocket/JSON listener through the accepted
M12.2 coordinator, shared Application JSON codec, bounded delivery path, serialized
`Application`, and `SessionStore`. TCP plus WebSocket retains one eight-client pool.

The endpoint enforces exact path, numeric Host, Origin allowlist and required
subprotocol checks before Upgrade. Handshake/message/frame/write state, text-only
Application messages, ping/pong/close progression, readiness and failure isolation
are all explicitly bounded. It adds no async runtime, TLS, authentication framework,
Application operations/DTOs/errors, UI/scripting semantics, or second
coordinator/Application/session store.

Implementation, dependency review, exact bounds and verification evidence are in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.3 is accepted. M12.4 parity, reconnect,
backpressure and fault acceptance is complete in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` and accepted. M12.5 real-browser
ClojureScript smoke evidence is recorded in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is accepted. The consolidated source,
bounds, security, parity and browser review is complete in
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and accepted. No production Rust changed
during the consolidation.

## Current authorization

```text
M12: ACCEPTED
M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2 Steel:
BLOCKED ON STEEL DEPENDENCY SAFETY
NOT AUTHORIZED
M14.1 Workbench architecture audit: ACCEPTED
M14.2 bounded native Application client: ACCEPTED
M14.3 WorkbenchModel + PresentationDocument: ACCEPTED
M14.4 minimal native GUI: ACCEPTED
M14.5 operator controls + properties/config: ACCEPTED
M14.6A recovery/fault audit: ACCEPTED
M14.6B1 recovery UI + one-shot operation_status: ACCEPTED
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: ACCEPTED
M14.6B2B2 Exact Retry GUI: ACCEPTED
M14.6B3 bounded fault reattach: ACCEPTED
M14.6B4 consolidated recovery/fault acceptance: ACCEPTED
M14.6 recovery/reconnect/fault acceptance: ACCEPTED
M14 consolidated acceptance: ACCEPTED
M15.1 documentation/productization audit: ACCEPTED
M15.2 README + getting started: ACCEPTED
M15.3 architecture + Application API reference: ACCEPTED
M15.4 Workbench user guide: ACCEPTED
M15.5–M15.8:
DEFERRED UNTIL M16 CONSOLIDATED ACCEPTANCE
NOT AUTHORIZED
M16.1 declarative simple-device architecture/API audit:
ACCEPTED
M16.2 read-only simple-device vertical slice:
FOCUSED REMEDIATION COMPLETE / READY FOR EXTERNAL RE-REVIEW
M16.3–M16.7: NOT AUTHORIZED

Current phase: M16.2 focused remediation external re-review

STATUS: M16_2_FOCUSED_REMEDIATION_READY_FOR_EXTERNAL_RE_REVIEW
```

M13.1 is accepted in `M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. Its external
one-shot TCP/NDJSON host architecture is historical accepted evidence. The later
dependency-safety investigation found no acceptable current Steel candidate, so
M13.2 remains blocked while Workbench work proceeds independently.

The accepted dependency investigation is
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`. M13.2 remains blocked. Steel is a
deferred optional Workbench scripting candidate and must not be added while its
dependency gate remains unresolved.

The accepted read-only M14.1 audit is
`M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The accepted private bounded TCP/NDJSON
Application client and its evidence are in `M14_2_MINIMAL_WORKBENCH_CLIENT.md`. It
adds no GUI, PresentationDocument or Steel dependency/source and changes no
Runtime/Application semantics. The client-owned Workbench model, presentation model
and bounded persistence are accepted after focused model-state and
recovery-projection remediation in `M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The
minimal native GUI and focused overflow/live-continuity remediation are accepted in
`M14_4_MINIMAL_GUI.md`. The typed operator controls, property/configuration workflows,
real Runtime acceptance, and native GUI smoke are accepted in
`M14_5_OPERATOR_CONTROLS.md` at implementation commit
`ab097ed5207ea426cbbd48611015da12ce534a43`. The recovery/fault audit in
`M14_6_RECOVERY_FAULT_AUDIT.md` is accepted at commit
`4e18801930939404a8e856521b86c55c20d71dc9`. M14.6B1 recovery status UI and
one-shot `operation_status` are accepted at
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`. M14.6B2A quarantine projection
and restart classification are accepted at implementation commit
`f945d910cfeabbe8552634c53d95016e79a6eafa`. The accepted M14.6B2B1 implementation
commit is `21e4a6f623e39c102109703b22a5266a17f5e0e0`. The accepted M14.6B2B2
implementation commit is `eafd73adf43a8a566336bfe1a14370066fe06c5c`. The accepted M14.6B3
implementation commit is `0319b1d1a7917903c1cf0aa6e4852c149543f7fd`. The accepted M14.6B4
implementation commit is `801a4b559d82a903e9232cc08f5e7d27d714b1d5`. M14 consolidated
acceptance is granted. M15.1 is accepted at audit commit
`8cd89e2b58ba11248c2ce2532c165270a9c60797`; M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. The accepted M15.3 architecture +
Application API reference implementation commit is
`48383185309fc6810dff54b9656067280a535171`. The accepted M15.4 Workbench user guide
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15 is paused
without consolidated acceptance. M15.5–M15.8 are deferred and unauthorized until M16
consolidated acceptance so final API tutorials, recovery/automation documentation,
and documentation acceptance describe the post-M16 product once.

## M16.1 accepted audit

M16.1 is a completed read-only, source-derived declarative-device architecture,
Application API, and bounds audit. Its accepted report is
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`. It changes no production Rust,
tests, Cargo/dependencies, configuration schemas, public Application behavior,
Recorder/SQLite schemas, Workbench behavior, examples, or public documentation.
External review is required before any implementation.

The product goal is that a user who knows only the small useful subset of a device
protocol—such as READ measurement plus WRITE actuator plus ACK/readback—can describe
it as bounded declarative configuration, provision it through the language-neutral
Application boundary, and obtain ordinary Runtime Instrument, Signal, and Actuator
entities without a device-specific Rust driver.

The audit freezes:

- the exact v1 bounded protocol feature subset and explicit non-goals;
- separate definition and deployment-instance schema direction;
- the bounded normalized/compiled trusted internal representation;
- parsing, validation, ownership, activation, and atomic candidate rejection;
- resource binding, reservation, generation, queue, deadline, and execution model;
- READ/decode/quality semantics and ordinary Signal integration;
- WRITE/encode, ACK, independent readback, and ambiguity semantics;
- the unchanged Runtime-owned OutputAuthority path and final authority/generation/
  deadline check before any possible output byte;
- the smallest bounded language-neutral provisioning/configuration extension needed
  for client-supplied candidates;
- exact limits, Recorder provenance, fault matrix, deterministic acceptance strategy,
  and M16.2–M16.7 implementation slices.

After activation, declarative signals and actuators must be ordinary entities. Signal
discovery, current measurements, subscriptions, Workbench plots, Runtime recent
history, Recorder/durable history, and controller inputs use existing generic paths.
Writable values use ordinary OutputAuthority, resource reservation, generation and
deadline fences before declarative encoding and transport. Workbench, Recorder,
history, controller, and ordinary Application operations receive no special semantic
branches for declarative devices.

The audit must preserve this boundary:

```text
persistent deployment configuration != current Runtime state != experiment procedure

deployment configuration = installed laboratory and safe reproducible baseline
device definition         = bounded protocol mapped to typed laboratory parameters
Application API           = authoritative execution-time state/configuration changes
future external procedure = actions over already-defined semantic laboratory entities
```

A future Clojure external Application client may know Reference, controller, Signal,
and Actuator identities. It must not know COM port, baud rate, device address,
protocol bytes, CRC, register offsets, raw scaling, SQLite paths, or OutputAuthority
internals. Runtime/API mutations may change active state without silently rewriting
the original deployment source. No scripting implementation or Runtime-owned
automation language is authorized.

The candidate freezes serial/COM first; exact fixed-length binary request/response up
to 64 bytes; bounded address/channel, literals, matches, and one scalar extractor;
signed/unsigned 8/16/32-bit integers and IEEE binary32; endian; finite scale/offset;
sum8, xor8, and CRC16/Modbus; periodic reads; typed writes; strict ACK; optional
independent readback; and finite queue, frame, parser, poll, and timeout bounds.
Delimiter framing and ASCII numeric parsing are deferred from v1.

V1 excludes arbitrary code, scripts, callbacks, loops, general expressions, dynamic
evaluation, unbounded parser state or allocation, generic protocol state machines,
and raw public byte access. Complex devices remain native Rust adapters. M16 must not
add public operations equivalent to `send_raw_bytes`, `raw_serial_write`,
`execute_device_command`, or `unchecked_register_write`.

Declarative data is configuration, not executable code or authority. It must not
access arbitrary filesystem/network endpoints, open transports, bypass configured
resources or OutputAuthority, fabricate observation quality/ACK/readback/safe state,
write Recorder SQLite directly, or create Application operations dynamically.

Recorder provenance must identify the stable/hash/versioned definition, normalized
configuration, instrument instance, resource/binding generation, and existing build/
Runtime provenance without copying mutable client state into scientific authority.

The audit must freeze explicit limits for definitions, encoded definition size,
parameters per definition, instances, request/response bytes, template/extractor
fields, checksum inputs, pending transactions, timeouts, poll rates, and ACK/readback
state. It must account for malformed/unsupported/oversized definitions, truncated or
invalid replies, prefix/status/checksum/numeric/range failures, timeout before send,
ambiguous timeout after possible output, bad ACK, readback mismatch, reconnect/rebind
with stale completion, queue saturation, revision conflict, failed apply preserving
the active deployment, and client/Workbench death with Runtime continuing.

Current `stage_configuration` handles Runtime-known deployment candidates and does
not accept arbitrary client-supplied definitions. M16.1 freezes one additive
`stage_simple_device_candidate` mutation for one complete bounded candidate, followed
by existing `apply_configuration`. Staging does not activate. Exact DTO, ownership,
8 KiB candidate/6 KiB definition bounds, shared-codec lexical/depth compatibility,
SessionStore retained-payload credit, one-slot/30-second lifecycle, process-local
restart semantics, and single-owner provisional output preparation are in the audit
report.

M16.1 is accepted. M16.2 focused remediation is complete and ready for external re-review. M16.3 writable
actuator, M16.4 provisioning implementation, M16.5 generic integration acceptance,
M16.6 fault/bounds/provenance/recovery acceptance, M16.7 minimal unknown-device
acceptance, and M16 consolidated review remain unauthorized. M13.2 remains blocked
on Steel dependency safety.

## M16.2 implemented review candidate

The M16.2 review candidate implements only this persistent, file-backed read-only
vertical slice:

```text
persistent/file-backed read-only simple_device startup/composition
    -> frozen deployment-relative JSON definition artifact
    -> strict bounded definition parse/validation
    -> canonical normalization/hash
    -> compiled fixed-length READ plan
    -> existing configured COM resource
    -> existing ResourceExecutor
    -> ordinary Signal
    -> generic discovery/current/events/recent history/Recorder/Workbench path
```

The accepted M16.1 constraints remain authoritative:

- the persistent deployment instrument is `[[instruments]]` with
  `kind = "simple_device"`;
- its deployment-relative immutable JSON definition artifact is resolved, bounded,
  read once, frozen, and hashed through the accepted deployment artifact model;
- raw definition artifact bytes are at most 8,192 and canonical normalized definition
  bytes are at most 6,144;
- a definition contains at most 16 parameters, and request/response transactions are
  fixed-length and at most 64 bytes;
- every scalar parameter has exactly one `encoding { raw, scale, offset }` used by
  its read plan;
- read-only measurement and diagnostic engineering values may be integer or float;
  actuators are outside M16.2;
- a read-only instance may bind an existing `windows_com_read_only` or `windows_com`
  resource;
- multiple read-only simple-device instances may share one eligible resource only
  when their bounded response correlation is sufficient; sharing a resource between
  simple-device and native/Metakon protocol adapters is invalid and deferred in v1;
- M16.2 may introduce only the minimal protocol-neutral 64-byte transport seam needed
  by this slice; the Metakon codec retains its existing 38-byte limit and behavior.

### M16.2 explicit non-goals

M16.2 does not authorize WRITE, Actuator, safe-profile or controller activation, ACK,
readback, output ambiguity/quarantine, or OutputAuthority changes beyond a strictly
necessary protocol-neutral type seam. It does not authorize
`stage_simple_device_candidate`, Application provisioning, incremental provisioning
apply, process-local API overlays, SessionStore provisioning credit, new COM/resource
creation, delimiter framing, ASCII-numeric protocol fields, public raw-byte
operations, scripting, Lua, Steel, Clojure integration, or a Simple Device-specific
Workbench UI. Generic consumers must not gain `SimpleDevice`-specific branches merely
to consume ordinary signals.

### M16.2 required implementation evidence

External implementation review must prove all of the following:

1. Exact persistent TOML composition using `kind = "simple_device"`.
2. The deployment-relative definition artifact is resolved, bounded, read once,
   frozen, and hashed using the accepted deployment artifact model.
3. The strict closed JSON definition grammar produces a canonical form of at most
   6,144 bytes.
4. Every parameter uses the exact `encoding { raw, scale, offset }` shape.
5. The READ request grammar has the exact tagged variants `literal`,
   `instance_field`, and `checksum`.
6. The response grammar has the exact tagged variants `literal_match`,
   `instance_match`, `scalar_extract`, and `checksum`.
7. Fixed response length and every offset, overlap, and byte bound are validated.
8. Integer engineering ranges use integral `i64` bounds; float ranges use finite
   `f64` bounds.
9. Instance address/channel values produce the exact compiled request bytes.
10. Response length, matches, checksum, extraction, and engineering transform are
    strict and deterministic.
11. Bad length, bad literal or instance match, bad checksum, malformed scalar,
    non-finite `f32`, and engineering-range failures commit no Good sample.
12. READ retry is bounded by the already accepted clean-recovery policy and retains
    the original deadline.
13. `binding_generation` and `mapping_revision` fence stale completions.
14. Two or more read-only simple-device instances may safely share one eligible
    resource when response correlation is sufficient.
15. The resulting measurement is an ordinary Signal and reaches Core discovery,
    latest and window; Application discovery, current, events and history; the
    ordinary Workbench model/plot path; and Recorder durable history/provenance.
16. Generic Workbench, Application measurement/history, controller-input, and
    Recorder consumers contain no SimpleDevice-specific branch.
17. Existing Metakon behavior and its 38-byte codec validation remain unchanged.

M16.2 requires external review before M16.3. M16.3–M16.7 remain unauthorized.

The M14.6B3 worker-owned episode, explicit-disconnect boundary, non-replay proof,
overflow unification, bounds, and verification evidence are recorded in
`M14_6B3_BOUNDED_FAULT_REATTACH.md`.

The executable 30-row recovery/fault matrix, A-J scenarios, process/fault-injection
methodology, and final evidence are recorded in `M14_6B4_RECOVERY_FAULT_ACCEPTANCE.md`.


## Accepted M14 consolidated invariants

Runtime owns experiment semantics and Workbench owns presentation semantics.
Workbench/GUI/future-script lifetime is not Runtime or experiment lifetime; Workbench
failure, clean close, or OS termination does not stop Runtime, controllers, Recorder,
or the experiment.

One private bounded Rust Application client owns the socket, framing, hello/session/
scope, message IDs, mutation sequence, subscription/cursor, recovery-journal
interaction, and bounded retained-scope reconnect. No second Application owner was
introduced. One WorkbenchModel and one PresentationDocument remain client-owned and
never become Runtime experiment authority.

Check Status and Exact Retry are explicit/manual; mutation, status, and Exact Retry
are never replayed automatically. Exact bounded worker-owned recovery evidence and
quarantined old-boot/scope evidence remain visible. Explicit Disconnect has no
automatic reconnect and uses the accepted out-of-band/coalesced command-admission
fence. Unexpected continuity loss gets one retained-scope episode with one absolute
3 s deadline, connect timeout no greater than `min(2 s, remaining)`, and retry
spacing of at least 10 ms.

Fresh requires the complete authoritative rebuild barrier. Hello, process liveness,
socket reconnect, an old event cursor, or mutation terminal outcome alone does not
prove Fresh.

The frozen 30-row M14.6A matrix and A-J scenarios pass. Process evidence comprises
eight real-Runtime acceptances and one self-spawn Workbench plus scripted Application
peer acceptance. Native Glow OS-termination evidence preserves Runtime, workspace,
journal, and Reference authority; controller identity `1`, state `ready`, revision
`1` are unchanged, and Recorder state `idle` with null `active_run`/`run_id` is
unchanged. Recorder evidence intentionally proves the weaker authoritative idle
continuity invariant, not active-run survival.

Accepted verification: Workbench `160 passed; 9 ignored; 0 failed` three times;
workspace debug and release each `680 passed; 11 ignored; 0 failed`; real Runtime
`8 passed; 0 failed`; self-spawn/scripted `1 passed; 0 failed`; A-J and native
Glow PASS; recorded focused fault/race repetitions 10/10; fmt, Clippy with warnings
denied, and diff-check PASS.

M13.2 remains blocked on Steel dependency safety and is not authorized. Discard/
Forget, a later milestone, and automatic release are not authorized.
