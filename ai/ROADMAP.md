# Roadmap after the developer preview

## Current state

See the canonical current-state summary and authorization in
[`ai/WORK.md`](WORK.md#canonical-current-state). This roadmap retains milestone
sequencing, durable boundaries, and historical evidence below.

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
consolidated acceptance. The read-only M16.1 audit is accepted in
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`. The accepted M16.2
read-only simple-device implementation commit is
`aac377470d7142005ad5e0d098fddf6c16734db8`. The accepted M16.3 writable
simple-device implementation commit is `df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`.
The accepted M16.4 Application provisioning implementation commit is
`92847a046c4ef2e4e69d24ea51fc56e28e42af38`. External re-review accepted the original
implementation plus the pending-apply property fence, existing `operation_failed`
mapping for internal owner failures, and phase-aware absolute apply deadline with
quarantine based on actual `send_started && !safe_confirmed` evidence. M16.5 generic
integration acceptance is externally accepted at
`799e7f969393b960ae54409f90842ff2ab1b84cf`. M16.6 fault/bounds/provenance/recovery
acceptance is externally accepted at `6021586096453fa03bdbcc28ef740b660f32eeb5`.
M16 fast/simple/declarative instrument onboarding is complete and externally accepted;
prepared SimpleDevice topology is physically inert until publication, participating
resource reconnect is fenced while publication is pending, and failed pre-durable
apply leaves no hidden topology or pending transport work.
The atomic publication boundary, held EventLog capacity, controller/output semantics,
reconnect/restart behavior, Recorder/provenance, and Workbench acceptance are complete.
Final external review result: M16 READY TO CLOSE. M17 and the future dual
Runtime/Workbench API remain unauthorized and unstarted.

The accepted headless Runtime core is implemented and technically hardened for a
developer preview. Remaining work concerns reference material, practical integration
validation and release preparation, not an unplanned feature milestone.

## Developer Preview Preparation

The compact developer-facing reference is complete:

1. architecture and concepts guide;
2. Application API reference;
3. Recorder/SQLite archive reference;
4. safety/failure/recovery cheat sheet;
5. configuration and instrument/component extension notes;
6. getting-started/build/run guide;

Preview packaging/build is complete. The checksummed Windows x86_64 artifact is
ready locally, and `v0.1.0-preview.1` is published as a GitHub pre-release. Its tag
remains fixed at the packaged source commit; later coordination commits are not
release inputs.

This is not yet the final polished release-documentation pass. Each execution slice
requires explicit authorization in `WORK.md`.

## Practical learning and integration

After preview preparation, the planned exercise is:

```text
M16 declarative simple-device layer
→ real Arduino thermal plant implemented as a declarative definition/instance
→ real COM acceptance with no Arduino-specific Runtime driver
→ external Clojure Application client / lab-orchestrator
→ experiment procedures
→ sealed Recorder SQLite archives
→ Clojure/Clay analysis and system identification
```

Arduino is the first intended real-device acceptance/use case after the future dual
Runtime/Workbench API boundary, not the architecture of M16 itself. Runtime continues
to own experiment semantics. The future Clojure client is an external Application
client and does not know the Arduino wire protocol. This remains future sequencing,
not current authorization.

The published preview contains TCP/NDJSON. The accepted post-preview M12.3 commit on
`main` additionally contains the optional loopback WebSocket/JSON implementation.

## M12 — WebSocket transport

M12 adds WebSocket/JSON as a second bounded loopback transport for the same
Application API:

```text
TCP / NDJSON ------+
                   +-> one shared delivery boundary -> ONE Application
WebSocket / JSON --+
```

- **M12.1:** read-only architecture audit — accepted;
- **M12.2:** transport-neutral server seam, with unchanged TCP behavior — accepted;
- **M12.3:** bounded local WebSocket/JSON transport and browser Origin policy —
  accepted;
- **M12.4:** parity, reconnect, backpressure and fault acceptance — accepted;
- **M12.5:** browser/ClojureScript smoke acceptance — accepted;
- **M12 external review:** accepted.

M12.1 through M12.5 are accepted. See
`M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md` and
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`; M12.3 evidence is in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.4 evidence is in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md`. M12.5 evidence is in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is accepted. Consolidated evidence is in
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is accepted.

## M13 — external Steel scripting host

Steel is a future external Application client. It must not execute inside Runtime
ownership, acquire transport/output authority, or make script lifetime equal
experiment lifetime. Native real-time components remain native Rust.

The M13.1 source/dependency audit is accepted in
`M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. It establishes a
one-process-per-script external host over the existing TCP/NDJSON Application
endpoint. The read-only dependency safety resolution for the rejected
`steel-core 0.8.3` graph is complete. Its report,
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`, found no acceptable current release or
upstream commit. M13.2 remains blocked and unauthorized.

The earlier separate `lab-steel` implementation sequence is deferred. Current
sequencing builds the external native Workbench client first. If a later dependency
review accepts Steel, the preferred placement is an optional subsystem inside the
Workbench, using its one Application client boundary. Steel remains neither a
Runtime subsystem nor a managed component.

## M14 — GUI/Workbench/presentation schema

GUI and any future transport/language-neutral `PresentationDocument` are
client-owned. Runtime must not acquire window, tab, row, column, plot, panel,
layout, widget, slider, button, egui, or other presentation semantics.

The read-only M14.1 architecture audit is accepted in
`M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The minimal private bounded native
Application client is accepted in `M14_2_MINIMAL_WORKBENCH_CLIENT.md`; the M14.3
client-owned model and persistence implementation is accepted in
`M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The minimal native GUI implementation and
evidence and overflow/live-continuity remediation are in `M14_4_MINIMAL_GUI.md`,
and accepted. M14.5 operator controls and property/configuration workflows are
accepted in `M14_5_OPERATOR_CONTROLS.md` at implementation commit
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
implementation commit is `029b82ab5bd7ef39acf00844824be8887f45ffd1`. M15.5–M15.8
are deferred and unauthorized until M16 consolidated acceptance. This sequencing
lets final API/tutorial/recovery/automation and documentation-acceptance work describe
the post-M16 provisioning/configuration surface once. Consolidated M15 acceptance is
not claimed.

## M16 — declarative simple-device integration

M16 is mandatory before the remaining final M15 documentation work. Its product goal
is that a user who knows only the small useful subset of a device protocol—such as
READ measurement plus WRITE actuator plus ACK/readback—can describe it as bounded
declarative configuration, provision it through the language-neutral Application
boundary, and obtain ordinary Runtime Instrument, Signal, and Actuator entities
without writing a device-specific Rust driver.

After activation, declarative entities use existing generic paths:

```text
declarative Signal
    → ordinary discovery
    → current measurements
    → subscriptions
    → Workbench live plot
    → Runtime recent history
    → Recorder/durable history
    → controller input

declarative Actuator
    → ordinary Runtime OutputAuthority path
    → final authority/generation/deadline checks
    → declarative encoder
    → physical transport
    → ACK
    → optional separate readback
```

Workbench, Recorder, history, controller, and ordinary Application operation paths
must not acquire special semantic branches merely because a device is declarative.
Complex protocols that do not fit the bounded v1 model remain explicit native Rust
adapters.

### Configuration and procedure boundary

The M16/M18 product boundary is:

```text
persistent deployment configuration != current Runtime state != experiment procedure

TOML / deployment configuration
    = what laboratory installation exists and its safe reproducible baseline

declarative device definition
    = how a bounded physical protocol maps to typed laboratory parameters

Application API
    = authoritative way to change Runtime state and configuration during execution

future Clojure experiment script
    = what to do over time with already-defined laboratory entities
```

The future external script knows semantic Reference, controller, Signal, and Actuator
identities. It must not know COM port, baud rate, device address, protocol bytes, CRC,
register offsets, raw scaling, SQLite paths, or OutputAuthority internals. Runtime/API
mutations may change active state during execution without silently rewriting the
original deployment source. This roadmap does not authorize scripting implementation
or select a Runtime-owned automation language.

### M16 sequence

- **M16.1:** accepted read-only, source-derived declarative-device
  architecture/API/bounds audit.
- **M16.2:** accepted bounded serial request/response READ to a typed ordinary Signal.
- **M16.3:** accepted ordinary OutputAuthority to declarative WRITE, ACK, and
  optional separate readback.
- **M16.4:** accepted bounded Application configuration-candidate provisioning:
  validate,
  stage, explicit safe apply, then ordinary rediscovery.
- **M16.5:** accepted generic Workbench/plot/history/Recorder/controller/reconnect
  integration with no declarative-device special branches.
- **M16.6:** externally accepted acceptance-first fault, bounds, provenance, and
  recovery matrix.
- **M16.7:** accepted minimal unknown-device acceptance for READ measurement, WRITE
  actuator, ACK, and READBACK without device-specific Runtime/Application/Workbench
  code.
- **M16 consolidated external review:** accepted with no blockers; M16 READY TO CLOSE.

M16.2 is accepted at `aac377470d7142005ad5e0d098fddf6c16734db8`. M16.3 is accepted
at `df5ec6b7b0a10b3bcd3e43f4b213095c1f846c35`. M16.4 is accepted at
`92847a046c4ef2e4e69d24ea51fc56e28e42af38`. M16.5 generic integration acceptance is
externally accepted at `799e7f969393b960ae54409f90842ff2ab1b84cf`. M16.6 is externally accepted at
`6021586096453fa03bdbcc28ef740b660f32eeb5`; M16.7 is accepted and consolidated
external review recorded M16 READY TO CLOSE. The accepted M16.1 audit freezes the
operation, DTO/schema direction, exact bounds, and implementation slicing in
`M16_1_DECLARATIVE_DEVICE_ARCHITECTURE_API_AUDIT.md`.

### V1 direction and authority

M16.1 audits a serial/COM-first, finite request/response subset: fixed-length or
delimiter-terminated replies; bounded address/channel and template/extractor fields;
integer, justified IEEE floating-point, and bounded ASCII numeric representations;
signedness, endian, finite scale/offset, bounded prefix/suffix checks, selected fixed
CRC/checksum algorithms, periodic reads, typed writes, ACK validation, optional
independent readback, and finite timeout/queue/frame/parser bounds.

V1 explicitly excludes arbitrary code, scripts, callbacks, loops, a general
expression language, dynamic evaluation, unbounded parsing, general protocol state
machines, user-defined allocation, and raw public byte operations. M16 must not add
public operations equivalent to `send_raw_bytes`, `raw_serial_write`,
`execute_device_command`, or `unchecked_register_write`.

Declarative configuration never acquires transport or output authority. A writable
parameter may only encode the final engineering value after existing Runtime-owned
authority, generation, and deadline fences. ACK, readback, and physical effect remain
distinct; ambiguity retains the existing fail-closed/no-blind-retry semantics.
Recorder provenance must retain stable definition/hash/version, normalized
configuration, instance, resource/binding generation, and existing build/Runtime
identity. Declarative configuration cannot access arbitrary filesystem/network
endpoints, open transports, alter Recorder storage directly, fabricate quality or
ACK/readback, or create operations dynamically.

Current Application configuration operations stage/apply Runtime-known candidates;
they do not accept arbitrary externally supplied deployment definitions. M16.1 must
audit the smallest bounded language-neutral additive surface by which a future client
can submit a definition/instance candidate, obtain complete Runtime validation, stage
it, explicitly safe-apply it, and rediscover an ordinary instrument. Candidate upload
must not activate anything, and no operation names are chosen in this coordination
transition.

## Final release gate

The later final documentation/package audit will require polished architecture,
protocol, archive, configuration, safety/recovery and operational documentation plus
a reproducible checksummed Windows package. M15.5–M15.8 remain deferred and
unauthorized until M16 consolidated acceptance.

## Explicit non-goals

- no GUI or Presentation API;
- no bundled scripting runtime or first-party client SDK;
- no dynamic plugin framework;
- no new convenience API merely for documentation;
- no schema or accepted semantic changes;
- no claim of production certification, hard real-time or exhaustive physical
  qualification.


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
