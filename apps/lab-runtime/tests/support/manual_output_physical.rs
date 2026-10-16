//! Public API over the existing declarative byte-transport fixture, without a COM port.

use super::*;
use lab_core::{
    Command, InstrumentId, ParameterId,
    output::{ActuatorId, OutputCommand},
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    configuration::{FileArtifactReader, parse_runtime_toml},
    host::{Clock, HostCore},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::Path,
    rc::Rc,
};

#[derive(Default)]
struct Wire {
    bytes: VecDeque<u8>,
    value: u16,
    writes: usize,
}
struct Device(Rc<RefCell<Wire>>);
impl ByteTransport for Device {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        match bytes {
            [b'S', b'E', b'T', b' ', high, low, b'\r', b'\n'] => {
                wire.value = u16::from_be_bytes([*high, *low]);
                wire.writes += 1;
                wire.bytes.extend(b"OK\r\n");
            }
            b"GET\r\n" => {
                let [high, low] = wire.value.to_be_bytes();
                wire.bytes.extend([b'V', b'=', high, low, b'\r', b'\n']);
            }
            _ => return Err(TransportIoError::Other),
        }
        Ok(bytes.len())
    }
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.bytes.len());
        for byte in &mut bytes[..count] {
            *byte = wire.bytes.pop_front().unwrap();
        }
        Ok(count)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Complete)
    }
}

fn fixture() -> (Fixture, Rc<RefCell<Wire>>) {
    let path = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/simple-device/runtime.writable.toml"),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("1001", "1");
    let deployment = parse_runtime_toml(
        text.as_bytes(),
        path.parent().unwrap(),
        &mut FileArtifactReader,
    )
    .unwrap();
    let wire = Rc::new(RefCell::new(Wire::default()));
    let transports = BTreeMap::from([(
        ResourceId::new(7),
        Box::new(Device(wire.clone())) as Box<dyn ByteTransport>,
    )]);
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    host.command(Command::Output {
        actuator: ActuatorId::new(InstrumentId::new(1), ParameterId::new(2)),
        command: OutputCommand::RequestSafe,
        at: Duration::ZERO,
    })
    .unwrap();
    for _ in 0..8 {
        host.command(Command::ServiceSafety { at: Duration::ZERO })
            .unwrap();
    }
    let mut service = ServiceHost::startup_from_trusted_host(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
        host,
    )
    .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
    );
    (
        Fixture {
            service,
            app,
            scope: hello[0]["result"]["scope"].as_str().unwrap().into(),
            seq: 1,
        },
        wire,
    )
}

#[test]
fn physical_acceptance_survives_detach_and_reattach_never_replays_the_write() {
    let (mut fixture, wire) = fixture();
    fixture.acquire();
    let mut args = fixture.args();
    args["value"] = json!(17.3);
    args["proposal_ttl_ns"] = json!("200000000");
    let seq = fixture.seq;
    let replies = fixture.operation("output_write", args.clone());
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(
        wire.borrow().writes,
        1,
        "only startup safe, command still queued"
    );
    fixture.app.detach(&fixture.service, 1);
    let mut terminals = Vec::new();
    for _ in 0..8 {
        let at = fixture.service.clock().now();
        fixture
            .service
            .owner_mut()
            .command(Command::ServiceSafety { at })
            .unwrap();
        terminals.extend(fixture.app.poll_outputs(&mut fixture.service));
    }
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].1["state"], "completed");
    assert_eq!(terminals[0].1["result"]["readback"]["value"], 17.3);
    assert_eq!(wire.borrow().writes, 2);
    let hello = ask(
        &mut fixture.service,
        &mut fixture.app,
        1,
        json!({"v":1,"msg_id":"reattach","op":"hello","args":{"scope":fixture.scope}}),
    );
    assert_eq!(hello[0]["type"], "result");
    let retry = ask(
        &mut fixture.service,
        &mut fixture.app,
        1,
        json!({"v":1,"msg_id":"retry","op":"output_write","args":args,"request_id":{"scope":fixture.scope,"seq":seq.to_string()}}),
    );
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0]["result"], terminals[0].1["result"]);
    assert_eq!(wire.borrow().writes, 2);
    assert_eq!(wire.borrow().value, 173);
}
