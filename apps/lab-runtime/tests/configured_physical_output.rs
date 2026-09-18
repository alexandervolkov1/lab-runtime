//! M9D host-composition acceptance for the authority-gated Metakon output path.

use lab_core::{
    Command, Query, QueryResult, SampleQuality, SignalId,
    control::{ControllerId, ControllerState},
    metakon::crc,
    output::{ActuatorId, DispatchOutcome, OutputState},
    reference::{ReferenceConfig, ReferenceId},
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    configuration::load_runtime_toml,
    host::{Clock, HostCore},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::Path,
    rc::Rc,
    time::Duration,
};

#[derive(Default)]
struct Wire {
    readable: VecDeque<u8>,
    frames: Vec<Vec<u8>>,
    output: i8,
    suppress_write_acks: usize,
    recovery_fails: bool,
}

struct Device(Rc<RefCell<Wire>>);

impl ByteTransport for Device {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.frames.push(bytes.to_vec());
        let mut response = match bytes {
            [5, 0, 0, 0, _] => vec![5, 0, 0, 0, 0x41, 3],
            [5, 0, 1, 0, _] => vec![5, 0, 1, 0, 0x44, 20, 0],
            [5, 0, 6, 1, 2, raw, _] => {
                wire.output = *raw as i8;
                if wire.suppress_write_acks > 0 {
                    wire.suppress_write_acks -= 1;
                    Vec::new()
                } else {
                    vec![5, 0, 6, 1]
                }
            }
            [5, 0, 6, 0, _] => vec![5, 0, 6, 0, 0xc2, wire.output as u8],
            _ => Vec::new(),
        };
        if !response.is_empty() {
            let checksum = crc(&response);
            response.push(checksum);
            wire.readable.extend(response);
        }
        Ok(bytes.len())
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
        if self.0.borrow().recovery_fails {
            Err(TransportIoError::Disconnected)
        } else {
            Ok(RecoveryStatus::Complete)
        }
    }
}

#[derive(Default)]
struct TestClock(Duration);

impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn output(host: &HostCore) -> lab_core::output::OutputSnapshot {
    let QueryResult::Output(snapshot) = host
        .query(Query::Output(ActuatorId::new(
            lab_core::InstrumentId::new(1),
            lab_core::ParameterId::new(3),
        )))
        .unwrap()
    else {
        panic!("unexpected output query")
    };
    snapshot
}

fn controller(host: &HostCore) -> lab_core::control::ControllerSnapshot {
    let QueryResult::Controller(snapshot) =
        host.query(Query::Controller(ControllerId::new(1))).unwrap()
    else {
        panic!("unexpected controller query")
    };
    snapshot
}

fn output_writes(wire: &Wire) -> impl Iterator<Item = i8> + '_ {
    wire.frames
        .iter()
        .filter_map(|frame| match frame.as_slice() {
            [5, 0, 6, 1, 2, raw, _] => Some(*raw as i8),
            _ => None,
        })
}

fn advance(host: &mut HostCore, clock: &mut TestClock, milliseconds: u64) {
    for _ in 0..milliseconds / 10 {
        clock.0 += Duration::from_millis(10);
        host.service(clock).unwrap();
    }
}

#[test]
fn configured_controller_reaches_physical_output_and_safe_zero_through_one_path() {
    let path = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/runtime.metakon-513-com5-output.toml"),
    )
    .unwrap();
    let deployment = load_runtime_toml(&path).unwrap();
    let wire = Rc::new(RefCell::new(Wire::default()));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(1), Box::new(Device(wire.clone())));
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();

    assert_eq!(output(&host).state, OutputState::Unverified);
    host.begin_configured_probes(clock.now()).unwrap();
    advance(&mut host, &mut clock, 100);
    assert!(host.configured_probes_ready().unwrap());
    host.request_configured_physical_safe(clock.now()).unwrap();
    advance(&mut host, &mut clock, 100);
    assert!(host.configured_physical_outputs_safe().unwrap());
    host.prepare_configured_physical_controllers().unwrap();

    advance(&mut host, &mut clock, 1_100);
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(1),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!("physical temperature missing")
    };
    assert_eq!(sample.quality(), SampleQuality::Good);
    host.command(Command::ReconfigureReference {
        reference: ReferenceId::new(1),
        config: ReferenceConfig::Fixed {
            id: ReferenceId::new(1),
            value: 40.0,
            unit: lab_core::Unit::CELSIUS,
        },
        expected_revision: 1,
    })
    .unwrap();
    host.command(Command::StartController {
        controller: lab_core::control::ControllerId::new(1),
        at: clock.now(),
    })
    .unwrap();

    // The controller slot at 2 s runs before the queued 2 s serial response is
    // committed. The following 3 s slot consumes that new physical sample.
    advance(&mut host, &mut clock, 2_200);
    let delivered = output(&host);
    assert_eq!(delivered.outcome, Some(DispatchOutcome::ReadbackVerified));
    assert_eq!(delivered.readback.unwrap().value, 10.0);
    assert_eq!(wire.borrow().output, 10);

    host.command(Command::PauseController {
        controller: lab_core::control::ControllerId::new(1),
        at: clock.now(),
    })
    .unwrap();
    advance(&mut host, &mut clock, 100);
    let safe = output(&host);
    assert_eq!(safe.state, OutputState::Disarmed);
    assert!(safe.safe_confirmed);
    assert_eq!(safe.readback.unwrap().value, 0.0);
    assert_eq!(wire.borrow().output, 0);
}

#[test]
fn reconnect_and_fresh_input_do_not_rearm_a_controller_after_ambiguous_output() {
    let path = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/runtime.metakon-513-com5-output.toml"),
    )
    .unwrap();
    let deployment = load_runtime_toml(&path).unwrap();
    let old_wire = Rc::new(RefCell::new(Wire::default()));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(1), Box::new(Device(old_wire.clone())));
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();

    host.begin_configured_probes(clock.now()).unwrap();
    advance(&mut host, &mut clock, 100);
    host.request_configured_physical_safe(clock.now()).unwrap();
    advance(&mut host, &mut clock, 100);
    host.prepare_configured_physical_controllers().unwrap();
    advance(&mut host, &mut clock, 1_100);
    host.command(Command::ReconfigureReference {
        reference: ReferenceId::new(1),
        config: ReferenceConfig::Fixed {
            id: ReferenceId::new(1),
            value: 40.0,
            unit: lab_core::Unit::CELSIUS,
        },
        expected_revision: 1,
    })
    .unwrap();
    host.command(Command::StartController {
        controller: ControllerId::new(1),
        at: clock.now(),
    })
    .unwrap();
    advance(&mut host, &mut clock, 2_200);
    let old_lease = output(&host)
        .lease
        .expect("controller owns physical output");
    assert_eq!(old_wire.borrow().output, 10);

    // Model a WRITE that reached the device, followed by lost ACK and a failed
    // transport recovery. This is resource loss after send-started, not proof
    // that the nonzero write did or did not take physical effect.
    {
        let mut wire = old_wire.borrow_mut();
        wire.suppress_write_acks = 1;
        wire.recovery_fails = true;
    }
    advance(&mut host, &mut clock, 2_100);

    let faulted = output(&host);
    assert_eq!(faulted.state, OutputState::SafePending);
    assert!(faulted.fault_latched);
    assert!(!faulted.safe_confirmed);
    assert!(faulted.lease.is_none());
    assert_eq!(controller(&host).state, ControllerState::Failed);
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    let old_nonzero_count = output_writes(&old_wire.borrow())
        .filter(|value| *value != 0)
        .count();

    host.begin_configured_resource_reconnect(ResourceId::new(1))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(1), clock.now())
            .unwrap()
    );
    let replacement = Rc::new(RefCell::new(Wire::default()));
    host.rebind_configured_transport(
        ResourceId::new(1),
        Box::new(Device(replacement.clone())),
        clock.now(),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(1), Some(2));
    host.begin_configured_probes_for_resource(ResourceId::new(1), clock.now())
        .unwrap();
    advance(&mut host, &mut clock, 100);
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(1))
            .unwrap()
    );
    host.request_configured_physical_safe(clock.now()).unwrap();
    advance(&mut host, &mut clock, 100);
    assert!(host.configured_physical_outputs_safe().unwrap());
    host.activate_configured_resource_after_reconnect(ResourceId::new(1), clock.now())
        .unwrap();

    // Recovered transport, verified safe-zero and fresh Good input are all
    // necessary evidence, but none of them is authority to restart control.
    advance(&mut host, &mut clock, 2_200);
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(1),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!("replacement physical temperature missing")
    };
    assert_eq!(sample.quality(), SampleQuality::Good);
    assert_eq!(controller(&host).state, ControllerState::Failed);
    let recovered = output(&host);
    assert_eq!(recovered.state, OutputState::Disarmed);
    assert!(recovered.safe_confirmed);
    assert!(recovered.lease.is_none());
    assert!(output_writes(&replacement.borrow()).all(|value| value == 0));
    assert_eq!(
        output_writes(&old_wire.borrow())
            .filter(|value| *value != 0)
            .count(),
        old_nonzero_count,
        "the ambiguous ordinary WRITE is never resent on the retired generation"
    );

    host.command(Command::ResetFailedController {
        controller: ControllerId::new(1),
        at: clock.now(),
    })
    .unwrap();
    assert_eq!(controller(&host).state, ControllerState::Paused);
    host.command(Command::ResumeController {
        controller: ControllerId::new(1),
        at: clock.now(),
    })
    .unwrap();
    advance(&mut host, &mut clock, 2_200);

    let rearmed = output(&host);
    let fresh_lease = rearmed
        .lease
        .expect("explicit recovery reacquires authority");
    assert_ne!(fresh_lease.instance(), old_lease.instance());
    assert_eq!(controller(&host).state, ControllerState::Running);
    assert!(
        output_writes(&replacement.borrow()).any(|value| value != 0),
        "only explicit reset_failed + resume may produce a fresh nonzero output"
    );
}
