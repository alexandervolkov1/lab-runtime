//! Resource-scoped reconnect keeps Recorder FIFO identities contiguous.

use super::*;
use crate::{
    configuration::{ArtifactReader, ConfigurationError, parse_runtime_toml},
    recorder::{RecorderLimits, WriterBarrier},
};
use lab_core::{
    ParameterId,
    metakon::crc,
    transport::{RecoveryStatus, TransportIoError},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
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
display_name="Recorder reconnect ordering"
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
struct Wire {
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
    requests: Vec<Vec<u8>>,
    disconnect_when_empty: bool,
}

struct Transport(Rc<RefCell<Wire>>);
impl ByteTransport for Transport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.requests.push(bytes.to_vec());
        if let Some(response) = wire.responses.pop_front() {
            wire.readable.extend(response);
        }
        Ok(bytes.len())
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        if wire.readable.is_empty() && wire.disconnect_when_empty {
            return Err(TransportIoError::Disconnected);
        }
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
            *byte = wire.readable.pop_front().expect("bounded readable byte");
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Pending)
    }
}

#[derive(Clone, Copy, Default)]
struct Clock(Duration);
impl super::Clock for Clock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn channel_type_response(value: u8) -> Vec<u8> {
    let mut bytes = vec![1, 0, 0, 0, 0x41, value];
    bytes.push(crc(&bytes));
    bytes
}

fn temperature_response(raw: i16) -> Vec<u8> {
    let [low, high] = raw.to_le_bytes();
    let mut bytes = vec![1, 0, 1, 0, 0x44, low, high];
    bytes.push(crc(&bytes));
    bytes
}

fn temporary_database(label: &str) -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-m8-{label}-{suffix}.sqlite"))
}

fn deployment() -> FrozenDeployment {
    parse_runtime_toml(CONFIG, Path::new("C:/bench"), &mut Reader).unwrap()
}

fn host_with_old_wire(old: Rc<RefCell<Wire>>) -> HostCore {
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(Transport(old)));
    HostCore::configured_with_transports(&deployment(), transports).unwrap()
}

fn attach_and_start(host: &mut HostCore, worker: RecorderWorker) {
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !host.recording_activation_committed().unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    host.start_recording("reconnect ordering", Duration::ZERO)
        .unwrap();
    while host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        host.poll_recorder(Duration::ZERO);
        std::thread::yield_now();
    }
}

fn service_until_offline(host: &mut HostCore, clock: &mut Clock) {
    host.begin_configured_probes(clock.0).unwrap();
    for millisecond in (0..=700).step_by(10) {
        clock.0 = Duration::from_millis(millisecond);
        host.service(clock).unwrap();
    }
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    assert_eq!(host.resource_records()[0]["data"]["generation"], "1");
    let QueryResult::Latest(Some(sample)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(11),
            ParameterId::new(2),
        )))
        .unwrap()
    else {
        panic!("disconnect must publish temperature Unavailable")
    };
    assert_eq!(sample.quality(), SampleQuality::Unavailable);
}

fn lifecycle(at: Duration) -> ConfigurationLifecycleRecord {
    ConfigurationLifecycleRecord {
        operation_id: 1,
        operation_kind: "reconnect_resource",
        base_revision: 1,
        committed_revision: 1,
        toml_hash: deployment().toml_hash(),
        affected: vec!["resource:7:binding_generation:2".into()],
        reason: None,
        at,
    }
}

fn reserve_reconnect(host: &mut HostCore, record: &ConfigurationLifecycleRecord) -> Option<u64> {
    host.try_reserve_configuration_activation(record, record.at)
        .unwrap()
        .expect("bounded reservation credit")
}

fn install_replacement(host: &mut HostCore, wire: Rc<RefCell<Wire>>, at: Duration) {
    host.begin_configured_resource_reconnect(ResourceId::new(7))
        .unwrap();
    assert!(
        host.prepare_configured_transport_replacement(ResourceId::new(7), at)
            .unwrap()
    );
    host.rebind_configured_transport(ResourceId::new(7), Box::new(Transport(wire)), at)
        .unwrap();
    assert_eq!(host.configured_binding_generation(11), Some(2));
    host.begin_configured_probes_for_resource(ResourceId::new(7), at)
        .unwrap();
}

fn await_recorder_empty(host: &mut HostCore, at: Duration) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().outstanding_groups != 0 {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        host.poll_recorder(at);
        std::thread::yield_now();
    }
}

fn await_only_activation_reservation(host: &mut HostCore, at: Duration) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().outstanding_groups != 1 {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        host.poll_recorder(at);
        std::thread::yield_now();
    }
    let status = host.recording_status().unwrap();
    assert_eq!(status.outstanding_records, 1);
    assert_eq!(status.state, RecordingState::Recording);
}

fn close_recording(host: &mut HostCore, at: Duration) {
    await_recorder_empty(host, at);
    host.stop_recording_at(at).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Idle {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        host.poll_recorder(at);
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        host.poll_recorder(at);
        std::thread::yield_now();
    }
}

fn remove_database(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(path).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

fn record_sequence(bytes: Vec<u8>) -> u64 {
    u64::from_be_bytes(bytes.try_into().unwrap())
}

#[test]
fn reconnect_probe_fact_precedes_durable_activation_and_later_good_temperature() {
    let path = temporary_database("reconnect-success");
    let old = Rc::new(RefCell::new(Wire {
        responses: VecDeque::from([channel_type_response(3), temperature_response(200)]),
        disconnect_when_empty: true,
        ..Wire::default()
    }));
    let mut host = host_with_old_wire(old);
    attach_and_start(
        &mut host,
        RecorderWorker::open(&path, RecorderLimits::default()).unwrap(),
    );
    let mut clock = Clock::default();
    service_until_offline(&mut host, &mut clock);

    let record = lifecycle(Duration::from_millis(710));
    let reservation = reserve_reconnect(&mut host, &record);
    let replacement = Rc::new(RefCell::new(Wire {
        responses: VecDeque::from([channel_type_response(3), temperature_response(251)]),
        ..Wire::default()
    }));
    install_replacement(&mut host, replacement.clone(), record.at);
    for millisecond in [711, 720, 730, 800, 810] {
        clock.0 = Duration::from_millis(millisecond);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .unwrap()
    );
    assert_eq!(replacement.borrow().requests.len(), 1, "probe only");
    let temperature = SignalId::new(InstrumentId::new(11), ParameterId::new(2));
    let QueryResult::Latest(Some(fenced)) =
        host.query(Query::GetLatestSignal(temperature)).unwrap()
    else {
        panic!("replacement must retain unavailable temperature")
    };
    assert_eq!(fenced.quality(), SampleQuality::Unavailable);
    await_only_activation_reservation(&mut host, clock.0);
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );

    host.commit_reserved_configuration_activation(reservation, record)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !host
        .live_activation_committed(reservation, false, Duration::from_millis(710), clock.0)
        .unwrap()
    {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        std::thread::yield_now();
    }
    host.activate_configured_resource_after_reconnect(ResourceId::new(7), clock.0)
        .unwrap();
    clock.0 = Duration::from_millis(909);
    host.service(&clock).unwrap();
    assert_eq!(replacement.borrow().requests.len(), 1, "no catch-up read");
    for millisecond in [910, 920, 930] {
        clock.0 = Duration::from_millis(millisecond);
        host.service(&clock).unwrap();
    }
    let QueryResult::Latest(Some(good)) = host.query(Query::GetLatestSignal(temperature)).unwrap()
    else {
        panic!("ordinary acquisition must resume after durability")
    };
    assert_eq!(good.quality(), SampleQuality::Good);
    assert_eq!(good.value(), Some(&lab_core::Value::Float(25.1)));
    assert_eq!(host.recording_status().unwrap().coverage, "complete");
    assert_ne!(
        host.recording_status().unwrap().first_error.as_deref(),
        Some("fact record reservation mismatch")
    );
    close_recording(&mut host, Duration::from_millis(940));
    drop(host);

    let db = rusqlite::Connection::open(&path).unwrap();
    let probe = record_sequence(
        db.query_row(
            "SELECT record_seq FROM measurements WHERE parameter_id=?1 AND generation=?2 AND quality='good' ORDER BY record_seq DESC LIMIT 1",
            rusqlite::params![1u64.to_be_bytes().as_slice(), 2u64.to_be_bytes().as_slice()],
            |row| row.get(0),
        )
        .unwrap(),
    );
    let activation = record_sequence(
        db.query_row(
            "SELECT record_seq FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    );
    let temperature = record_sequence(
        db.query_row(
            "SELECT record_seq FROM measurements WHERE parameter_id=?1 AND generation=?2 AND quality='good'",
            rusqlite::params![2u64.to_be_bytes().as_slice(), 2u64.to_be_bytes().as_slice()],
            |row| row.get(0),
        )
        .unwrap(),
    );
    assert!(probe < activation && activation < temperature);
    let sequences: Vec<u64> = {
        let mut statement = db
            .prepare("SELECT record_seq FROM records ORDER BY record_seq")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|value| record_sequence(value.unwrap()))
            .collect()
    };
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    drop(db);
    remove_database(&path);
}

#[test]
fn failed_probe_cancels_after_recorded_fact_without_gap_or_good_temperature() {
    let path = temporary_database("reconnect-bad-probe");
    let old = Rc::new(RefCell::new(Wire {
        responses: VecDeque::from([channel_type_response(3), temperature_response(200)]),
        disconnect_when_empty: true,
        ..Wire::default()
    }));
    let mut host = host_with_old_wire(old);
    attach_and_start(
        &mut host,
        RecorderWorker::open(&path, RecorderLimits::default()).unwrap(),
    );
    let mut clock = Clock::default();
    service_until_offline(&mut host, &mut clock);
    let record = lifecycle(Duration::from_millis(710));
    let reservation = reserve_reconnect(&mut host, &record);
    let replacement = Rc::new(RefCell::new(Wire {
        responses: VecDeque::from([channel_type_response(2), temperature_response(999)]),
        ..Wire::default()
    }));
    install_replacement(&mut host, replacement.clone(), record.at);
    for millisecond in [711, 720, 730, 800, 810] {
        clock.0 = Duration::from_millis(millisecond);
        host.service(&clock).unwrap();
    }
    assert!(
        host.configured_probes_ready_for_resource(ResourceId::new(7))
            .is_err()
    );
    await_only_activation_reservation(&mut host, clock.0);
    host.cancel_configuration_activation(reservation).unwrap();
    assert!(
        host.retire_failed_configured_reconnect(ResourceId::new(7), clock.0)
            .unwrap()
    );
    assert_eq!(host.configured_binding_generation(11), Some(2));
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    assert_eq!(replacement.borrow().requests.len(), 1);
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    assert_eq!(host.recording_status().unwrap().coverage, "complete");
    close_recording(&mut host, Duration::from_millis(820));
    drop(host);

    let db = rusqlite::Connection::open(&path).unwrap();
    let lifecycle_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let unavailable_probe_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM measurements WHERE parameter_id=?1 AND generation=?2 AND quality='unavailable' AND integer_value IS NULL",
            rusqlite::params![1u64.to_be_bytes().as_slice(), 2u64.to_be_bytes().as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    let escaped_temperature: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM measurements WHERE parameter_id=?1 AND generation=?2 AND quality='good'",
            rusqlite::params![2u64.to_be_bytes().as_slice(), 2u64.to_be_bytes().as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle_count, 0);
    assert!(unavailable_probe_count >= 1);
    assert_eq!(escaped_temperature, 0);
    drop(db);
    remove_database(&path);
}

#[test]
fn required_recorder_failure_after_rebind_stays_quiesced_and_never_rolls_generation_back() {
    let path = temporary_database("reconnect-recorder-failure");
    let barrier = WriterBarrier::panic_before_fact_sql();
    let old = Rc::new(RefCell::new(Wire::default()));
    let mut host = host_with_old_wire(old);
    attach_and_start(
        &mut host,
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap(),
    );
    let mut clock = Clock::default();
    let record = lifecycle(Duration::from_millis(10));
    let reservation = reserve_reconnect(&mut host, &record);
    let replacement = Rc::new(RefCell::new(Wire {
        responses: VecDeque::from([channel_type_response(3)]),
        ..Wire::default()
    }));
    install_replacement(&mut host, replacement, record.at);
    for millisecond in [11, 20, 30] {
        clock.0 = Duration::from_millis(millisecond);
        host.service(&clock).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Failed {
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        host.poll_recorder(clock.0);
        std::thread::yield_now();
    }
    assert!(
        host.commit_reserved_configuration_activation(reservation, record)
            .is_err()
    );
    host.configuration_recording_failed(clock.0);
    assert!(
        host.retire_failed_configured_reconnect(ResourceId::new(7), clock.0)
            .unwrap()
    );
    assert_eq!(host.configured_binding_generation(11), Some(2));
    assert!(host.configured_resource_reconnect_quiesced(ResourceId::new(7)));
    assert_eq!(host.resource_records()[0]["data"]["state"], "offline");
    assert_eq!(host.recording_status().unwrap().coverage, "unknown_tail");
    assert_ne!(
        host.recording_status().unwrap().first_error.as_deref(),
        Some("fact record reservation mismatch")
    );
    drop(host);
    remove_database(&path);
}
