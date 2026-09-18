# Metakon write-path audit

Date: 2026-09-18

Audited HEAD: `5d29cd171303c771278a0d46ad3f838d1911b631`

Method: static source and documentation audit only. COM5 was not opened and no
hardware or protocol transaction was performed.

## Result

**B — PARTIAL WRITE PATH.**

The production Core contains a bounded, authority-gated Metakon register-6
`WRITE` plus strict ACK implementation. The accepted v0.1 product does not connect
that primitive to a physical controller or host schedule: active physical
definitions are required to be read-only, the configured host never submits
`QueueMetakonOutput`, and native controller preparation accepts only the native
thermal plant as its output instrument. Physical readback is also not implemented.
Consequently the accepted product is not ready for a Metakon output hardware smoke
without adding production integration and a writable trusted definition.

## Actual call chains

The active native-controller path stops before physical transport:

```text
Application controller_start / scheduled TickController
  -> Runtime::tick_controller
  -> OutputProposal
  -> Runtime::deliver_simulated
  -> OutputAuthority
  -> ThermalPlantInstrument simulated effect
  -> DispatchOutcome::ReadbackVerified
```

This is production code in `apps/lab-runtime/src/application.rs`,
`apps/lab-runtime/src/host.rs`, and `crates/lab-core/src/runtime.rs`. It cannot bind
to Metakon: `Runtime::validate_controller_config` requires the output instrument to
exist in `thermal_plants` (`runtime.rs:2079-2126`). The simulated completion is not
a physical ACK or readback.

A separate production Core primitive exists, but has no active host caller:

```text
trusted Runtime caller
  -> Command::Output(... Propose(OutputProposal))
  -> OutputAuthority lease/unit/range/TTL validation
  -> Command::QueueMetakonOutput
  -> OutputAuthority::reserve_transport
  -> encode_scaled_i8
  -> encode_write(Address(device, channel, 6), I8(raw))
  -> ResourceExecutor::enqueue_output (retryable = false)
  -> Command::PollTransports / Runtime::poll_transports
  -> binding generation + mapping revision checks
  -> OutputAuthority::validate_transport
  -> ByteTransport::try_write
  -> ComTransport one-slot worker / SerialPortDevice::write_once
  -> OutputAuthority::begin_transport after positive local admission
  -> bounded five-byte response
  -> decode_ack(Address(device, channel, 6))
  -> OutputAuthority::complete_transport(Acknowledged)
```

The relevant production locations are:

| Step | Type/function | Source |
| --- | --- | --- |
| Proposal and authority state | `OutputProposal`, `OutputAuthority` | `crates/lab-core/src/output.rs`; `crates/lab-core/src/output/authority.rs` |
| Typed output reservation and encoding | `Command::QueueMetakonOutput` | `crates/lab-core/src/runtime.rs:1528` |
| Final binding/authority gate and completion | `Runtime::poll_transports`, `handle_transport_event` | `crates/lab-core/src/runtime.rs:3826-4075` |
| Non-retrying bounded transaction | `ResourceExecutor::enqueue_output`, `poll_authorized` | `crates/lab-core/src/transport.rs:393-450`, `593-628` |
| Protocol codec | `encode_write`, `decode_ack`, `encode_scaled_i8` | `crates/lab-core/src/metakon.rs:241`, `284`, `358` |
| Windows byte worker | `ComTransport::try_write`, `write_same_frame`, `SerialPortDevice::write_once` | `apps/lab-runtime/src/serial.rs:546`, `781`, `866` |

The M3 transport and authority tests exercise this second path with fake adapters and
hand-built writable definitions, especially
`crates/lab-core/tests/milestone3_output_transport.rs`. Recorder tests also invoke
the Core command directly. Those tests prove the primitive; they do not supply the
missing accepted-product host/controller integration or real-device evidence.

## Register 0x06 profile

Current production code statically maps `KnownOperation::Output` to register `6`.
Its supported representation is:

| Property | Current code |
| --- | --- |
| Register | `0x06` |
| Domain type | finite `Value::Float` |
| Wire type | signed `I8` (type code `0x02`, one two's-complement byte) |
| Raw range | `-100..=100` |
| Scale | exactly `1.0` for a validated output definition |
| Intended unit | percent in the M3 profile and fixtures; the validator binds and checks the declared unit rather than hard-coding `Unit::PERCENT` |
| Write | function/opcode `0x01`; address uses the bound device and channel |
| Endian | irrelevant for the one-byte `I8`; supported multi-byte integers are little-endian |
| CRC | custom one-byte CRC, initial value `0xFF`, no final XOR |
| Read | the generic read queue can request `I8` for a `ReadWrite` output definition |
| Readback | not implemented: `apply_metakon_value` discards `KnownOperation::Output` values |

Registers `0x07` and `0x08` have no current `KnownOperation`, protocol integration,
or readback implementation. The accepted Metakon 513 definition in
`examples/definitions/metakon-513-thermocouple.json` contains only read-only
registers 0 and 1. Static donor-derived profile knowledge and fake-adapter tests are
not evidence that register 6 was physically verified on the currently connected
device.

## Authority and safety boundary

For the Core transport primitive, ordinary output requires an existing actuator and
profile, a current finite `OutputLease`, matching authority instance and epoch,
finite/range/unit-valid proposal data, an unexpired proposal/queue deadline, and the
current physical binding generation and mapping revision. `validate_transport` is
called immediately before adapter admission. `ComTransport` documents positive
local admission as send-start evidence because the worker may emit bytes after that
point. A zero-byte result is not send-start and is revalidated on the next attempt.

Requested, admitted/sent, acknowledged, read back, and physical effect remain
distinct. A valid write ACK is exactly five bytes containing matching
device/channel/register/function `0x01` plus a valid CRC. It contains no echoed value.
The Runtime records `Acknowledged`; it does not synthesize `ReadbackVerified`.
There is no post-ACK register-6 readback sequence, no register-7/8 readback, and no
physical-effect claim.

Output transactions have `retryable = false`. An error or timeout after positive
admission produces `OutputUncertain`, revokes ordinary authority, requests safe
state, fences the ordinary queue, performs bounded recovery, and settles as
`Ambiguous`; it does not blindly retry the write. Failed or ambiguous safe delivery
does not spin and does not rearm.

No `set_output`, `raw_write`, or `write_register` operation exists in the Application
API. Its `output` operation is query-only, and controller mutations cannot bind a
physical Metakon actuator in the current Runtime.

There is nevertheless an in-process encapsulation caveat: `lab-runtime` publicly
exports `serial::ComTransport`, `ComTransport::open_windows`, and the public
`lab_core::transport::ByteTransport` trait exposes `try_write(&[u8])`. A Rust library
consumer can therefore open a serial resource independently and submit arbitrary
bytes without `OutputAuthority`. The accepted executable does not use or expose this
as an Application API operation, but the public low-level adapter surface is not an
enforced OutputAuthority-only boundary. It must not be used as evidence of a safe
product write path.

## Safe-zero status

`OutputCommand::RequestSafe` is the production Core transition: it advances the
authority epoch, removes the lease and pending ordinary proposal, and reserves the
profile's explicit `safe_value`. Zero is supported only when the selected
`SafeProfile` declares zero; zero is not a universal Metakon rule. A trusted caller
must then submit `QueueMetakonOutput` and poll the transport. The same register-6
`WRITE` and ACK path is used. ACK confirms safe state only for a profile whose
required evidence is `Acknowledgement`; a `Readback` profile remains unconfirmed
because physical readback is absent. No successful safe transition automatically
rearms output.

The active configured host has no physical safe-zero flow: it rejects physical
actuator/output definitions in `configuration::freeze_definition`
(`apps/lab-runtime/src/configuration.rs:938-963`), registers accepted Metakon
bindings with `expected_output_unit: None`, and never issues
`Command::QueueMetakonOutput`. Its simulated safe completion is limited to the
native thermal plant.

## Missing links preventing classification A

1. No accepted writable Metakon register-6 definition can pass deployment
   validation.
2. No production host scheduler bridges native controller proposals or physical
   safe requests to `QueueMetakonOutput`.
3. Native controller validation deliberately rejects a Metakon output binding.
4. There is no physical register-6 readback-to-`ReadbackVerified` implementation.
5. The low-level serial adapter is publicly raw-byte callable by an in-process Rust
   consumer, so OutputAuthority exclusivity is architectural convention rather than
   an enforced crate boundary.

Therefore the precise status is: **Core WRITE+ACK primitive present; active product
integration and readback missing; safe-zero primitive present only at Core level;
hardware write smoke not ready.**
