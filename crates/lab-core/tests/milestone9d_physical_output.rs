//! M9D acceptance for generic native control through physical Metakon output.

use lab_core::{
    AccessMode, Command, InstrumentId, ParameterId, ParameterRole, Query, QueryResult, Runtime,
    SignalId, Unit, ValueSpec, VirtualInstrumentConfig, WriteEffect,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputReadbackFailure,
        OutputState, SafeProfile,
    },
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

const METAKON: InstrumentId = InstrumentId::new(50);
const INPUT: InstrumentId = InstrumentId::new(51);
const OUTPUT: ParameterId = ParameterId::new(3);
const CONTROLLER: ControllerId = ControllerId::new(7);
const REFERENCE: ReferenceId = ReferenceId::new(8);

#[derive(Default)]
struct Wire {
    frames: Vec<Vec<u8>>,
    readable: VecDeque<u8>,
    last_output: i8,
    readback_offsets: VecDeque<i8>,
    suppress_readback: bool,
    write_limits: VecDeque<usize>,
    fail_before_first_write: bool,
    started_writes: usize,
    recoveries: usize,
}

struct MetakonFake(Rc<RefCell<Wire>>);

impl ByteTransport for MetakonFake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        if wire.fail_before_first_write && matches!(bytes, [15, 0, 6, 1, ..]) {
            wire.fail_before_first_write = false;
            return Err(TransportIoError::Disconnected);
        }
        let count = bytes
            .len()
            .min(wire.write_limits.pop_front().unwrap_or(bytes.len()));
        if count > 0 && matches!(bytes, [15, 0, 6, 1, ..]) {
            wire.started_writes += 1;
        }
        wire.frames.push(bytes[..count].to_vec());
        if count != bytes.len() {
            return Ok(count);
        }
        match bytes {
            [15, 0, 6, 1, 2, raw, _] => {
                wire.last_output = *raw as i8;
                let body = [15, 0, 6, 1];
                wire.readable.extend(body);
                wire.readable.push_back(crc(&body));
            }
            [15, 0, 6, 0, _] if !wire.suppress_readback => {
                let offset = wire.readback_offsets.pop_front().unwrap_or(0);
                let raw = wire.last_output.saturating_add(offset) as u8;
                let body = [15, 0, 6, 0, 0xc2, raw];
                wire.readable.extend(body);
                wire.readable.push_back(crc(&body));
            }
            _ => {}
        }
        Ok(count)
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
            *byte = wire.readable.pop_front().expect("bounded readable length");
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.recoveries += 1;
        wire.readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

fn actuator() -> ActuatorId {
    ActuatorId::new(METAKON, OUTPUT)
}

fn metakon() -> MetakonInstrumentConfig {
    MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: METAKON,
            name: "Physical Metakon output".into(),
            parameters: vec![DataParameterDefinition {
                id: OUTPUT,
                name: "output_power".into(),
                value_spec: ValueSpec::Float {
                    min: -100.0,
                    max: 100.0,
                },
                unit: Unit::PERCENT,
                access: AccessMode::ReadWrite,
                role: ParameterRole::Actuator,
                write_effect: WriteEffect::OutputAffecting,
                operation: KnownOperation::Output,
                scale: 1.0,
            }],
        },
        binding: MetakonBinding {
            resource: ResourceId::new(1),
            device: 15,
            channel: 0,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: Some(Unit::PERCENT),
            output_queue_ttl: Some(Duration::from_millis(50)),
            output_timeout: Some(Duration::from_millis(10)),
        },
        history_capacity: 2,
    }
}

fn setup() -> (Runtime, Rc<RefCell<Wire>>) {
    setup_with_wire(Wire::default())
}

fn setup_with_wire(state: Wire) -> (Runtime, Rc<RefCell<Wire>>) {
    let wire = Rc::new(RefCell::new(state));
    let mut runtime = Runtime::new();
    runtime
        .register_transport(ResourceId::new(1), Box::new(MetakonFake(wire.clone())))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(metakon()))
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: actuator(),
            command: OutputCommand::BindProfile(SafeProfile {
                min: -100.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(3),
                max_proposal_ttl: Duration::from_secs(1),
                required_evidence: EvidenceLevel::Readback,
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    (runtime, wire)
}

fn service(runtime: &mut Runtime, at_ms: u64) {
    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_millis(at_ms),
        })
        .unwrap();
}

fn output(runtime: &Runtime) -> lab_core::output::OutputSnapshot {
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!("unexpected output query")
    };
    snapshot
}

fn establish_safe(runtime: &mut Runtime) {
    runtime
        .command(Command::Output {
            actuator: actuator(),
            command: OutputCommand::RequestSafe,
            at: Duration::ZERO,
        })
        .unwrap();
    service(runtime, 0);
    service(runtime, 1);
    let snapshot = output(runtime);
    assert!(snapshot.safe_confirmed);
    assert_eq!(snapshot.acknowledged.unwrap().value, 0.0);
    assert_eq!(snapshot.readback.unwrap().value, 0.0);
    assert_eq!(snapshot.reported_readback.unwrap().value, 0.0);
}

fn install_controller(runtime: &mut Runtime) {
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: INPUT,
            name: "Controller input".into(),
            history_capacity: 4,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: REFERENCE,
            value: 20.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: CONTROLLER,
            input: SignalId::new(INPUT, lab_core::TEMPERATURE),
            output: actuator(),
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_millis(100),
                warmup_samples: 1,
                unit: Unit::CELSIUS,
            },
            pid: PidConfig {
                kp: 0.0,
                ki: 0.0,
                kd: 0.0,
                output_min: 10.0,
                output_max: 11.0,
            },
            max_input_age: Duration::from_secs(2),
            max_tick_gap: Duration::from_secs(2),
            lease_lifetime: Duration::from_secs(3),
            proposal_ttl: Duration::from_secs(1),
        }))
        .unwrap();
    runtime
        .command(Command::PrepareController(CONTROLLER))
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: INPUT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(2),
        })
        .unwrap();
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::from_millis(2),
        })
        .unwrap();
}

#[test]
fn native_controller_uses_write_ack_readback_and_same_safe_zero_path() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    install_controller(&mut runtime);
    runtime
        .command(Command::RefreshMeasurement {
            instrument: INPUT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    service(&mut runtime, 1_000);
    assert!(output(&runtime).acknowledged.is_some());
    assert!(output(&runtime).readback.is_none());
    service(&mut runtime, 1_001);
    let delivered = output(&runtime);
    assert_eq!(delivered.outcome, Some(DispatchOutcome::ReadbackVerified));
    assert_eq!(delivered.readback.unwrap().value, 10.0);
    assert_eq!(delivered.reported_readback.unwrap().value, 10.0);

    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_millis(1_002),
        })
        .unwrap();
    service(&mut runtime, 1_002);
    service(&mut runtime, 1_003);
    let safe = output(&runtime);
    assert_eq!(safe.state, OutputState::Disarmed);
    assert!(safe.safe_confirmed);
    assert_eq!(safe.readback.unwrap().value, 0.0);
    let QueryResult::Controller(controller) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!("unexpected controller query")
    };
    assert_eq!(controller.state, ControllerState::Paused);

    let frames = &wire.borrow().frames;
    assert!(frames.iter().any(|frame| frame.get(5) == Some(&10)));
    assert!(
        frames
            .iter()
            .filter(|frame| frame.get(5) == Some(&0))
            .count()
            >= 2
    );
    assert_eq!(wire.borrow().recoveries, 0);
}

#[test]
fn mismatching_readback_is_explicit_and_requests_safe_without_success() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    install_controller(&mut runtime);
    wire.borrow_mut().readback_offsets.push_back(-1);
    runtime
        .command(Command::RefreshMeasurement {
            instrument: INPUT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    service(&mut runtime, 1_000);
    service(&mut runtime, 1_001);
    let snapshot = output(&runtime);
    assert_eq!(snapshot.acknowledged.unwrap().value, 10.0);
    assert!(snapshot.readback.is_none());
    assert_eq!(snapshot.reported_readback.unwrap().value, 9.0);
    assert_eq!(
        snapshot.readback_failure,
        Some(OutputReadbackFailure::Mismatch)
    );
    assert_eq!(snapshot.outcome, Some(DispatchOutcome::Failed));
    assert!(snapshot.fault_latched);
    assert!(!snapshot.safe_confirmed);
    let QueryResult::Controller(controller) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!("unexpected controller query")
    };
    assert_eq!(controller.state, ControllerState::Failed);
    service(&mut runtime, 1_002);
    service(&mut runtime, 1_003);
    let recovered = output(&runtime);
    assert_eq!(recovered.state, OutputState::FaultLatched);
    assert!(recovered.safe_confirmed);
    assert_eq!(recovered.readback.unwrap().value, 0.0);
}

#[test]
fn readback_timeout_is_ambiguous_and_never_retries_the_write() {
    // Once any WRITE byte may have reached hardware, timeout cannot prove absence
    // of effect. This oracle protects the non-retriable ambiguous state and the
    // separate safe obligation from a tempting queue-level retry simplification.
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    install_controller(&mut runtime);
    wire.borrow_mut().suppress_readback = true;
    runtime
        .command(Command::RefreshMeasurement {
            instrument: INPUT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    service(&mut runtime, 1_000);
    let writes_before_timeout = wire
        .borrow()
        .frames
        .iter()
        .filter(|frame| frame.get(3) == Some(&1))
        .count();
    service(&mut runtime, 1_001);
    service(&mut runtime, 1_011);
    service(&mut runtime, 1_012);
    let snapshot = output(&runtime);
    assert!(snapshot.acknowledged.is_some());
    assert!(snapshot.readback.is_none());
    assert_eq!(
        snapshot.readback_failure,
        Some(OutputReadbackFailure::Unavailable)
    );
    assert_eq!(snapshot.outcome, Some(DispatchOutcome::Ambiguous));
    assert_eq!(
        wire.borrow()
            .frames
            .iter()
            .filter(|frame| frame.get(3) == Some(&1))
            .count(),
        writes_before_timeout
    );
    assert_eq!(wire.borrow().recoveries, 1);
}

#[test]
fn safe_write_ambiguity_retains_the_obligation_without_an_automatic_resend() {
    let (mut runtime, wire) = setup_with_wire(Wire {
        write_limits: [2].into_iter().collect(),
        ..Wire::default()
    });
    runtime
        .command(Command::Output {
            actuator: actuator(),
            command: OutputCommand::RequestSafe,
            at: Duration::ZERO,
        })
        .unwrap();

    service(&mut runtime, 0);
    assert_eq!(wire.borrow().started_writes, 1);
    let started = output(&runtime);
    assert!(started.in_flight.is_some());
    assert!(!started.safe_confirmed);

    service(&mut runtime, 10);
    service(&mut runtime, 11);
    for at in 12..=32 {
        service(&mut runtime, at);
    }

    let unresolved = output(&runtime);
    assert_eq!(unresolved.state, OutputState::FaultLatched);
    assert!(unresolved.fault_latched);
    assert!(!unresolved.safe_confirmed);
    assert!(unresolved.lease.is_none());
    assert!(unresolved.in_flight.is_none());
    assert_eq!(unresolved.sent.unwrap().value, 0.0);
    assert_eq!(unresolved.outcome, Some(DispatchOutcome::Ambiguous));
    assert_eq!(wire.borrow().started_writes, 1);
    assert_eq!(wire.borrow().last_output, 0);

    let QueryResult::Transport(resource) =
        runtime.query(Query::Transport(ResourceId::new(1))).unwrap()
    else {
        panic!("unexpected transport query")
    };
    assert_eq!(resource.generation, 2);
    assert_eq!(resource.queue_len, 0);
    let blocked_epoch = unresolved.epoch;
    runtime
        .command(Command::Output {
            actuator: actuator(),
            command: OutputCommand::RequestSafe,
            at: Duration::from_millis(33),
        })
        .unwrap();
    for at in 34..=44 {
        service(&mut runtime, at);
    }
    let still_unresolved = output(&runtime);
    assert!(still_unresolved.epoch > blocked_epoch);
    assert_eq!(still_unresolved.state, OutputState::FaultLatched);
    assert!(!still_unresolved.safe_confirmed);
    assert_eq!(wire.borrow().started_writes, 1);
    let QueryResult::Transport(resource) =
        runtime.query(Query::Transport(ResourceId::new(1))).unwrap()
    else {
        panic!("unexpected transport query")
    };
    assert_eq!(resource.queue_len, 0);
    assert!(
        runtime
            .command(Command::QueueMetakonOutput {
                actuator: actuator(),
                at: Duration::from_millis(45),
                queue_ttl: Duration::from_millis(50),
                timeout: Duration::from_millis(10),
            })
            .is_err()
    );
    assert_eq!(wire.borrow().started_writes, 1);
    assert_eq!(
        runtime
            .shutdown_transport(ResourceId::new(1), Duration::from_millis(46))
            .unwrap(),
        lab_core::transport::TransportShutdown::Complete
    );
}

#[test]
fn safe_failure_before_send_can_use_the_existing_later_delivery_policy() {
    let (mut runtime, wire) = setup_with_wire(Wire {
        fail_before_first_write: true,
        ..Wire::default()
    });
    runtime
        .command(Command::Output {
            actuator: actuator(),
            command: OutputCommand::RequestSafe,
            at: Duration::ZERO,
        })
        .unwrap();

    service(&mut runtime, 0);
    assert_eq!(wire.borrow().started_writes, 0);
    service(&mut runtime, 1);
    service(&mut runtime, 2);
    service(&mut runtime, 3);

    let settled = output(&runtime);
    assert_eq!(wire.borrow().started_writes, 1);
    assert_eq!(settled.state, OutputState::Disarmed);
    assert!(settled.safe_confirmed);
    assert_eq!(settled.outcome, Some(DispatchOutcome::ReadbackVerified));
}
