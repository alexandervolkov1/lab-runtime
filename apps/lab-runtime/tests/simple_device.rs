//! M16.2 acceptance for persistent read-only declarative simple devices.

use lab_core::{
    InstrumentId, ParameterId, Query, QueryResult, SampleQuality, SignalId, Value,
    simple_device::{SimpleChecksum, SimpleScalarEncoding},
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    application::Application,
    configuration::{ArtifactReader, ConfigurationError, parse_runtime_toml},
    host::{Clock, HostCore},
    recorder::RecordingState,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

mod support;

const DEFINITION: &[u8] = br#"{
  "format_version": 1,
  "definition_id": "simple-furnace",
  "definition_version": 1,
  "parameters": [{
    "parameter_id": 1,
    "key": "temperature",
    "display_name": "Temperature",
    "role": "measurement",
    "access": "read_only",
    "unit_id": "degC",
    "unit_symbol": "C",
    "value_type": "float",
    "engineering_min": -50.0,
    "engineering_max": 500.0,
    "write_effect": "none",
    "encoding": {"raw": "i16_be", "scale": 0.1, "offset": 0.0},
    "read": {
      "request": {"segments": [
        {"type": "literal", "hex": "10"},
        {"type": "instance_field", "field": "address", "encoding": "u8"},
        {"type": "instance_field", "field": "channel", "encoding": "u8"},
        {"type": "checksum", "algorithm": "crc16_modbus"}
      ]},
      "response": {
        "exact_length": 7,
        "matches": [
          {"type": "literal_match", "offset": 0, "hex": "10"},
          {"type": "instance_match", "offset": 1, "field": "address", "encoding": "u8"},
          {"type": "instance_match", "offset": 2, "field": "channel", "encoding": "u8"}
        ],
        "extract": {"type": "scalar_extract", "offset": 3},
        "checksum": {"type": "checksum", "offset": 5, "algorithm": "crc16_modbus"}
      }
    }
  }]
}"#;

fn config(second_instance: bool) -> Vec<u8> {
    let second = if second_instance {
        r#"
[[instruments]]
kind="simple_device"
id=1002
key="furnace-2"
display_name="Furnace 2"
definition="definitions/simple-furnace-v1.json"
resource_id=7
address=2
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#
    } else {
        ""
    };
    format!(
        r#"schema_version=1
[runtime]
key="simple-bench"
display_name="Simple bench"
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
kind="simple_device"
id=1001
key="furnace-1"
display_name="Furnace 1"
definition="definitions/simple-furnace-v1.json"
resource_id=7
address=1
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
{second}"#
    )
    .into_bytes()
}

#[derive(Default)]
struct Reader {
    reads: usize,
}

impl ArtifactReader for Reader {
    fn read(&mut self, _: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
        self.reads += 1;
        assert_eq!(maximum, 8_192);
        Ok(DEFINITION.to_vec())
    }
}

#[derive(Default)]
struct Wire {
    writes: Vec<Vec<u8>>,
    readable: VecDeque<u8>,
}

struct ScriptedSimpleTransport(Rc<RefCell<Wire>>);

impl ByteTransport for ScriptedSimpleTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.writes.push(bytes.to_vec());
        let address = bytes.get(1).copied().ok_or(TransportIoError::Other)?;
        let channel = bytes.get(2).copied().ok_or(TransportIoError::Other)?;
        let raw = i16::from(address) * 100;
        let mut response = vec![0x10, address, channel];
        response.extend_from_slice(&raw.to_be_bytes());
        let prefix = response.clone();
        SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
        wire.readable.extend(response);
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

fn ask(
    service: &mut ServiceHost,
    application: &mut Application,
    request: JsonValue,
) -> Vec<JsonValue> {
    application.handle(
        service,
        1,
        decode_frame(&encode_frame(&request).unwrap()).unwrap(),
    )
}

#[test]
fn persistent_simple_devices_share_one_resource_and_reach_generic_application_paths() {
    let mut reader = Reader::default();
    let deployment = parse_runtime_toml(&config(true), Path::new("C:/bench"), &mut reader).unwrap();
    assert_eq!(
        reader.reads, 1,
        "a shared immutable artifact is frozen once"
    );
    assert_eq!(deployment.artifacts().len(), 1);
    assert_eq!(deployment.artifacts()[0].bytes(), DEFINITION);

    let wire = Rc::new(RefCell::new(Wire::default()));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(ScriptedSimpleTransport(wire.clone())),
    );
    let host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let mut application = Application::new(service.boot_id()).unwrap();
    let hello = ask(
        &mut service,
        &mut application,
        json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
    );
    assert_eq!(hello[0]["type"], "result");
    let boot_id = service.boot_id().to_owned();
    let subscribed = ask(
        &mut service,
        &mut application,
        json!({"v":1,"msg_id":"subscribe","op":"subscribe","args":{
            "after":{"boot_id":boot_id,"seq":"0"},
            "filter":{"kinds":["signal"],"targets":[]}}}),
    );
    assert!(subscribed[0]["result"]["subscription"].is_string());

    let mut clock = TestClock::default();
    for milliseconds in (0..=100).step_by(10) {
        clock.0 = Duration::from_millis(milliseconds);
        service.owner_mut().service(&clock).unwrap();
    }

    for (instrument, expected) in [(1001, 10.0), (1002, 20.0)] {
        let signal = SignalId::new(InstrumentId::new(instrument), ParameterId::new(1));
        let QueryResult::Latest(Some(sample)) = service
            .owner()
            .query(Query::GetLatestSignal(signal))
            .unwrap()
        else {
            panic!("simple-device sample missing for {instrument}");
        };
        assert_eq!(sample.quality(), SampleQuality::Good);
        assert_eq!(sample.value(), Some(&Value::Float(expected)));
        let QueryResult::Window(window) = service
            .owner()
            .query(Query::GetSignalWindow(signal))
            .unwrap()
        else {
            panic!("ordinary signal window missing");
        };
        assert_eq!(window.last(), Some(&sample));
    }
    assert_eq!(
        wire.borrow().writes,
        [vec![0x10, 1, 0, 0x71, 0x95], vec![0x10, 2, 0, 0x71, 0x65]],
        "the resource owner serialized exact concrete instance requests"
    );

    let mut events = Vec::new();
    for _ in 0..8 {
        let batch = application.pump_events(&service, 1);
        if batch.is_empty() {
            break;
        }
        events.extend(batch);
    }
    assert!(
        events.iter().any(|event| {
            event["type"] == "event"
                && event["kind"] == "signal"
                && event["target"] == json!({"instrument":"1001","parameter":"1"})
                && event["data"]["quality"] == "good"
                && event["data"]["value"] == 10.0
        }),
        "ordinary simple-device signal event missing: {events:#?}"
    );
    let discovery = ask(
        &mut service,
        &mut application,
        json!({"v":1,"msg_id":"discover","op":"discover","args":{}}),
    );
    let records = discovery[0]["result"]["records"].as_array().unwrap();
    assert!(
        records
            .iter()
            .any(|row| row["kind"] == "instrument" && row["id"] == "1001")
    );
    assert!(
        records.iter().any(|row| row["kind"] == "signal"
            && row["id"] == json!({"instrument":"1002","parameter":"1"}))
    );
    let current = ask(
        &mut service,
        &mut application,
        json!({"v":1,"msg_id":"current","op":"measurements_current","args":{}}),
    );
    let current = current[0]["result"]["records"].as_array().unwrap();
    assert!(current.iter().any(|row| row["signal"]
        == json!({"instrument":"1001","parameter":"1"})
        && row["quality"] == "good"
        && row["value"] == 10.0));
    let window = ask(
        &mut service,
        &mut application,
        json!({"v":1,"msg_id":"window","op":"measurement_window","args":{
            "signal":{"instrument":"1002","parameter":"1"},"max_records":16}}),
    );
    assert_eq!(window[0]["result"]["source"], "runtime_recent");
    assert_eq!(window[0]["result"]["rows"][0]["value"], 20.0);
}

#[test]
fn insufficient_shared_resource_correlation_and_native_mixing_are_rejected() {
    let mut without_instance_match: JsonValue = serde_json::from_slice(DEFINITION).unwrap();
    without_instance_match["parameters"][0]["read"]["response"]["matches"] =
        json!([{"type":"literal_match","offset":0,"hex":"10"}]);
    struct FixedReader(Vec<u8>);
    impl ArtifactReader for FixedReader {
        fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
            Ok(self.0.clone())
        }
    }
    assert!(
        parse_runtime_toml(
            &config(true),
            Path::new("C:/bench"),
            &mut FixedReader(serde_json::to_vec(&without_instance_match).unwrap()),
        )
        .is_err()
    );

    let mut mixed = String::from_utf8(config(false)).unwrap();
    mixed.push_str(
        r#"
[[instruments]]
kind="metakon"
id=2001
key="native"
definition="definitions/metakon.json"
resource_id=7
address=3
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
"#,
    );
    assert!(
        parse_runtime_toml(
            mixed.as_bytes(),
            Path::new("C:/bench"),
            &mut FixedReader(DEFINITION.to_vec()),
        )
        .is_err()
    );
}

#[test]
fn core_scalar_widths_remain_the_frozen_twelve_value_set() {
    assert_eq!(SimpleScalarEncoding::U8.width(), 1);
    assert_eq!(SimpleScalarEncoding::I8.width(), 1);
    for encoding in [
        SimpleScalarEncoding::U16Le,
        SimpleScalarEncoding::U16Be,
        SimpleScalarEncoding::I16Le,
        SimpleScalarEncoding::I16Be,
    ] {
        assert_eq!(encoding.width(), 2);
    }
    for encoding in [
        SimpleScalarEncoding::U32Le,
        SimpleScalarEncoding::U32Be,
        SimpleScalarEncoding::I32Le,
        SimpleScalarEncoding::I32Be,
        SimpleScalarEncoding::F32Le,
        SimpleScalarEncoding::F32Be,
    ] {
        assert_eq!(encoding.width(), 4);
    }
}

#[test]
fn ordinary_simple_measurement_and_frozen_provenance_reach_sqlite() {
    let mut reader = Reader::default();
    let deployment =
        parse_runtime_toml(&config(false), Path::new("C:/bench"), &mut reader).unwrap();
    let wire = Rc::new(RefCell::new(Wire::default()));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(ResourceId::new(7), Box::new(ScriptedSimpleTransport(wire)));
    let host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let database = support::temporary_database("m16-simple");
    let database_text = database.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        database_text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let clock = service.clock_copy();
    service
        .owner_mut()
        .start_recording("simple-device acceptance", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(
            Instant::now() < deadline,
            "Recorder did not enter Recording"
        );
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    support::wait_until(deadline, "simple measurement was not produced", || {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        let QueryResult::Latest(sample) = service
            .owner()
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(1001),
                ParameterId::new(1),
            )))
            .unwrap()
        else {
            unreachable!();
        };
        sample
            .filter(|sample| sample.quality() == SampleQuality::Good)
            .map(|_| ())
    });
    service.request_shutdown().unwrap();
    let shutdown_deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed);
            break;
        }
        assert!(
            Instant::now() < shutdown_deadline,
            "bounded shutdown expired"
        );
        std::thread::yield_now();
    }
    drop(service);

    let database_connection = rusqlite::Connection::open(&database).unwrap();
    let measurements: i64 = database_connection
        .query_row(
            "SELECT COUNT(*) FROM measurements WHERE quality='good' AND float_value=10.0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(measurements >= 1);
    for kind in [
        "simple_device_definition_raw",
        "simple_device_definition_canonical",
        "simple_device_instance",
    ] {
        let count: i64 = database_connection
            .query_row(
                "SELECT COUNT(*) FROM provenance_content WHERE kind=?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing durable provenance kind {kind}");
    }
    let canonical: Vec<u8> = database_connection
        .query_row(
            "SELECT content FROM provenance_content WHERE kind='simple_device_definition_canonical'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let canonical_hash: [u8; 32] = Sha256::digest(&canonical).into();
    let raw_hash: [u8; 32] = Sha256::digest(DEFINITION).into();
    let object_id = 1001u64.to_be_bytes();
    let (logical_key, binding, definition_hash, source_hash, descriptor): (
        String,
        String,
        Vec<u8>,
        Vec<u8>,
        String,
    ) = database_connection
        .query_row(
            "SELECT logical_key,instance_binding,definition_hash,source_hash,descriptor
             FROM object_snapshots WHERE object_kind='instrument' AND object_id=?1
             ORDER BY activation_no DESC LIMIT 1",
            [object_id.as_slice()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(logical_key, "instrument:furnace-1");
    assert_eq!(definition_hash, canonical_hash);
    assert_eq!(source_hash, raw_hash);
    let binding: JsonValue = serde_json::from_str(&binding).unwrap();
    assert_eq!(
        binding,
        json!({"kind":"simple_v1","r":"7","a":1,"c":0,"b":"1","m":"1"})
    );
    let descriptor: JsonValue = serde_json::from_str(&descriptor).unwrap();
    assert_eq!(
        descriptor["simple_device"]["definition_id"],
        "simple-furnace"
    );
    assert_eq!(descriptor["simple_device"]["definition_version"], 1);
    assert_eq!(descriptor["simple_device"]["configuration_revision"], "1");
    assert_eq!(
        descriptor["simple_device"]["canonical_sha256"],
        canonical_hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        descriptor["simple_device"]["raw_sha256"],
        raw_hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    drop(database_connection);
    support::remove_database(&database);
}

#[test]
fn active_definition_and_instance_cardinalities_fail_before_composition() {
    let mut too_many_instances = String::from_utf8(config(false)).unwrap();
    for ordinal in 2..=33 {
        too_many_instances.push_str(&format!(
            r#"
[[instruments]]
kind="simple_device"
id={}
key="furnace-{ordinal}"
display_name="Furnace {ordinal}"
definition="definitions/simple-furnace-v1.json"
resource_id=7
address={ordinal}
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#,
            1000 + ordinal
        ));
    }
    let mut reader = Reader::default();
    assert!(
        parse_runtime_toml(
            too_many_instances.as_bytes(),
            Path::new("C:/bench"),
            &mut reader,
        )
        .is_err()
    );
    assert_eq!(
        reader.reads, 0,
        "instance cardinality precedes artifact I/O"
    );

    let mut too_many_definitions = String::from_utf8(config(false)).unwrap();
    too_many_definitions = too_many_definitions.replace(
        "definitions/simple-furnace-v1.json",
        "definitions/simple-1.json",
    );
    for ordinal in 2..=17 {
        too_many_definitions.push_str(&format!(
            r#"
[[instruments]]
kind="simple_device"
id={}
key="furnace-{ordinal}"
display_name="Furnace {ordinal}"
definition="definitions/simple-{ordinal}.json"
resource_id=7
address={ordinal}
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#,
            1000 + ordinal
        ));
    }
    let mut reader = Reader::default();
    assert!(
        parse_runtime_toml(
            too_many_definitions.as_bytes(),
            Path::new("C:/bench"),
            &mut reader,
        )
        .is_err()
    );
    assert_eq!(reader.reads, 16, "the seventeenth artifact is never read");
}
