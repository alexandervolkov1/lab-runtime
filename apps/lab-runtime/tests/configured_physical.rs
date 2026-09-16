//! C12-C17 software acceptance for configured read-only physical acquisition.

use lab_core::{
    Query, QueryResult, SignalId,
    metakon::crc,
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    configuration::{ArtifactReader, ConfigurationError, load_runtime_toml, parse_runtime_toml},
    deployment::{DeploymentLifecycle, DiffEffect},
    host::{Clock, HostCore},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::Path,
    rc::Rc,
    time::Duration,
};

const DEFINITION: &[u8] = br#"{
 "schema_version":1,"profile":"metakon-5x3-v1","id":11,"name":"Metakon",
 "parameters":[
  {"id":1,"name":"channel_type","value_type":"integer","unit":{"id":"1","symbol":"1"},"min":0,"max":255,"role":"diagnostic","access":"read_only","operation":"channel_type","scale":1,"write_effect":"none"},
  {"id":2,"name":"temperature","value_type":"float","unit":{"id":"degC","symbol":"C"},"min":-99.9,"max":999.9,"role":"measurement","access":"read_only","operation":"temperature","scale":0.1,"write_effect":"none"}
 ]}"#;
const CONFIG: &[u8] = br#"schema_version=1
[runtime]
key="bench"
display_name="Bench"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com_read_only"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
id=11
key="temperature"
kind="metakon"
definition="metakon.json"
resource_id=7
address=1
poll_period_ms=100
queue_timeout_ms=50
transaction_timeout_ms=50
"#;

struct Reader;
impl ArtifactReader for Reader {
    fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
        Ok(DEFINITION.to_vec())
    }
}

#[derive(Default)]
struct ScriptedTransport {
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
    writes: Vec<Vec<u8>>,
}
impl ByteTransport for ScriptedTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        self.writes.push(bytes.to_vec());
        if let Some(response) = self.responses.pop_front() {
            self.readable.extend(response);
        }
        Ok(bytes.len())
    }
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let count = bytes.len().min(self.readable.len());
        for byte in &mut bytes[..count] {
            *byte = self.readable.pop_front().unwrap();
        }
        Ok(count)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Complete)
    }
}

#[derive(Default)]
struct DisconnectWire {
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
    writes: usize,
    requests: Vec<Vec<u8>>,
    fail_reads_when_empty: bool,
}

struct DisconnectingTransport(Rc<RefCell<DisconnectWire>>);

impl ByteTransport for DisconnectingTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.writes += 1;
        wire.requests.push(bytes.to_vec());
        if let Some(response) = wire.responses.pop_front() {
            wire.readable.extend(response);
        }
        Ok(bytes.len())
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        if wire.readable.is_empty() && wire.fail_reads_when_empty {
            return Err(TransportIoError::Disconnected);
        }
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
            *byte = wire.readable.pop_front().unwrap();
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Pending)
    }
}

#[derive(Default)]
struct TestClock(Duration);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn temperature_response(raw: i16) -> Vec<u8> {
    let [low, high] = raw.to_le_bytes();
    let mut bytes = vec![1, 0, 1, 0, 0x44, low, high];
    bytes.push(crc(&bytes));
    bytes
}

fn channel_type_response() -> Vec<u8> {
    let mut bytes = vec![1, 0, 0, 0, 0x41, 3];
    bytes.push(crc(&bytes));
    bytes
}

fn response_for(address: u8, flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![address, 0, (payload.len() - 1) as u8, 0, flag];
    bytes.extend_from_slice(payload);
    bytes.push(crc(&bytes));
    bytes
}

struct MultiReader;
impl ArtifactReader for MultiReader {
    fn read(&mut self, path: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
        let id = if path.to_string_lossy().contains("metakon-12") {
            12
        } else {
            11
        };
        Ok(std::str::from_utf8(DEFINITION)
            .unwrap()
            .replacen("\"id\":11", &format!("\"id\":{id}"), 1)
            .into_bytes())
    }
}

fn two_resource_config() -> Vec<u8> {
    let mut text = std::str::from_utf8(CONFIG).unwrap().to_owned();
    text.push_str(
        r#"
[[resources]]
id=8
key="second-bus"
kind="windows_com_read_only"
port="COM4"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
id=12
key="second-temperature"
kind="metakon"
definition="metakon-12.json"
resource_id=8
address=2
poll_period_ms=100
queue_timeout_ms=50
transaction_timeout_ms=50
"#,
    );
    text.into_bytes()
}

#[test]
fn c16_metakon_513_thermocouple_scale_preserves_raw_degree_values() {
    let path = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/runtime.metakon-513-com5.toml"),
    )
    .unwrap();
    let deployment = load_runtime_toml(&path).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(1),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([
                response_for(5, 0x41, &[3]),
                response_for(5, 0x44, &23i16.to_le_bytes()),
                response_for(5, 0x44, &24i16.to_le_bytes()),
            ]),
            ..ScriptedTransport::default()
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in [0, 10, 20, 30] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());
    let signal = SignalId::new(
        lab_core::InstrumentId::new(1),
        lab_core::ParameterId::new(2),
    );
    let QueryResult::Latest(Some(first)) = host.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!("first real-device-profile temperature missing")
    };
    assert_eq!(first.value(), Some(&lab_core::Value::Float(23.0)));

    for milliseconds in [1000, 1010, 1020] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    let QueryResult::Latest(Some(second)) = host.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!("second real-device-profile temperature missing")
    };
    assert_eq!(second.value(), Some(&lab_core::Value::Float(24.0)));
}

#[test]
fn c12_c17_configured_metakon_publishes_normal_signal_with_binding_identity() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([channel_type_response(), temperature_response(234)]),
            ..ScriptedTransport::default()
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in [0, 10, 20, 30] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());

    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    let Some(lab_core::Value::Float(value)) = sample.value() else {
        panic!()
    };
    assert!((value - 23.4).abs() < 1.0e-9);
    assert_eq!(host.resource_records()[0]["target"]["id"], "7");
}

#[test]
fn c2_missing_configured_transport_rejects_atomically() {
    let mut reader = Reader;
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut reader).unwrap();
    assert!(HostCore::configured_with_transports(&deployment, BTreeMap::new()).is_err());
}

#[test]
fn c14_c15_explicit_rebind_keeps_logical_id_and_fences_old_measurement() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([channel_type_response(), temperature_response(200)]),
            ..ScriptedTransport::default()
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in [0, 10, 20, 30] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());

    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([channel_type_response(), temperature_response(251)]),
            ..ScriptedTransport::default()
        }),
        Duration::from_millis(40),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(2));
    let QueryResult::Latest(Some(fenced)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_ne!(fenced.quality(), lab_core::SampleQuality::Good);

    host.begin_configured_probes(Duration::from_millis(40))
        .unwrap();
    for milliseconds in [41, 50, 60, 100, 110, 120] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    let Some(lab_core::Value::Float(value)) = sample.value() else {
        panic!()
    };
    assert!((value - 25.1).abs() < 1.0e-9);
    assert_eq!(host.resource_records()[0]["target"]["id"], "7");
}

#[test]
fn c14_periodic_reads_do_not_fill_queue_while_finite_recovery_reaches_offline() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let wire = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(280)]),
        fail_reads_when_empty: true,
        ..DisconnectWire::default()
    }));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(wire.clone())),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();

    let mut saw_recovery = false;
    let mut recovery_at = None;
    let mut offline_at = None;
    for milliseconds in (0..=700).step_by(10) {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
        let record = &host.resource_records()[0]["data"];
        if record["state"] == "recovering" || record["state"] == "offline" {
            saw_recovery = true;
            assert_eq!(record["queue_len"], 0);
        }
        if record["state"] == "recovering" && recovery_at.is_none() {
            recovery_at = Some(milliseconds);
        }
        if record["state"] == "offline" && offline_at.is_none() {
            offline_at = Some(milliseconds);
        }
    }

    assert!(saw_recovery);
    assert_eq!(offline_at.unwrap() - recovery_at.unwrap(), 500);
    let record = &host.resource_records()[0]["data"];
    assert_eq!(record["state"], "offline");
    assert_eq!(record["generation"], "1");
    assert_eq!(wire.borrow().writes, 3);
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.quality(), lab_core::SampleQuality::Unavailable);
    assert_eq!(sample.value(), None);
}

#[test]
fn c14_explicit_rebind_after_offline_fences_old_session_and_resumes_new_generation() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let old_wire = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(280)]),
        fail_reads_when_empty: true,
        ..DisconnectWire::default()
    }));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(old_wire.clone())),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in (0..=700).step_by(10) {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    assert_eq!(host.configured_binding_generation(11), Some(1));

    old_wire
        .borrow_mut()
        .readable
        .extend(temperature_response(999));
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([channel_type_response(), temperature_response(251)]),
            ..ScriptedTransport::default()
        }),
        Duration::from_millis(710),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(2));
    host.begin_configured_probes(Duration::from_millis(710))
        .unwrap();
    for milliseconds in [711, 720, 730, 800, 810] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }

    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(25.1)));
    assert_eq!(sample.quality(), lab_core::SampleQuality::Good);
    assert_eq!(host.resource_records()[0]["target"]["id"], "7");
}

#[test]
fn c14_reconnect_replacement_stays_quiesced_until_probe_and_lifecycle_activation() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([channel_type_response(), temperature_response(200)]),
            ..ScriptedTransport::default()
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in [0, 10, 20, 30] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());

    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    let replacement = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(251)]),
        ..DisconnectWire::default()
    }));
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(replacement.clone())),
        Duration::from_millis(40),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(2));
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(40))
        .unwrap();
    assert_eq!(
        host.configured_probes_ready_for_resource(ResourceId::new(7)),
        Ok(false),
        "the rebind fence is a probe baseline, not an immediate probe failure"
    );

    for milliseconds in [41, 50, 60, 100, 110, 120, 200] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .unwrap()
    );
    assert_eq!(replacement.borrow().writes, 1, "only the probe may run");
    let temperature = SignalId::new(
        lab_core::InstrumentId::new(11),
        lab_core::ParameterId::new(2),
    );
    let QueryResult::Latest(Some(fenced)) =
        host.query(Query::GetLatestSignal(temperature)).unwrap()
    else {
        panic!("rebind must leave an explicit unavailable observation")
    };
    assert_eq!(fenced.quality(), lab_core::SampleQuality::Unavailable);
    assert_eq!(fenced.value(), None);

    host.activate_configured_resource_after_reconnect(
        ResourceId::new(7),
        Duration::from_millis(200),
    )
    .unwrap();
    assert!(!host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    clock.0 = Duration::from_millis(299);
    host.service(&clock).unwrap();
    assert_eq!(replacement.borrow().writes, 1);
    for milliseconds in [300, 310, 320] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert_eq!(replacement.borrow().writes, 2);
    let QueryResult::Latest(Some(sample)) =
        host.query(Query::GetLatestSignal(temperature)).unwrap()
    else {
        panic!("ordinary acquisition did not resume after activation")
    };
    assert_eq!(sample.quality(), lab_core::SampleQuality::Good);
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(25.1)));
}

#[test]
fn c14_failed_post_install_probe_keeps_new_generation_quiesced_and_offline() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(ScriptedTransport::default()));
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );

    let wrong_probe = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([response_for(1, 0x41, &[2]), temperature_response(999)]),
        ..DisconnectWire::default()
    }));
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(wrong_probe.clone())),
        Duration::from_millis(10),
    )
    .unwrap();
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(10))
        .unwrap();
    let mut clock = TestClock::default();
    for milliseconds in [11, 20, 30, 100, 200] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .is_err()
    );
    assert!(
        host.activate_configured_resource_after_reconnect(
            ResourceId::new(7),
            Duration::from_millis(200),
        )
        .is_err(),
        "failed compatibility cannot create an acquisition-ready resource"
    );
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert_eq!(
        wrong_probe.borrow().writes,
        1,
        "temperature must stay gated"
    );
    assert_eq!(host.configured_binding_generation(11), Some(2));
    assert!(
        host.retire_failed_configured_reconnect(ResourceId::new(7), clock.now())
            .unwrap()
    );
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    clock.0 = Duration::from_millis(1000);
    host.service(&clock).unwrap();
    assert_eq!(wrong_probe.borrow().writes, 1);
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!("failed replacement must retain fenced temperature")
    };
    assert_eq!(sample.quality(), lab_core::SampleQuality::Unavailable);
    assert_eq!(sample.value(), None);

    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    let next = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(252)]),
        ..DisconnectWire::default()
    }));
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(next.clone())),
        Duration::from_millis(1010),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(3));
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(1010))
        .unwrap();
    for milliseconds in [1011, 1020, 1030] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .unwrap()
    );
    assert_eq!(next.borrow().writes, 1);
}

#[test]
fn c14_probe_timeout_never_releases_ordinary_reconnect_acquisition() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(ScriptedTransport::default()));
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    let silent = Rc::new(RefCell::new(DisconnectWire::default()));
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(silent.clone())),
        Duration::from_millis(10),
    )
    .unwrap();
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(10))
        .unwrap();
    let mut clock = TestClock::default();
    for milliseconds in (11..=700).step_by(10) {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .is_err()
    );
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert!(
        silent.borrow().writes <= 2,
        "only the bounded probe retry is allowed"
    );
    assert_eq!(host.configured_binding_generation(11), Some(2));
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!("probe timeout must preserve an unavailable temperature")
    };
    assert_eq!(sample.quality(), lab_core::SampleQuality::Unavailable);
    assert_eq!(sample.value(), None);
}

#[test]
fn c14_reconnect_probe_is_resource_scoped_while_unrelated_resource_continues() {
    let config = two_resource_config();
    let deployment = parse_runtime_toml(&config, Path::new("C:/bench"), &mut MultiReader).unwrap();
    let first = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(200)]),
        ..DisconnectWire::default()
    }));
    let second = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([
            response_for(2, 0x41, &[3]),
            response_for(2, 0x44, &210i16.to_le_bytes()),
            response_for(2, 0x44, &211i16.to_le_bytes()),
        ]),
        ..DisconnectWire::default()
    }));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(first.clone())),
    );
    transports.insert(
        ResourceId::new(8),
        Box::new(DisconnectingTransport(second.clone())),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let mut clock = TestClock::default();
    host.begin_configured_probes(clock.now()).unwrap();
    for milliseconds in [0, 10, 20, 30] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());

    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    let replacement = Rc::new(RefCell::new(DisconnectWire {
        responses: VecDeque::from([channel_type_response(), temperature_response(999)]),
        ..DisconnectWire::default()
    }));
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(DisconnectingTransport(replacement.clone())),
        Duration::from_millis(40),
    )
    .unwrap();
    first.borrow_mut().requests.clear();
    second.borrow_mut().requests.clear();
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(40))
        .unwrap();
    for milliseconds in [41, 50, 60, 100, 110, 120] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }

    let replacement = replacement.borrow();
    assert_eq!(replacement.requests.len(), 1);
    assert_eq!(
        replacement.requests[0][2], 0,
        "target receives only register 0 probe"
    );
    let second = second.borrow();
    assert!(
        second.requests.iter().any(|request| request[2] == 1),
        "unrelated resource must continue ordinary temperature acquisition"
    );
    assert!(
        second.requests.iter().all(|request| request[2] == 1),
        "targeted reconnect must not reprobe an unrelated resource"
    );
}

#[test]
fn c19_shutdown_during_reconnect_probe_fences_work_and_closes_finitely() {
    let deployment = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(ScriptedTransport::default()));
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7))
            .unwrap()
    );
    host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(ScriptedTransport::default()),
        Duration::from_millis(10),
    )
    .unwrap();
    host.begin_configured_probes_for_resource(ResourceId::new(7), Duration::from_millis(10))
        .unwrap();
    let mut clock = TestClock(Duration::from_millis(11));
    host.service(&clock).unwrap();

    host.begin_shutdown(&clock).unwrap();
    for milliseconds in [12, 20, 30, 40] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    let status = host.shutdown_status();
    assert!(status.transports_closed);
    assert_eq!(status.unfinished_transports, 0);
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
}

#[test]
fn c6_configuration_rebind_replaces_address_definition_and_mapping_revision() {
    let active = parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap();
    let candidate_toml = std::str::from_utf8(CONFIG)
        .unwrap()
        .replace("address=1", "address=2");
    struct CandidateReader;
    impl ArtifactReader for CandidateReader {
        fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
            Ok(std::str::from_utf8(DEFINITION)
                .unwrap()
                .replace("\"scale\":0.1", "\"scale\":0.2")
                .into_bytes())
        }
    }
    let candidate = parse_runtime_toml(
        candidate_toml.as_bytes(),
        Path::new("C:/bench"),
        &mut CandidateReader,
    )
    .unwrap();
    let mut lifecycle = DeploymentLifecycle::new(active.clone());
    let staged = lifecycle.stage(candidate.clone(), Duration::ZERO).unwrap();
    assert!(
        staged
            .diff()
            .effects()
            .contains(&DiffEffect::TransportRebind)
    );
    assert!(staged.diff().requires_safe_barrier());
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(ScriptedTransport::default()));
    let mut host = HostCore::configured_with_transports(&active, transports).unwrap();

    host.rebind_configured_transport_from_configuration(
        &candidate,
        ResourceId::new(7),
        Box::new(ScriptedTransport {
            responses: VecDeque::from([
                response_for(2, 0x41, &[3]),
                response_for(2, 0x44, &100i16.to_le_bytes()),
            ]),
            ..ScriptedTransport::default()
        }),
        Duration::from_millis(1),
    )
    .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(2));
    host.begin_configured_probes(Duration::from_millis(1))
        .unwrap();
    let mut clock = TestClock::default();
    for milliseconds in [2, 10, 20, 100, 110, 120] {
        clock.0 = Duration::from_millis(milliseconds);
        host.service(&clock).unwrap();
    }
    assert!(host.configured_probes_ready().unwrap());
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(20.0)));
}
