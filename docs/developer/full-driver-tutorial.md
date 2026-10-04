# Add a native Rust instrument

This guide describes the current source-level path for adding a trusted native
physical instrument to `lab-runtime`. A native integration is compiled into the
Runtime executable, runs inside the Runtime authority boundary, and is reviewed as
product code.

There is no stable dynamic driver ABI, loadable plugin system, external driver crate
API, registration macro, or binary-compatibility promise. A native driver must not
create another experiment owner, transport owner, output authority, event stream, or
Recorder. Metakon is the current physical reference implementation; it is a concrete
integration, not an implementation of a universal driver trait.

Adding a driver requires a source checkout and a new Runtime build. The portable
preview package can use drivers already compiled into `lab-runtime.exe`, but it does
not contain the source tree or a driver SDK.

Read [Architecture and concepts](../architecture.md),
[Configuration](../configuration.md), and
[Safety and failure behavior](../safety-and-failures.md) first.

## Choose the right path

Use the smallest integration that can state the protocol and fault model honestly.

| Concern | SimpleDevice | Native Rust integration |
|---|---|---|
| Fixed bounded request/response | preferred | possible, usually unnecessary |
| Custom or variable framing | unsupported | implement a bounded codec |
| Protocol state machine | unsupported | explicit trusted code |
| Transport | existing Windows COM owner | extend the Runtime-owned resource path |
| Device-specific recovery | unsupported | explicit, bounded lifecycle work |
| Multi-stage transaction | only fixed WRITE/ACK/READBACK | explicit state machine |
| Coupled outputs | one actuator per definition | possible through central authority |
| Scheduling | fixed periodic reads | explicit bounded owner scheduling |
| Development effort | definition and deployment | Core, host, config, and tests |
| Trust level | validated data | trusted code inside Runtime |
| Safety responsibility | compiler plus shared Runtime | driver plus shared Runtime |
| Test burden | parser/compiler and device qualification | full fault/lifecycle matrix |

Start with [SimpleDevice](../simple-device.md) when its grammar fits. Native Rust is
appropriate for unsupported framing/checksums, a real protocol state machine, custom
recovery, unusual scheduling, a new transport family, or coupled multi-stage device
behavior. JSON being less interesting to write is not a reason to choose the larger
trusted-code surface.

## Authority boundary

Runtime remains the sole experiment owner. A driver contributes strict protocol
handling and trustworthy evidence; it does not own experiment semantics.

```text
deployment
    -> Runtime resource owner
    -> bounded transaction executor
    -> native protocol adapter
    -> Runtime signal/output state
    -> generic API, events, history, Recorder
```

The driver must not:

- open a COM port behind the resource owner;
- publish directly to Workbench or another client;
- write SQLite or invent provenance outside Recorder;
- keep a second authoritative instrument/output state machine;
- bypass `OutputAuthority` with raw writes;
- report an ACK, readback, or physical effect that the protocol did not prove;
- create an unbounded queue, retry loop, thread, or retained history.

## Current source map

The following paths are current source owners. They are inline paths rather than
package links because adding a driver requires a source checkout.

| Concern | Current source owner or reference |
|---|---|
| deployment DTOs, limits, cross-validation | `apps/lab-runtime/src/configuration.rs` |
| frozen Metakon definition loader | `apps/lab-runtime/src/definition.rs` |
| instrument descriptors and binding | `crates/lab-core/src/instrument.rs` |
| strict Metakon codec | `crates/lab-core/src/metakon.rs` |
| resource transaction executor | `crates/lab-core/src/transport.rs` |
| Windows COM handle/worker | `apps/lab-runtime/src/serial.rs` |
| Core read/output admission and completion | `crates/lab-core/src/runtime/physical_io.rs` |
| central output authority | `crates/lab-core/src/output/authority.rs` |
| compile-time instrument composition | `apps/lab-runtime/src/host/instruments.rs` |
| owner-turn cadence and priority | `apps/lab-runtime/src/host/scheduler.rs` |
| rebind and compatibility gates | `apps/lab-runtime/src/host/configuration.rs` |
| reconnect lifecycle | `apps/lab-runtime/src/service/reconnect.rs` |
| startup composition/readiness | `apps/lab-runtime/src/service.rs` |
| semantic event projection | `apps/lab-runtime/src/events.rs` |
| Recorder activation/provenance | `apps/lab-runtime/src/host/recording.rs` |
| finite shutdown | `apps/lab-runtime/src/host/lifecycle.rs`, `service/shutdown.rs` |
| generic discovery/API projection | `apps/lab-runtime/src/application/` |

The integration is deliberately explicit. Expect concrete enum variants and match
arms in configuration, host composition, and Core physical I/O. Do not hide those
review points behind a speculative universal protocol trait.

## Development lifecycle

A new native integration should be developed in this order:

1. write the protocol and failure model;
2. decide why SimpleDevice is insufficient;
3. implement and test a pure bounded codec;
4. add strict configuration and frozen artifacts, if any;
5. add semantic descriptors and binding identity;
6. enter the existing resource executor;
7. implement reads and stale-completion checks;
8. integrate writable output through `OutputAuthority`, if applicable;
9. compose schedules, compatibility gates, reconnect, and shutdown;
10. preserve generic discovery, EventLog, and Recorder behavior;
11. wire concrete modules into the existing crates and workspace;
12. pass software acceptance before physical qualification.

Do not start with hardware I/O. A pure codec and a precise failure model make later
transport and ambiguity tests deterministic.

## Step 1: model the protocol and faults

Write down before coding:

- request and response framing, maximum byte counts, encoding, and checksums;
- device addressing and correlation rules;
- exact response variants and scalar domains;
- whether transactions can have multiple phases;
- timeout meaning before and after the first output byte;
- what constitutes protocol ACK;
- whether independent readback exists;
- how disconnect, reconnect, and late bytes are recognized;
- which errors permit a new read and which leave a write ambiguous.

Metakon keeps these mechanics in `crates/lab-core/src/metakon.rs`. Its maximum frame
is 38 bytes, its address is device/channel/register, and typed decode validates exact
length, CRC, address, function, flags, and scalar encoding. Those are Metakon choices,
not universal driver requirements. Every new codec still needs explicit finite
bounds and exact correlation.

Prefer a pure codec that accepts/returns typed values and has no serial handle,
clock, retry policy, Runtime mutation, or GUI knowledge. Golden byte-vector tests
should cover every supported frame and every rejection class.

## Step 2: define strict configuration

The deployment schema is a closed, tagged enum in
`apps/lab-runtime/src/configuration.rs`. To make a new kind selectable:

1. add one `InstrumentDto` variant with `deny_unknown_fields` inherited from the
   tagged enum;
2. give it stable nonzero numeric identity and a validated logical key;
3. reference an existing resource by ID rather than a raw port opened by the driver;
4. validate cadence, queue lifetime, transaction timeout, address, counts, and all
   protocol-specific ranges;
5. cross-validate controllers, safe profiles, units, roles, and resource capability;
6. expose only legitimate live properties through `InstrumentPropertySource`;
7. classify each property as read-only, deployment-only, ordinary live, live-safe,
   or reinitializing according to its real lifecycle.

Metakon's current deployment shape is:

```toml
[[instruments]]
id = 1
key = "metakon-513"
kind = "metakon"
definition = "definitions/metakon-513-output.json"
resource_id = 1
address = 5
poll_period_ms = 1000
queue_timeout_ms = 1000
transaction_timeout_ms = 1000
```

The exact fields and bounds are in [Configuration](../configuration.md). Metakon
accepts address `1..=247`, poll/queue/transaction times of `1..=60000` ms, and a
strict definition no larger than 16 KiB with at most 16 parameters. Its only current
live property is `poll_period_ms`; the generic property projection consumes neutral
metadata rather than matching Metakon in the Application layer.

Configuration validation must finish before hardware work. Unknown fields, duplicate
IDs or keys, missing resources, unsafe output metadata, and unresolved references
must fail the candidate without partial Runtime mutation.

## Step 3: load and freeze artifacts

A protocol definition file is optional architecture, not a driver requirement. Use
one only when it represents bounded declarative metadata that is valuable to preserve
separately from code.

The current Metakon loader demonstrates the rules:

- `parse_runtime_toml` first parses and structurally validates deployment TOML;
- artifact paths resolve relative to the deployment file, with parent traversal
  rejected;
- exact admitted bytes are read once, bounded, SHA-256 identified, and retained in
  `FrozenDeployment`;
- composition consumes frozen bytes, never a later reread of the pathname;
- the definition selects only trusted `KnownOperation` values; it cannot supply raw
  frames or executable behavior;
- schema/domain validation checks unique identities, operations, units, access,
  roles, value bounds, write effect, and positive scale.

If a new driver uses an artifact, add one explicit artifact kind and one bounded
parser. Include exact admitted bytes and a stable content identity in activation
provenance. Do not accept scripts, expressions, arbitrary register access, or an
unbounded generic data language as a shortcut to native code.

## Step 4: bind the transport resource

A native serial driver does not own the OS port. The current path is:

```text
deployment resource
    -> ComSettings validation
    -> one ComTransport worker owns the handle
    -> one Core ResourceExecutor serializes transactions
    -> protocol-specific Runtime admission/completion
```

`ServiceHost::startup` constructs one `ComTransport` for each declared resource.
`HostCore::configured_with_transports` rejects a missing or undeclared adapter and
registers the accepted adapter with Core. The blocking serial worker has bounded
request/completion slots and finite OS call timeouts; the serialized Core owner polls
it nonblockingly.

The driver's transaction code must use the registered `ResourceExecutor`. It must
carry a finite queue deadline and transaction deadline and respect the executor's
fixed byte and queue bounds. It must not spawn a second port owner, bypass resource
generation, poll an OS handle from the GUI/client thread, or retry a logical write in
the serial worker.

A genuinely new transport family requires a separately reviewed Runtime-owned
adapter with the same ownership, bound, timeout, retirement, and generation
properties. It is not permission for an individual instrument to open arbitrary I/O.

## Step 5: construct descriptors and identities

Core exposes instruments through `InstrumentDescriptor` and
`ParameterDescriptor`. A descriptor supplies:

- stable `InstrumentId` and per-instrument `ParameterId` values;
- a bounded human-readable name;
- `ValueSpec` scalar type and inclusive range;
- engineering `Unit` identity and symbol;
- `AccessMode`;
- `ParameterRole` such as measurement, diagnostic, or actuator;
- `WriteEffect`;
- a `SignalId` for parameters that produce observations.

Metakon keeps reusable metadata in `DataInstrumentDefinition` and physical identity
in the separate `MetakonBinding`. That binding carries resource, device, channel,
nonzero binding generation, nonzero mapping revision, and optional output unit and
timing. A new driver may use different concrete types, but it needs the same semantic
separation:

```text
stable identity and descriptor
    != current physical binding generation
    != current protocol mapping revision
    != output authority epoch
```

Stable IDs and keys are configuration/provenance identities. Generations and
revisions fence process-lifetime evidence; they are not replacements for stable IDs.
Do not derive identity from array order, COM enumeration order, or a client session.

## Step 6: compose the instrument

Add one explicit arm to `register_configured_instruments` in
`apps/lab-runtime/src/host/instruments.rs`. It should:

1. retrieve only frozen, already validated artifacts;
2. build the concrete Core registration candidate;
3. register descriptors/signals/bindings through a typed Core command;
4. record the instrument as physical;
5. create bounded read schedules and any compatibility probes;
6. retain only the small provenance metadata needed by the host.

Metakon calls `Command::RegisterMetakon` with `MetakonInstrumentConfig`, creates one
periodic temperature schedule, and creates a channel-type compatibility probe. Its
history capacity is currently fixed at 64 in composition. A new driver should expose
a validated deployment field only when it is a genuine supported setting.

Once ordinary descriptors and signals are registered, generic discovery, current
measurements, recent history, subscriptions, controller input, Workbench rebuild, and
Recorder facts should work without a device-specific Application operation or GUI
branch.

The typed registration itself is concrete Core code. Add the new instrument storage,
registration command handling, binding lookup, and activation-descriptor exposure to
`crates/lab-core/src/runtime.rs` and its focused submodules. Extend only the commands
needed for the real read/output/rebind lifecycle. Do not create a generic driver
registry just to replace visible match arms.

## Step 7: implement the read path

The complete path is:

```text
scheduled work
    -> bounded queue admission
    -> resource transaction
    -> strict protocol decode
    -> current-generation check
    -> typed observation
    -> signal, history, EventLog, Recorder
```

For Metakon, `HostCore::service` services safety first and yields before lower-priority
work if safety becomes due. When a read slot is due it issues
`Command::QueueMetakonRead` with finite queue and transaction times. Core encodes a
trusted register request, enqueues it on the bound resource, correlates completion,
validates binding generation and mapping revision, decodes the response, and only
then commits a typed sample.

A native read path is responsible for:

- exact request encoding and expected response size;
- strict response correlation, integrity checks, and typed decode;
- unit/scaling/range validation;
- carrying instrument, parameter, resource, binding generation, and mapping revision;
- turning timeout/protocol/transport failure into truthful unavailable/bad evidence;
- rejecting a completion that no longer belongs to the current binding;
- publishing only through the Runtime signal path.

Do not publish a stale response after reconnect. Do not turn a malformed or missing
response into a previous good value. Scheduler catch-up must be bounded; the current
periodic scheduler coalesces missed slots rather than replaying an old burst.

## Step 8: integrate writable output

Writable native code has a larger safety obligation. The only accepted sequence is:

```text
requested
    -> authorized
    -> queued and prepared
    -> final authority check
    -> first possible output byte (send started)
    -> protocol ACK
    -> separate READBACK
    -> observed output evidence
```

The concrete reference seam is `Runtime::queue_metakon_output` plus
`Runtime::poll_transports` in `crates/lab-core/src/runtime/physical_io.rs`. The path:

1. obtains an immutable transport intent from the actuator's `OutputAuthority`;
2. captures authority epoch, lease/proposal deadline, binding generation, and mapping
   revision;
3. encodes the typed WRITE;
4. reserves one bounded resource transaction;
5. rechecks required Recorder admission, current binding, mapping, authority, and
   deadline in the serialized authorization callback immediately before bytes;
6. records send start only when the transport crosses that boundary;
7. validates protocol ACK;
8. starts a distinct bounded readback transaction when policy requires it;
9. completes or fails authority with exact evidence.

Do not add a generic raw-write operation. A driver cannot treat a client request,
controller proposal, queue admission, or prepared frame as permission to send.
Authority is finite; it can expire or be revoked while a request waits.

The current physical-I/O module has concrete Metakon and SimpleDevice branches. A new
writable native protocol needs another narrowly reviewed concrete branch or equivalent
explicit composition inside that owner. It must not add a second output sequencer.

## Safe profile and controller integration

Central Runtime policy validates and owns:

- the declared actuator range and unit;
- `SafeProfile` minimum, maximum, safe value, maximum lease, maximum proposal TTL,
  and required evidence;
- controller input/output/reference cross-references and unit compatibility;
- controller lifecycle and finite leases;
- final authority, fault latch, ambiguity, and safe obligation.

The driver validates protocol representability and the device-specific binding. It
does not duplicate or weaken central policy. For writable Metakon, deployment
validation currently requires a writable percent actuator at register 6, scale 1,
range `-100..=100`, a writable COM resource, safe value zero, and readback evidence.
Those details are Metakon policy, not a universal driver contract.

At startup, physical controllers remain unprepared until compatibility probes succeed
and every configured physical output has matching safe readback with no lease or
in-flight command. Reconnect does not restart or rearm a failed controller.

## ACK semantics

ACK is strict protocol-response evidence for the WRITE transaction. It proves only
that the current correlated response passed the driver codec after the accepted send
boundary.

For Metakon, `decode_ack` requires exactly five bytes with matching device, channel,
register, write function, and CRC. Another protocol needs its own equally explicit
definition of ACK.

ACK does not prove:

- the requested value was authorized for all time;
- independent readback;
- downstream actuator motion, heating, flow, or other physical effect;
- that a timeout means no bytes reached the device.

Malformed/missing ACK or connection loss after send start makes the ordinary output
uncertain/ambiguous and revokes the lease. A stale-generation or mismatched-identity
ACK is rejected. No blind retry is permitted. If failure occurred before send start,
the existing bounded later-delivery policy may decide whether a distinct attempt is
safe; the transport worker itself still does not retry the logical WRITE.

## READBACK semantics

Readback is a second protocol transaction and evidence stage. It is not the bytes in
the ACK response.

For current writable Metakon, a valid ACK causes a read of the same output register.
Core checks the current binding/mapping, decodes the typed value, applies scale, and
compares it with the dispatched value through `OutputAuthority`. Match records
readback evidence; mismatch or unavailable/malformed/timeout records explicit
readback failure and fails closed.

A native driver with readback must carry the original dispatch identity and all
current-generation fences into that second transaction. It must not accept a late
readback from an old mapping or compare values using undocumented tolerance.

Some devices can support only ACK evidence. Such a driver may be usable only where a
safe profile explicitly permits acknowledgement evidence. It must not fabricate
readback, and ACK still does not prove physical effect. If the deployment requires
readback, an ACK-only device/configuration must be rejected.

## Step 9: reconnect and stale-completion fencing

Reconnect replaces resource ownership; it does not continue the old physical
session.

```text
old binding                    new binding
WRITE admitted
    -> send started
    -> transport lost
                               reconnect
                                   -> generation + 1
late ACK ---------------------------X rejected
```

Current fencing dimensions include:

- resource/executor generation;
- instrument binding generation;
- mapping/configuration revision;
- output authority epoch, lease, and proposal deadline;
- transport transaction and dispatch identity.

`ServiceHost` quiesces reads on the reconnecting resource, retires the old adapter,
opens one replacement, increments binding/mapping identity with checked arithmetic,
rebinds central safe profiles, runs current-generation compatibility probes, requests
safe output through the ordinary authority path, records the lifecycle, and resumes
ordinary acquisition with a fresh deadline. Unrelated resources continue.

If replacement probing fails, the new generation remains fenced/offline; old evidence
does not become current again. A successful reconnect does not automatically rearm a
controller. A new driver must extend this one lifecycle rather than add private retry
or reconnect logic.

## Step 10: publication and activation

Keep three phases distinct:

```text
configuration validation
    -> physical preparation and compatibility/safe gates
    -> public readiness or atomic live publication
```

The static native Metakon path is a startup composition. After the complete deployment
is validated and its artifacts frozen, bounded resource workers are created and Core
descriptors/schedules are composed. Before readiness, actual opens and compatibility
probes must succeed, safe readback is established for writable outputs, controllers
are prepared, listeners and Recorder are established, and only then does
`ServiceHost` report ready. `EventLog::new` captures the already composed static
topology as its baseline; the Metakon driver does not append discovery records by
hand.

The later live SimpleDevice candidate path has stronger explicit prepare/publish
machinery: hidden prepared topology, inert prepared reads, held EventLog sequence
capacity, Recorder activation reservation, atomic install/publication, and cleanup on
failure. That transaction is not currently a generic native-driver plugin API.

If a new native kind is startup-only, preserve the existing pre-readiness composition
and failure cleanup. If it genuinely needs live topology addition, it must be designed
through the accepted configuration/service publication lifecycle—including hidden
preparation, EventLog capacity, durable activation, atomic visibility, reconnect
fencing, and rollback—rather than publishing from driver code.

The service/EventLog layer owns reservation and publication. A driver supplies a
validated descriptor, binding, and truthful state; it does not manipulate event
sequence numbers. Held capacity prevents unrelated appenders from stealing the exact
space required by an accepted live publication.

## Step 11: Recorder provenance

Recorder is Runtime-owned. A driver supplies stable identity and evidence; it never
writes SQLite.

The current deployment loader contributes exact Runtime TOML and frozen instrument
definition bytes to the activation provenance bundle. The host adds Rust build/native
composition identity. Instrument provenance includes descriptor metadata and, for
Metakon, resource/device/channel plus binding generation, mapping revision, and
expected output unit. Runtime recording facts carry observations and each output
evidence stage.

For a new native driver, preserve at least:

- instrument and parameter identity;
- exact definition/artifact identity and bytes when artifacts exist;
- protocol/device address and resource identity;
- descriptor units, ranges, roles, and access;
- current binding generation and mapping/configuration revision;
- observation quality and timing;
- requested/authorized/send-start/ACK/readback/uncertain evidence;
- reconnect and activation lifecycle identity.

See [Recorder and SQLite](../recorder-sqlite.md) for the storage contract. Clients and
scripts consume Runtime semantics; they do not own scientific provenance.
For operator-facing retry, reconnect, and ambiguity decisions, see
[Recovery and fault handling](../recovery-and-faults.md).

## Step 12: failure cleanup

Define cleanup for every phase.

| Failure | Required outcome |
|---|---|
| config/parse/cross-reference | reject before hardware work |
| artifact load/validation | reject exact candidate; publish nothing |
| resource open/settings | no readiness; retire bounded worker |
| instrument composition | no partial accepted service |
| compatibility probe | no ordinary acquisition/controller activation |
| pre-send authority failure | send no byte; release reservation truthfully |
| partial/started write failure | record ambiguity; revoke; never blind retry |
| read/ACK/readback failure | publish truthful failure, not stale success |
| reconnect replacement failure | keep new generation fenced/offline; retire candidate |
| live publication failure | cancel reservations and remove hidden candidate work |
| shutdown | stop producers, seek safe evidence, retire resource, flush Recorder finitely |

Driver-owned cleanup covers protocol-local state and any concrete typed pending work.
Core/service-owned cleanup covers authority, resource executor, reconnect, EventLog,
Recorder, and process shutdown. Ownership should be visible in types and tests; a
`Drop` side effect is not a substitute for explicit evidence-preserving cleanup.

## Step 13: boundedness checklist

For every new integration, document and test:

- configuration and artifact byte/depth/value limits;
- descriptor/instrument/parameter counts;
- maximum request and response sizes;
- resource queue and pending-correlation capacity;
- queue TTL and transaction timeout;
- scheduling cadence and missed-deadline behavior;
- signal history capacity;
- event/Recorder admission behavior;
- retry policy (normally no logical write retry);
- reconnect attempt/deadline policy;
- worker/mailbox count and byte bounds;
- finite shutdown and what is reported when it cannot finish.

Not every bound is universal. Use canonical constants beside the owning
implementation and tests. Do not copy Metakon's 38-byte frame or a current deployment
count into a new protocol unless that is the real bound.

## Step 14: shutdown

Shutdown raises the producer barrier before new work, quiesces managed production,
pauses active controllers, and requests safe through each tracked central authority.
Safety and transport progress continue in bounded owner turns. Resource executors
retire their workers/handles, and Recorder is flushed/sealed within the finite
shutdown lifecycle.

A native driver must:

- stop scheduling after the owner stop barrier;
- admit no new private work;
- leave pending transaction outcome truthful;
- use central safe output handling;
- release resource ownership through the existing transport shutdown path;
- report unfinished cleanup rather than detach an uncontrolled worker.

Do not block the Runtime owner on a serial call or unbounded thread join. Do not close
Recorder from driver code.

## Step 15: registration and build integration

There is no runtime plugin registration. Wire the concrete integration into the
existing source owners:

1. declare/export the codec and domain module only at the narrow visibility required;
2. add concrete Core state and typed `Command`/`Query` handling needed by the
   lifecycle;
3. add the `InstrumentDto` variant, validation, artifact freezing, change
   classification, and neutral property projection;
4. add the host composition, schedule, and compatibility-probe arm;
5. add physical-I/O completion/output handling and reconnect rebind logic;
6. include the source in the existing crates and workspace build;
7. update public configuration and contributor documentation.

Ordinary signals should then flow through the existing generic Application registry.
Do not add a device-named public operation, Workbench branch, Recorder table, or new
crate unless the device introduces genuinely new product semantics and that larger
change is separately reviewed.

The smallest build proof is a warning-denied workspace check plus focused tests that
instantiate the new deployment and exercise the actual Core/host path. A successful
codec-only unit test is not registration evidence.

## Implementation steps at a glance

| Step | Primary source | Completion evidence |
|---|---|---|
| protocol | new bounded codec beside current adapters | golden and rejection vectors |
| config | `configuration.rs` | strict valid/invalid/cross-reference tests |
| artifact | loader plus `FrozenDeployment`, if used | exact frozen bytes/hash |
| transport | existing resource/executor path | sole owner, queue/deadline tests |
| descriptors | Core instrument types | generic discovery and unit/range tests |
| composition | `host/instruments.rs` | full configured host builds atomically |
| reads | Core physical I/O plus host scheduler | good/fault/stale/reconnect tests |
| outputs | physical I/O plus `OutputAuthority` | final-check and evidence matrix |
| reconnect | host configuration plus service lifecycle | generation/fence tests |
| publication | existing startup/live lifecycle | no partial visibility/work |
| provenance | host recording projection | archive reopen assertions |
| shutdown | host/service lifecycle | finite clean and incomplete outcomes |
| build | existing crates/workspace | warning-denied build and source review |

At every step, the forbidden shortcut is the same: do not create a parallel owner to
avoid integrating with the source listed in the middle column.

## Metakon worked reference

The repository's physical examples are qualification/development inputs, not safe
virtual package examples. In a source checkout, trace the read-only path as follows:

1. `examples/runtime.metakon-513-com5.toml` selects `kind = "metakon"`, a COM
   resource, address, cadence, and frozen definition.
2. `examples/definitions/metakon-513-thermocouple.json` selects trusted
   `channel_type` and `temperature` operations and supplies descriptor metadata.
3. `configuration.rs` parses/cross-validates both and freezes exact definition bytes.
4. `host/instruments.rs` registers `MetakonInstrumentConfig`, the temperature
   schedule, and the channel-type compatibility probe.
5. `service.rs` creates the sole COM worker and requires the startup probe before
   readiness.
6. `host/scheduler.rs` queues the periodic typed read.
7. `runtime/physical_io.rs` correlates and decodes it with `metakon.rs`, rejects stale
   binding/mapping evidence, and commits the signal.
8. Generic Application discovery/measurement/history and Recorder paths expose the
   result without a Metakon-specific client operation.

Trace writable output with `examples/runtime.metakon-513-com5-output.toml` and
`examples/definitions/metakon-513-output.json`:

```text
controller proposal
    -> OutputAuthority lease and intent
    -> Metakon register-6 WRITE
    -> exact Metakon ACK
    -> register-6 READBACK
    -> matched output evidence or explicit failure
```

The deployment also provides the safe profile and controller. The concrete output
branch is in `runtime/physical_io.rs`; policy state is in `output/authority.rs`; host
ordering remains in `host/scheduler.rs`. Do not copy the Metakon register/CRC rules
into another driver as if they were framework requirements.

## Required test matrix

Tailor the matrix to the driver. A read-only driver does not need fabricated output
tests, but every applicable row needs deterministic evidence.

### Configuration and artifacts

- valid minimal and full deployment;
- unknown/duplicate field and identity;
- invalid address, cadence, timeout, count, and numeric bound;
- missing/ineligible/duplicate resource;
- unresolved controller/reference/safe-profile relationship;
- missing, oversized, malformed, changed, or unsupported artifact;
- exact frozen-byte/hash provenance;
- no physical open before structural and safety validation succeeds.

### Codec and reads

- golden request and good response bytes;
- wrong length/address/function/type/checksum;
- out-of-range or unrepresentable scalar;
- queue expiry, transaction timeout, disconnect, partial/trailing bytes;
- unavailable evidence without stale-value fabrication;
- old-generation completion rejected after reconnect;
- current-generation recovery produces a fresh observation;
- scheduler does not replay missed intervals as a burst.

### Writable output

- out-of-range and unrepresentable value;
- absent authority, expired lease/proposal, and expiry while queued;
- final authority/epoch/binding/mapping check failure sends no byte;
- send-start boundary recorded exactly once;
- exact ACK success and malformed/missing ACK;
- disconnect before send versus after send;
- partial/started write becomes ambiguous and is not blindly retried;
- readback success, mismatch, timeout, and malformed response;
- old-generation ACK/readback rejected;
- safe command follows the same output path and is not automatic rearm;
- required Recorder failure prevents authority-increasing output.

### Lifecycle and ownership

- sole resource owner and bounded queue;
- startup compatibility gate before readiness;
- no partial visible topology after failed setup;
- live prepare is hidden/inert if live addition is supported;
- EventLog/Recorder publication is atomic if live addition is supported;
- reconnect quiesces only the affected resource and fences late work;
- failed replacement is retired and stays fail-closed;
- clean and incomplete shutdown report honest terminal evidence;
- no instrument-specific Application, Workbench, or SQLite bypass.

### Recorder and clients

- descriptor, artifact/build, resource, binding, generation, and mapping provenance;
- good/bad observations and all applicable output evidence stages;
- reconnect/reconfiguration activation ordering;
- generic Runtime discovery/current/history/subscription behavior;
- generic Workbench rebuild/projection, without device-specific presentation logic.

Current regression references include:

- `crates/lab-core/tests/milestone3_codec_units.rs`;
- `crates/lab-core/tests/milestone3_metakon_runtime.rs`;
- `crates/lab-core/tests/milestone3_output_transport.rs`;
- `crates/lab-core/tests/milestone9d_physical_output.rs`;
- `apps/lab-runtime/tests/configuration_validation.rs`;
- `apps/lab-runtime/tests/configured_physical.rs`;
- `apps/lab-runtime/tests/configured_physical_output.rs`;
- `apps/lab-runtime/tests/host_scheduler.rs`;
- `apps/lab-runtime/tests/recorder_provenance.rs`;
- `apps/lab-runtime/tests/recorder_evidence.rs`;
- `apps/lab-runtime/tests/runtime_shutdown.rs`.

These tests are examples of required evidence, not an API compatibility suite for an
external driver SDK.

## Physical qualification

Passing software tests does not qualify physical hardware. Record a separate bounded
qualification for each supported combination:

- exact device model and hardware revision;
- firmware version;
- adapter/interface and driver version;
- wiring, isolation, termination, and power arrangement;
- serial settings and protocol-document revision;
- normal reads and the supported value range;
- malformed/truncated/trailing responses;
- disconnect, reconnect, cable removal, and power cycle;
- queue and transaction timeouts;
- WRITE, exact ACK, independent readback, and mismatch;
- safe value and loss-of-authority behavior;
- ambiguity after send start and confirmation that no blind retry occurs;
- controlled fault injection where physically safe;
- known limits and observations not proved by the test.

Keep physical test inputs clearly labeled. A parser/compiler test using a placeholder
COM port is not hardware qualification, and ACK/readback still does not prove an
independent downstream physical effect.

## Review checklist

Before requesting review, confirm:

- [ ] SimpleDevice is demonstrably insufficient.
- [ ] The codec is pure, strict, bounded, and independently tested.
- [ ] Configuration is closed, cross-validated, and fails before hardware work.
- [ ] Exact source artifacts are frozen and recorded when applicable.
- [ ] One Runtime resource owner controls the physical handle.
- [ ] Reads carry and check current binding/mapping identity.
- [ ] Descriptors use stable IDs, truthful units, roles, ranges, and access.
- [ ] Generic API/EventLog/Recorder paths need no device-specific bypass.
- [ ] Every output passes central authority and the final pre-byte check.
- [ ] Send start, ACK, readback, ambiguity, and physical effect stay distinct.
- [ ] Post-send uncertainty never causes blind retry.
- [ ] Reconnect advances/fences identity and never rearms control.
- [ ] Startup/live publication cannot expose half-built topology.
- [ ] All queues, deadlines, histories, workers, and shutdown are bounded.
- [ ] Recorder provenance is Runtime-owned and complete enough to reconstruct context.
- [ ] Software acceptance and physical qualification are reported separately.

## Packaging and public documentation

Update [Configuration](../configuration.md) with the new deployment kind and public
bounds. Update [Extending the Runtime](../extending-runtime.md) only if the integration
changes the source map. Add safe public examples only when they do not unexpectedly
open physical hardware; keep physical qualification inputs clearly identified.

The preview package intentionally includes this guide but not the Rust source tree.
Users can configure native drivers compiled into the shipped Runtime. Developers who
add one need the repository, Rust toolchain, focused acceptance suite, and a new
release build.

## What remains internal

Concrete Core registration commands, host composition types, scheduler structures,
resource executor details, and Recorder activation records are current internal Rust
interfaces. They are documented here to guide contributors, not promised as a stable
external SDK. A future refactor may move those seams while preserving the authority,
evidence, boundedness, and lifecycle invariants in this guide.
