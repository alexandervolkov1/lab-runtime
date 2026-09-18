//! M9D host-composition acceptance for the authority-gated Metakon output path.

use lab_core::{
    Command, Query, QueryResult, SampleQuality, SignalId,
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
                vec![5, 0, 6, 1]
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
        Ok(RecoveryStatus::Complete)
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
