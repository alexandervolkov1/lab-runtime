# Current work — M16.7 minimal unknown-device acceptance

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: M16.7 minimal unknown-device acceptance
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: COMPLETE
Developer Preview artifact: READY LOCALLY
v0.1.0-preview.1: PUBLISHED
Practical integration architecture: M16.1–M16.6 ACCEPTED
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
M16.1–M16.6: ACCEPTED
M16.7:
AUTHORIZED
M16 consolidated external review:
NOT AUTHORIZED
STATUS: M16_7_UNKNOWN_DEVICE_ACCEPTANCE_AUTHORIZED
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
consolidated acceptance. M16.1 is accepted. The accepted M16.2 read-only simple-device
implementation commit is `aac377470d7142005ad5e0d098fddf6c16734db8`. The accepted
M16.3 writable simple-device implementation commit is
`df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`. M16.4 is accepted at
`92847a046c4ef2e4e69d24ea51fc56e28e42af38` after external re-review of the original
implementation plus the pending-apply property fence,
existing `operation_failed` mapping for internal owner failures, and phase-aware
absolute apply deadline with quarantine based on actual
`send_started && !safe_confirmed` evidence. M16.5 generic integration acceptance is
externally accepted at `799e7f969393b960ae54409f90842ff2ab1b84cf`. M16.6
fault/bounds/provenance/recovery acceptance is externally accepted at
`6021586096453fa03bdbcc28ef740b660f32eeb5`. M16.7 minimal unknown-device acceptance
is authorized; M16 consolidated external review is not authorized.

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
M16.1–M16.6: ACCEPTED
M16.7:
AUTHORIZED
M16 consolidated external review:
NOT AUTHORIZED

Current phase: M16.7 minimal unknown-device acceptance

STATUS: M16_7_UNKNOWN_DEVICE_ACCEPTANCE_AUTHORIZED
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

M16.1 is accepted. M16.2 is accepted at
`aac377470d7142005ad5e0d098fddf6c16734db8`. M16.3 writable actuator is accepted at
`df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`.
M16.4 provisioning and M16.5 generic integration acceptance are externally accepted;
M16.5 is accepted at `799e7f969393b960ae54409f90842ff2ab1b84cf`. M16.6
fault/bounds/provenance/recovery acceptance is accepted at
`6021586096453fa03bdbcc28ef740b660f32eeb5`. M16.7 minimal unknown-device acceptance
is authorized; M16 consolidated external review remains unauthorized. M13.2 remains blocked on
Steel dependency safety.

## M16.2 accepted implementation

The accepted M16.2 implementation at
`aac377470d7142005ad5e0d098fddf6c16734db8` implements only this persistent,
file-backed read-only
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

M16.2–M16.6 are accepted; M16.7 is authorized under the frozen scope below.

## M16.3 accepted implementation

M16.3 extends only the accepted persistent/file-backed SimpleDevice deployment path:

```text
persistent simple_device definition
    -> at most one actuator parameter
    -> existing deployment safe profile
    -> optional existing native controller
    -> existing OutputAuthority
    -> compiled bounded WRITE
    -> final send fence
    -> existing ResourceExecutor
    -> strict semantic ACK
    -> optional independent exact-raw readback
    -> ordinary output/safety/controller/Recorder evidence
```

It does not authorize Application upload or provisioning. The accepted M16.1 audit
and M16.2 read-only implementation remain authoritative and must not be redesigned.

### Actuator and protocol boundary

A definition may contain at most one actuator. That parameter must be `actuator`,
`read_write` or `write_only`, `output_affecting`, and engineering `float`. Existing
measurement/diagnostic parameters remain read-only, `write_effect = none`, and
integer or float. Raw representation may use any accepted M16 scalar encoding through
the one parameter-level `raw + scale + offset` transform. No integer
OutputAuthority/OutputProposal, second output owner, device-specific Application
operation, or public raw-byte operation is authorized.

M16.3 activates exactly one `value_field` in each WRITE request. The request may also
contain the accepted literal, instance-field, and final optional checksum segments.
It remains bounded to eight segments, 1..=64 bytes, and the accepted nonliteral-insert
limit. Encoding computes `raw = (engineering - offset) / scale`; the result must be
finite, exactly representable by the selected raw type, integral for integer raw
encodings, and finite/representable for f32. Clipping, saturation, and silent rounding
are forbidden. Prepared bytes carry no authority.

Immediately before the first possible output byte, the Runtime-owned path must recheck
current OutputAuthority state, authority epoch/lease, proposal deadline, DispatchId and
output intent, resource binding generation, mapping revision, and transaction deadline.
First positive transport write admission establishes `send_started`. The serial worker
does not own authority, and declarative configuration never grants it.

ACK uses an exact-length response with zero scalar extracts, at least one semantic
literal or instance match, and an optional checksum. Checksum-only ACK is invalid.
Shared-resource ACK evidence must distinguish the target instance. Malformed length,
literal, instance, or checksum fails closed. ACK means protocol acknowledgement only,
not readback or physical effect.

An actuator may have one optional independent readback transaction after WRITE/ACK.
The prepared exact raw scalar retained for comparison is at most four bytes. Integer
raw readback requires exact raw numeric equality. f32 readback rejects nonfinite
values, normalizes both signed zeros to positive zero, and otherwise requires exact
binary32 identity. Engineering equality or a tolerance never establishes
`ReadbackVerified`.

### Existing safety, controller, resource, and evidence ownership

Persistent writable SimpleDevice composition uses existing deployment-level
`[[safe_profiles]]` and optional native `[[controllers]]`; it does not duplicate these
inside the instrument. An active actuator requires an eligible safe profile. A native
controller targets the ordinary ActuatorId and uses the existing Reference, lifecycle,
lease, OutputAuthority, and OutputProposal path. Normal and safe writes use the same
compiled physical WRITE/ACK/readback mechanism, distinguished by existing authority
and output-intent evidence. Reconnect, rebind, ambiguity, configuration transition, or
fault recovery never automatically rearms controller or authority.

Failure before `send_started` is definitely pre-send. Failure after `send_started`
without required terminal evidence is physically ambiguous, fails closed, and is
never blindly or automatically resent. M16.3 must reuse existing Runtime-owned
OutputAuthority/transport ambiguity evidence rather than introduce a second state
machine.

Output-capable SimpleDevice instances may bind only `windows_com`; binding to
`windows_com_read_only` is rejected before activation. Multiple SimpleDevice instances
may share one eligible resource only under accepted strict request/ACK/readback
correlation. SimpleDevice plus Metakon/native sharing remains invalid and deferred.
M16.2 read-only behavior and the Metakon 38-byte codec bound remain unchanged.

Recorder provenance must extend the existing bounded store with the exact canonical
and raw definition identities, definition ID/version, instrument/parameter identities,
resource and binding/mapping/configuration revisions, scalar transform, WRITE/ACK/
readback plan identities, safe profile identity, optional controller/reference
binding, and persistent-startup source. Definition blobs are not duplicated per output
fact, and no second provenance store is authorized.

### Accepted M16.3 evidence

External implementation review accepted evidence that:

1. The writable schema remains strict, closed, and bounded, with at most one actuator.
2. Actuator role/access/write-effect/value type exactly match the frozen float domain.
3. `windows_com_read_only` fails output-capable composition before activation.
4. An eligible safe profile is required for active persistent actuator composition.
5. Optional controller targeting uses ordinary ActuatorId/OutputAuthority paths.
6. Nonfinite, nonrepresentable, fractional-integer, and overflowing inverse transforms
   fail without clipping or rounding.
7. Exact WRITE bytes are deterministic, but prepared bytes do not grant authority.
8. The final send fence rechecks authority, epoch/lease/deadlines, binding generation,
   and mapping revision before byte zero.
9. Pre-send failure and post-send ambiguity remain distinguishable.
10. Ambiguous output is never blindly or automatically retried.
11. Strict ACK requires semantic evidence; checksum alone is insufficient.
12. Another shared-resource instance cannot satisfy the target ACK.
13. Optional readback is an independent physical transaction.
14. Retained expected raw scalar evidence is at most four bytes.
15. Integer raw readback requires exact raw equality.
16. f32 readback requires finite exact binary32 identity after signed-zero
    normalization.
17. Engineering equality alone cannot satisfy exact readback.
18. Normal and safe outputs share the compiled protocol path.
19. Stale binding/mapping completions cannot settle current authority.
20. Failed ACK/readback cannot fabricate OutputApplied, ReadbackVerified, or safe
    evidence.
21. Output/controller/safety Recorder facts remain generic after protocol settlement.
22. M16.2 read-only behavior remains unchanged.
23. Metakon behavior and its 38-byte codec bound remain unchanged.
24. The accepted M16.3 commit introduced no provisioning, Workbench special path,
    raw public operation, scripting, or later-slice work.
25. No new dependency is introduced.

### M16.3 implementation boundary

The accepted M16.3 commit does not contain `stage_simple_device_candidate`, Application
definition upload, process-local overlays, incremental provisioning apply, SessionStore
candidate credit, new resource or COM provisioning, delimiter/ASCII/binary64/BCD/
packed-bit grammars, general expressions/callbacks/state machines, a special Workbench
UI, public raw I/O, scripting, or external-language integration. The M16.4 candidate
implements only the bounded provisioning surface below. M16.5 is externally accepted;
M16.6 is externally accepted under the frozen acceptance scope below, and M16.7 is
authorized only for the frozen minimal unknown-device acceptance.

## M16.4 implemented Application provisioning slice

M16.4 adds the accepted M16.1 provisioning surface without redesigning the accepted
M16.2/M16.3 protocol compiler or physical execution paths:

```text
stage_simple_device_candidate
    -> strict complete candidate validation
    -> retained typed mutation / dedup
    -> one staged candidate slot
    -> existing apply_configuration
    -> immediate Accepted
    -> Runtime-owned incremental preparation
    -> process-local active overlay
    -> atomic publication or terminal failure
    -> post-send quarantine when physical output outcome is ambiguous
```

### Public surface and candidate

Exactly one new mutation, `stage_simple_device_candidate`, and one discovery capability,
`simple_device_provisioning`, are authorized. Existing `apply_configuration`,
`configuration_status`, `operation_status`, and `reconnect_resource` retain their
roles. Application envelope v1 remains unchanged. The registry is expected to become
43 operations: 22 queries and 21 mutations, with 26 capabilities. The capability is
discovery only and grants no authority.

The mutation carries decimal `expected_revision` and one complete strict candidate:

```text
candidate
    schema_version = 1
    definition = one existing strict SimpleDevice definition
    instances = complete instance objects
```

The root has exactly those three fields. A read-only definition admits 1..=4 instances;
an output-capable definition admits exactly one. Read-only instances contain neither
safe profile nor controller. An output-capable instance contains exactly one safe
profile and zero or one controller using the frozen M16.1 schemas. Persistent and API
forms compile through the same typed definition/compiler representation.

Frozen limits are: candidate bytes <= 8,192; canonical definition bytes <= 6,144;
candidate lexical values <= 900; whole mutation envelope values <= 909; candidate
depth <= 8; whole message depth <= 10; canonical compact maximal request <= 8,496
bytes; and shared raw Application body <= 16,383 bytes. Chunking, multipart upload,
server-side builders, paths, and artifact references are not authorized.

### Staging, dedup, and retained-memory credit

Staging validates, normalizes, compiles, hashes, cross-references, classifies, and
retains the complete normalized typed candidate without activating it. Runtime owns
one staged slot with a 30-second monotonic lifetime. Occupied capacity rejects a
second candidate; expiry releases ownership. Client disconnect neither activates nor
cancels the candidate.

The existing SessionStore request identity and normalized typed mutation equality are
authoritative. Exact retry returns retained state/outcome without staging twice;
different payload under one identity is `request_conflict`; an old unknown retained
identity is `outcome_unknown`. Raw JSON spelling is not dedup identity.

Provisioning receives one process-wide 524,288-byte retained-payload credit. Each
record is charged exactly `align_up(canonical_normalized_candidate_bytes, 64) + 8,192`
bytes, at most 16,384 bytes, so at most 32 maximum charges fit. Credit is reserved
before mutation retention, sequence advance, or staged ownership. Capacity failure
creates no retained or staged state. Pre-admission unwind, scope removal, terminal
record removal/expiry/eviction, and Runtime shutdown release exact credit. Existing
SessionStore cardinality bounds remain independently authoritative.

### Incremental apply and ownership

Activation uses only the existing `apply_configuration` as a separate mutation and
request identity. Application owns admission, SessionStore correlation, and outcome
delivery. ServiceHost/Runtime owns one process-wide pending apply, one prepared
topology, physical/safety progress, commit, failure, and quarantine. Accepted becomes
deliverable immediately; later serialized owner turns perform preparation under one
absolute 15-second deadline. Application handling never blocks for physical
preparation. Disconnect does not cancel admitted work, and Exact Retry cannot start a
second apply.

A read-only apply does not enter global configuration quiesce while preparing;
unrelated acquisition/control continues. Candidate entities remain non-discoverable
until atomic commit. Failure preserves the old active topology.

For an output-capable apply, unrelated work may progress before the global safety
boundary. The Runtime then pauses affected Running/Warming controllers, revokes
affected leases, establishes the existing active-output safe barrier, and enters the
single existing `configuration_quiesced` state. While quiesced, no new ordinary
acquisition/controller production occurs; safety, admitted transport, Recorder,
Application/network, query/status, and already-produced event delivery continue.
Every terminal commit/failure/quarantine transition clears quiesce. Scheduling resumes
from current monotonic time without catch-up, and controllers never auto-rearm.

Prepared instruments, signals, actuators, controllers, safe profiles, schedules, and
provenance are Runtime-owned and non-discoverable. Successful commit publishes the
whole topology atomically. Prepared output authority is never Application authority.

### Failure, ambiguity, and reconciliation

If preparation fails before `send_started`, the candidate is consumed, prepared state
is dropped, the apply becomes terminal Failed, no candidate entity is published, and
a later attempt requires a new stage and apply.

After `send_started`, missing required safe evidence retains one bounded process-wide,
Runtime-owned, non-discoverable provisional-output quarantine. It retains candidate
and resource identity, OutputAuthority, DispatchId/output intent, binding generation,
mapping revision, send/ACK/readback evidence, and fail-closed resend/fault latches. It
does not expire with time, resend, become an Instrument/Actuator, or publish authority;
it blocks another output preparation on its resource. Explicit current-resource
`reconnect_resource` is the accepted reconciliation lifecycle. No blind safe resend or
automatic mutation replay is authorized.

`configuration_status` may expose bounded semantic candidate/resource identity,
configuration revision, binding/mapping generation, preparation/evidence phase, and
`safe_resend_blocked`. It must not expose raw bytes, transport handles, client-supplied
COM settings, or OutputAuthority internals.

### Resource, overlay, provenance, and errors

Candidates reference only preconfigured resources. Read-only candidates may use
`windows_com_read_only` or `windows_com`; output candidates require `windows_com`.
Missing resources, SimpleDevice/native or SimpleDevice/Metakon mixing, and output on a
read-only resource reject under the frozen failure contract. M16.4 does not create or
configure COM resources.

Successful provisioning creates a Runtime-owned process-local active overlay. It
survives client/Workbench loss, does not survive Runtime restart, and never rewrites
`runtime.toml`, definition files, or hidden deployment artifacts. No persist/export
operation is authorized. While an API overlay is active, file `stage_configuration`
and `reload_configuration` reject before occupying or changing the stage slot.
Supported `property_configure` operations clone and preserve the current active overlay.

Recorder uses the existing provenance store and distinguishes process-local
Application candidate source from persistent startup deployment while retaining
canonical/raw definition, instance, resource/binding/mapping/configuration revision,
safety, and controller identities. Complete definition content is not copied into
each measurement/output fact.

Only the existing public taxonomy is used: malformed or structurally over-bound input
maps to `invalid_args`/`invalid_request`; semantic invalidity to
`invalid_configuration`/`invalid_configuration`; missing resource to
`unknown_resource`/`not_found`; stale revision to
`revision_conflict`/`revision_conflict`; occupied stage/apply/quarantine capacity to
`busy`/`capacity_exhausted`; identity conflicts to `request_conflict`; and unknown old
outcomes to `outcome_unknown`, with the frozen retryable/resync flags. No new public
error category is authorized.

### M16.4 explicit non-goals

M16.4 does not authorize chunks, multipart or builder APIs, generic manifest or path
uploads, new resources/COM settings, overlay persistence/export, runtime.toml rewrite,
raw byte/serial operations, a new controller or authority/quarantine owner, new
protocol grammars, expressions/callbacks/state machines, special Workbench UI,
scripting or external-language integration. M16.4 did not itself authorize later
slices; M16.6 is externally accepted, while M16.7 is authorized by the current
coordination gate and remains subject to external review.

## M16.5 accepted generic integration

M16.5 is an externally accepted integration/acceptance milestone. Its evidence proves
that an API-provisioned SimpleDevice participates in the ordinary generic
Workbench discovery/rebuild/model and live-plot paths, current measurements,
Application events, bounded recent history, Recorder and durable history, native
controller input/output, and resource reconnect. Generic Workbench, plotting,
Recorder/history, controller, discovery, and ordinary Application consumers must not
gain a SimpleDevice-specific semantic branch merely because the entity came from
declarative provisioning.

The required end-to-end evidence starts with API provisioning and ordinary
rediscovery, then exercises the ordinary Workbench model identity and live plot,
current/event/recent-history projections, and Recorder/durable-history path. Writable
candidates must remain ordinary `ActuatorId` entities using the existing controller,
`OutputAuthority`, and physical settlement paths without a SimpleDevice-specific
controller implementation.

Reconnect acceptance must cover two or more read-only SimpleDevice instances sharing
one eligible resource: one explicit `reconnect_resource` advances the resource fence,
fences all stale old work, and allows all affected ordinary signals to reacquire
generically. It must also cover an output-capable SimpleDevice resource reconnect under
the existing safe-recovery semantics, with no automatic controller restart or rearm.

Expected work is acceptance coverage around the existing Application, Workbench
rebuild/model, Recorder/history, controllers, and resource-reconnect owners. Production
changes are permitted only for concrete defects exposed by red acceptance tests.
The implemented candidate adds end-to-end API-provisioned read-only and writable
acceptance, ordinary Workbench rebuild/live-trace evidence, and resource-scoped shared
SimpleDevice reconnect coverage. Red oracles required two generic-seam corrections:
configured-resource reconnect now binds every SimpleDevice on the resource, and native
controller validation can use non-discoverable prepared descriptors during atomic
activation. Writable reconnect replaces authority, re-establishes safe evidence, and
does not rearm a paused controller. No new operation, capability, DTO, transport kind,
authority, or consumer-specific SimpleDevice presentation/history path is introduced.
M16.5 did not itself authorize a SimpleDevice-specific Workbench panel/editor, plot,
Recorder, or controller path; a new Application operation, public DTO/error/category,
transport/resource type, authority path, or raw serial/output API; configuration
persistence/export; later-slice work; scripting or external-language work; or the
separately discussed post-M16 UI/API roadmap.

The authority boundaries remain unchanged: Runtime owns experiment semantics,
Workbench owns presentation semantics, Workbench consumes the same Runtime Application
API as other clients, and `requested != authorized != send_started != ACK != readback
!= physical effect`.

The M14.6B3 worker-owned episode, explicit-disconnect boundary, non-replay proof,
overflow unification, bounds, and verification evidence are recorded in
`M14_6B3_BOUNDED_FAULT_REATTACH.md`.

The executable 30-row recovery/fault matrix, A-J scenarios, process/fault-injection
methodology, and final evidence are recorded in `M14_6B4_RECOVERY_FAULT_ACCEPTANCE.md`.

## M16.6 accepted fault/bounds/provenance/recovery acceptance

M16.6 was acceptance-first. The implementation executes the frozen section-17 boundary matrix and
section-18 deterministic failure acceptance in
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`. Production changes were limited
to a concrete defect demonstrated by a red oracle.

The implemented acceptance scope is exactly:

- malformed grammar and conversions;
- exact capacities and deadlines;
- pre-send versus post-send ambiguity;
- rebind and stale-mapping fencing;
- prepared-state capacity, identity, and deadline;
- safe-barrier behavior;
- Required Recorder failure;
- SessionStore retained provisioning credit;
- client death;
- Runtime restart;
- exact in-memory and SQLite provenance; and
- preservation and restoration of the prior persistent deployment.

The focused M16.6 suites pass 32/32 tests. One deterministic RED oracle found that a
provisional safe write failing before its first byte retained the safe obligation but
did not expose a terminal pre-send transport failure, leaving apply pending until its
absolute deadline. The internal authority now records the existing `Failed` outcome
without fabricating `sent` evidence; apply maps that exact state to the existing
retryable `transport_unavailable` result and retires prepared state without quarantine.
No public schema, operation, capability, DTO, error, grammar, dependency, or retry
mechanism changed.

External source-review remediation adds a production-DTO legal maximal-shape oracle:
the compact candidate reaches the exact 6,144-byte canonical-definition boundary,
keeps its candidate/envelope lexical counts and depths within 900/909 and 8/10, uses
maximal valid mutation identities, passes the production Application decoder and
SimpleDevice compiler, and rejects both byte-over and structural-over mutations.
Application-candidate provenance now recomputes `SHA256(candidate.canonical)` and
asserts the complete process-local overlay identity in both frozen memory and SQLite.

Acceptance proves the frozen 8,192 / 6,144 / 900 / 909 / 16,383 limits, exact timing,
history and queue edges, the 524,288-byte SessionStore provisioning credit, strict
reply/conversion/ACK/readback behavior, pre-send versus post-send ambiguity, stale
binding/mapping fencing, one-slot stage/pending/prepared/quarantine ownership, Required
Recorder durability failure, client death, and clean Runtime restart without overlay,
mutation, output, or evidence replay. In-memory objects and SQLite rows agree on
definition/candidate hashes, instance and parameter identity, resource and generation,
mapping/configuration revision, safety/controller/reference association, source class,
and Runtime/build/deployment provenance. Immutable definition content remains one
content object across multiple measurements and instances.

Final remediation verification passes formatting and warnings-denied Clippy. Debug and
release workspace suites each pass 750 tests with 11 explicitly ignored and zero
failures.

M16.6 adds no policy and does not expand the SimpleDevice grammar. It adds no
operations, capabilities, DTOs, errors, or dependencies; changes no Workbench feature;
did not begin M16.7; and did not begin Runtime API or Workbench UI API work. Public
documentation and `.gitignore` remained out of scope. M16.7 is now separately authorized
under the frozen scope below.

## M16.7 authorized minimal unknown-device acceptance

M16.7 provisions one deterministic unknown, non-Metakon device entirely through the
existing generic declarative/Application path and demonstrates:

```text
Application candidate
-> READ temperature
-> ordinary Signal
-> WRITE power
-> ordinary ActuatorId / OutputAuthority
-> strict ACK
-> independent READBACK
-> controller
-> Workbench generic discovery/live plot
-> Recorder/history
-> reconnect fencing
-> restart non-persistence
```

The acceptance fixture represents a non-native device. Runtime, Application, and
Workbench contain no device-specific implementation branch for it. Expected owners are
primarily test fixture/data, a scripted physical `ByteTransport` peer, and the existing
Application/Runtime/Workbench acceptance harnesses.

M16.7 uses the accepted M16.2–M16.6 semantics unchanged. Production changes are
permitted only when a deterministic RED oracle proves a defect in the already accepted
generic seam. M16.7 does not authorize:

- a device-specific Runtime driver, Application operation or DTO, or Workbench path;
- new grammar, policy, transport/resource kind, controller, or authority owner;
- raw serial or output authority;
- real Arduino hardware or a real Arduino driver;
- Steel, Clojure, or Python work;
- post-M16 Runtime API or Workbench UI API work;
- public documentation;
- Cargo dependencies; or
- `.gitignore` changes.

M16 consolidated external review remains not authorized.


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
