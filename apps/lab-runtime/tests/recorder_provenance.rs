//! Exact loaded bytes and their SHA-256 survive source-file changes and reopen.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Runtime,
    TEMPERATURE, Unit, Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, ComponentResult,
        ComponentStatus, Invocation, PlainData,
    },
    metakon::crc,
    output::{
        ActuatorId, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal, OutputResult,
        SafeProfile,
    },
    recording::RecordingFact,
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    build_identity::runtime_binary_sha256,
    host::{Clock, HostCore},
    recorder::{
        HistoryFilter, ProvenanceEntry, ProvenanceObject, RecorderLimits, RecorderWorker,
        RecordingPolicy, RecordingState, SqliteStore,
    },
    service::{ServiceHost, ServiceOptions},
};

#[test]
fn recording_boundary_and_ordered_revisions_reconstruct_current_config() {
    let path = temporary_database();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    let host_reference = host.reference_id();
    let host_controller = host.controller_id();
    let host_plant = host.plant_id();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    host.command(Command::RetuneRampReference {
        reference: host.reference_id(),
        target: 57.0,
        rate: 1.5,
        expected_revision: 1,
        at: Duration::ZERO,
    })
    .unwrap();
    host.command(Command::ConfigureControllerPid {
        controller: host.controller_id(),
        pid: lab_core::control::PidConfig {
            kp: 4.5,
            ki: 0.6,
            kd: 0.1,
            output_min: 5.0,
            output_max: 80.0,
        },
        expected_revision: 1,
    })
    .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("current at start", now)
        .unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let retune_at = service.clock().now();
    service
        .owner_mut()
        .command(Command::RetuneRampReference {
            reference: host_reference,
            target: 62.0,
            rate: 2.25,
            expected_revision: 2,
            at: retune_at,
        })
        .unwrap();
    service
        .owner_mut()
        .command(Command::ConfigureControllerPid {
            controller: host_controller,
            pid: lab_core::control::PidConfig {
                kp: 3.25,
                ki: 0.4,
                kd: 0.05,
                output_min: 7.0,
                output_max: 75.0,
            },
            expected_revision: 2,
        })
        .unwrap();
    assert!(
        service
            .owner_mut()
            .command(Command::ConfigureControllerPid {
                controller: host_controller,
                pid: lab_core::control::PidConfig {
                    kp: 99.0,
                    ki: 0.4,
                    kd: 0.05,
                    output_min: 80.0,
                    output_max: 5.0,
                },
                expected_revision: 3,
            })
            .is_err()
    );
    let failure_at = service.clock().now();
    service
        .owner_mut()
        .command(Command::InjectPlantMeasurementFailure {
            instrument: host_plant,
            at: failure_at,
        })
        .unwrap();
    let drained_by = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .is_some_and(|status| status.outstanding_groups > 1)
    {
        assert!(Instant::now() < drained_by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    service.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    let terminal = loop {
        if let Some(terminal) = service.shutdown_step().unwrap() {
            break terminal;
        }
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    };
    assert!(
        terminal.recorder_flushed,
        "{terminal:?} {:?}",
        service.owner().recording_status()
    );
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let payload: Vec<u8> = db
        .query_row(
            "SELECT payload FROM records WHERE kind='boundary_snapshot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let boundary: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(boundary["controller_configurations"][0]["revision"], "2");
    assert_eq!(boundary["controller_configurations"][0]["pid"]["kp"], 4.5);
    assert_eq!(
        boundary["controller_configurations"][0]["pid"]["output_min"],
        5.0
    );
    assert_eq!(boundary["reference_configurations"][0]["revision"], "2");
    assert_eq!(boundary["reference_configurations"][0]["target"], 57.0);
    assert_eq!(boundary["reference_configurations"][0]["rate"], 1.5);
    let reference_revision: (f64, f64, f64) = db
        .query_row(
            "SELECT value,target,rate FROM reference_events WHERE revision=x'0000000000000003'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(reference_revision.1, 62.0);
    assert_eq!(reference_revision.2, 2.25);
    assert!(reference_revision.0.is_finite());
    let controller_revision: (String, String) = db
        .query_row(
            "SELECT event_kind,diagnostics FROM controller_events WHERE config_revision=x'0000000000000003'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(controller_revision.0, "configuration");
    let pid: serde_json::Value = serde_json::from_str(&controller_revision.1).unwrap();
    assert_eq!(pid["kp"], 3.25);
    assert_eq!(pid["output_min"], 7.0);
    let rejected: i64 = db
        .query_row(
            "SELECT count(*) FROM controller_events WHERE config_revision=x'0000000000000004' OR diagnostics LIKE '%99.0%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rejected, 0, "rejected PID candidate cannot become active");
    let after_revision: i64 = db
        .query_row(
            "SELECT count(*) FROM measurements m JOIN records r ON r.boot_id=m.boot_id AND r.record_seq=m.record_seq WHERE m.quality='unavailable' AND r.record_seq>(SELECT record_seq FROM controller_events WHERE config_revision=x'0000000000000003') AND r.record_seq>(SELECT record_seq FROM reference_events WHERE revision=x'0000000000000003')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        after_revision >= 1,
        "committed revisions must precede dependent measurement"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct BindingAckWire {
    readable: VecDeque<u8>,
    device: u8,
    channel: u8,
}
impl ByteTransport for BindingAckWire {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        if bytes.len() == 7 {
            let body = [self.device, self.channel, 6, 1];
            self.readable.extend(body);
            self.readable.push_back(crc(&body));
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
        self.readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

struct TestClock(Duration);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn m3_profile() -> SafeProfile {
    SafeProfile {
        min: 0.0,
        max: 100.0,
        safe_value: 0.0,
        max_lease: Duration::from_secs(1),
        max_proposal_ttl: Duration::from_millis(200),
        required_evidence: EvidenceLevel::Acknowledgement,
    }
}

fn dispatch_m3(host: &mut HostCore, actuator: ActuatorId, at: Duration, value: f64) {
    let acquired = host.command(Command::Output {
        actuator,
        at,
        command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(91),
            lifetime: Duration::from_millis(100),
        },
    });
    let CommandResult::Output(OutputResult::Lease(lease)) = acquired.unwrap_or_else(|error| {
        panic!(
            "M3 acquire for {value} failed: {error}; snapshot={:?}",
            host.query(lab_core::Query::Output(actuator))
        )
    }) else {
        panic!("manual M3 lease absent")
    };
    host.command(Command::Output {
        actuator,
        at,
        command: OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(value),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(50),
        }),
    })
    .unwrap();
    host.command(Command::QueueMetakonOutput {
        actuator,
        at,
        queue_ttl: Duration::from_millis(100),
        timeout: Duration::from_millis(20),
    })
    .unwrap();
    host.command(Command::PollTransports { at }).unwrap();
}

#[test]
fn activation_binding_and_rebound_output_rows_reconstruct_exact_m3_provenance() {
    let path = temporary_database();
    let mut host = HostCore::virtual_demo().unwrap();
    let resource_one = ResourceId::new(41);
    let resource_two = ResourceId::new(42);
    host.register_transport(
        resource_one,
        Box::new(BindingAckWire {
            device: 15,
            channel: 2,
            ..BindingAckWire::default()
        }),
    )
    .unwrap();
    host.register_transport(
        resource_two,
        Box::new(BindingAckWire {
            device: 16,
            channel: 3,
            ..BindingAckWire::default()
        }),
    )
    .unwrap();
    let instrument = InstrumentId::new(741);
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    host.command(Command::RegisterMetakon(MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: instrument,
            name: "binding provenance output".into(),
            parameters: vec![DataParameterDefinition {
                id: parameter,
                name: "power".into(),
                value_spec: ValueSpec::Float {
                    min: 0.0,
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
            resource: resource_one,
            device: 15,
            channel: 2,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: Some(Unit::PERCENT),
        },
        history_capacity: 2,
    }))
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::BindProfile(m3_profile()),
    })
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::RequestSafe,
    })
    .unwrap();
    host.command(Command::QueueMetakonOutput {
        actuator,
        at: Duration::ZERO,
        queue_ttl: Duration::from_millis(100),
        timeout: Duration::from_millis(20),
    })
    .unwrap();
    host.command(Command::PollTransports { at: Duration::ZERO })
        .unwrap();
    let lab_core::QueryResult::Output(initial_output) =
        host.query(lab_core::Query::Output(actuator)).unwrap()
    else {
        panic!("M3 output query changed kind")
    };
    assert!(
        initial_output.safe_confirmed,
        "initial M3 safe ACK absent: {initial_output:?}"
    );

    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    host.service(&TestClock(Duration::ZERO)).unwrap();
    host.start_recording("binding provenance", Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    let lab_core::QueryResult::Output(before_first) =
        host.query(lab_core::Query::Output(actuator)).unwrap()
    else {
        panic!("M3 output query changed kind")
    };
    assert!(
        before_first.safe_confirmed
            && !before_first.fault_latched
            && before_first.lease.is_none()
            && !before_first.pending
            && before_first.in_flight.is_none()
            && before_first.state == lab_core::output::OutputState::Disarmed,
        "M3 output changed before first dispatch: {before_first:?}"
    );
    dispatch_m3(&mut host, actuator, Duration::from_millis(1), 25.0);
    while host.recording_status().unwrap().outstanding_groups != 0 {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::from_millis(1))).unwrap();
        std::thread::yield_now();
    }

    host.command(Command::RebindMetakon {
        instrument,
        binding: MetakonBinding {
            resource: resource_two,
            device: 16,
            channel: 3,
            binding_generation: 2,
            mapping_revision: 2,
            expected_output_unit: Some(Unit::PERCENT),
        },
        at: Duration::from_millis(2),
    })
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::from_millis(2),
        command: OutputCommand::BindProfile(m3_profile()),
    })
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::from_millis(2),
        command: OutputCommand::RequestSafe,
    })
    .unwrap();
    host.command(Command::QueueMetakonOutput {
        actuator,
        at: Duration::from_millis(2),
        queue_ttl: Duration::from_millis(100),
        timeout: Duration::from_millis(20),
    })
    .unwrap();
    for _ in 0..3 {
        host.command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    }
    while host.recording_status().unwrap().outstanding_groups != 0 {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::from_millis(2))).unwrap();
        std::thread::yield_now();
    }
    dispatch_m3(&mut host, actuator, Duration::from_millis(3), 35.0);
    while host.recording_status().unwrap().outstanding_groups != 0 {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::from_millis(3))).unwrap();
        std::thread::yield_now();
    }
    host.stop_recording().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Idle {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::from_millis(3))).unwrap();
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed {
        assert!(Instant::now() < deadline);
        host.service(&TestClock(Duration::from_millis(3))).unwrap();
        std::thread::yield_now();
    }
    drop(host);

    let db = rusqlite::Connection::open(&path).unwrap();
    let binding: Option<String> = db
        .query_row(
            "SELECT instance_binding FROM object_snapshots
             WHERE object_kind='instrument' AND logical_key='instrument:741'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let binding: serde_json::Value =
        serde_json::from_str(binding.as_deref().expect("baseline M3 binding absent")).unwrap();
    assert_eq!(binding["resource"], "41");
    assert_eq!(binding["device"], 15);
    assert_eq!(binding["channel"], 2);
    assert_eq!(binding["binding_generation"], "1");
    assert_eq!(binding["mapping_revision"], "1");
    assert_eq!(binding["expected_output_unit"], Unit::PERCENT.id());

    type OutputBinding = (Vec<u8>, Vec<u8>, Vec<u8>);
    let rows: Vec<OutputBinding> = db
        .prepare(
            "SELECT resource_id,generation,revision FROM output_events
             WHERE instrument_id=?1 AND stage='acknowledged'
             ORDER BY record_seq",
        )
        .unwrap()
        .query_map([instrument.get().to_be_bytes().as_slice()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            (
                resource_one.get().to_be_bytes().to_vec(),
                1u64.to_be_bytes().to_vec(),
                1u64.to_be_bytes().to_vec(),
            ),
            (
                resource_two.get().to_be_bytes().to_vec(),
                2u64.to_be_bytes().to_vec(),
                2u64.to_be_bytes().to_vec(),
            ),
        ]
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-provenance-{suffix}.sqlite"))
}

#[test]
fn serving_activation_records_neutral_lua_and_native_provenance_and_links_the_new_run() {
    let path = temporary_database();
    let path_text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        path_text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let clock = service.clock_copy();
    service
        .owner_mut()
        .start_recording("activation oracle", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    service.request_shutdown().unwrap();
    let shutdown_deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed, "{status:?}");
            break;
        }
        assert!(Instant::now() < shutdown_deadline);
        std::thread::yield_now();
    }
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let source = lab_lua::fixtures::VIRTUAL_MODEL_SOURCE;
    let hash = Sha256::digest(source.as_bytes());
    let exact: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content \
            WHERE kind='managed_component_source' AND content_hash=?1",
            [hash.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exact, source.as_bytes());
    let implementations: Vec<String> = db
        .prepare(
            "SELECT CAST(content AS TEXT) FROM provenance_content \
             WHERE kind='managed_component_implementation' ORDER BY content",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(implementations.len(), 2);
    let implementations: Vec<serde_json::Value> = implementations
        .iter()
        .map(|entry| serde_json::from_str(entry).unwrap())
        .collect();
    assert!(
        implementations
            .iter()
            .any(|entry| entry["implementation"] == "lua.v1"
                && entry["source_content_sha256"]
                    == hex_sha256(Sha256::digest(source.as_bytes()).into()))
    );
    let binary_hash = runtime_binary_sha256().unwrap();
    assert!(implementations.iter().any(|entry| {
        entry["implementation"] == "native.moving_mean.v1"
            && entry["artifact"] == "built_in"
            && entry["config"]["window"]["value"] == 3.0
            && entry["runtime_binary_sha256"] == hex_sha256(binary_hash)
    }));
    let (native_definition_hash, native_source_hash): (Vec<u8>, Option<Vec<u8>>) = db
        .query_row(
            "SELECT definition_hash,source_hash FROM object_snapshots
             WHERE object_kind='managed_component' AND object_id=x'00000000000000CA'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(native_source_hash, None, "BuiltIn cannot claim text source");
    let native_metadata: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content
             WHERE content_hash=?1 AND kind='managed_component_implementation'",
            [native_definition_hash],
            |row| row.get(0),
        )
        .unwrap();
    let native_metadata: serde_json::Value = serde_json::from_slice(&native_metadata).unwrap();
    assert_eq!(native_metadata["implementation"], "native.moving_mean.v1");
    assert_eq!(
        native_metadata["runtime_binary_sha256"],
        hex_sha256(binary_hash)
    );
    let (root, activation): (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT manifest_root_hash,activation_no FROM configurations LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let linked: Vec<u8> = db
        .query_row(
            "SELECT initial_activation_id FROM runs \
        WHERE label='activation oracle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(linked, activation);
    let root_content: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content WHERE content_hash=?1",
            [root.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(Sha256::digest(&root_content).to_vec(), root);
    let object_count: i64 = db
        .query_row(
            "SELECT count(*) FROM object_snapshots \
        WHERE boot_id=(SELECT boot_id FROM runs WHERE label='activation oracle')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        object_count >= 5,
        "committed activation must freeze actual object baseline"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn managed_source_hash_without_the_exact_source_blob_is_rejected_atomically() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    let implementation = ProvenanceEntry {
        kind: "managed_component_implementation".into(),
        encoding: "json_v1".into(),
        content: br#"{"component":"201","implementation":"lua.v1"}"#.to_vec(),
    };
    let object = ProvenanceObject {
        kind: "managed_component",
        id: 201u64.to_be_bytes().to_vec(),
        logical_key: "component:201".into(),
        label: "Managed component 201".into(),
        descriptor: "{}".into(),
        unit_key: None,
        generation: Some(1),
        binding: None,
        definition_entry_index: 0,
        source_content_sha256: Some([0x5a; 32]),
    };
    assert!(
        store
            .commit_activation(&[implementation], &[object])
            .is_err()
    );
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM configurations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn component_source_provenance_survives_source_swaps_without_index_identity() {
    let config = temporary_database().with_extension("toml");
    let database = config.with_extension("sqlite");
    let first_source = config.with_extension("first.lua");
    let second_source = config.with_extension("second.lua");
    let source_a = source_fixture("source A", 21.0);
    let source_b = source_fixture("source B", 42.0);
    std::fs::write(&first_source, &source_a).unwrap();
    std::fs::write(&second_source, &source_b).unwrap();
    let first = first_source.to_string_lossy().replace('\\', "\\\\");
    let second = second_source.to_string_lossy().replace('\\', "\\\\");
    let database_text = database.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        &config,
        format!(
            r#"schema_version=1
[runtime]
key="source-association"
display_name="Source association"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=true
path="{database_text}"
policy="required"
[[managed_components]]
id=201
instrument_id=201
key="first"
display_name="First"
implementation="lua.v1"
source="{first}"
period_ms=100
[[managed_components]]
id=202
instrument_id=202
key="second"
display_name="Second"
implementation="lua.v1"
source="{second}"
period_ms=100
"#
        ),
    )
    .unwrap();

    record_one_configured_activation(&config);
    std::fs::write(&first_source, &source_b).unwrap();
    std::fs::write(&second_source, &source_a).unwrap();
    record_one_configured_activation(&config);

    let db = rusqlite::Connection::open(&database).unwrap();
    let activations: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = db
        .prepare(
            "SELECT boot_id,activation_no,manifest_root_hash FROM configurations ORDER BY rowid",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(activations.len(), 2);
    assert_ne!(activations[0].2, activations[1].2);
    let source_hash = |boot: &[u8], activation: &[u8], component: u64| -> Vec<u8> {
        db.query_row(
            "SELECT source_hash FROM object_snapshots
             WHERE boot_id=?1 AND activation_no=?2
               AND object_kind='managed_component' AND object_id=?3",
            rusqlite::params![boot, activation, component.to_be_bytes().as_slice()],
            |row| row.get(0),
        )
        .unwrap()
    };
    let first_a = source_hash(&activations[0].0, &activations[0].1, 201);
    let first_b = source_hash(&activations[0].0, &activations[0].1, 202);
    let second_b = source_hash(&activations[1].0, &activations[1].1, 201);
    let second_a = source_hash(&activations[1].0, &activations[1].1, 202);
    assert_eq!(first_a, Sha256::digest(source_a.as_bytes()).to_vec());
    assert_eq!(first_b, Sha256::digest(source_b.as_bytes()).to_vec());
    assert_eq!(second_b, first_b);
    assert_eq!(second_a, first_a);
    for (hash, expected) in [(first_a, source_a), (first_b, source_b)] {
        let exact: Vec<u8> = db
            .query_row(
                "SELECT content FROM provenance_content
                 WHERE content_hash=?1 AND kind='managed_component_source' AND encoding='utf8'",
                [hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exact, expected.as_bytes());
    }
    drop(db);
    std::fs::remove_file(config).unwrap();
    std::fs::remove_file(database).unwrap();
    std::fs::remove_file(first_source).unwrap();
    std::fs::remove_file(second_source).unwrap();
}

fn source_fixture(marker: &str, value: f64) -> String {
    format!(
        "-- {marker}\nreturn function(ctx)\n  if ctx.phase == 'init' then return {{state={{}},diagnostics={{}}}} end\n  return {{status='ready',value={value},unit_id=ctx.unit_id,state={{}},diagnostics={{}}}}\nend\n"
    )
}

fn record_one_configured_activation(config: &std::path::Path) {
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

fn hex_sha256(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn provenance_commits_loaded_script_bytes_not_later_path_contents_and_deduplicates() {
    let path = temporary_database();
    let source_path = path.with_extension("lua");
    let loaded = b"return 21.5\n".to_vec();
    std::fs::write(&source_path, &loaded).unwrap();
    let captured = std::fs::read(&source_path).unwrap();
    std::fs::write(&source_path, b"return 99.0\n").unwrap();
    let entry = ProvenanceEntry {
        kind: "managed_lua_source".into(),
        encoding: "utf8".into(),
        content: captured,
    };
    let expected = Sha256::digest(&loaded);
    let mut store = SqliteStore::open(&path).unwrap();
    let single_root = store
        .commit_provenance(std::slice::from_ref(&entry))
        .unwrap();
    let root = store.commit_provenance(&[entry.clone(), entry]).unwrap();
    assert_eq!(
        root, single_root,
        "duplicate loaded content cannot change the manifest root"
    );
    assert_ne!(root, [0u8; 32]);
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM provenance_content", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        rows, 2,
        "activation root JSON is itself immutable stored content"
    );
    let (hash, content): (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT content_hash,content FROM provenance_content WHERE kind='managed_lua_source'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(hash, expected.to_vec());
    assert_eq!(content, loaded);
    let manifest: Vec<u8> = connection
        .query_row(
            "SELECT content FROM provenance_content WHERE kind='activation_manifest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let canonical: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(canonical["encoding_version"], 1);
    assert_eq!(canonical["entries"].as_array().unwrap().len(), 1);
    drop(connection);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(source_path).unwrap();
}

#[test]
fn identical_bytes_in_distinct_provenance_roles_reopen_with_both_typed_references() {
    let path = temporary_database();
    let bytes = b"same trusted bytes".to_vec();
    let hash = Sha256::digest(&bytes);
    let mut store = SqliteStore::open(&path).unwrap();
    let root = store
        .commit_provenance(&[
            ProvenanceEntry {
                kind: "native_definition".into(),
                encoding: "utf8".into(),
                content: bytes.clone(),
            },
            ProvenanceEntry {
                kind: "managed_lua_source".into(),
                encoding: "utf8".into(),
                content: bytes.clone(),
            },
            ProvenanceEntry {
                kind: "native_definition".into(),
                encoding: "opaque".into(),
                content: bytes.clone(),
            },
        ])
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    for (kind, encoding) in [
        ("native_definition", "utf8"),
        ("managed_lua_source", "utf8"),
        ("native_definition", "opaque"),
    ] {
        let stored: Vec<u8> = db
            .query_row(
                "SELECT content FROM provenance_content WHERE content_hash=?1 AND kind=?2 AND encoding=?3",
                rusqlite::params![hash.as_slice(), kind, encoding],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, bytes);
    }
    let manifest: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content WHERE content_hash=?1 AND kind='activation_manifest' AND encoding='json_v1'",
            [root.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    let refs: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(refs["entries"].as_array().unwrap().len(), 3);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn oversized_managed_source_and_manifest_pressure_reject_without_any_activation_rows() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    let large_source = ProvenanceEntry {
        kind: "managed_lua_source".into(),
        encoding: "utf8".into(),
        content: vec![b'a'; 32 * 1024 + 1],
    };
    assert!(store.commit_provenance(&[large_source]).is_err());
    let entry = ProvenanceEntry {
        kind: "native_definition".into(),
        encoding: "utf8".into(),
        content: b"trusted".to_vec(),
    };
    assert!(store.commit_provenance(&[]).is_err());
    assert!(store.commit_provenance(&vec![entry.clone(); 129]).is_err());
    let pressure = (0..16u8)
        .map(|ordinal| ProvenanceEntry {
            kind: "native_definition".into(),
            encoding: "opaque".into(),
            content: vec![ordinal; 64 * 1024],
        })
        .collect::<Vec<_>>();
    assert!(store.commit_provenance(&pressure).is_err());
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    for table in ["provenance_content", "configurations"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} must remain unchanged after rejection");
    }
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[derive(Default)]
struct ComponentMailbox {
    submitted: VecDeque<Invocation>,
    completed: VecDeque<ComponentCompletion>,
}
struct ComponentFake(Arc<Mutex<ComponentMailbox>>);
impl ComponentExecutor for ComponentFake {
    fn try_submit(&mut self, job: Invocation) -> Result<(), lab_core::managed::ComponentError> {
        self.0.lock().unwrap().submitted.push_back(job);
        Ok(())
    }
    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        self.0.lock().unwrap().completed.pop_front()
    }
    fn try_cancel(&mut self, _: lab_core::managed::Correlation) {}
}
fn component_definition(id: u64) -> ComponentDefinition {
    ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: ComponentId::new(id),
            instrument: InstrumentId::new(id),
            name: format!("managed archive {id}"),
            parameter: TEMPERATURE,
            kind: ComponentKind::Source,
            unit: Unit::CELSIUS,
            min: -100.0,
            max: 500.0,
            warmup_samples: 1,
            max_input_age: Duration::from_secs(2),
            history_capacity: 1,
        },
        implementation: ComponentImplementation::text(
            "test.recorder.v1",
            "return function(ctx) return ctx end",
        )
        .unwrap(),
        config: PlainData::default(),
    }
}
fn complete_component(
    mailbox: &Arc<Mutex<ComponentMailbox>>,
    job: Invocation,
    status: ComponentStatus,
    value: Option<f64>,
) {
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: job.correlation,
            timely: true,
            outcome: Ok(ComponentResult {
                status,
                value,
                unit: Unit::CELSIUS,
                state: PlainData::default(),
                diagnostics: vec![],
            }),
        });
}

#[test]
fn replaced_managed_generation_reopens_with_failure_and_ignores_old_late_result() {
    let path = temporary_database();
    let id = ComponentId::new(821);
    let instrument = InstrumentId::new(821);
    let mailbox = Arc::new(Mutex::new(ComponentMailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(ComponentFake(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: component_definition(id.get()),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, init, ComponentStatus::Init, None);
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(Command::InvokeComponent {
            component: id,
            at: Duration::ZERO,
        })
        .unwrap();
    let first = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, first, ComponentStatus::Ready, Some(42.0));
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: id,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let old_pending = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    runtime
        .command(Command::StageComponent {
            definition: component_definition(id.get()),
            replaces: Some(id),
            at: Duration::from_secs(1),
        })
        .unwrap();
    let replacement = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, replacement, ComponentStatus::Init, None);
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    let before_late = runtime.take_recording_facts();
    complete_component(&mailbox, old_pending, ComponentStatus::Ready, Some(999.0));
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert!(
        runtime
            .take_recording_facts()
            .iter()
            .all(|fact| !matches!(fact, RecordingFact::Measurement { .. })),
        "obsolete completion cannot publish any new measurement"
    );
    let measurements: Vec<_> = before_late
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Measurement {
                sample,
                generation,
                revision,
                ..
            } => Some((sample.clone(), *generation, *revision)),
            _ => None,
        })
        .collect();
    assert_eq!(measurements.len(), 2);
    assert_eq!(measurements[0].1, 1);
    assert_eq!(measurements[1].1, 2);
    assert_eq!(
        measurements[1].0.failure(),
        Some(lab_core::MeasurementFailure::ComponentFailure)
    );
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("managed generation archive").unwrap();
    store.append_facts(&before_late).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(2)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let rows = archive
        .read_measurements(instrument, TEMPERATURE, 8)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value, Some(Value::Float(42.0)));
    assert_eq!(rows[0].generation, 1);
    assert_eq!(
        rows[0].revision, 1,
        "definition is immutable within generation"
    );
    assert_eq!(rows[0].state_revision, Some(1));
    assert_eq!(rows[1].generation, 2);
    assert_eq!(rows[1].revision, 1);
    assert_eq!(rows[1].state_revision, Some(0));
    assert_eq!(rows[1].quality, "unavailable");
    assert_eq!(rows[1].failure.as_deref(), Some("ComponentFailure"));
    assert_eq!(rows[1].value, None);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn delayed_managed_transform_reopens_original_upstream_identity_and_state_revision() {
    let path = temporary_database();
    let source_id = ComponentId::new(822);
    let transform_id = ComponentId::new(823);
    let source_signal = lab_core::SignalId::new(InstrumentId::new(822), TEMPERATURE);
    let mut transform = component_definition(transform_id.get());
    transform.manifest.kind = ComponentKind::Transform {
        input: source_signal,
    };
    let mailbox = Arc::new(Mutex::new(ComponentMailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(ComponentFake(mailbox.clone())))
        .unwrap();
    for definition in [component_definition(source_id.get()), transform] {
        runtime
            .command(Command::StageComponent {
                definition,
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        complete_component(&mailbox, init, ComponentStatus::Init, None);
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
    }
    runtime.enable_recording_facts();
    runtime
        .command(Command::InvokeComponent {
            component: source_id,
            at: Duration::ZERO,
        })
        .unwrap();
    let first = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, first, ComponentStatus::Ready, Some(42.0));
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: transform_id,
            at: Duration::ZERO,
        })
        .unwrap();
    let held_transform = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    let captured = held_transform
        .input
        .expect("transform input was not captured");
    assert_eq!(captured.signal, source_signal);
    assert_eq!(captured.value, 42.0);
    assert_eq!(captured.freshness_at, Duration::ZERO);
    runtime
        .command(Command::InvokeComponent {
            component: source_id,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let later_source = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, later_source, ComponentStatus::Ready, Some(43.0));
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    complete_component(&mailbox, held_transform, ComponentStatus::Ready, Some(84.0));
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    let mut store = SqliteStore::open(&path).unwrap();
    let recorded_boot = store.boot_id().to_owned();
    store.start_run("managed transform lineage").unwrap();
    store.append_facts(&facts).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(2)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let upstream = archive
        .read_measurements(InstrumentId::new(822), TEMPERATURE, 8)
        .unwrap();
    let derived = archive
        .read_measurements(InstrumentId::new(823), TEMPERATURE, 8)
        .unwrap();
    assert_eq!(upstream.len(), 2);
    assert_eq!(upstream[1].state_revision, Some(2));
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].value, Some(Value::Float(84.0)));
    assert_eq!(derived[0].revision, 1);
    assert_eq!(derived[0].state_revision, Some(1));
    assert_eq!(derived[0].published_at, Duration::from_secs(1));
    assert_eq!(derived[0].observed_at, Duration::ZERO);
    let lineage = derived[0]
        .lineage
        .as_ref()
        .expect("lost captured input lineage");
    assert_eq!(lineage.signal, source_signal);
    assert_eq!(lineage.value, 42.0);
    assert_eq!(lineage.unit, Unit::CELSIUS.id());
    assert_eq!(lineage.source_generation, 1);
    assert_eq!(lineage.source_revision, 1);
    assert_eq!(lineage.source_state_revision, Some(1));
    assert_eq!(lineage.published_at, Duration::ZERO);
    assert_eq!(lineage.observed_at, Duration::ZERO);
    let page = archive
        .read_history_measurements(
            &HistoryFilter {
                boot_id: recorded_boot,
                run_no: 1,
                instrument: InstrumentId::new(823),
                parameter: TEMPERATURE,
                from: Duration::ZERO,
                to: Duration::from_secs(2),
            },
            None,
            8,
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].state_revision, Some(1));
    assert_eq!(page.rows[0].lineage, derived[0].lineage);
    assert!(page.next_cursor.is_none());
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn warming_transform_attempt_reopens_captured_input_without_claiming_a_good_observation() {
    let path = temporary_database();
    let source_id = ComponentId::new(824);
    let transform_id = ComponentId::new(825);
    let source_signal = lab_core::SignalId::new(InstrumentId::new(824), TEMPERATURE);
    let mut transform = component_definition(transform_id.get());
    transform.manifest.kind = ComponentKind::Transform {
        input: source_signal,
    };
    let mailbox = Arc::new(Mutex::new(ComponentMailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(ComponentFake(mailbox.clone())))
        .unwrap();
    for definition in [component_definition(source_id.get()), transform] {
        runtime
            .command(Command::StageComponent {
                definition,
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        complete_component(&mailbox, init, ComponentStatus::Init, None);
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
    }
    runtime.enable_recording_facts();
    runtime
        .command(Command::InvokeComponent {
            component: source_id,
            at: Duration::ZERO,
        })
        .unwrap();
    let source = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete_component(&mailbox, source, ComponentStatus::Ready, Some(42.0));
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: transform_id,
            at: Duration::from_millis(100),
        })
        .unwrap();
    let warming = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    let captured = warming
        .input
        .expect("warming invocation still captured its input");
    assert_eq!(captured.value, 42.0);
    complete_component(&mailbox, warming, ComponentStatus::Warming, None);
    runtime
        .command(Command::PollComponents {
            at: Duration::from_millis(100),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("warming transform lineage").unwrap();
    store.append_facts(&facts).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(1)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let rows = archive
        .read_measurements(InstrumentId::new(825), TEMPERATURE, 8)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].quality, "unavailable");
    assert_eq!(rows[0].failure.as_deref(), Some("ProcessingWarmup"));
    assert_eq!(rows[0].observed_at, Duration::from_millis(100));
    assert_eq!(rows[0].lineage.as_ref().unwrap().value, 42.0);
    assert_eq!(
        rows[0].lineage.as_ref().unwrap().observed_at,
        Duration::ZERO
    );
    drop(archive);
    std::fs::remove_file(path).unwrap();
}
