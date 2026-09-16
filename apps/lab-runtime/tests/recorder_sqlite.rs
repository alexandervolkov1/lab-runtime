//! Real-file acceptance for the storage adapter, including reopen from WAL.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Query,
    QueryResult, Runtime, Unit, Value, ValueSpec, VirtualInstrumentConfig, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal,
        OutputResult, SafeProfile,
    },
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    application::Application,
    host::{Clock, HostCore},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore, WriterBarrier,
    },
    service::{ServiceHost, ServiceOptions},
    wire::{WireRequest, decode_frame, encode_frame},
};
use serde_json::{Value as JsonValue, json};
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};

fn frame(value: JsonValue) -> WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

#[test]
fn explicit_stop_waits_for_held_pending_fact_and_reopens_its_wal_prefix_and_seal() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let old_boot = worker.boot_id().to_owned();
    worker.request_start("held pending before Stop").unwrap();
    let start_by = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording {
        assert!(Instant::now() < start_by);
        std::thread::yield_now();
    }
    let start_watermark = worker.poll().persisted_through_sequence;
    let instrument = InstrumentId::new(777);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "held Stop source".into(),
            history_capacity: 1,
            base_temperature: 31.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    assert_eq!(facts.len(), 1);
    let assigned = worker.try_admit_at(facts, Duration::from_secs(1)).unwrap();
    let fact_id = *assigned.start();
    worker
        .request_stop_with_summary(
            json!({"pending_operations":[]}),
            Duration::from_secs(1) + Duration::from_millis(1),
        )
        .unwrap();
    let held_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < held_by);
        std::thread::yield_now();
    }
    let pending = worker.poll();
    assert_eq!(pending.state, RecordingState::Stopping);
    assert_eq!(pending.outstanding_records, 1);
    assert_eq!(pending.persisted_through_sequence, start_watermark);
    barrier.release();
    let stop_by = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Idle {
        assert!(Instant::now() < stop_by);
        std::thread::yield_now();
    }
    let sealed = worker.poll();
    assert!(sealed.persisted_through_sequence > fact_id);
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    let wal = PathBuf::from(wal_name);
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 0,
        "a real WAL file must contain committed work before close"
    );
    worker.request_finish().unwrap();
    let close_by = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Closed {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(worker);
    let new_boot = "fefefefefefefefefefefefefefefefe";
    let reopened = SqliteStore::open_with_boot(&path, new_boot).unwrap();
    let old_page = reopened
        .read_history_measurements(
            &lab_runtime::recorder::HistoryFilter {
                boot_id: old_boot.clone(),
                run_no: 1,
                instrument,
                parameter: lab_core::TEMPERATURE,
                from: Duration::ZERO,
                to: Duration::from_secs(2),
            },
            None,
            8,
        )
        .unwrap();
    assert_eq!(old_page.coverage, "complete");
    assert_eq!(old_page.rows.len(), 1);
    assert_eq!(old_page.rows[0].record_sequence, fact_id);
    assert_eq!(old_page.rows[0].value, Some(Value::Float(32.0)));
    reopened.close().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let old_boot_bytes: Vec<u8> = (0..32)
        .step_by(2)
        .map(|at| u8::from_str_radix(&old_boot[at..at + 2], 16).unwrap())
        .collect();
    let (checkpoint, last): (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT d.persisted_through_seq,(SELECT MAX(record_seq) FROM records r
         WHERE r.boot_id=d.boot_id) FROM durable_checkpoints d WHERE d.boot_id=?1",
            [old_boot_bytes.clone()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(checkpoint, last);
    let fact_and_seal: Vec<String> = db
        .prepare(
            "SELECT kind FROM records WHERE boot_id=?1 AND kind IN ('measurement','interval_seal')
         ORDER BY record_seq",
        )
        .unwrap()
        .query_map([old_boot_bytes], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(fact_and_seal, ["measurement", "interval_seal"]);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn native_and_managed_batches_reopen_through_public_indexed_history_with_exact_sql_ids() {
    let path = temporary_database();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let old_boot = service.boot_id().to_owned();
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("native plus managed", now)
        .unwrap();
    let ready_by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < ready_by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let started = Instant::now();
    let work_by = started + Duration::from_secs(3);
    let mut native_times = BTreeSet::new();
    let mut managed_times = BTreeSet::new();
    let mut confirmed = BTreeSet::new();
    while Instant::now() < work_by {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        for (instrument, times) in [
            (InstrumentId::new(1), &mut native_times),
            (InstrumentId::new(201), &mut managed_times),
        ] {
            if let QueryResult::Latest(Some(sample)) = service
                .owner()
                .query(Query::GetLatestSignal(lab_core::SignalId::new(
                    instrument,
                    lab_core::TEMPERATURE,
                )))
                .unwrap()
            {
                times.insert(sample.at());
            }
        }
        confirmed.insert(
            service
                .owner()
                .recording_status()
                .unwrap()
                .persisted_through_sequence,
        );
        if started.elapsed() >= Duration::from_millis(600)
            && native_times.len() >= 3
            && managed_times.len() >= 2
            && confirmed.len() >= 3
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(native_times.len() >= 3, "native refresh did not advance");
    assert!(
        managed_times.len() >= 2,
        "real managed Source did not publish twice"
    );
    assert!(
        confirmed.len() >= 3,
        "facts did not confirm across several SQL batches"
    );
    let stop_at = service.clock().now();
    service.owner_mut().stop_recording_at(stop_at).unwrap();
    let stop_by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Idle {
        assert!(Instant::now() < stop_by);
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
    assert!(terminal.recorder_flushed);
    drop(service);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let old_boot_bytes: Vec<u8> = (0..32)
        .step_by(2)
        .map(|index| u8::from_str_radix(&old_boot[index..index + 2], 16).unwrap())
        .collect();
    let expected = |instrument: u64| {
        archive
            .prepare(
                "SELECT record_seq,quality,float_value FROM measurements
             WHERE boot_id=?1 AND run_no=?2 AND instrument_id=?3 AND parameter_id=?4
             AND published_at>=?5 AND published_at<?6 ORDER BY published_at,record_seq",
            )
            .unwrap()
            .query_map(
                rusqlite::params![
                    old_boot_bytes,
                    1u64.to_be_bytes().to_vec(),
                    instrument.to_be_bytes().to_vec(),
                    1u64.to_be_bytes().to_vec(),
                    0u64.to_be_bytes().to_vec(),
                    10_000_000_000u64.to_be_bytes().to_vec(),
                ],
                |row| {
                    let id: Vec<u8> = row.get(0)?;
                    Ok((
                        u64::from_be_bytes(id.try_into().unwrap()),
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                    ))
                },
            )
            .unwrap()
            .map(|row| row.unwrap())
            .collect::<Vec<_>>()
    };
    let native = expected(1);
    let managed = expected(201);
    assert!(native.len() >= 3);
    assert!(managed.len() >= 2);
    assert!(
        native
            .iter()
            .filter(|(_, quality, value)| quality == "good" && value.is_some())
            .count()
            >= 3
    );
    assert!(
        managed
            .iter()
            .filter(|(_, quality, value)| quality == "good" && value.is_some())
            .count()
            >= 2,
        "two real managed source values must reach the archive"
    );
    let (checkpoint, maximum): (Vec<u8>, Vec<u8>) = archive
        .query_row(
            "SELECT d.persisted_through_seq,(SELECT MAX(record_seq) FROM records r
         WHERE r.boot_id=d.boot_id) FROM durable_checkpoints d WHERE d.boot_id=?1",
            [old_boot_bytes.clone()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        checkpoint, maximum,
        "reopened checkpoint must cover exact committed prefix"
    );
    let seals: i64 = archive
        .query_row(
            "SELECT COUNT(*) FROM records WHERE boot_id=?1 AND kind='interval_seal'",
            [old_boot_bytes],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seals, 1);
    drop(archive);
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut reopened = ServiceHost::startup(options).unwrap();
    assert_ne!(reopened.boot_id(), old_boot);
    let database = reopened.owner().recording_database_id().unwrap().to_owned();
    let mut app = Application::new(reopened.boot_id()).unwrap();
    let hello = app.handle(
        &mut reopened,
        1,
        frame(json!({
            "v":1,"msg_id":"h","op":"hello","args":{"scope":null}
        })),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let mut sequence = 1;
    for (instrument, known) in [(1, &native), (201, &managed)] {
        let mut cursor = JsonValue::Null;
        let mut received = Vec::new();
        loop {
            let read = app.handle(
                &mut reopened,
                1,
                frame(json!({
                    "v":1,"msg_id":format!("read-{sequence}"),"op":"history_read",
                    "request_id":{"scope":scope,"seq":sequence.to_string()},
                    "args":{"mode":"measurements","database_id":database,"boot_id":old_boot,
                        "run_id":{"boot_id":old_boot,"run_no":"1"},
                        "signal":{"instrument":instrument.to_string(),"parameter":"1"},
                        "from_ns":"0","to_ns":"10000000000","max_records":8,"cursor":cursor}
                })),
            );
            assert_eq!(read[0]["state"], "accepted", "{read:?}");
            let result_by = Instant::now() + Duration::from_secs(2);
            let result = loop {
                let result = app.poll_history(&mut reopened);
                if !result.is_empty() {
                    break result;
                }
                assert!(Instant::now() < result_by);
                std::thread::yield_now();
            };
            assert_eq!(result[0].1["state"], "completed", "{result:?}");
            let token = result[0].1["result"]["page_token"].clone();
            let page = app.handle(
                &mut reopened,
                1,
                frame(json!({
                    "v":1,"msg_id":"page","op":"history_page","args":{"page_token":token}
                })),
            );
            assert_eq!(page[0]["type"], "result", "{page:?}");
            for row in page[0]["result"]["rows"].as_array().unwrap() {
                received.push((
                    row["record_seq"].as_str().unwrap().parse::<u64>().unwrap(),
                    row["quality"].as_str().unwrap().to_owned(),
                    row["value"]["value"].as_f64(),
                ));
            }
            cursor = page[0]["result"]["next_cursor"].clone();
            let released = app.handle(
                &mut reopened,
                1,
                frame(json!({
                    "v":1,"msg_id":"release","op":"history_release",
                    "args":{"page_token":token}
                })),
            );
            assert_eq!(released[0]["type"], "result");
            sequence += 1;
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(
            &received, known,
            "public history changed exact committed IDs/values"
        );
    }
    reopened.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while reopened.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(app);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[derive(Clone, Copy)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0
    }
}

#[test]
fn run_boundary_freezes_the_preexisting_latest_sample_without_reobserving_it() {
    let path = temporary_database();
    let mut host = HostCore::virtual_demo().unwrap();
    host.service(&FakeClock(Duration::from_millis(1))).unwrap();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    host.attach_recorder(
        worker,
        RecordingPolicy::BestEffort,
        Duration::from_millis(1),
    )
    .unwrap();
    host.start_recording("boundary oracle", Duration::from_millis(2))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(2))).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    host.stop_recording().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(2))).unwrap();
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(2))).unwrap();
        std::thread::yield_now();
    }
    drop(host);
    let db = rusqlite::Connection::open(&path).unwrap();
    let boundary: Vec<u8> = db
        .query_row(
            "SELECT payload FROM records \
        WHERE kind='boundary_snapshot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&boundary).unwrap();
    let latest = &snapshot["latest_samples"];
    assert!(
        latest
            .as_array()
            .unwrap()
            .iter()
            .any(|sample| sample["instrument"] == "1"
                && sample["parameter"] == "1"
                && sample["published_at_ns"] == "1000000")
    );
    let historical: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        historical, 0,
        "boundary snapshot cannot invent a new observation"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn best_effort_observation_during_held_start_is_queued_behind_the_new_run_barrier() {
    let path = temporary_database();
    let barrier = WriterBarrier::held_start();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    host.start_recording("held start", Duration::ZERO).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() && Instant::now() < deadline {
        std::thread::yield_now()
    }
    assert!(barrier.reached());
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Starting
    );
    let plant = host.plant_id();
    host.command(Command::RefreshMeasurement {
        instrument: plant,
        parameter: lab_core::TEMPERATURE,
        at: Duration::from_millis(10),
    })
    .unwrap();
    assert_eq!(
        host.recording_status().unwrap().outstanding_records,
        1,
        "post-boundary fact must be charged while Start SQL is held"
    );
    assert_eq!(
        host.recording_status().unwrap().persisted_through_sequence,
        0
    );
    barrier.release();
    while (host.recording_status().unwrap().state != RecordingState::Recording
        || host.recording_status().unwrap().outstanding_records != 0)
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(11))).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    assert!(host.recording_status().unwrap().persisted_through_sequence >= 1);
    host.stop_recording().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(11))).unwrap();
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(11))).unwrap();
        std::thread::yield_now();
    }
    drop(host);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM measurements m JOIN runs r \
        ON m.boot_id=r.boot_id AND m.run_no=r.run_no WHERE r.label='held start'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-{suffix}.sqlite"))
}

#[derive(Default)]
struct AckWire {
    readable: VecDeque<u8>,
}
impl ByteTransport for AckWire {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let body = [15, 0, 6, 1];
        self.readable.extend(body);
        self.readable.push_back(crc(&body));
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

#[test]
fn real_m3_ack_reopens_with_original_unit_epoch_resource_and_binding_revision() {
    let path = temporary_database();
    let mut runtime = Runtime::new();
    let resource = ResourceId::new(1);
    let instrument = InstrumentId::new(711);
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    runtime
        .register_transport(resource, Box::new(AckWire::default()))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "M3 archive identity".into(),
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
                resource,
                device: 15,
                channel: 0,
                binding_generation: 1,
                mapping_revision: 1,
                expected_output_unit: Some(Unit::PERCENT),
            },
            history_capacity: 2,
        }))
        .unwrap();
    let output = |runtime: &mut Runtime, at, command| {
        runtime.command(Command::Output {
            actuator,
            at,
            command,
        })
    };
    output(
        &mut runtime,
        Duration::ZERO,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(5),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Acknowledgement,
        }),
    )
    .unwrap();
    output(&mut runtime, Duration::ZERO, OutputCommand::RequestSafe).unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::ZERO,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports { at: Duration::ZERO })
        .unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("M3 identity").unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = output(
        &mut runtime,
        at,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(1),
            lifetime: Duration::from_secs(1),
        },
    )
    .unwrap() else {
        panic!("safe ACK did not permit lease")
    };
    output(
        &mut runtime,
        at,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(40.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(200),
        }),
    )
    .unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime.command(Command::PollTransports { at }).unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    type Row = (
        String,
        String,
        Option<String>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
    );
    let rows: Vec<Row> = db
        .prepare(
            "SELECT stage,evidence_source,unit_key,authority_epoch,resource_id,generation,revision
         FROM output_events ORDER BY record_seq",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        ["requested", "authorized", "send_started", "acknowledged"]
    );
    assert_eq!(rows[3].1, "transport_protocol");
    for row in rows {
        assert_eq!(row.2.as_deref(), Some(Unit::PERCENT.id()));
        assert_eq!(row.3, Some(lease.epoch().to_be_bytes().to_vec()));
        assert_eq!(row.4, Some(resource.get().to_be_bytes().to_vec()));
        assert_eq!(row.5, Some(1u64.to_be_bytes().to_vec()));
        assert_eq!(row.6, Some(1u64.to_be_bytes().to_vec()));
    }
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn queued_virtual_attempt_reopens_as_superseded_before_distinct_safe_epoch() {
    let path = temporary_database();
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(778);
    let actuator = ActuatorId::new(instrument, lab_core::HEATER_POWER);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "superseded archive".into(),
            history_capacity: 2,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    let output = |runtime: &mut Runtime, at, command| {
        runtime.command(Command::Output {
            actuator,
            at,
            command,
        })
    };
    output(
        &mut runtime,
        Duration::ZERO,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(2),
            max_proposal_ttl: Duration::from_millis(100),
            required_evidence: EvidenceLevel::Readback,
        }),
    )
    .unwrap();
    output(&mut runtime, Duration::ZERO, OutputCommand::RequestSafe).unwrap();
    let CommandResult::Output(OutputResult::Dispatched(safe)) =
        output(&mut runtime, Duration::ZERO, OutputCommand::BeginDispatch).unwrap()
    else {
        panic!("safe dispatch missing")
    };
    output(
        &mut runtime,
        Duration::ZERO,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    )
    .unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("superseded identity").unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = output(
        &mut runtime,
        at,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(778),
            lifetime: Duration::from_secs(1),
        },
    )
    .unwrap() else {
        panic!("manual lease missing")
    };
    output(
        &mut runtime,
        at,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(37.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(100),
        }),
    )
    .unwrap();
    output(
        &mut runtime,
        at + Duration::from_millis(1),
        OutputCommand::RequestSafe,
    )
    .unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    type StageRow = (String, Option<Vec<u8>>, Option<Vec<u8>>, Option<String>);
    let mut statement = db.prepare(
        "SELECT stage,attempt_id,authority_epoch,unit_key FROM output_events ORDER BY record_seq",
    ).unwrap();
    let rows: Vec<StageRow> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        [
            "requested",
            "superseded_before_send",
            "revoked",
            "safe_requested"
        ]
    );
    assert_eq!(rows[0].1, rows[1].1);
    assert_ne!(rows[1].1, rows[3].1);
    assert_eq!(rows[2].1, rows[3].1);
    assert_eq!(rows[0].2, Some(lease.epoch().to_be_bytes().to_vec()));
    assert_eq!(rows[1].2, rows[0].2);
    assert!(
        u64::from_be_bytes(rows[3].2.as_ref().unwrap().as_slice().try_into().unwrap())
            > lease.epoch()
    );
    assert!(
        rows.iter()
            .all(|row| row.3.as_deref() == Some(Unit::PERCENT.id()))
    );
    drop(statement);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn one_virtual_attempt_reopens_with_one_boot_scoped_attempt_and_dispatch_correlation() {
    let path = temporary_database();
    let instrument = InstrumentId::new(777);
    let actuator = ActuatorId::new(instrument, lab_core::HEATER_POWER);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "correlated output".into(),
            history_capacity: 2,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    let output = |runtime: &mut Runtime, at, command| {
        runtime.command(Command::Output {
            actuator,
            at,
            command,
        })
    };
    output(
        &mut runtime,
        Duration::ZERO,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(5),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Readback,
        }),
    )
    .unwrap();
    output(&mut runtime, Duration::ZERO, OutputCommand::RequestSafe).unwrap();
    let CommandResult::Output(OutputResult::Dispatched(safe)) =
        output(&mut runtime, Duration::ZERO, OutputCommand::BeginDispatch).unwrap()
    else {
        panic!("safe dispatch missing")
    };
    output(
        &mut runtime,
        Duration::ZERO,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    )
    .unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("virtual correlation").unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = output(
        &mut runtime,
        at,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(777),
            lifetime: Duration::from_secs(1),
        },
    )
    .unwrap() else {
        panic!("manual lease missing")
    };
    output(
        &mut runtime,
        at,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(33.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(100),
        }),
    )
    .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
        output(&mut runtime, at, OutputCommand::BeginDispatch).unwrap()
    else {
        panic!("ordinary dispatch missing")
    };
    output(
        &mut runtime,
        at,
        OutputCommand::Complete {
            dispatch_id: dispatch.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    )
    .unwrap();
    output(
        &mut runtime,
        at,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(44.0),
            unit: Unit::CELSIUS,
            ttl: Duration::from_millis(100),
        }),
    )
    .unwrap_err();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    type CorrelationRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);
    let rows: Vec<CorrelationRow> = {
        let mut statement = db
            .prepare("SELECT stage,attempt_id,dispatch_id FROM output_events ORDER BY record_seq")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].0, "requested");
    assert_eq!(rows[1].0, "authorized");
    assert_eq!(rows[2].0, "send_started");
    assert_eq!(rows[3].0, "readback_verified");
    assert_eq!(rows[4].0, "rejected_before_send");
    assert!(rows[0].1.is_some(), "proposal identity was omitted");
    assert!(rows[..4].iter().all(|row| row.1 == rows[0].1));
    assert_eq!(rows[0].2, None);
    assert!(rows[1].2.is_some(), "trusted dispatch identity was omitted");
    assert!(rows[1..4].iter().all(|row| row.2 == rows[1].2));
    assert!(rows[4].1.is_some());
    assert_ne!(rows[4].1, rows[0].1);
    assert_eq!(rows[4].2, None);
    let identity: (Option<String>, Option<Vec<u8>>) = db
        .query_row(
            "SELECT unit_key,authority_epoch FROM output_events WHERE stage='requested'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(identity.0.as_deref(), Some(Unit::PERCENT.id()));
    assert_eq!(identity.1, Some(lease.epoch().to_be_bytes().to_vec()));
    let rejected_unit: Option<String> = db
        .query_row(
            "SELECT unit_key FROM output_events WHERE stage='rejected_before_send'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rejected_unit.as_deref(), Some(Unit::CELSIUS.id()));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn committed_measurements_survive_close_and_reopen_with_exact_identity_and_quality() {
    let path = temporary_database();
    let instrument = InstrumentId::new(71);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "SQLite fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let database_id;
    {
        let mut store = SqliteStore::open(&path).unwrap();
        database_id = store.database_id().to_owned();
        store.start_run("recorded fixture").unwrap();
        runtime
            .command(Command::RefreshMeasurement {
                instrument,
                parameter: lab_core::TEMPERATURE,
                at: Duration::from_secs(1),
            })
            .unwrap();
        store.append_facts(&runtime.take_recording_facts()).unwrap();
        runtime
            .command(Command::ConfigureParameter {
                instrument,
                parameter: lab_core::MEASUREMENT_ENABLED,
                value: Value::Boolean(false),
            })
            .unwrap();
        assert!(
            runtime
                .command(Command::RefreshMeasurement {
                    instrument,
                    parameter: lab_core::TEMPERATURE,
                    at: Duration::from_secs(2),
                })
                .is_err()
        );
        store.append_facts(&runtime.take_recording_facts()).unwrap();
        store.stop_run().unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.database_id(), database_id);
    let rows = store
        .read_measurements(instrument, lab_core::TEMPERATURE, 16)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value, Some(Value::Float(21.0)));
    assert_eq!(rows[0].unit, lab_core::Unit::CELSIUS.id());
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[0].published_at, Duration::from_secs(1));
    assert_eq!(rows[1].published_at, Duration::from_secs(2));
    assert!(rows[0].record_sequence < rows[1].record_sequence);
    assert_eq!(rows[0].quality, "good");
    assert_eq!(rows[1].quality, "unavailable");
    drop(store);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn successive_clean_runs_in_one_boot_have_distinct_durable_identity() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("first").unwrap();
    store.stop_run().unwrap();
    store.start_run("second").unwrap();
    store.stop_run().unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 2);
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reopened_run_retains_the_selected_best_effort_policy() {
    let path = temporary_database();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    host.start_recording("best effort fixture", Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    host.stop_recording().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    drop(host);
    let reopened = SqliteStore::open(&path).unwrap();
    let page = reopened.read_history_runs(None, 32).unwrap();
    let run = page
        .runs
        .iter()
        .find(|run| run.label == "best effort fixture")
        .unwrap();
    assert_eq!(run.policy, "best-effort");
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
