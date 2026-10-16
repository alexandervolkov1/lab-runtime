//! Prepared topology, reconnect and durable publication fence regressions.
use super::*;
use crate::{
    application::Application,
    configuration::{ArtifactReader, ConfigurationError, parse_runtime_toml},
    configuration_api,
    recorder::RecordingState,
    serial::{OpenRetryPolicy, SerialDevice},
    wire::{decode_frame, encode_frame},
};
use lab_core::{
    ParameterId, Query, QueryResult, SampleQuality, SignalId,
    metakon::crc,
    simple_device::SimpleChecksum,
    transport::{RecoveryStatus, TransportIoError, TransportShutdown},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
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
enabled=true
path="history.sqlite"
policy="required"
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
read_timeout_ms=1
write_timeout_ms=1
open_timeout_ms=100
recovery_timeout_ms=100
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

const SIMPLE_DEFINITION: &[u8] = br#"{"format_version":1,"definition_id":"simple-v1","definition_version":1,"parameters":[{"parameter_id":1,"key":"temperature","display_name":"Temperature","role":"measurement","access":"read_only","unit_id":"degC","unit_symbol":"C","value_type":"float","engineering_min":-50.0,"engineering_max":500.0,"write_effect":"none","encoding":{"raw":"i16_be","scale":0.1,"offset":0.0},"read":{"request":{"segments":[{"type":"literal","hex":"10"},{"type":"instance_field","field":"address","encoding":"u8"}]},"response":{"exact_length":4,"matches":[{"type":"literal_match","offset":0,"hex":"10"},{"type":"instance_match","offset":1,"field":"address","encoding":"u8"}],"extract":{"type":"scalar_extract","offset":2},"checksum":null}}}]}"#;

const SIMPLE_CONFIG: &[u8] = br#"schema_version=1
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
read_timeout_ms=1
write_timeout_ms=1
open_timeout_ms=100
recovery_timeout_ms=100
[[instruments]]
kind="simple_device"
id=1001
key="simple-1"
display_name="Simple 1"
definition="simple.json"
resource_id=7
address=1
channel=0
poll_period_ms=100
queue_timeout_ms=50
transaction_timeout_ms=50
history_capacity=8
"#;

struct Reader;

impl ArtifactReader for Reader {
    fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
        Ok(DEFINITION.to_vec())
    }
}

struct SimpleReader;

impl ArtifactReader for SimpleReader {
    fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
        Ok(SIMPLE_DEFINITION.to_vec())
    }
}

struct OldTransport {
    shutdown_calls: Arc<AtomicUsize>,
    never_finishes: bool,
}

impl ByteTransport for OldTransport {
    fn try_write(&mut self, _: &[u8]) -> Result<usize, TransportIoError> {
        Err(TransportIoError::Disconnected)
    }

    fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Pending)
    }

    fn try_shutdown(&mut self) -> TransportShutdown {
        self.shutdown_calls.fetch_add(1, Ordering::AcqRel);
        if self.never_finishes {
            TransportShutdown::Pending
        } else {
            TransportShutdown::Complete
        }
    }
}

struct ProbeDevice {
    readable: VecDeque<u8>,
    channel_type: u8,
}

struct SimpleProbeDevice {
    readable: VecDeque<u8>,
}

impl SerialDevice for ProbeDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        let mut response = vec![1, 0, 0, 0, 0x41, self.channel_type];
        response.push(crc(&response));
        self.readable.extend(response);
        Ok(bytes.len())
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        let count = maximum.min(self.readable.len());
        Ok(self.readable.drain(..count).collect())
    }
}

impl SerialDevice for SimpleProbeDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        let address = *bytes.get(1).ok_or(SerialError::Other)?;
        let raw = u16::from(address).saturating_mul(100);
        self.readable
            .extend([0x10, address, (raw >> 8) as u8, raw as u8]);
        Ok(bytes.len())
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        let count = maximum.min(self.readable.len());
        Ok(self.readable.drain(..count).collect())
    }
}

fn temporary_database() -> PathBuf {
    crate::test_support::temporary_database("m8-reconnect")
}

fn service_with_old_transport(never_finishes: bool) -> (ServiceHost, PathBuf) {
    service_with_old_transport_and_writer_barrier(never_finishes, None)
}

fn service_with_old_transport_and_writer_barrier(
    never_finishes: bool,
    barrier: Option<crate::recorder::WriterBarrier>,
) -> (ServiceHost, PathBuf) {
    let deployment =
        parse_runtime_toml(CONFIG, Path::new("C:\\m8-test"), &mut Reader).expect("test deployment");
    let shutdown_calls = Arc::new(AtomicUsize::new(0));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(OldTransport {
            shutdown_calls,
            never_finishes,
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let boot_id = "0123456789abcdef0123456789abcdef".to_owned();
    host.set_boot_id(&boot_id);
    let clock = SystemClock::new();
    host.begin_configured_probes(clock.now()).unwrap();
    loop {
        host.service(&clock).unwrap();
        if host.resource_records()[0]["data"]["state"] == "offline" {
            break;
        }
        std::thread::yield_now();
    }
    let database = temporary_database();
    if barrier.is_some() {
        drop(crate::recorder::SqliteStore::open(&database).unwrap());
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch(
            "CREATE TABLE review_gate(id INTEGER PRIMARY KEY);
                CREATE TABLE review_fail(gate_id INTEGER REFERENCES review_gate(id)
                    DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER review_fail_reference AFTER INSERT ON reference_events
                    BEGIN INSERT INTO review_fail(gate_id) VALUES(999); END;",
        )
        .unwrap();
    }
    let anchor = TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = if let Some(barrier) = barrier {
        RecorderWorker::open_with_barrier(&database, RecorderLimits::default(), barrier)
    } else {
        RecorderWorker::open_with_boot_clock(
            &database,
            RecorderLimits::default(),
            &boot_id,
            anchor,
            clock,
        )
    }
    .unwrap();
    host.attach_recorder(recorder, RecordingPolicy::Required, clock.now())
        .unwrap();
    await_recorder_activation(&mut host).unwrap();
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let bound = listener.local_addr().unwrap();
    (
        ServiceHost {
            host,
            clock,
            listener,
            bound,
            websocket: None,
            boot_id,
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: Some(DeploymentLifecycle::new(deployment)),
            configuration_path: None,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        },
        database,
    )
}

fn service_with_simple_transport() -> (ServiceHost, Arc<AtomicUsize>) {
    let deployment =
        parse_runtime_toml(SIMPLE_CONFIG, Path::new("C:\\m16-test"), &mut SimpleReader)
            .expect("simple test deployment");
    let shutdown_calls = Arc::new(AtomicUsize::new(0));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(OldTransport {
            shutdown_calls: shutdown_calls.clone(),
            never_finishes: false,
        }),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let boot_id = "1123456789abcdef0123456789abcdef".to_owned();
    host.set_boot_id(&boot_id);
    let clock = SystemClock::new();
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let bound = listener.local_addr().unwrap();
    (
        ServiceHost {
            host,
            clock,
            listener,
            bound,
            websocket: None,
            boot_id,
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: Some(DeploymentLifecycle::new(deployment)),
            configuration_path: None,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        },
        shutdown_calls,
    )
}

fn application_read_only_candidate(instrument_id: u64, address: u16) -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,
        "definition":serde_json::from_slice::<serde_json::Value>(SIMPLE_DEFINITION).unwrap(),
        "instances":[{
            "instrument_id":instrument_id,
            "key":format!("api-simple-{instrument_id}"),
            "display_name":format!("API Simple {instrument_id}"),
            "resource_id":7,
            "address":address,
            "channel":0,
            "poll_period_ms":100,
            "queue_timeout_ms":50,
            "transaction_timeout_ms":50,
            "history_capacity":8
        }]
    })
}

const API_OUTPUT_BASE_CONFIG: &[u8] = br#"schema_version=1
[runtime]
key="api-output"
display_name="API output"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=1
write_timeout_ms=1
open_timeout_ms=100
recovery_timeout_ms=100
[[references]]
id=10
key="setpoint"
kind="fixed"
value=25.0
unit_id="degC"
unit_symbol="C"
"#;

#[derive(Default)]
struct ProvisioningWire {
    readable: VecDeque<u8>,
    raw: [u8; 2],
    writes: Vec<Vec<u8>>,
    bad_ack: bool,
    withhold_ack: bool,
    fail_output_before_send: bool,
    partial_output_once: bool,
    fail_after_partial: bool,
    mismatch_readback: bool,
    malformed_readback: bool,
}

struct ProvisioningTransport(Arc<Mutex<ProvisioningWire>>);

struct ProvisioningDevice(Arc<Mutex<ProvisioningWire>>);

const UNKNOWN_THERMAL_INSTRUMENT_ID: u64 = 1607;
const UNKNOWN_THERMAL_CONTROLLER_ID: u64 = 1607;
const UNKNOWN_THERMAL_ADDRESS: u8 = 0x2a;
const UNKNOWN_THERMAL_CHANNEL: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnknownThermalRequest {
    ReadTemperature,
    WritePower([u8; 2]),
    ReadbackPower,
}

struct UnknownThermalWire {
    readable: VecDeque<u8>,
    requests: Vec<Vec<u8>>,
    recognized: Vec<UnknownThermalRequest>,
    temperature_raw: i16,
    power_raw: [u8; 2],
}

impl UnknownThermalWire {
    fn new(temperature_raw: i16) -> Self {
        Self {
            readable: VecDeque::new(),
            requests: Vec::new(),
            recognized: Vec::new(),
            temperature_raw,
            power_raw: [0, 0],
        }
    }

    fn checked_prefix(bytes: &[u8], length: usize) -> Option<&[u8]> {
        if bytes.len() != length || length < 2 {
            return None;
        }
        let prefix = &bytes[..length - 2];
        let mut expected = prefix.to_vec();
        SimpleChecksum::Crc16Modbus.append(prefix, &mut expected);
        (expected == bytes).then_some(prefix)
    }

    fn enqueue_crc(&mut self, prefix: &[u8]) {
        let mut response = prefix.to_vec();
        SimpleChecksum::Crc16Modbus.append(prefix, &mut response);
        self.readable.extend(response);
    }

    fn accept(&mut self, bytes: &[u8]) -> Result<(), ()> {
        self.requests.push(bytes.to_vec());
        match bytes.first().copied() {
            Some(0xa1) => {
                let prefix = Self::checked_prefix(bytes, 5).ok_or(())?;
                if prefix != [0xa1, UNKNOWN_THERMAL_ADDRESS, UNKNOWN_THERMAL_CHANNEL] {
                    return Err(());
                }
                self.recognized.push(UnknownThermalRequest::ReadTemperature);
                let raw = self.temperature_raw.to_be_bytes();
                self.enqueue_crc(&[
                    0xa1,
                    UNKNOWN_THERMAL_ADDRESS,
                    UNKNOWN_THERMAL_CHANNEL,
                    raw[0],
                    raw[1],
                ]);
            }
            Some(0xb1) => {
                let prefix = Self::checked_prefix(bytes, 7).ok_or(())?;
                if prefix[..3] != [0xb1, UNKNOWN_THERMAL_ADDRESS, UNKNOWN_THERMAL_CHANNEL] {
                    return Err(());
                }
                self.power_raw.copy_from_slice(&prefix[3..5]);
                self.recognized
                    .push(UnknownThermalRequest::WritePower(self.power_raw));
                self.enqueue_crc(&[0xb1, UNKNOWN_THERMAL_ADDRESS, 0x00]);
            }
            Some(0xb2) => {
                let prefix = Self::checked_prefix(bytes, 5).ok_or(())?;
                if prefix != [0xb2, UNKNOWN_THERMAL_ADDRESS, UNKNOWN_THERMAL_CHANNEL] {
                    return Err(());
                }
                self.recognized.push(UnknownThermalRequest::ReadbackPower);
                self.enqueue_crc(&[
                    0xb2,
                    UNKNOWN_THERMAL_ADDRESS,
                    UNKNOWN_THERMAL_CHANNEL,
                    self.power_raw[0],
                    self.power_raw[1],
                ]);
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

struct UnknownThermalTransport(Arc<Mutex<UnknownThermalWire>>);

struct UnknownThermalDevice(Arc<Mutex<UnknownThermalWire>>);

impl ByteTransport for UnknownThermalTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        self.0
            .lock()
            .unwrap()
            .accept(bytes)
            .map_err(|()| TransportIoError::Other)?;
        Ok(bytes.len())
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.lock().unwrap();
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

impl SerialDevice for UnknownThermalDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        self.0
            .lock()
            .unwrap()
            .accept(bytes)
            .map_err(|()| SerialError::Other)?;
        Ok(bytes.len())
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        let mut wire = self.0.lock().unwrap();
        let count = maximum.min(wire.readable.len());
        Ok(wire.readable.drain(..count).collect())
    }
}

impl SerialDevice for ProvisioningDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        let mut wire = self.0.lock().unwrap();
        wire.writes.push(bytes.to_vec());
        match bytes.first().copied() {
            Some(0x10) => {
                let address = *bytes.get(1).ok_or(SerialError::Other)?;
                let raw = u16::from(address).saturating_mul(100);
                wire.readable
                    .extend([0x10, address, (raw >> 8) as u8, raw as u8]);
            }
            Some(0x20) => {
                wire.raw
                    .copy_from_slice(bytes.get(3..5).ok_or(SerialError::Other)?);
                let status = u8::from(wire.bad_ack);
                wire.readable.extend([0x20, bytes[1], status]);
            }
            Some(0x21) => {
                let raw = wire.raw;
                if wire.malformed_readback {
                    wire.readable.extend([0x22, bytes[1], raw[0], raw[1]]);
                } else if wire.mismatch_readback {
                    let raw = u16::from_be_bytes(raw).saturating_add(1).to_be_bytes();
                    wire.readable.extend([0x21, bytes[1], raw[0], raw[1]]);
                } else {
                    wire.readable.extend([0x21, bytes[1], raw[0], raw[1]]);
                }
            }
            _ => return Err(SerialError::Other),
        }
        Ok(bytes.len())
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        let mut wire = self.0.lock().unwrap();
        let count = maximum.min(wire.readable.len());
        Ok(wire.readable.drain(..count).collect())
    }
}

impl ByteTransport for ProvisioningTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.lock().unwrap();
        if wire.fail_after_partial {
            return Err(TransportIoError::Disconnected);
        }
        if bytes.first() == Some(&0x20) && wire.fail_output_before_send {
            return Err(TransportIoError::Disconnected);
        }
        if bytes.first() == Some(&0x20) && wire.partial_output_once {
            wire.partial_output_once = false;
            wire.fail_after_partial = true;
            wire.writes.push(bytes[..1].to_vec());
            return Ok(1);
        }
        wire.writes.push(bytes.to_vec());
        match bytes.first().copied() {
            Some(0x10) => {
                let address = *bytes.get(1).ok_or(TransportIoError::Other)?;
                let raw = u16::from(address).saturating_mul(100);
                wire.readable
                    .extend([0x10, address, (raw >> 8) as u8, raw as u8]);
            }
            Some(0x20) => {
                wire.raw
                    .copy_from_slice(bytes.get(3..5).ok_or(TransportIoError::Other)?);
                if !wire.withhold_ack {
                    let status = u8::from(wire.bad_ack);
                    wire.readable.extend([0x20, bytes[1], status]);
                }
            }
            Some(0x21) => {
                let raw = wire.raw;
                if wire.malformed_readback {
                    wire.readable.extend([0x22, bytes[1], raw[0], raw[1]]);
                } else if wire.mismatch_readback {
                    let raw = u16::from_be_bytes(raw).saturating_add(1).to_be_bytes();
                    wire.readable.extend([0x21, bytes[1], raw[0], raw[1]]);
                } else {
                    wire.readable.extend([0x21, bytes[1], raw[0], raw[1]]);
                }
            }
            _ => return Err(TransportIoError::Other),
        }
        Ok(bytes.len())
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.lock().unwrap();
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

fn service_for_api_output(bad_ack: bool) -> (ServiceHost, Arc<Mutex<ProvisioningWire>>) {
    let deployment = parse_runtime_toml(
        API_OUTPUT_BASE_CONFIG,
        Path::new("C:\\m16-api-output"),
        &mut SimpleReader,
    )
    .unwrap();
    let wire = Arc::new(Mutex::new(ProvisioningWire {
        bad_ack,
        ..ProvisioningWire::default()
    }));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(ProvisioningTransport(wire.clone())),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    let boot_id = "2123456789abcdef0123456789abcdef".to_owned();
    host.set_boot_id(&boot_id);
    let clock = SystemClock::new();
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let bound = listener.local_addr().unwrap();
    (
        ServiceHost {
            host,
            clock,
            listener,
            bound,
            websocket: None,
            boot_id,
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: Some(DeploymentLifecycle::new(deployment)),
            configuration_path: None,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        },
        wire,
    )
}

fn application_output_candidate() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,
        "definition":{
            "format_version":1,"definition_id":"api-heater-v1","definition_version":1,
            "parameters":[{
                "parameter_id":2,"key":"heater","display_name":"Heater",
                "role":"actuator","access":"write_only","unit_id":"percent","unit_symbol":"%",
                "value_type":"float","engineering_min":0.0,"engineering_max":100.0,
                "write_effect":"output_affecting",
                "encoding":{"raw":"u16_be","scale":0.1,"offset":0.0},
                "write":{
                    "request":{"segments":[{"type":"literal","hex":"20"},
                        {"type":"instance_field","field":"address","encoding":"u8"},
                        {"type":"instance_field","field":"channel","encoding":"u8"},
                        {"type":"value_field"}]},
                    "ack":{"exact_length":3,"matches":[
                        {"type":"literal_match","offset":0,"hex":"20"},
                        {"type":"instance_match","offset":1,"field":"address","encoding":"u8"},
                        {"type":"literal_match","offset":2,"hex":"00"}]},
                    "readback":{
                        "request":{"segments":[{"type":"literal","hex":"21"},
                            {"type":"instance_field","field":"address","encoding":"u8"}]},
                        "response":{"exact_length":4,"matches":[
                            {"type":"literal_match","offset":0,"hex":"21"},
                            {"type":"instance_match","offset":1,"field":"address","encoding":"u8"}],
                            "extract":{"type":"scalar_extract","offset":2}}
                    }
                }
            }]
        },
        "instances":[{
            "instrument_id":2001,"key":"api-heater","display_name":"API Heater",
            "resource_id":7,"address":1,"channel":2,"poll_period_ms":100,
            "queue_timeout_ms":50,"transaction_timeout_ms":50,"history_capacity":8,
            "safe_profile":{"min":0.0,"max":100.0,"safe_value":0.0,
                "max_lease_ms":2000,"max_proposal_ttl_ms":200,
                "required_evidence":"readback"}
        }]
    })
}

fn application_control_candidate() -> serde_json::Value {
    let mut candidate = application_output_candidate();
    candidate["definition"]["definition_id"] = serde_json::json!("api-controlled-heater-v1");
    let measurement =
        serde_json::from_slice::<serde_json::Value>(SIMPLE_DEFINITION).unwrap()["parameters"][0]
            .clone();
    candidate["definition"]["parameters"]
        .as_array_mut()
        .unwrap()
        .insert(0, measurement);
    candidate["instances"][0]["controller"] = serde_json::json!({
        "id":20,
        "key":"api-pid",
        "input_instrument_id":2001,
        "input_parameter_id":1,
        "output_instrument_id":2001,
        "output_parameter_id":2,
        "reference_id":10,
        "period_ms":100,
        "ema_time_constant_ms":100,
        "ema_warmup_samples":1,
        "kp":1.0,
        "ki":0.0,
        "kd":0.0,
        "output_min":0.0,
        "output_max":100.0,
        "max_input_age_ms":500,
        "max_tick_gap_ms":500,
        "lease_lifetime_ms":2000,
        "proposal_ttl_ms":200
    });
    candidate
}

fn unknown_thermal_candidate() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,
        "definition":{
            "format_version":1,
            "definition_id":"m16-unknown-thermal-v1",
            "definition_version":1,
            "parameters":[
                {
                    "parameter_id":1,
                    "key":"temperature",
                    "display_name":"Temperature",
                    "role":"measurement",
                    "access":"read_only",
                    "unit_id":"degC",
                    "unit_symbol":"C",
                    "value_type":"float",
                    "engineering_min":-50.0,
                    "engineering_max":150.0,
                    "write_effect":"none",
                    "encoding":{"raw":"i16_be","scale":0.1,"offset":0.0},
                    "read":{
                        "request":{"segments":[
                            {"type":"literal","hex":"a1"},
                            {"type":"instance_field","field":"address","encoding":"u8"},
                            {"type":"instance_field","field":"channel","encoding":"u8"},
                            {"type":"checksum","algorithm":"crc16_modbus"}
                        ]},
                        "response":{
                            "exact_length":7,
                            "matches":[
                                {"type":"literal_match","offset":0,"hex":"a1"},
                                {"type":"instance_match","offset":1,"field":"address","encoding":"u8"},
                                {"type":"instance_match","offset":2,"field":"channel","encoding":"u8"}
                            ],
                            "extract":{"type":"scalar_extract","offset":3},
                            "checksum":{"type":"checksum","offset":5,"algorithm":"crc16_modbus"}
                        }
                    }
                },
                {
                    "parameter_id":2,
                    "key":"power",
                    "display_name":"Power",
                    "role":"actuator",
                    "access":"write_only",
                    "unit_id":"percent",
                    "unit_symbol":"%",
                    "value_type":"float",
                    "engineering_min":0.0,
                    "engineering_max":100.0,
                    "write_effect":"output_affecting",
                    "encoding":{"raw":"u16_be","scale":0.1,"offset":0.0},
                    "write":{
                        "request":{"segments":[
                            {"type":"literal","hex":"b1"},
                            {"type":"instance_field","field":"address","encoding":"u8"},
                            {"type":"instance_field","field":"channel","encoding":"u8"},
                            {"type":"value_field"},
                            {"type":"checksum","algorithm":"crc16_modbus"}
                        ]},
                        "ack":{
                            "exact_length":5,
                            "matches":[
                                {"type":"literal_match","offset":0,"hex":"b1"},
                                {"type":"instance_match","offset":1,"field":"address","encoding":"u8"},
                                {"type":"literal_match","offset":2,"hex":"00"}
                            ],
                            "checksum":{"type":"checksum","offset":3,"algorithm":"crc16_modbus"}
                        },
                        "readback":{
                            "request":{"segments":[
                                {"type":"literal","hex":"b2"},
                                {"type":"instance_field","field":"address","encoding":"u8"},
                                {"type":"instance_field","field":"channel","encoding":"u8"},
                                {"type":"checksum","algorithm":"crc16_modbus"}
                            ]},
                            "response":{
                                "exact_length":7,
                                "matches":[
                                    {"type":"literal_match","offset":0,"hex":"b2"},
                                    {"type":"instance_match","offset":1,"field":"address","encoding":"u8"},
                                    {"type":"instance_match","offset":2,"field":"channel","encoding":"u8"}
                                ],
                                "extract":{"type":"scalar_extract","offset":3},
                                "checksum":{"type":"checksum","offset":5,"algorithm":"crc16_modbus"}
                            }
                        }
                    }
                }
            ]
        },
        "instances":[{
            "instrument_id":UNKNOWN_THERMAL_INSTRUMENT_ID,
            "key":"unknown-thermal-1",
            "display_name":"Unknown Thermal 1",
            "resource_id":7,
            "address":UNKNOWN_THERMAL_ADDRESS,
            "channel":UNKNOWN_THERMAL_CHANNEL,
            "poll_period_ms":200,
            "queue_timeout_ms":50,
            "transaction_timeout_ms":50,
            "history_capacity":16,
            "safe_profile":{
                "min":0.0,
                "max":100.0,
                "safe_value":0.0,
                "max_lease_ms":2000,
                "max_proposal_ttl_ms":200,
                "required_evidence":"readback"
            },
            "controller":{
                "id":UNKNOWN_THERMAL_CONTROLLER_ID,
                "key":"unknown-thermal-controller",
                "input_instrument_id":UNKNOWN_THERMAL_INSTRUMENT_ID,
                "input_parameter_id":1,
                "output_instrument_id":UNKNOWN_THERMAL_INSTRUMENT_ID,
                "output_parameter_id":2,
                "reference_id":10,
                "period_ms":100,
                "ema_time_constant_ms":100,
                "ema_warmup_samples":1,
                "kp":1.0,
                "ki":0.0,
                "kd":0.0,
                "output_min":0.0,
                "output_max":100.0,
                "max_input_age_ms":500,
                "max_tick_gap_ms":500,
                "lease_lifetime_ms":2000,
                "proposal_ttl_ms":200
            }
        }]
    })
}

fn service_for_unknown_thermal(
    boot_id: &str,
    temperature_raw: i16,
) -> (ServiceHost, Arc<Mutex<UnknownThermalWire>>) {
    let deployment = parse_runtime_toml(
        API_OUTPUT_BASE_CONFIG,
        Path::new("C:\\m16-unknown-thermal"),
        &mut SimpleReader,
    )
    .unwrap();
    let wire = Arc::new(Mutex::new(UnknownThermalWire::new(temperature_raw)));
    let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
    transports.insert(
        ResourceId::new(7),
        Box::new(UnknownThermalTransport(wire.clone())),
    );
    let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
    host.set_boot_id(boot_id);
    let clock = SystemClock::new();
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let bound = listener.local_addr().unwrap();
    (
        ServiceHost {
            host,
            clock,
            listener,
            bound,
            websocket: None,
            boot_id: boot_id.to_owned(),
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: Some(DeploymentLifecycle::new(deployment)),
            configuration_path: None,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        },
        wire,
    )
}

fn stage_and_begin_api_output(service: &mut ServiceHost) -> u64 {
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_output_candidate()).unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    let id = staged.staged.id();
    service.begin_simple_device_apply(id, 1).unwrap();
    id
}

fn poll_simple_apply_to_terminal(service: &mut ServiceHost) -> SimpleApplyCompletion {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        service.host.service(&service.clock).unwrap();
        if let Some(completion) = service.poll_simple_device_apply() {
            return completion;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn output_candidate_is_hidden_until_exact_safe_evidence_then_published() {
    let (mut service, wire) = service_for_api_output(false);
    let candidate_id = stage_and_begin_api_output(&mut service);
    let candidate = InstrumentId::new(2001);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(candidate))
            .is_err()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let completion = loop {
        service.host.service(&service.clock).unwrap();
        if let Some(completion) = service.poll_simple_device_apply() {
            break completion;
        }
        assert!(std::time::Instant::now() < deadline);
        if !service.api_simple_overlay_active {
            assert!(
                service
                    .owner()
                    .query(lab_core::Query::DescribeInstrument(candidate))
                    .is_err()
            );
        }
        std::thread::yield_now();
    };
    assert_eq!(completion.candidate_id, candidate_id);
    assert_eq!(completion.result, Ok(2));
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(candidate))
            .is_ok()
    );
    let writes = &wire.lock().unwrap().writes;
    assert_eq!(writes.iter().filter(|write| write[0] == 0x20).count(), 1);
    assert!(writes.iter().any(|write| write == &[0x21, 1]));
    assert!(service.quarantined_simple_output.is_none());
}

#[test]
fn post_send_safe_failure_keeps_one_hidden_quarantine_without_resend() {
    let (mut service, wire) = service_for_api_output(true);
    let candidate_id = stage_and_begin_api_output(&mut service);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let completion = loop {
        service.host.service(&service.clock).unwrap();
        if let Some(completion) = service.poll_simple_device_apply() {
            break completion;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(completion.candidate_id, candidate_id);
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::OutputRejected)
    );
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(2001)))
            .is_err()
    );
    let quarantine = service.quarantined_simple_output.as_ref().unwrap();
    assert_eq!(quarantine.candidate_id, candidate_id);
    assert_eq!(quarantine.resource, ResourceId::new(7));
    let writes_before = wire.lock().unwrap().writes.len();
    for _ in 0..20 {
        service.host.service(&service.clock).unwrap();
    }
    assert_eq!(wire.lock().unwrap().writes.len(), writes_before);
    let status = service.simple_configuration_status();
    assert_eq!(status["quarantine"]["phase"], "reconciliation_required");
    assert_eq!(status["quarantine"]["safe_resend_blocked"], true);
    let resource = configuration_api::resource_json(&service, 7).unwrap();
    assert_eq!(resource["binding_generation"], "1");
    assert_eq!(resource["capabilities"]["reconnect"], true);

    let reconciled = service
        .reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(ProbeDevice {
                    readable: VecDeque::new(),
                    channel_type: 0,
                }))
            })
        })
        .unwrap();
    assert_eq!(reconciled.binding_generation, 2);
    assert!(service.quarantined_simple_output.is_none());
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(2001)))
            .is_err()
    );
}

#[test]
fn m16_6_pre_send_transport_failure_retires_without_quarantine() {
    let (mut service, wire) = service_for_api_output(false);
    wire.lock().unwrap().fail_output_before_send = true;
    let candidate_id = stage_and_begin_api_output(&mut service);
    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(completion.candidate_id, candidate_id);
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::TransportUnavailable)
    );
    assert!(wire.lock().unwrap().writes.is_empty());
    assert!(service.pending_simple_apply.is_none());
    assert!(service.quarantined_simple_output.is_none());
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(2001)))
            .is_err()
    );
}

#[test]
fn m16_6_partial_send_is_ambiguous_quarantined_and_never_retried() {
    let (mut service, wire) = service_for_api_output(false);
    wire.lock().unwrap().partial_output_once = true;
    let candidate_id = stage_and_begin_api_output(&mut service);
    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(completion.candidate_id, candidate_id);
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::OutputRejected)
    );
    let writes = wire.lock().unwrap().writes.clone();
    assert_eq!(writes, [vec![0x20]]);
    assert!(service.quarantined_simple_output.is_some());
    for _ in 0..20 {
        service.host.service(&service.clock).unwrap();
    }
    assert_eq!(wire.lock().unwrap().writes, writes);
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
}

#[test]
fn m16_6_malformed_and_mismatching_readback_never_publish_or_resend() {
    for malformed in [false, true] {
        let (mut service, wire) = service_for_api_output(false);
        if malformed {
            wire.lock().unwrap().malformed_readback = true;
        } else {
            wire.lock().unwrap().mismatch_readback = true;
        }
        let candidate_id = stage_and_begin_api_output(&mut service);
        let completion = poll_simple_apply_to_terminal(&mut service);
        assert_eq!(completion.candidate_id, candidate_id);
        assert_eq!(
            completion.result,
            Err(LifecycleOperationError::OutputRejected)
        );
        let writes = wire.lock().unwrap().writes.clone();
        assert_eq!(writes.iter().filter(|write| write[0] == 0x20).count(), 1);
        assert_eq!(writes.iter().filter(|write| write[0] == 0x21).count(), 1);
        assert!(service.quarantined_simple_output.is_some());
        assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
        assert!(
            service
                .owner()
                .query(lab_core::Query::DescribeInstrument(InstrumentId::new(2001)))
                .is_err()
        );
        for _ in 0..20 {
            service.host.service(&service.clock).unwrap();
        }
        assert_eq!(wire.lock().unwrap().writes, writes);
    }
}

#[test]
fn m16_6_stage_pending_and_identity_slots_are_exact_and_stable() {
    let (mut service, _) = service_with_simple_transport();
    let first =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let second =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1003, 3))
            .unwrap();
    let staged = service.stage_simple_device_candidate(&first, 1).unwrap();
    let identity = staged.staged.id();
    assert_eq!(
        service.stage_simple_device_candidate(&second, 1),
        Err(LifecycleOperationError::Capacity)
    );
    assert_eq!(
        service.deployment.as_ref().unwrap().staged().unwrap().id(),
        identity
    );
    assert_eq!(
        service.begin_simple_device_apply(identity + 1, 1),
        Err(LifecycleOperationError::Conflict)
    );
    assert_eq!(
        service.begin_simple_device_apply(identity, 2),
        Err(LifecycleOperationError::Conflict)
    );
    assert_eq!(
        service.deployment.as_ref().unwrap().staged().unwrap().id(),
        identity
    );
    service.begin_simple_device_apply(identity, 1).unwrap();
    assert!(service.deployment.as_ref().unwrap().staged().is_none());
    assert_eq!(
        service.begin_simple_device_apply(identity, 1),
        Err(LifecycleOperationError::Capacity)
    );
    assert_eq!(
        service
            .pending_simple_apply
            .as_ref()
            .unwrap()
            .candidate
            .snapshot
            .id(),
        identity
    );
    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(completion.candidate_id, identity);
    assert_eq!(completion.result, Ok(2));
}

#[test]
fn m16_6_client_death_does_not_cancel_staged_or_accepted_work() {
    let drive_client = |service: &mut ServiceHost, application: &mut Application| {
        let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
        let hello = application.handle(
            service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                    "args":{"scope":null}})),
        );
        let scope = hello[0]["result"]["scope"].clone();
        let staged = application.handle(
            service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"stage",
                    "op":"stage_simple_device_candidate",
                    "request_id":{"scope":scope,"seq":"1"},
                    "args":{"expected_revision":"1",
                        "candidate":application_read_only_candidate(1002,2)}})),
        );
        (scope, staged[1]["result"]["candidate_id"].clone())
    };

    let (mut staged_service, _) = service_with_simple_transport();
    let mut staged_client = Application::new(staged_service.boot_id()).unwrap();
    let (_, staged_id) = drive_client(&mut staged_service, &mut staged_client);
    staged_client.detach(&staged_service, 1);
    assert_eq!(
        staged_service
            .deployment
            .as_ref()
            .unwrap()
            .staged()
            .unwrap()
            .id()
            .to_string(),
        staged_id.as_str().unwrap()
    );
    assert!(
        staged_service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );

    let (mut accepted_service, _) = service_with_simple_transport();
    let mut accepted_client = Application::new(accepted_service.boot_id()).unwrap();
    let (scope, candidate_id) = drive_client(&mut accepted_service, &mut accepted_client);
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let accepted = accepted_client.handle(
        &mut accepted_service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration","request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    accepted_client.detach(&accepted_service, 1);
    let completion = poll_simple_apply_to_terminal(&mut accepted_service);
    assert_eq!(completion.result, Ok(2));
    assert!(
        accepted_service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_ok()
    );
}

#[test]
fn m16_6_restart_restores_persistent_baseline_without_overlay_or_replay() {
    let (mut read_only, _) = service_with_simple_transport();
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let staged = read_only
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    read_only
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    assert_eq!(poll_simple_apply_to_terminal(&mut read_only).result, Ok(2));
    assert!(read_only.api_simple_overlay_active);
    drop(read_only);

    let (restarted, _) = service_with_simple_transport();
    assert_eq!(restarted.deployment.as_ref().unwrap().revision(), 1);
    assert!(!restarted.api_simple_overlay_active);
    assert!(restarted.pending_simple_apply.is_none());
    assert!(restarted.quarantined_simple_output.is_none());
    assert!(
        restarted
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1001)))
            .is_ok()
    );
    assert!(
        restarted
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );

    let (mut output, output_wire) = service_for_api_output(false);
    stage_and_begin_api_output(&mut output);
    assert_eq!(poll_simple_apply_to_terminal(&mut output).result, Ok(2));
    assert!(!output_wire.lock().unwrap().writes.is_empty());
    drop(output);

    let (mut restarted_output, restarted_wire) = service_for_api_output(false);
    for _ in 0..20 {
        restarted_output
            .host
            .service(&restarted_output.clock)
            .unwrap();
    }
    assert_eq!(restarted_output.deployment.as_ref().unwrap().revision(), 1);
    assert!(!restarted_output.api_simple_overlay_active);
    assert!(restarted_output.pending_simple_apply.is_none());
    assert!(restarted_output.quarantined_simple_output.is_none());
    assert!(restarted_wire.lock().unwrap().writes.is_empty());
    assert!(
        restarted_output
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(2001)))
            .is_err()
    );
}

#[test]
fn application_candidate_stages_then_publishes_atomically_as_process_local_overlay() {
    let (mut service, _) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    assert!(
        hello[0]["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability["name"] == "simple_device_provisioning")
    );

    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1",
                    "candidate":application_read_only_candidate(1002,2)}})),
    );
    assert_eq!(staged.len(), 2);
    assert_eq!(staged[0]["state"], "accepted");
    assert_eq!(staged[1]["state"], "completed");
    let staged_result = &staged[1]["result"];
    let candidate_id = staged_result["candidate_id"].clone();
    assert_eq!(staged_result["definition"]["id"], "simple-v1");
    assert_eq!(staged_result["instances"], serde_json::json!(["1002"]));

    let before = service
        .owner()
        .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)));
    assert!(before.is_err());
    let applied = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["state"], "accepted");
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );

    let terminal = loop {
        service.host.service(&service.clock).unwrap();
        let replies = application.poll_configuration(&mut service);
        if !replies.is_empty() {
            break replies;
        }
        std::thread::yield_now();
    };
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_ok()
    );
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 2);
    assert!(service.api_simple_overlay_active);
    assert_eq!(
        service.stage_configuration(),
        Err(LifecycleOperationError::InvalidCandidate)
    );
    assert!(service.deployment.as_ref().unwrap().staged().is_none());

    application.detach(&service, 1);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_ok()
    );
    let configured = service
        .configure_property(
            "instrument",
            1002,
            "poll_period_ms",
            PropertyValue::Integer(200),
            2,
        )
        .unwrap();
    assert_eq!(configured.revision, 3);
    assert!(service.api_simple_overlay_active);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_ok()
    );
}

#[test]
fn event_capacity_failure_before_recorder_submission_preserves_previous_topology() {
    let (mut service, _) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let before = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"before","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert!(
        !before
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == serde_json::json!({"id":"2002"}))
    );
    let before_event_facts = service.host.event_log().projection_records();
    let (before_provenance_entries, before_provenance_objects) =
        service.host.frozen_activation_entries().unwrap();
    let before_provenance_objects = format!("{before_provenance_objects:?}");
    let staged = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"stage","op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},"args":{"expected_revision":"1",
                    "candidate":application_read_only_candidate(1002,2)}}),
        ),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let applied = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"apply","op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},"args":{"candidate_id":candidate_id,
                    "expected_revision":"1"}}),
        ),
    );
    assert_eq!(applied[0]["state"], "accepted");

    service
        .host
        .event_log_mut()
        .force_sequence_for_test(u64::MAX);
    let terminal = (0..8)
        .find_map(|_| {
            let replies = application.poll_configuration(&mut service);
            (!replies.is_empty()).then_some(replies)
        })
        .unwrap_or_default();
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "failed");
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert!(!service.api_simple_overlay_active);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );
    let after = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"after","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert_eq!(after, before);
    assert_eq!(
        service.host.event_log().projection_records(),
        before_event_facts
    );
    let (after_provenance_entries, after_provenance_objects) =
        service.host.frozen_activation_entries().unwrap();
    assert_eq!(after_provenance_entries, before_provenance_entries);
    assert_eq!(
        format!("{after_provenance_objects:?}"),
        before_provenance_objects
    );
}

#[test]
fn consume_simple_device_expires_at_the_exact_frozen_deadline() {
    let (mut service, _) = service_with_simple_transport();
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    let expiry = staged.staged.expires_at();
    let candidate_id = staged.staged.id();
    let lifecycle = service.deployment.as_mut().unwrap();
    assert!(matches!(
        lifecycle.consume_simple_device(candidate_id, 1, expiry),
        Err(ApplyError::Expired)
    ));
    assert!(lifecycle.staged().is_none());
    assert_eq!(lifecycle.revision(), 1);
}

#[test]
fn api_provisioned_signals_use_generic_application_history_recorder_and_shared_reconnect() {
    let (mut service, _) = service_for_api_output(false);
    let database = temporary_database();
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::Required, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();
    service
        .host
        .start_recording("M16.5 generic integration", service.clock.now())
        .unwrap();
    let recording_deadline = std::time::Instant::now() + Duration::from_secs(3);
    while service.host.recording_status().unwrap().state != RecordingState::Recording {
        service.host.service(&service.clock).unwrap();
        assert!(std::time::Instant::now() < recording_deadline);
        std::thread::yield_now();
    }

    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let cursor = service.owner().event_log().latest_cursor();
    let boot_id = service.boot_id().to_owned();
    let subscribed = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"subscribe","op":"subscribe",
                "args":{"after":{"boot_id":boot_id,"seq":cursor.to_string()},
                    "filter":{"kinds":["signal"],"targets":[]}}}),
        ),
    );
    assert!(subscribed[0]["result"]["subscription"].is_string());

    let mut candidate = application_read_only_candidate(1001, 1);
    candidate["instances"][0]["poll_period_ms"] = serde_json::json!(100);
    candidate["instances"][0]["queue_timeout_ms"] = serde_json::json!(500);
    candidate["instances"][0]["transaction_timeout_ms"] = serde_json::json!(500);
    let mut second = candidate["instances"][0].clone();
    second["instrument_id"] = serde_json::json!(1002);
    second["key"] = serde_json::json!("api-simple-1002");
    second["display_name"] = serde_json::json!("API Simple 1002");
    second["address"] = serde_json::json!(2);
    candidate["instances"].as_array_mut().unwrap().push(second);
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1","candidate":candidate}})),
    );
    assert_eq!(staged[1]["state"], "completed");
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        service.host.poll_recorder_for_test(service.clock.now());
        let terminal = application.poll_configuration(&mut service);
        if !terminal.is_empty() {
            assert_eq!(terminal.len(), 1);
            assert_eq!(
                terminal[0].1["state"],
                "completed",
                "{terminal:#?}; recorder={:#?}",
                service.host.recording_status()
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }

    let signals = [
        SignalId::new(InstrumentId::new(1001), ParameterId::new(1)),
        SignalId::new(InstrumentId::new(1002), ParameterId::new(1)),
    ];
    loop {
        service.host.service(&service.clock).unwrap();
        if signals.iter().all(|signal| {
            matches!(
                service.owner().query(Query::GetLatestSignal(*signal)),
                Ok(QueryResult::Latest(Some(sample))) if sample.quality() == SampleQuality::Good
            )
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }

    let discovery = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"discover","op":"discover","args":{}})),
    );
    let records = discovery[0]["result"]["records"].as_array().unwrap();
    for instrument in ["1001", "1002"] {
        assert!(
            records
                .iter()
                .any(|row| { row["kind"] == "instrument" && row["id"] == instrument })
        );
        let described = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":format!("describe-{instrument}"),
                    "op":"describe","args":{"instrument":instrument}}),
            ),
        );
        assert_eq!(described[0]["result"]["id"], instrument);
    }
    let current = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"current","op":"measurements_current",
                "args":{}}),
        ),
    );
    let current = current[0]["result"]["records"].as_array().unwrap();
    for (instrument, value) in [("1001", 10.0), ("1002", 20.0)] {
        assert!(current.iter().any(|row| row["signal"]
            == serde_json::json!({"instrument":instrument,"parameter":"1"})
            && row["quality"] == "good"
            && row["value"] == value));
        let latest = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":format!("latest-{instrument}"),
                    "op":"latest","args":{"signal":{"instrument":instrument,"parameter":"1"}}}),
            ),
        );
        assert_eq!(latest[0]["result"]["value"], value);
        let window = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":format!("window-{instrument}"),
                    "op":"measurement_window","args":{"signal":{"instrument":instrument,
                        "parameter":"1"},"max_records":8}}),
            ),
        );
        assert_eq!(window[0]["result"]["source"], "runtime_recent");
        assert_eq!(window[0]["result"]["rows"][0]["value"], value);
    }
    let mut events = Vec::new();
    for _ in 0..8 {
        let batch = application.pump_events(&service, 1);
        if batch.is_empty() {
            break;
        }
        events.extend(batch);
    }
    for instrument in ["1001", "1002"] {
        assert!(events.iter().any(|event| event["kind"] == "signal"
            && event["target"] == serde_json::json!({"instrument":instrument,"parameter":"1"})
            && event["data"]["quality"] == "good"));
    }

    let reconnect = service
        .reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(SimpleProbeDevice {
                    readable: VecDeque::new(),
                }))
            })
        })
        .unwrap();
    assert_eq!(reconnect.binding_generation, 2);
    let resource = configuration_api::resource_json(&service, 7).unwrap();
    assert_eq!(resource["binding_generation"], "2");
    assert_eq!(resource["transport_generation"], "2");
    assert_eq!(resource["instruments"], serde_json::json!(["1001", "1002"]));
    for instrument in [1001, 1002] {
        let QueryResult::Latest(Some(sample)) = service
            .owner()
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(instrument),
                ParameterId::new(1),
            )))
            .unwrap()
        else {
            panic!("rebound signal missing");
        };
        assert_eq!(sample.quality(), SampleQuality::Unavailable);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        service.host.service(&service.clock).unwrap();
        if signals.iter().all(|signal| {
            matches!(
                service.owner().query(Query::GetLatestSignal(*signal)),
                Ok(QueryResult::Latest(Some(sample))) if sample.quality() == SampleQuality::Good
            )
        }) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let latest: Vec<_> = signals
                .iter()
                .map(|signal| service.owner().query(Query::GetLatestSignal(*signal)))
                .collect();
            panic!(
                "shared reconnect did not reacquire: latest={latest:#?} resource={:#?} transport={:#?}",
                configuration_api::resource_json(&service, 7),
                service.owner().query(Query::Transport(ResourceId::new(7)))
            );
        }
        std::thread::yield_now();
    }

    service.request_shutdown().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);
    let connection = rusqlite::Connection::open(&database).unwrap();
    for value in [10.0, 20.0] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE quality='good' AND float_value=?1",
                [value],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 1, "missing durable ordinary measurement {value}");
    }
    drop(connection);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
}

#[test]
fn api_provisioned_controller_uses_ordinary_authority_and_stays_paused_after_reconnect() {
    let (mut service, wire) = service_for_api_output(false);
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1","candidate":application_control_candidate()}})),
    );
    assert_eq!(staged[1]["state"], "completed");
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        service.host.service(&service.clock).unwrap();
        let terminal = application.poll_configuration(&mut service);
        if !terminal.is_empty() {
            assert_eq!(terminal[0].1["state"], "completed", "{terminal:#?}");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let controller = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"controller","op":"controller",
                "args":{"controller":"20"}}),
        ),
    );
    assert_eq!(controller[0]["result"]["state"], "ready");
    assert_eq!(
        controller[0]["result"]["bindings"]["input"],
        serde_json::json!({"instrument":"2001","parameter":"1"})
    );
    assert_eq!(
        controller[0]["result"]["bindings"]["output"],
        serde_json::json!({"instrument":"2001","parameter":"2"})
    );

    let input = SignalId::new(InstrumentId::new(2001), ParameterId::new(1));
    loop {
        service.host.service(&service.clock).unwrap();
        if matches!(
            service.owner().query(Query::GetLatestSignal(input)),
            Ok(QueryResult::Latest(Some(sample))) if sample.quality() == SampleQuality::Good
        ) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let before_start = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"before-start","op":"output",
                "args":{"actuator":{"instrument":"2001","parameter":"2"}}}),
        ),
    );
    assert_eq!(
        before_start[0]["result"]["state"], "disarmed",
        "{before_start:#?}"
    );
    assert_eq!(before_start[0]["result"]["safe_confirmed"], true);
    let started = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"start","op":"controller_start",
                "request_id":{"scope":scope,"seq":"3"},"args":{"controller":"20"}}),
        ),
    );
    assert_eq!(started[1]["state"], "completed");
    loop {
        service.host.service(&service.clock).unwrap();
        let controller = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":"running","op":"controller",
                    "args":{"controller":"20"}}),
            ),
        );
        if controller[0]["result"]["state"] == "running"
            && wire
                .lock()
                .unwrap()
                .writes
                .iter()
                .any(|bytes| bytes.first() == Some(&0x20) && bytes.get(3..5) != Some(&[0, 0]))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "controller did not dispatch: controller={controller:#?} writes={:#?}",
            wire.lock().unwrap().writes
        );
        std::thread::yield_now();
    }
    let output = loop {
        service.host.service(&service.clock).unwrap();
        let output = application.handle(
            &mut service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"output","op":"output",
                    "args":{"actuator":{"instrument":"2001","parameter":"2"}}})),
        );
        if output[0]["result"]["outcome"] == "readback_verified" {
            break output;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(output[0]["result"]["outcome"], "readback_verified");
    assert_eq!(output[0]["result"]["owner"]["kind"], "automatic");
    assert_eq!(output[0]["result"]["owner"]["id"], "20");

    let paused = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"pause","op":"controller_pause",
                "request_id":{"scope":scope,"seq":"4"},"args":{"controller":"20"}}),
        ),
    );
    assert_eq!(paused[1]["result"]["state"], "paused");
    loop {
        service.host.service(&service.clock).unwrap();
        if service.host.configured_physical_outputs_safe().unwrap() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let replacement_wire = Arc::new(Mutex::new(ProvisioningWire::default()));
    let device_wire = replacement_wire.clone();
    let reconnect = service
        .reconnect_resource_with_factory(7, 1, move |settings, _| {
            let device_wire = device_wire.clone();
            ComTransport::with_device_factory(settings, move || {
                Ok(Box::new(ProvisioningDevice(device_wire.clone())))
            })
        })
        .unwrap();
    assert_eq!(reconnect.binding_generation, 2);
    let after = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"after","op":"controller",
                "args":{"controller":"20"}})),
    );
    assert_eq!(after[0]["result"]["state"], "paused");
    let output = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"rebound-output","op":"output",
                "args":{"actuator":{"instrument":"2001","parameter":"2"}}}),
        ),
    );
    assert!(output[0]["result"]["owner"].is_null());
    assert_eq!(output[0]["result"]["safe_confirmed"], true);
    loop {
        service.host.service(&service.clock).unwrap();
        if matches!(
            service.owner().query(Query::GetLatestSignal(input)),
            Ok(QueryResult::Latest(Some(sample))) if sample.quality() == SampleQuality::Good
        ) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let fresh = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"fresh","op":"controller",
                "args":{"controller":"20"}})),
    );
    assert_eq!(fresh[0]["result"]["state"], "paused");
    let output = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"fresh-output","op":"output",
                "args":{"actuator":{"instrument":"2001","parameter":"2"}}}),
        ),
    );
    assert!(output[0]["result"]["owner"].is_null());
}

#[test]
fn m16_7_unknown_device_uses_only_generic_application_runtime_and_recorder_paths() {
    let candidate_json = unknown_thermal_candidate();
    let candidate = crate::simple_device::parse_simple_candidate(&candidate_json).unwrap();
    assert_eq!(candidate.definition.definition_id, "m16-unknown-thermal-v1");
    assert_eq!(candidate.instances[0].key, "unknown-thermal-1");
    let hash_hex = |hash: [u8; 32]| {
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let definition_hash = candidate.definition.canonical_sha256;
    let candidate_hash = hash_hex(Sha256::digest(candidate.canonical.as_ref()).into());
    let instrument_id = UNKNOWN_THERMAL_INSTRUMENT_ID.to_string();

    let (mut service, old_wire) =
        service_for_unknown_thermal("3123456789abcdef0123456789abcdef", 215);
    let database = temporary_database();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);

    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    for forbidden in [
        "send_raw_bytes",
        "raw_serial_write",
        "execute_device_command",
        "unchecked_register_write",
    ] {
        assert!(crate::protocol::operation_spec(forbidden).is_none());
    }
    assert!(
        hello[0]["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|capability| {
                let name = capability["name"].as_str().unwrap();
                !name.contains("raw")
                    && !name.contains("unknown_thermal")
                    && !name.contains("m16_unknown")
            })
    );
    let before = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"before","op":"discover","args":{}})),
    );
    assert!(
        !before[0]["result"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["kind"] == "instrument" && record["id"] == instrument_id)
    );

    let cursor = service.owner().event_log().latest_cursor();
    let boot_id = service.boot_id().to_owned();
    let subscribed = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"subscribe","op":"subscribe",
                "args":{"after":{"boot_id":boot_id,"seq":cursor.to_string()},
                    "filter":{"kinds":["signal"],"targets":[]}}}),
        ),
    );
    assert!(subscribed[0]["result"]["subscription"].is_string());

    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1","candidate":candidate_json}})),
    );
    assert_eq!(staged[1]["state"], "completed", "{staged:#?}");
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    loop {
        service.host.service(&service.clock).unwrap();
        let pending_debug = service.pending_simple_apply.as_ref().map(|pending| {
            (
                pending.phase,
                pending.actuator,
                pending
                    .actuator
                    .and_then(|actuator| service.host.prepared_simple_output(actuator)),
            )
        });
        let terminal = application.poll_configuration(&mut service);
        if !terminal.is_empty() {
            assert_eq!(terminal.len(), 1);
            assert_eq!(
                terminal[0].1["state"],
                "completed",
                "terminal={terminal:#?} pending={pending_debug:#?} requests={:#?}",
                old_wire.lock().unwrap().requests,
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }

    let safe_sequence = old_wire.lock().unwrap().recognized.clone();
    assert!(safe_sequence.windows(2).any(|pair| {
        pair == [
            UnknownThermalRequest::WritePower([0, 0]),
            UnknownThermalRequest::ReadbackPower,
        ]
    }));

    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::BestEffort, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();
    service
        .host
        .start_recording("M16.7 unknown device", service.clock.now())
        .unwrap();
    while service.host.recording_status().unwrap().state != RecordingState::Recording {
        service.host.service(&service.clock).unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let recorded_read_target = old_wire
        .lock()
        .unwrap()
        .recognized
        .iter()
        .filter(|request| **request == UnknownThermalRequest::ReadTemperature)
        .count()
        + 1;

    let after = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"after","op":"discover","args":{}})),
    );
    let records = after[0]["result"]["records"].as_array().unwrap();
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "instrument" && record["id"] == instrument_id)
    );
    assert!(records.iter().any(|record| record["kind"] == "signal"
        && record["id"] == serde_json::json!({"instrument":instrument_id,"parameter":"1"})));
    assert!(records.iter().any(|record| record["kind"] == "output"
        && record["id"] == serde_json::json!({"instrument":instrument_id,"parameter":"2"})));
    assert!(records.iter().any(|record| record["kind"] == "controller"
        && record["id"] == serde_json::json!({"id": UNKNOWN_THERMAL_CONTROLLER_ID.to_string()})));
    let described = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"describe","op":"describe",
                "args":{"instrument":instrument_id}}),
        ),
    );
    let power = described[0]["result"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["id"] == "2")
        .unwrap();
    assert_eq!(power["role"], "actuator");
    assert_eq!(power["access"], "write_only");
    assert_eq!(power["write_effect"], "output_affecting");

    let temperature = SignalId::new(
        InstrumentId::new(UNKNOWN_THERMAL_INSTRUMENT_ID),
        ParameterId::new(1),
    );
    loop {
        service.host.service(&service.clock).unwrap();
        if matches!(
            service.owner().query(Query::GetLatestSignal(temperature)),
            Ok(QueryResult::Latest(Some(sample)))
                if sample.quality() == SampleQuality::Good
                    && sample.value() == Some(&lab_core::Value::Float(21.5))
        ) && old_wire
            .lock()
            .unwrap()
            .recognized
            .iter()
            .filter(|request| **request == UnknownThermalRequest::ReadTemperature)
            .count()
            >= recorded_read_target
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(
        old_wire
            .lock()
            .unwrap()
            .recognized
            .contains(&UnknownThermalRequest::ReadTemperature)
    );
    let current = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"current",
                "op":"measurements_current","args":{}})),
    );
    assert!(
        current[0]["result"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["signal"]
                == serde_json::json!({"instrument":instrument_id,"parameter":"1"})
                && record["quality"] == "good"
                && record["value"] == 21.5)
    );
    let latest = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"latest","op":"latest",
                "args":{"signal":{"instrument":instrument_id,"parameter":"1"}}})),
    );
    assert_eq!(latest[0]["result"]["value"], 21.5);
    let window = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"window","op":"measurement_window",
                "args":{"signal":{"instrument":instrument_id,"parameter":"1"},
                    "max_records":8}}),
        ),
    );
    assert_eq!(window[0]["result"]["source"], "runtime_recent");
    assert!(
        window[0]["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["quality"] == "good" && record["value"] == 21.5)
    );
    let mut signal_events = Vec::new();
    for _ in 0..8 {
        let batch = application.pump_events(&service, 1);
        if batch.is_empty() {
            break;
        }
        signal_events.extend(batch);
    }
    assert!(signal_events.iter().any(|event| event["kind"] == "signal"
        && event["target"] == serde_json::json!({"instrument":instrument_id,"parameter":"1"})
        && event["data"]["quality"] == "good"
        && event["data"]["value"] == 21.5));
    service.host.stop_recording().unwrap();
    while service.host.recording_status().unwrap().state != RecordingState::Idle {
        service.host.service(&service.clock).unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let controller = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"controller","op":"controller",
                "args":{"controller":UNKNOWN_THERMAL_CONTROLLER_ID.to_string()}}),
        ),
    );
    assert_eq!(controller[0]["result"]["state"], "ready");
    let output_before = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"output-before","op":"output",
                "args":{"actuator":{"instrument":instrument_id,"parameter":"2"}}}),
        ),
    );
    assert_eq!(output_before[0]["result"]["state"], "disarmed");
    assert_eq!(output_before[0]["result"]["safe_confirmed"], true);

    let started = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"start","op":"controller_start",
                "request_id":{"scope":scope,"seq":"3"},
                "args":{"controller":UNKNOWN_THERMAL_CONTROLLER_ID.to_string()}}),
        ),
    );
    assert_eq!(started[1]["state"], "completed");
    loop {
        service.host.service(&service.clock).unwrap();
        let output = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":"output-running","op":"output",
                    "args":{"actuator":{"instrument":instrument_id,"parameter":"2"}}}),
            ),
        );
        if output[0]["result"]["outcome"] == "readback_verified"
                && output[0]["result"]["owner"]["kind"] == "automatic"
                && old_wire.lock().unwrap().recognized.iter().any(
                    |request| matches!(request, UnknownThermalRequest::WritePower(raw) if *raw != [0, 0]),
                )
            {
                assert_eq!(
                    output[0]["result"]["owner"]["id"],
                    UNKNOWN_THERMAL_CONTROLLER_ID.to_string()
                );
                assert!(output[0]["result"]["requested"].is_number());
                assert!(output[0]["result"]["sent"]["value"].is_number());
                assert!(output[0]["result"]["acknowledged"]["value"].is_number());
                assert!(output[0]["result"]["readback"]["value"].is_number());
                assert_eq!(
                    output[0]["result"]["requested"],
                    output[0]["result"]["readback"]["value"]
                );
                break;
            }
        assert!(std::time::Instant::now() < deadline, "{output:#?}");
        std::thread::yield_now();
    }
    let normal_sequences = old_wire.lock().unwrap().recognized.clone();
    let non_safe_writes: Vec<_> = normal_sequences
        .iter()
        .enumerate()
        .filter_map(|(index, request)| match request {
            UnknownThermalRequest::WritePower(raw) if *raw != [0, 0] => Some((index, *raw)),
            _ => None,
        })
        .collect();
    assert_eq!(non_safe_writes.len(), 1, "{normal_sequences:#?}");
    assert!(matches!(
        normal_sequences.get(non_safe_writes[0].0 + 1),
        Some(UnknownThermalRequest::ReadbackPower)
    ));

    let paused = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"pause","op":"controller_pause",
                "request_id":{"scope":scope,"seq":"4"},
                "args":{"controller":UNKNOWN_THERMAL_CONTROLLER_ID.to_string()}}),
        ),
    );
    assert_eq!(paused[1]["result"]["state"], "paused");
    loop {
        service.host.service(&service.clock).unwrap();
        if service.host.configured_physical_outputs_safe().unwrap() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let replacement_wire = Arc::new(Mutex::new(UnknownThermalWire::new(220)));
    let device_wire = replacement_wire.clone();
    let reconnect = service
        .reconnect_resource_with_factory(7, 1, move |settings, _| {
            let device_wire = device_wire.clone();
            ComTransport::with_device_factory(settings, move || {
                Ok(Box::new(UnknownThermalDevice(device_wire.clone())))
            })
        })
        .unwrap();
    assert_eq!(reconnect.binding_generation, 2);
    let resource = configuration_api::resource_json(&service, 7).unwrap();
    assert_eq!(resource["binding_generation"], "2");
    assert_eq!(resource["transport_generation"], "2");
    assert_eq!(resource["instruments"], serde_json::json!([instrument_id]));
    let rebound_binding = service
        .host
        .prepared_simple_binding(InstrumentId::new(UNKNOWN_THERMAL_INSTRUMENT_ID))
        .unwrap();
    assert_eq!(rebound_binding.binding_generation, 2);
    assert_eq!(rebound_binding.mapping_revision, 2);

    service
        .host
        .start_recording("M16.7 unknown device after reconnect", service.clock.now())
        .unwrap();
    while service.host.recording_status().unwrap().state != RecordingState::Recording {
        service.host.service(&service.clock).unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }

    {
        let mut stale = old_wire.lock().unwrap();
        stale.enqueue_crc(&[
            0xa1,
            UNKNOWN_THERMAL_ADDRESS,
            UNKNOWN_THERMAL_CHANNEL,
            0x03,
            0xe7,
        ]);
        stale.enqueue_crc(&[0xb1, UNKNOWN_THERMAL_ADDRESS, 0x00]);
        stale.enqueue_crc(&[
            0xb2,
            UNKNOWN_THERMAL_ADDRESS,
            UNKNOWN_THERMAL_CHANNEL,
            0x03,
            0xe7,
        ]);
    }
    loop {
        service.host.service(&service.clock).unwrap();
        if matches!(
            service.owner().query(Query::GetLatestSignal(temperature)),
            Ok(QueryResult::Latest(Some(sample)))
                if sample.quality() == SampleQuality::Good
                    && sample.value() == Some(&lab_core::Value::Float(22.0))
        ) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let reconnect_discovery = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"reconnect-discovery",
                "op":"discover","args":{}})),
    );
    let mut reconnect_records: Vec<_> = reconnect_discovery[0]["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| {
            (record["kind"] == "instrument" && record["id"] == instrument_id)
                || (record["kind"] == "signal"
                    && record["id"]
                        == serde_json::json!({"instrument":instrument_id,"parameter":"1"}))
        })
        .cloned()
        .collect();
    reconnect_records.sort_by_key(|record| if record["kind"] == "instrument" { 0 } else { 1 });
    let reconnect_current = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"reconnect-current",
                "op":"measurements_current","args":{}})),
    );
    let mut reconnect_current: Vec<_> = reconnect_current[0]["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| {
            record["signal"] == serde_json::json!({"instrument":instrument_id,"parameter":"1"})
        })
        .cloned()
        .collect();
    let mut reconnect_event = None;
    for _ in 0..8 {
        let batch = application.pump_events(&service, 1);
        if let Some(event) = batch.into_iter().find(|event| {
            event["kind"] == "signal"
                && event["target"]
                    == serde_json::json!({"instrument":instrument_id,"parameter":"1"})
                && event["data"]["generation"] == "2"
                && event["data"]["value"] == 22.0
        }) {
            reconnect_event = Some(serde_json::json!({
                "kind":event["kind"],
                "target":event["target"],
                "data":event["data"]
            }));
            break;
        }
    }
    let mut reconnect_event = reconnect_event.expect("replacement-generation signal event");
    let normalize_time = |sample: &mut serde_json::Value| {
        for field in ["observed_at", "observed_at_ns", "source_at_ns"] {
            sample[field] = serde_json::json!("3000000000");
        }
    };
    normalize_time(&mut reconnect_current[0]);
    normalize_time(&mut reconnect_event["data"]);
    let captured_application_shape = serde_json::json!({
        "records":reconnect_records,
        "current":reconnect_current,
        "event":reconnect_event
    });
    let workbench_fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../test-data/m16_unknown_reconnect_application.json"
    ))
    .unwrap();
    assert_eq!(captured_application_shape, workbench_fixture);
    let rebound_latest = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"rebound-latest","op":"latest",
                "args":{"signal":{"instrument":instrument_id,"parameter":"1"}}}),
        ),
    );
    assert_eq!(rebound_latest[0]["result"]["value"], 22.0);
    let rebound_controller = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"rebound-controller","op":"controller",
                "args":{"controller":UNKNOWN_THERMAL_CONTROLLER_ID.to_string()}}),
        ),
    );
    assert_eq!(rebound_controller[0]["result"]["state"], "paused");
    let rebound_output = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"rebound-output","op":"output",
                "args":{"actuator":{"instrument":instrument_id,"parameter":"2"}}}),
        ),
    );
    assert!(rebound_output[0]["result"]["owner"].is_null());
    assert_eq!(rebound_output[0]["result"]["safe_confirmed"], true);
    let replacement_sequences = replacement_wire.lock().unwrap().recognized.clone();
    assert!(replacement_sequences.windows(2).any(|pair| {
        pair == [
            UnknownThermalRequest::WritePower([0, 0]),
            UnknownThermalRequest::ReadbackPower,
        ]
    }));

    let (entries, _) = service.host.frozen_activation_entries().unwrap();
    assert!(entries.iter().any(|entry| {
        entry.kind == "simple_device_definition_canonical"
            && Sha256::digest(&entry.content).as_slice() == definition_hash
    }));
    let overlay: serde_json::Value = serde_json::from_slice(
        &entries
            .iter()
            .find(|entry| entry.kind == "runtime_configuration_overlay")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(overlay["source"], "process_local_application_candidate");
    assert_eq!(overlay["definition_id"], "m16-unknown-thermal-v1");
    assert_eq!(overlay["definition_version"], 1);
    assert_eq!(overlay["definition_sha256"], hash_hex(definition_hash));
    assert_eq!(overlay["candidate_sha256"], candidate_hash);
    assert_eq!(overlay["instances"], serde_json::json!([instrument_id]));
    let activation_instance: serde_json::Value = serde_json::from_slice(
        &entries
            .iter()
            .find(|entry| {
                entry.kind == "simple_device_instance"
                    && String::from_utf8_lossy(&entry.content)
                        .contains("\"instrument_id\":\"1607\"")
            })
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(activation_instance["binding_generation"], "1");
    assert_eq!(activation_instance["mapping_revision"], "1");

    service.request_shutdown().unwrap();
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let (generation, revision, value): (Vec<u8>, Vec<u8>, f64) = connection
        .query_row(
            "SELECT generation,revision,float_value FROM measurements
                 WHERE instrument_id=?1 AND parameter_id=?2 AND quality='good'
                   AND float_value=?3
                 ORDER BY record_seq DESC LIMIT 1",
            rusqlite::params![
                UNKNOWN_THERMAL_INSTRUMENT_ID.to_be_bytes().as_slice(),
                1u64.to_be_bytes().as_slice(),
                21.5_f64,
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generation, 1u64.to_be_bytes());
    assert_eq!(revision, 1u64.to_be_bytes());
    assert_eq!(value, 21.5);
    let (reconnected_generation, reconnected_revision, reconnected_value): (Vec<u8>, Vec<u8>, f64) =
        connection
            .query_row(
                "SELECT generation,revision,float_value FROM measurements
                 WHERE instrument_id=?1 AND parameter_id=?2 AND quality='good'
                   AND float_value=?3
                 ORDER BY record_seq DESC LIMIT 1",
                rusqlite::params![
                    UNKNOWN_THERMAL_INSTRUMENT_ID.to_be_bytes().as_slice(),
                    1u64.to_be_bytes().as_slice(),
                    22.0_f64,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    assert_eq!(reconnected_generation, 2u64.to_be_bytes());
    assert_eq!(reconnected_revision, 2u64.to_be_bytes());
    assert_eq!(reconnected_value, 22.0);
    let instance_content: String = connection
        .query_row(
            "SELECT CAST(content AS TEXT) FROM provenance_content
                 WHERE kind='simple_device_instance'
                   AND CAST(content AS TEXT) LIKE '%\"instrument_id\":\"1607\"%'
                 ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let instance: serde_json::Value = serde_json::from_str(&instance_content).unwrap();
    assert_eq!(instance["definition_id"], "m16-unknown-thermal-v1");
    assert_eq!(instance["source"], "process_local_application_candidate");
    assert_eq!(instance["resource_id"], "7");
    assert_eq!(instance["binding_generation"], "1");
    assert_eq!(instance["mapping_revision"], "1");
    let overlay_content: String = connection
        .query_row(
            "SELECT CAST(content AS TEXT) FROM provenance_content
                 WHERE kind='runtime_configuration_overlay'
                 ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let persisted_overlay: serde_json::Value = serde_json::from_str(&overlay_content).unwrap();
    assert_eq!(
        persisted_overlay["source"],
        "process_local_application_candidate"
    );
    assert_eq!(persisted_overlay["definition_id"], "m16-unknown-thermal-v1");
    assert_eq!(persisted_overlay["definition_version"], 1);
    assert_eq!(
        persisted_overlay["definition_sha256"],
        hash_hex(definition_hash)
    );
    assert_eq!(persisted_overlay["candidate_sha256"], candidate_hash);
    assert_eq!(
        persisted_overlay["instances"],
        serde_json::json!([instrument_id])
    );
    let fixture_schema_objects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
                 WHERE name LIKE '%unknown_thermal%' OR sql LIKE '%unknown_thermal%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fixture_schema_objects, 0);
    drop(connection);

    let (mut restarted, restarted_wire) =
        service_for_unknown_thermal("4123456789abcdef0123456789abcdef", 230);
    assert_eq!(restarted.deployment.as_ref().unwrap().revision(), 1);
    assert!(!restarted.api_simple_overlay_active);
    assert!(
        restarted
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(
                UNKNOWN_THERMAL_INSTRUMENT_ID,
            )))
            .is_err()
    );
    let mut restarted_application = Application::new(restarted.boot_id()).unwrap();
    let restarted_hello = restarted_application.handle(
        &mut restarted,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"restart-hello","op":"hello",
                "args":{"scope":null}}),
        ),
    );
    assert!(restarted_hello[0]["result"]["scope"].is_string());
    let restarted_discovery = restarted_application.handle(
        &mut restarted,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"restart-discover","op":"discover",
                "args":{}}),
        ),
    );
    let restarted_records = restarted_discovery[0]["result"]["records"]
        .as_array()
        .unwrap();
    assert!(
        restarted_records
            .iter()
            .any(|record| record["kind"] == "resource" && record["id"] == "7")
    );
    assert!(
        restarted_records
            .iter()
            .any(|record| record["kind"] == "reference"
                && record["id"] == serde_json::json!({"id":"10"}))
    );
    assert!(
        !restarted_records
            .iter()
            .any(|record| { record["kind"] == "instrument" && record["id"] == instrument_id })
    );
    for _ in 0..16 {
        restarted.host.service(&restarted.clock).unwrap();
    }
    assert!(restarted_wire.lock().unwrap().requests.is_empty());

    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
}

#[test]
fn pending_application_simple_apply_fences_property_configuration_without_mutation() {
    let (mut service, _) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1",
                    "candidate":application_read_only_candidate(1002,2)}})),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    let pending = service.pending_simple_apply.as_ref().unwrap();
    let pending_id = pending.candidate.snapshot.id();
    let pending_phase = pending.phase;
    let pending_deadline = pending.deadline;

    let rejected = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"property",
                "op":"property_configure",
                "request_id":{"scope":scope,"seq":"3"},
                "args":{"target":{"kind":"instrument","id":"1001"},
                    "property":"poll_period_ms","value":200,"expected_revision":"1"}})),
    );
    assert_eq!(rejected.len(), 2);
    assert_eq!(rejected[0]["state"], "accepted");
    assert_eq!(rejected[1]["state"], "failed");
    assert_eq!(rejected[1]["code"], "busy");
    assert_eq!(rejected[1]["category"], "capacity_exhausted");
    assert_eq!(rejected[1]["retryable"], true);
    assert_eq!(rejected[1]["resync_required"], false);
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert!(service.deployment.as_ref().unwrap().staged().is_none());
    let active = &service
        .deployment
        .as_ref()
        .unwrap()
        .active()
        .effective()
        .dto;
    assert!(matches!(
        &active.instruments[0],
        crate::configuration::InstrumentDto::SimpleDevice {
            poll_period_ms: 100,
            ..
        }
    ));
    let pending = service.pending_simple_apply.as_ref().unwrap();
    assert_eq!(pending.candidate.snapshot.id(), pending_id);
    assert_eq!(pending.phase, pending_phase);
    assert_eq!(pending.deadline, pending_deadline);

    let terminal = loop {
        service.host.service(&service.clock).unwrap();
        let replies = application.poll_configuration(&mut service);
        if !replies.is_empty() {
            break replies;
        }
        std::thread::yield_now();
    };
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    assert!(application.poll_configuration(&mut service).is_empty());
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 2);
}

#[test]
fn prepared_owner_failure_is_operation_failed_without_partial_publication() {
    let (mut service, _) = service_for_api_output(false);
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1","candidate":application_output_candidate()}})),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    while service.pending_simple_apply.as_ref().unwrap().phase != SimpleApplyPhase::EstablishSafe {
        assert!(application.poll_configuration(&mut service).is_empty());
    }
    service
        .host
        .discard_prepared_simple_overlay(vec![InstrumentId::new(2001)])
        .unwrap();
    let terminal = application.poll_configuration(&mut service);
    assert_eq!(terminal.len(), 1);
    let terminal = &terminal[0].1;
    assert_eq!(terminal["state"], "failed");
    assert_eq!(terminal["code"], "operation_failed");
    assert_eq!(terminal["category"], "operation_failed");
    assert_eq!(terminal["retryable"], false);
    assert_eq!(terminal["resync_required"], false);
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert!(service.quarantined_simple_output.is_none());
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(2001)))
            .is_err()
    );
}

#[test]
fn read_only_apply_deadline_is_operation_failed_without_output_quarantine() {
    let (mut service, _) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1",
                    "candidate":application_read_only_candidate(1002,2)}})),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    service.pending_simple_apply.as_mut().unwrap().deadline = Duration::ZERO;
    let terminal = application.poll_configuration(&mut service);
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "failed");
    assert_eq!(terminal[0].1["code"], "operation_failed");
    assert_ne!(terminal[0].1["code"], "output_rejected");
    assert!(service.quarantined_simple_output.is_none());
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert!(
        service
            .owner()
            .query(lab_core::Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );
}

#[test]
fn recorder_reservation_deadline_is_recording_unavailable() {
    let (mut service, _) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1",
                    "candidate":application_read_only_candidate(1002,2)}})),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    assert!(application.poll_configuration(&mut service).is_empty());
    assert_eq!(
        service.pending_simple_apply.as_ref().unwrap().phase,
        SimpleApplyPhase::ReserveRecording
    );
    service.pending_simple_apply.as_mut().unwrap().deadline = Duration::ZERO;
    let terminal = application.poll_configuration(&mut service);
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "failed");
    assert_eq!(terminal[0].1["code"], "recording_unavailable");
    assert_eq!(terminal[0].1["category"], "recording_unavailable");
    assert_eq!(terminal[0].1["retryable"], true);
    assert_eq!(terminal[0].1["resync_required"], false);
    assert!(service.quarantined_simple_output.is_none());
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
}

#[test]
fn m16_6_required_recorder_durability_failure_preserves_prior_deployment() {
    let (mut service, _) = service_with_simple_transport();
    let prior = service.deployment.as_ref().unwrap().active().clone();
    let database = temporary_database();
    drop(crate::recorder::SqliteStore::open(&database).unwrap());
    let injection = rusqlite::Connection::open(&database).unwrap();
    injection
        .execute_batch(
            "CREATE TRIGGER m16_6_fail_configuration BEFORE INSERT ON configurations
                 WHEN NEW.activation_no=X'0000000000000002'
                 BEGIN SELECT RAISE(ABORT,'m16.6 injected activation durability failure'); END;",
        )
        .unwrap();
    drop(injection);
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::Required, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();

    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::RecordingUnavailable)
    );
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert_eq!(service.deployment.as_ref().unwrap().active(), &prior);
    assert!(!service.api_simple_overlay_active);
    assert!(service.pending_simple_apply.is_none());
    assert!(service.quarantined_simple_output.is_none());
    assert!(
        service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1001)))
            .is_ok()
    );
    assert!(
        service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );
    drop(service);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(format!("{}-wal", database.display()));
    let _ = std::fs::remove_file(format!("{}-shm", database.display()));
}

#[test]
fn insufficient_event_capacity_fails_before_durable_activation() {
    let (mut service, _) = service_with_simple_transport();
    let prior = service.deployment.as_ref().unwrap().active().clone();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let _ = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let database = temporary_database();
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::BestEffort, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();
    let before = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"before","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert!(
        !before
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == serde_json::json!({"id":"2002"}))
    );
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        // This read-only candidate needs Recorder/lifecycle progress only.
        // Scheduling the old transport here can independently change its
        // discovery status, obscuring the no-publication assertion below.
        let _ = service.poll_simple_device_apply();
        if service
            .pending_simple_apply
            .as_ref()
            .is_some_and(|pending| pending.phase == SimpleApplyPhase::Commit)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    service
        .host
        .event_log_mut()
        .force_sequence_for_test(u64::MAX);
    let completion = service.poll_simple_device_apply().unwrap();
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::OwnerFailure)
    );
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    assert_eq!(service.deployment.as_ref().unwrap().active(), &prior);
    assert!(!service.api_simple_overlay_active);
    assert!(
        service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_err()
    );
    let after = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"after","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert_eq!(after, before);
    drop(service);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let activations: i64 = connection
        .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(activations, 1);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(format!("{}-wal", database.display()));
    let _ = std::fs::remove_file(format!("{}-shm", database.display()));
}

#[test]
fn prepared_read_apply_failure_starts_no_physical_work_and_cleans_topology() {
    let (mut service, wire) = service_for_api_output(false);
    let prior_revision = service.deployment.as_ref().unwrap().revision();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let _ = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let before = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"before","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert!(
        !before
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == serde_json::json!({"id":"2002"}))
    );
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(2002, 2))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, prior_revision)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), prior_revision)
        .unwrap();
    let before_sequence = service.host.event_log().latest_cursor();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        service.host.service(&service.clock).unwrap();
        let _ = service.poll_simple_device_apply();
        assert!(
            wire.lock()
                .unwrap()
                .writes
                .iter()
                .all(|bytes| bytes.first() != Some(&0x10))
        );
        assert!(
            service
                .owner()
                .query(Query::DescribeInstrument(InstrumentId::new(2002)))
                .is_err()
        );
        if service
            .pending_simple_apply
            .as_ref()
            .is_some_and(|pending| pending.phase == SimpleApplyPhase::Commit)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    service
        .host
        .event_log_mut()
        .force_sequence_for_test(u64::MAX);
    let completion = service.poll_simple_device_apply().unwrap();
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::OwnerFailure)
    );
    assert!(service.pending_simple_apply.is_none());
    assert_eq!(
        service.deployment.as_ref().unwrap().revision(),
        prior_revision
    );
    assert!(!service.api_simple_overlay_active);
    assert!(
        service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(2002)))
            .is_err()
    );
    assert!(wire.lock().unwrap().writes.is_empty());
    let after = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"after","op":"discover","args":{}})),
    )[0]["result"]["records"]
        .clone();
    assert!(
        !after
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == serde_json::json!({"id":"2002"}))
    );

    service
        .host
        .event_log_mut()
        .force_sequence_for_test(before_sequence);
    let staged = service
        .stage_simple_device_candidate(&candidate, prior_revision)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), prior_revision)
        .unwrap();
    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(completion.result, Ok(prior_revision + 1));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        service.host.service(&service.clock).unwrap();
        if wire
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|bytes| bytes.first() == Some(&0x10))
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn durable_publication_consumes_held_event_capacity_after_unrelated_activity() {
    let (mut service, _) = service_with_simple_transport();
    let database = temporary_database();
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::BestEffort, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();
    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        service.host.service(&service.clock).unwrap();
        let _ = service.poll_simple_device_apply();
        if service
            .pending_simple_apply
            .as_ref()
            .is_some_and(|pending| pending.phase == SimpleApplyPhase::Commit)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    service
        .host
        .event_log_mut()
        .force_sequence_for_test(u64::MAX - 2);
    assert!(service.poll_simple_device_apply().is_none());
    let generation = service
        .pending_simple_apply
        .as_ref()
        .and_then(|pending| pending.recording.as_ref())
        .and_then(|recording| recording.generation)
        .expect("Recorder generation reserved");
    assert_eq!(
        service.pending_simple_apply.as_ref().unwrap().phase,
        SimpleApplyPhase::ConfirmRecording
    );
    service
        .host
        .event_log_mut()
        .configuration_state(Duration::ZERO, serde_json::json!({"unrelated":true}))
        .unwrap();
    assert_eq!(service.host.event_log().latest_cursor(), u64::MAX - 1);
    assert_eq!(
        service.host.event_log_mut().configuration_state(
            Duration::ZERO,
            serde_json::json!({"would_steal_reserved_capacity":true})
        ),
        Err(crate::events::EventError::Exhausted)
    );
    while service
        .host
        .recording_status()
        .is_none_or(|status| status.activation_generation < generation)
    {
        service.host.poll_recorder_for_test(service.clock.now());
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let completion = service.poll_simple_device_apply().unwrap();
    assert_eq!(completion.result, Ok(2));
    assert_eq!(service.host.event_log().latest_cursor(), u64::MAX);
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 2);
    assert!(
        service
            .owner()
            .query(Query::DescribeInstrument(InstrumentId::new(1002)))
            .is_ok()
    );
    let published_facts = service.host.event_log().projection_records();
    assert!(published_facts.iter().any(|record| {
        record["kind"] == "signal"
            && record["target"] == serde_json::json!({"instrument":"1002","parameter":"1"})
    }));
    let replay = service
        .host
        .event_log()
        .scan_after(u64::MAX - 1, 1)
        .unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0]["seq"], u64::MAX.to_string());
    assert_eq!(replay[0]["kind"], "signal");
    let (entries, _) = service.host.frozen_activation_entries().unwrap();
    assert!(entries.iter().any(|entry| {
        entry.kind == "runtime_configuration_overlay"
            && String::from_utf8_lossy(&entry.content)
                .contains("process_local_application_candidate")
    }));
    drop(service);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let activations: i64 = connection
        .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(activations, 2);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(format!("{}-wal", database.display()));
    let _ = std::fs::remove_file(format!("{}-shm", database.display()));
}

#[test]
fn reconnect_is_fenced_while_simple_publication_batch_is_pending() {
    let (mut service, _) = service_for_api_output(false);
    let database = temporary_database();
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::Required, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();

    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(2003, 3))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        service.host.service(&service.clock).unwrap();
        let _ = service.poll_simple_device_apply();
        if service
            .pending_simple_apply
            .as_ref()
            .is_some_and(|pending| pending.phase == SimpleApplyPhase::ConfirmRecording)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        service
            .host
            .configured_resource_generation(ResourceId::new(7)),
        Some(1)
    );
    let reconnect = service.reconnect_resource_with_factory(7, 1, |settings, _| {
        ComTransport::with_device_factory(settings, || {
            Ok(Box::new(SimpleProbeDevice {
                readable: VecDeque::new(),
            }))
        })
    });
    assert_eq!(reconnect, Err(LifecycleOperationError::Capacity));
    assert_eq!(
        service
            .host
            .configured_resource_generation(ResourceId::new(7)),
        Some(1)
    );

    let completion = poll_simple_apply_to_terminal(&mut service);
    assert_eq!(completion.result, Ok(2));
    let reconnect = service
        .reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(SimpleProbeDevice {
                    readable: VecDeque::new(),
                }))
            })
        })
        .unwrap();
    assert_eq!(reconnect.binding_generation, 2);

    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(2004, 4))
            .unwrap();
    let staged = service
        .stage_simple_device_candidate(&candidate, 2)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 2)
        .unwrap();
    let before_sequence = service.host.event_log().latest_cursor();
    loop {
        service.host.service(&service.clock).unwrap();
        let _ = service.poll_simple_device_apply();
        if service
            .pending_simple_apply
            .as_ref()
            .is_some_and(|pending| pending.phase == SimpleApplyPhase::Commit)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    service
        .host
        .event_log_mut()
        .force_sequence_for_test(u64::MAX);
    let completion = service.poll_simple_device_apply().unwrap();
    assert_eq!(
        completion.result,
        Err(LifecycleOperationError::OwnerFailure)
    );
    service
        .host
        .event_log_mut()
        .force_sequence_for_test(before_sequence);
    let reconnect = service
        .reconnect_resource_with_factory(7, 2, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(SimpleProbeDevice {
                    readable: VecDeque::new(),
                }))
            })
        })
        .unwrap();
    assert_eq!(reconnect.binding_generation, 3);
    drop(service);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(format!("{}-wal", database.display()));
    let _ = std::fs::remove_file(format!("{}-shm", database.display()));
}

#[test]
fn post_send_apply_deadline_is_output_rejected_and_keeps_quarantine() {
    let (mut service, wire) = service_for_api_output(false);
    wire.lock().unwrap().withhold_ack = true;
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].clone();
    let staged = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"stage",
                "op":"stage_simple_device_candidate",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"expected_revision":"1","candidate":application_output_candidate()}})),
    );
    let candidate_id = staged[1]["result"]["candidate_id"].clone();
    let accepted = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"apply",
                "op":"apply_configuration",
                "request_id":{"scope":scope,"seq":"2"},
                "args":{"candidate_id":candidate_id,"expected_revision":"1"}})),
    );
    assert_eq!(accepted.len(), 1);
    while service.pending_simple_apply.as_ref().unwrap().phase != SimpleApplyPhase::EstablishSafe {
        assert!(application.poll_configuration(&mut service).is_empty());
    }
    let actuator = service
        .pending_simple_apply
        .as_ref()
        .unwrap()
        .actuator
        .unwrap();
    loop {
        service.host.service(&service.clock).unwrap();
        if service
            .host
            .prepared_simple_output(actuator)
            .is_some_and(|snapshot| snapshot.sent.is_some())
        {
            break;
        }
        assert!(application.poll_configuration(&mut service).is_empty());
    }
    let writes_before = wire.lock().unwrap().writes.len();
    service.pending_simple_apply.as_mut().unwrap().deadline = Duration::ZERO;
    let terminal = application.poll_configuration(&mut service);
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "failed");
    assert_eq!(terminal[0].1["code"], "output_rejected");
    assert!(service.quarantined_simple_output.is_some());
    assert_eq!(service.deployment.as_ref().unwrap().revision(), 1);
    for _ in 0..20 {
        service.host.service(&service.clock).unwrap();
    }
    assert_eq!(wire.lock().unwrap().writes.len(), writes_before);
}

#[test]
fn application_overlay_activation_is_durable_process_local_provenance() {
    let (mut service, _) = service_with_simple_transport();
    let database = temporary_database();
    let anchor =
        TimeAnchor::capture(|| service.clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let recorder = RecorderWorker::open_with_boot_clock(
        &database,
        RecorderLimits::default(),
        service.boot_id(),
        anchor,
        service.clock,
    )
    .unwrap();
    service
        .host
        .attach_recorder(recorder, RecordingPolicy::Required, service.clock.now())
        .unwrap();
    await_recorder_activation(&mut service.host).unwrap();

    let candidate =
        crate::simple_device::parse_simple_candidate(&application_read_only_candidate(1002, 2))
            .unwrap();
    let canonical_hash = candidate.definition.canonical_sha256;
    let candidate_hash: [u8; 32] = Sha256::digest(&candidate.canonical).into();
    let staged = service
        .stage_simple_device_candidate(&candidate, 1)
        .unwrap();
    service
        .begin_simple_device_apply(staged.staged.id(), 1)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        service.host.service(&service.clock).unwrap();
        if let Some(completion) = service.poll_simple_device_apply() {
            assert_eq!(completion.result, Ok(2));
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let (entries, objects) = service.host.frozen_activation_entries().unwrap();
    let canonical_hex: String = canonical_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let candidate_hex: String = candidate_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.kind == "simple_device_definition_canonical")
            .count(),
        1,
        "one immutable definition must be shared rather than copied per instance"
    );
    let canonical_index = entries
        .iter()
        .position(|entry| entry.kind == "simple_device_definition_canonical")
        .unwrap();
    assert_eq!(
        entries[canonical_index].content,
        candidate.definition.canonical.as_ref()
    );
    assert!(entries.iter().any(|entry| {
        entry.kind == "rust_build" && entry.content == env!("CARGO_PKG_VERSION").as_bytes()
    }));
    assert!(
        entries
            .iter()
            .any(|entry| { entry.kind == "runtime_toml" && entry.content == SIMPLE_CONFIG })
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.kind == "native_composition")
    );
    let overlay = entries
        .iter()
        .find(|entry| entry.kind == "runtime_configuration_overlay")
        .unwrap();
    let overlay: serde_json::Value = serde_json::from_slice(&overlay.content).unwrap();
    assert_eq!(overlay["source"], "process_local_application_candidate");
    assert_eq!(overlay["definition_id"], "simple-v1");
    assert_eq!(overlay["definition_version"], 1);
    assert_eq!(overlay["definition_sha256"], canonical_hex);
    assert_eq!(overlay["candidate_sha256"], candidate_hex);
    assert_eq!(overlay["instances"], serde_json::json!(["1002"]));
    let api_instance = entries
        .iter()
        .find(|entry| {
            entry.kind == "simple_device_instance"
                && String::from_utf8_lossy(&entry.content).contains("\"instrument_id\":\"1002\"")
        })
        .unwrap();
    let api_instance: serde_json::Value = serde_json::from_slice(&api_instance.content).unwrap();
    assert_eq!(
        api_instance["source"],
        "process_local_application_candidate"
    );
    assert_eq!(api_instance["definition_id"], "simple-v1");
    assert_eq!(api_instance["definition_version"], 1);
    assert_eq!(api_instance["definition_sha256"], canonical_hex);
    assert_eq!(api_instance["configuration_revision"], "2");
    assert_eq!(api_instance["resource_id"], "7");
    assert_eq!(api_instance["binding_generation"], "1");
    assert_eq!(api_instance["mapping_revision"], "1");
    let api_object = objects
        .iter()
        .find(|object| object.kind == "instrument" && object.id == 1002u64.to_be_bytes())
        .unwrap();
    assert_eq!(api_object.definition_entry_index, canonical_index);
    let object_binding: serde_json::Value =
        serde_json::from_str(api_object.binding.as_ref().unwrap()).unwrap();
    assert_eq!(object_binding["r"], "7");
    assert_eq!(object_binding["b"], "1");
    assert_eq!(object_binding["m"], "1");
    let object_descriptor: serde_json::Value =
        serde_json::from_str(&api_object.descriptor).unwrap();
    assert_eq!(
        object_descriptor["simple_device"]["definition_id"],
        "simple-v1"
    );
    assert_eq!(
        object_descriptor["simple_device"]["canonical_sha256"],
        canonical_hex
    );
    service.request_shutdown().unwrap();
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let instance_content: String = connection
        .query_row(
            "SELECT CAST(content AS TEXT) FROM provenance_content
                 WHERE kind='simple_device_instance'
                   AND CAST(content AS TEXT) LIKE '%\"instrument_id\":\"1002\"%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let instance: serde_json::Value = serde_json::from_str(&instance_content).unwrap();
    assert_eq!(instance["source"], "process_local_application_candidate");
    assert_eq!(instance["configuration_revision"], "2");
    assert_eq!(instance["resource_id"], "7");
    let overlay_content: String = connection
        .query_row(
            "SELECT CAST(content AS TEXT) FROM provenance_content
                 WHERE kind='runtime_configuration_overlay'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let overlay: serde_json::Value = serde_json::from_str(&overlay_content).unwrap();
    assert_eq!(overlay["source"], "process_local_application_candidate");
    assert_eq!(overlay["definition_id"], "simple-v1");
    assert_eq!(overlay["definition_version"], 1);
    assert_eq!(overlay["definition_sha256"], canonical_hex);
    assert_eq!(overlay["candidate_sha256"], candidate_hex);
    assert_eq!(overlay["instances"], serde_json::json!(["1002"]));
    let canonical_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provenance_content
                 WHERE kind='simple_device_definition_canonical'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canonical_count, 1);
    for kind in [
        "runtime_toml",
        "rust_build",
        "native_composition",
        "deployment_config",
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM provenance_content WHERE kind=?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing or duplicated {kind} provenance");
    }
    let object_id = 1002u64.to_be_bytes();
    let (definition_hash, binding, descriptor): (Vec<u8>, String, String) = connection
        .query_row(
            "SELECT definition_hash,instance_binding,descriptor
                 FROM object_snapshots WHERE object_kind='instrument' AND object_id=?1
                 ORDER BY activation_no DESC LIMIT 1",
            [object_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(definition_hash, canonical_hash);
    let binding: serde_json::Value = serde_json::from_str(&binding).unwrap();
    assert_eq!(binding["r"], "7");
    assert_eq!(binding["b"], "1");
    assert_eq!(binding["m"], "1");
    let descriptor: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
    assert_eq!(descriptor["simple_device"]["configuration_revision"], "2");
    drop(connection);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
}

#[test]
fn simple_only_resource_projects_and_reconnects_through_the_generic_resource_api() {
    let (mut service, shutdown_calls) = service_with_simple_transport();
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    assert!(hello[0]["result"]["scope"].is_string());
    let current = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"resource","op":"resource",
                "args":{"resource":"7"}}),
        ),
    );
    let resource = &current[0]["result"];
    assert_eq!(resource["resource"], "7");
    assert_eq!(resource["binding_generation"], "1");
    assert_eq!(resource["transport_generation"], "1");
    assert_eq!(resource["instruments"], serde_json::json!(["1001"]));
    assert_eq!(resource["configuration_revision"], "1");
    assert_eq!(resource["capabilities"]["reconnect"], true);
    assert_eq!(resource["capabilities"]["configuration"], true);

    let result = service
        .reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(SimpleProbeDevice {
                    readable: VecDeque::new(),
                }))
            })
        })
        .unwrap();
    assert_eq!(result.binding_generation, 2);
    assert!(shutdown_calls.load(Ordering::Acquire) > 0);
    assert_eq!(
        service.reconnect_diagnostic().unwrap().stage,
        ReconnectStage::Complete
    );
    assert_eq!(
        service
            .owner()
            .configured_resource_generation(ResourceId::new(7)),
        Some(2)
    );
    let signal = SignalId::new(InstrumentId::new(1001), ParameterId::new(1));
    let QueryResult::Latest(Some(sample)) = service
        .owner()
        .query(Query::GetLatestSignal(signal))
        .unwrap()
    else {
        panic!("reconnected simple signal missing");
    };
    assert_eq!(sample.quality(), SampleQuality::Unavailable);

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let sample = loop {
        service.host.service(&service.clock).unwrap();
        let QueryResult::Latest(Some(sample)) = service
            .owner()
            .query(Query::GetLatestSignal(signal))
            .unwrap()
        else {
            panic!("reconnected simple signal missing");
        };
        if sample.quality() == SampleQuality::Good {
            break sample;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(10.0)));
}

fn assert_required_healthy_and_no_outputs(service: &ServiceHost) {
    let status = service.owner().recording_status().unwrap();
    assert_ne!(status.state, RecordingState::Failed);
    assert!(status.first_error.is_none());
    assert!(service.owner().output_safe_records().is_empty());
}

fn shutdown_and_remove(mut service: ServiceHost, database: PathBuf) -> ShutdownStatus {
    service.request_shutdown().unwrap();
    let status = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        std::thread::yield_now();
    };
    if !status.recorder_flushed {
        eprintln!(
            "shutdown evidence: {status:?}; Recorder: {:?}",
            service.owner().recording_status()
        );
    }
    drop(service);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
    status
}

#[test]
fn semantic_resource_identity_matches_discovery_current_and_event_without_raw_handle() {
    let (mut service, database) = service_with_old_transport(false);
    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    assert_eq!(hello[0]["type"], "result");
    let current = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"resource","op":"resource",
                "args":{"resource":"7"}}),
        ),
    );
    let state = &current[0]["result"];
    assert_eq!(state["resource"], "7");
    assert_eq!(state["identity_class"], "physical");
    assert_eq!(state["physical"], true);
    assert_eq!(state["virtual"], false);
    assert_eq!(state["binding_generation"], "1");
    assert_eq!(state["instruments"], serde_json::json!(["11"]));
    assert_eq!(state["capabilities"]["reconnect"], true);
    assert_eq!(state["capabilities"]["configuration"], true);
    assert!(state.get("handle").is_none());

    let discovery = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"discover","op":"discover","args":{}})),
    );
    let resource = discovery[0]["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["kind"] == "resource")
        .unwrap();
    assert_eq!(resource["id"], "7");
    assert_eq!(resource["state"]["resource"], "7");

    let snapshot = configuration_api::resource_json(&service, 7).unwrap();
    let at = service.clock().now();
    service
        .owner_mut()
        .event_log_mut()
        .resource_state(at, 7, snapshot)
        .unwrap();
    let event = service
        .owner()
        .event_log()
        .scan_after(0, 1024)
        .unwrap()
        .into_iter()
        .find(|event| event["kind"] == "resource")
        .unwrap();
    assert_eq!(event["target"]["id"], "7");
    assert_eq!(event["data"]["resource"], "7");

    drop(application);
    let status = shutdown_and_remove(service, database);
    assert!(status.safe_confirmed);
}

#[test]
fn physical_signal_never_advertises_or_accepts_emulator_publication() {
    let (mut service, database) = service_with_old_transport(false);
    let signal = lab_core::SignalId::new(
        lab_core::InstrumentId::new(11),
        lab_core::ParameterId::new(2),
    );
    let before = service
        .owner()
        .query(lab_core::Query::GetLatestSignal(signal))
        .unwrap();
    assert!(!service.owner().emulator_writable(signal));
    assert!(!service.protocol_features().emulator_publication);

    let mut application = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
    );
    assert!(
        !hello[0]["result"]["operations"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("emulator_publish"))
    );
    let rejected = application.handle(
        &mut service,
        1,
        request(
            serde_json::json!({"v":1,"msg_id":"publish","op":"emulator_publish",
                "request_id":{"scope":hello[0]["result"]["scope"],"seq":"1"},
                "args":{"signal":{"instrument":"11","parameter":"2"},"state":"good",
                    "value":25.0,"expected_generation":"1"}}),
        ),
    );
    assert_eq!(rejected[0]["code"], "unsupported_operation");
    assert_eq!(
        service
            .owner()
            .query(lab_core::Query::GetLatestSignal(signal))
            .unwrap(),
        before
    );
    assert_required_healthy_and_no_outputs(&service);
    drop(application);
    let status = shutdown_and_remove(service, database);
    assert!(status.safe_confirmed);
}

#[test]
fn reconnect_retirement_timeout_reports_exact_pre_rebind_stage() {
    let (mut service, database) = service_with_old_transport(true);

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
        Err(LifecycleOperationError::TransportUnavailable)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::RetireOldTimeout);
    assert!(!diagnostic.old_worker_finished);
    assert!(!diagnostic.replacement_worker_spawned);
    assert!(!diagnostic.os_port_open_confirmed);
    assert!(!diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(!status.transports_closed);
    assert!(!status.exit_success);
}

fn review_writer_failure_before_rebind(reserve_before_failure: bool) {
    use crate::recorder::WriterBarrier;
    use lab_core::reference::{ReferenceConfig, ReferenceId};
    let barrier = WriterBarrier::held();
    struct Release(WriterBarrier);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    let release = Release(barrier.clone());
    let (mut service, database) =
        service_with_old_transport_and_writer_barrier(false, Some(barrier));
    let reference = ReferenceId::new(99);
    service
        .host
        .command(lab_core::Command::RegisterReference(
            ReferenceConfig::Fixed {
                id: reference,
                value: 1.0,
                unit: lab_core::Unit::CELSIUS,
            },
        ))
        .unwrap();
    let at = service.clock.now();
    service
        .host
        .start_recording("review reconnect writer failure", at)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while service.host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(std::time::Instant::now() < deadline);
        service.host.poll_recorder_for_test(service.clock.now());
        std::thread::yield_now();
    }
    // A deferred FK fails the real SQL COMMIT, only after the writer barrier
    // is released. No synthetic HostCore Failed state is assigned.
    service
        .host
        .command(lab_core::Command::EvaluateReference {
            reference,
            at: service.clock.now(),
        })
        .unwrap();
    assert!(release.0.wait_until_reached(Duration::from_secs(3)));

    // Execute the real ServiceHost reconnect phases, with a deterministic
    // writer failure between admission and rebind. Their order is identical
    // to reconnect_resource_with_factory; no transport or output contract is changed.
    let active = service.deployment.as_ref().unwrap().active().clone();
    let pending = service
        .begin_resource_recorded_lifecycle("reconnect_resource", &active)
        .unwrap();
    if reserve_before_failure {
        assert!(service.host.reserve_rebind_facts().unwrap());
        service
            .host
            .begin_configured_resource_reconnect(ResourceId::new(7))
            .unwrap();
        assert!(
            service
                .host
                .prepare_configured_transport_replacement(ResourceId::new(7), service.clock.now())
                .unwrap()
        );
    }
    assert_eq!(
        service.host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    assert_eq!(service.host.configured_binding_generation(11), Some(1));
    release.0.release();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        service.host.poll_recorder_for_test(service.clock.now());
        let status = service.host.recording_status().unwrap();
        if status.state == RecordingState::Failed && status.worker_closed {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{status:?}");
        std::thread::yield_now();
    }
    let reservation_after_failure = service.host.reserve_rebind_facts();
    if !reserve_before_failure {
        service
            .host
            .begin_configured_resource_reconnect(ResourceId::new(7))
            .unwrap();
        assert!(
            service
                .host
                .prepare_configured_transport_replacement(ResourceId::new(7), service.clock.now())
                .unwrap()
        );
    }
    let rebound = service.host.rebind_configured_transport(
        ResourceId::new(7),
        Box::new(OldTransport {
            shutdown_calls: Arc::new(AtomicUsize::new(0)),
            never_finishes: false,
        }),
        service.clock.now(),
    );
    let generation = service.host.configured_binding_generation(11);
    let finished = service.finish_recorded_lifecycle_detailed(pending);
    assert!(
        finished.is_err(),
        "the late lifecycle fence still rejects success"
    );
    assert!(
        service
            .host
            .configured_resource_reconnect_quiesced(ResourceId::new(7))
    );
    assert!(
        service
            .host
            .retire_failed_configured_reconnect(ResourceId::new(7), service.clock.now())
            .unwrap()
    );
    service.host.poll_recorder_for_test(service.clock.now());
    let failed_status = service.host.recording_status().unwrap().clone();
    assert_eq!(
        reservation_after_failure,
        Err(lab_core::Error::RecordingUnavailable)
    );
    assert_eq!(
        (
            failed_status.outstanding_groups,
            failed_status.outstanding_records
        ),
        (1, 1),
        "only the submitted, uncommitted fact remains charged; unused lifecycle and fact tokens are cancelled"
    );
    drop(service);
    let db = rusqlite::Connection::open(&database).unwrap();
    let activations: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let generation_two: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM measurements WHERE generation=?1",
            [2u64.to_be_bytes().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((activations, generation_two), (0, 0));
    assert_eq!(
        db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
    eprintln!(
        "review reconnect: reserved_before_failure={reserve_before_failure}, reserve_after={reservation_after_failure:?}, rebind={rebound:?}, generation={generation:?}, lifecycle={finished:?}, recorder={failed_status:?}, sqlite={}",
        database.display()
    );
    assert_eq!(
        generation,
        Some(1),
        "Required Failed must reject before transport replacement and Core generation change"
    );
    assert!(rebound.is_err());
}

#[test]
fn m18_review_required_writer_failure_before_first_rebind_reservation_blocks_effect() {
    review_writer_failure_before_rebind(false);
}

#[test]
fn m18_review_required_writer_failure_after_rebind_reservation_blocks_effect() {
    review_writer_failure_before_rebind(true);
}

#[test]
fn reconnect_worker_spawn_failure_keeps_old_generation_and_recorder_healthy() {
    let (mut service, database) = service_with_old_transport(false);

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
        Err(LifecycleOperationError::TransportUnavailable)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ReplacementWorkerSpawn);
    assert_eq!(diagnostic.serial_error, Some(SerialError::Other));
    assert!(diagnostic.old_worker_finished);
    assert!(!diagnostic.replacement_worker_spawned);
    assert!(!diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
}

#[test]
fn reconnect_retirement_cleanup_failure_is_recorded_without_changing_failure_result() {
    let (mut service, database) = service_with_old_transport(false);
    service.reconnect_cleanup_retire_failure = true;

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(ProbeDevice {
                    readable: VecDeque::new(),
                    channel_type: 2,
                }))
            })
        }),
        Err(LifecycleOperationError::OwnerFailure)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ProbeFailed);
    assert_eq!(
        diagnostic.cleanup_failure,
        Some("retirement_injected_failure")
    );
    assert!(diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(2));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.recorder_flushed);
}

#[test]
fn reconnect_retirement_service_failure_is_recorded_without_granting_success() {
    let (mut service, database) = service_with_old_transport(false);
    service.reconnect_cleanup_service_failure = true;
    let retirement_release = Arc::new(AtomicBool::new(false));

    // A fast worker can retire before cleanup needs a service turn. Hold the
    // observable retirement boundary so the injected service failure is
    // exercised regardless of worker scheduling, then release it for shutdown.
    let result = service.reconnect_resource_with_factory(7, 1, |settings, _| {
        let mut adapter = ComTransport::with_device_factory(settings, || {
            Ok(Box::new(ProbeDevice {
                readable: VecDeque::new(),
                channel_type: 2,
            }))
        })?;
        adapter.hold_retirement_until(retirement_release.clone());
        Ok(adapter)
    });
    retirement_release.store(true, Ordering::Release);
    assert_eq!(result, Err(LifecycleOperationError::OwnerFailure));
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ProbeFailed);
    assert_eq!(
        diagnostic.cleanup_failure,
        Some("retirement_service_failure")
    );
    assert!(diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(2));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.recorder_flushed);
}

#[test]
fn stale_expected_generation_is_conflict_without_prior_failure_diagnostics() {
    let (mut service, database) = service_with_old_transport(false);
    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
        Err(LifecycleOperationError::TransportUnavailable)
    );
    assert!(service.reconnect_diagnostic().is_some());

    assert_eq!(
        service.reconnect_resource_with_factory(7, 99, |_, _| Err(SerialError::Other)),
        Err(LifecycleOperationError::Conflict)
    );
    assert!(service.reconnect_diagnostic().is_none());
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
}

#[test]
fn asynchronous_candidate_open_failure_never_crosses_generation_fence() {
    let (mut service, database) = service_with_old_transport(false);

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |settings, open_deadline| {
            ComTransport::with_retrying_device_factory(
                settings,
                OpenRetryPolicy::transient_until(open_deadline),
                || Err(SerialError::Disconnected),
            )
        }),
        Err(LifecycleOperationError::TransportUnavailable)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
    assert_eq!(diagnostic.serial_error, Some(SerialError::Disconnected));
    assert!(diagnostic.replacement_worker_spawned);
    assert!(!diagnostic.os_port_open_confirmed);
    assert!(!diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
}

#[test]
fn unfinished_candidate_open_is_quarantined_until_worker_finishes() {
    let (mut service, database) = service_with_old_transport(false);
    let release = Arc::new(AtomicBool::new(false));
    let worker_release = release.clone();

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, move |settings, _| {
            ComTransport::with_device_factory(settings, move || {
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Ok(Box::new(ProbeDevice {
                    readable: VecDeque::new(),
                    channel_type: 3,
                }))
            })
        }),
        Err(LifecycleOperationError::TransportUnavailable)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
    assert_eq!(diagnostic.serial_error, Some(SerialError::Timeout));
    assert!(diagnostic.replacement_worker_spawned);
    assert!(!diagnostic.os_port_open_confirmed);
    assert!(!diagnostic.core_rebind_crossed);
    assert!(service.quarantined_reconnect_candidate.is_some());
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    release.store(true, Ordering::Release);
    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
    assert!(status.exit_success);
}

#[test]
fn failed_probe_is_distinct_and_keeps_installed_generation_quiesced() {
    let (mut service, database) = service_with_old_transport(false);

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(ProbeDevice {
                    readable: VecDeque::new(),
                    channel_type: 2,
                }))
            })
        }),
        Err(LifecycleOperationError::OwnerFailure)
    );
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ProbeFailed);
    assert!(diagnostic.os_port_open_confirmed);
    assert!(diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(2));
    assert!(
        service
            .owner()
            .configured_resource_reconnect_quiesced(ResourceId::new(7))
    );
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
    assert!(status.exit_success);
}

#[test]
fn ready_candidate_installs_once_then_probe_and_lifecycle_release_acquisition() {
    let (mut service, database) = service_with_old_transport(false);

    let result = service
        .reconnect_resource_with_factory(7, 1, |settings, _| {
            ComTransport::with_device_factory(settings, || {
                Ok(Box::new(ProbeDevice {
                    readable: VecDeque::new(),
                    channel_type: 3,
                }))
            })
        })
        .unwrap_or_else(|error| panic!("{error:?}: {:?}", service.reconnect_diagnostic()));
    assert_eq!(result.binding_generation, 2);
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::Complete);
    assert!(diagnostic.old_worker_finished);
    assert!(diagnostic.replacement_worker_spawned);
    assert!(diagnostic.os_port_open_confirmed);
    assert!(diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(2));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
    assert!(status.exit_success);
}

#[test]
fn transient_disconnected_open_retries_before_same_deadline_and_rebinds_once() {
    let (mut service, database) = service_with_old_transport(false);
    let attempts = Arc::new(AtomicUsize::new(0));
    let factory_attempts = attempts.clone();

    let result = service
        .reconnect_resource_with_factory(7, 1, move |settings, open_deadline| {
            ComTransport::with_retrying_device_factory(
                settings,
                OpenRetryPolicy::for_test(open_deadline, Duration::ZERO, 4),
                move || {
                    if factory_attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                        Err(SerialError::Disconnected)
                    } else {
                        Ok(Box::new(ProbeDevice {
                            readable: VecDeque::new(),
                            channel_type: 3,
                        }))
                    }
                },
            )
        })
        .unwrap_or_else(|error| panic!("{error:?}: {:?}", service.reconnect_diagnostic()));

    assert_eq!(attempts.load(Ordering::Acquire), 2);
    assert_eq!(result.binding_generation, 2);
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::Complete);
    assert_eq!(diagnostic.open_attempts, 2);
    assert_eq!(diagnostic.serial_error, Some(SerialError::Disconnected));
    assert!(diagnostic.old_worker_finished);
    assert!(diagnostic.os_port_open_confirmed);
    assert!(diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(2));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
    assert!(status.exit_success);
}

#[test]
fn invalid_settings_open_is_terminal_without_generation_or_probe() {
    let (mut service, database) = service_with_old_transport(false);
    let attempts = Arc::new(AtomicUsize::new(0));
    let factory_attempts = attempts.clone();

    assert_eq!(
        service.reconnect_resource_with_factory(7, 1, move |settings, open_deadline| {
            ComTransport::with_retrying_device_factory(
                settings,
                OpenRetryPolicy::transient_until(open_deadline),
                move || {
                    factory_attempts.fetch_add(1, Ordering::AcqRel);
                    Err(SerialError::InvalidSettings)
                },
            )
        }),
        Err(LifecycleOperationError::TransportUnavailable)
    );

    assert_eq!(attempts.load(Ordering::Acquire), 1);
    let diagnostic = service.reconnect_diagnostic().unwrap();
    assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
    assert_eq!(diagnostic.serial_error, Some(SerialError::InvalidSettings));
    assert_eq!(diagnostic.open_attempts, 1);
    assert!(!diagnostic.os_port_open_confirmed);
    assert!(!diagnostic.core_rebind_crossed);
    assert_eq!(service.owner().configured_binding_generation(11), Some(1));
    assert_required_healthy_and_no_outputs(&service);

    let status = shutdown_and_remove(service, database);
    assert!(status.transports_closed);
    assert!(status.recorder_flushed);
    assert!(status.exit_success);
}
