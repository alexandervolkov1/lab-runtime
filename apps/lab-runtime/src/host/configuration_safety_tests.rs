//! Configuration safe barriers must use actual physical transport evidence.

use super::*;
use crate::configuration::{FileArtifactReader, load_runtime_toml, parse_runtime_toml};
use lab_core::transport::{ByteTransport, RecoveryStatus, TransportIoError};
use std::{cell::RefCell, collections::VecDeque, path::Path, rc::Rc};

#[derive(Default)]
struct Wire {
    frames: Vec<Vec<u8>>,
    readable: VecDeque<u8>,
    value: u16,
    suppress_ack: bool,
}

struct Device(Rc<RefCell<Wire>>);

impl ByteTransport for Device {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.frames.push(bytes.to_vec());
        let mut response = match bytes {
            [b'S', b'E', b'T', b' ', high, low, b'\r', b'\n'] => {
                wire.value = u16::from_be_bytes([*high, *low]);
                if wire.suppress_ack {
                    Vec::new()
                } else {
                    b"OK\r\n".to_vec()
                }
            }
            b"GET\r\n" => {
                let [high, low] = wire.value.to_be_bytes();
                vec![b'V', b'=', high, low, b'\r', b'\n']
            }
            [5, 0, 6, 1, 2, value, _] => {
                wire.value = u16::from(*value);
                if wire.suppress_ack {
                    Vec::new()
                } else {
                    vec![5, 0, 6, 1]
                }
            }
            [5, 0, 6, 0, _] => vec![5, 0, 6, 0, 0xc2, wire.value as u8],
            _ => return Err(TransportIoError::Other),
        };
        if response.first() == Some(&5) {
            response.push(lab_core::metakon::crc(&response));
        }
        wire.readable.extend(response);
        Ok(bytes.len())
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
            *byte = wire.readable.pop_front().unwrap();
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Complete)
    }
}

fn fixture(
    simple: bool,
) -> (
    HostCore,
    FrozenDeployment,
    FrozenDeployment,
    ActuatorId,
    Rc<RefCell<Wire>>,
) {
    let relative = if simple {
        "examples/simple-device/runtime.writable.toml"
    } else {
        "test-data/fixtures/metakon/runtime.metakon-513-com5-output.toml"
    };
    let path = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative),
    )
    .unwrap();
    let active = load_runtime_toml(&path).unwrap();
    let original = std::fs::read_to_string(&path).unwrap();
    let replacement = if simple {
        original.replace("safe_value = 0.0", "safe_value = 10.0")
    } else {
        // Metakon's declared protocol requires safe zero. Changing the finite
        // lease policy still invalidates prior safe evidence on profile bind.
        original.replace("max_lease_ms = 3000", "max_lease_ms = 4000")
    };
    assert_ne!(original, replacement);
    let candidate = parse_runtime_toml(
        replacement.as_bytes(),
        path.parent().unwrap(),
        &mut FileArtifactReader,
    )
    .unwrap();
    let safe = &active.effective().dto.safe_profiles[0];
    let actuator = ActuatorId::new(
        InstrumentId::new(safe.instrument_id),
        lab_core::ParameterId::new(safe.parameter_id),
    );
    let wire = Rc::new(RefCell::new(Wire::default()));
    let resource = ResourceId::new(active.effective().dto.resources[0].id);
    let transports = BTreeMap::from([(
        resource,
        Box::new(Device(wire.clone())) as Box<dyn ByteTransport>,
    )]);
    let host = HostCore::configured_with_transports(&active, transports).unwrap();
    (host, active, candidate, actuator, wire)
}

fn snapshot(host: &HostCore, actuator: ActuatorId) -> lab_core::output::OutputSnapshot {
    let QueryResult::Output(snapshot) = host.query(Query::Output(actuator)).unwrap() else {
        panic!("output query")
    };
    snapshot
}

fn progress(host: &mut HostCore, at: &mut Duration, turns: usize) {
    for _ in 0..turns {
        *at += Duration::from_millis(10);
        host.command(Command::ServiceSafety { at: *at }).unwrap();
    }
}

#[test]
fn simple_configuration_barrier_needs_real_ack_and_readback() {
    let (mut host, _, _, actuator, wire) = fixture(true);
    assert!(
        !host
            .enter_configuration_safe_barrier(Duration::ZERO)
            .unwrap()
    );
    assert!(!snapshot(&host, actuator).safe_confirmed);
    assert!(snapshot(&host, actuator).readback.is_none());
    let mut at = Duration::ZERO;
    progress(&mut host, &mut at, 20);
    assert!(host.enter_configuration_safe_barrier(at).unwrap());
    assert!(
        wire.borrow()
            .frames
            .iter()
            .any(|frame| frame.starts_with(b"SET "))
    );
    assert!(wire.borrow().frames.iter().any(|frame| frame == b"GET\r\n"));
    assert_eq!(snapshot(&host, actuator).readback.unwrap().value, 0.0);
}

#[test]
fn physical_profile_replacement_does_not_fabricate_safe_confirmation() {
    for simple in [true, false] {
        let (mut host, active, candidate, actuator, wire) = fixture(simple);
        host.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
        let mut at = Duration::ZERO;
        progress(&mut host, &mut at, 20);
        assert!(snapshot(&host, actuator).safe_confirmed);
        wire.borrow_mut().suppress_ack = true;
        host.apply_configuration(&active, &candidate, at, 2)
            .unwrap();
        assert!(
            !snapshot(&host, actuator).safe_confirmed,
            "physical={simple}"
        );
        progress(&mut host, &mut at, 400);
        let state = snapshot(&host, actuator);
        assert!(!state.safe_confirmed);
        assert!(state.lease.is_none());
        assert!(state.readback.is_none(), "no new readback was received");
        assert_eq!(wire.borrow().value, if simple { 100 } else { 0 });
        let writes = wire
            .borrow()
            .frames
            .iter()
            .filter(|frame| frame.starts_with(b"SET ") || frame.starts_with(&[5, 0, 6, 1]))
            .count();
        assert_eq!(
            writes, 2,
            "initial safe plus one new safe; lost ACK is never retried"
        );
    }
}
