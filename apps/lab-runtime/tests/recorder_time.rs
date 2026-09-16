//! UTC display metadata must never replace monotonic fact order or control time.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Query,
    QueryResult, Runtime, Unit, Value, ValueSpec, VirtualInstrumentConfig, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal, OutputResult,
        SafeProfile,
    },
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    host::{Clock, HostCore, SystemClock},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingState, SqliteStore, TimeAnchor, WriterBarrier,
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

struct DeterministicClock(Cell<Duration>);
impl Clock for DeterministicClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

#[derive(Default)]
struct DeterministicWire {
    written: Vec<u8>,
    readable: VecDeque<u8>,
    write_limits: VecDeque<usize>,
    recoveries: usize,
}

struct DeterministicTransport(Rc<RefCell<DeterministicWire>>);
impl ByteTransport for DeterministicTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let limit = wire.write_limits.pop_front().unwrap_or(bytes.len());
        let count = bytes.len().min(limit);
        wire.written.extend_from_slice(&bytes[..count]);
        if count == 7 && count == bytes.len() {
            let body = [15, 0, 6, 1];
            wire.readable.extend(body);
            wire.readable.push_back(crc(&body));
        }
        Ok(count)
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
        let mut wire = self.0.borrow_mut();
        wire.recoveries += 1;
        wire.readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

fn m3_time_fixture() -> (
    Runtime,
    ActuatorId,
    ResourceId,
    Rc<RefCell<DeterministicWire>>,
) {
    let wire = Rc::new(RefCell::new(DeterministicWire {
        // Establish the initial safe evidence before ordinary authority exists.
        write_limits: [7].into_iter().collect(),
        ..DeterministicWire::default()
    }));
    let mut runtime = Runtime::new();
    let resource = ResourceId::new(17);
    let instrument = InstrumentId::new(717);
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    runtime
        .register_transport(resource, Box::new(DeterministicTransport(Rc::clone(&wire))))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "wall-independent M3 output".into(),
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
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(1),
                max_proposal_ttl: Duration::from_millis(200),
                required_evidence: EvidenceLevel::Acknowledgement,
            }),
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::ZERO,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(20),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports { at: Duration::ZERO })
        .unwrap();
    let mut initial = wire.borrow_mut();
    initial.written.clear();
    // The ordinary attempt accepts a real two-byte prefix. After recovery,
    // the next safe attempt accepts its complete seven-byte request.
    initial.write_limits = [2, 7].into_iter().collect();
    drop(initial);
    (runtime, actuator, resource, wire)
}

#[test]
fn m3_first_byte_recovery_and_safe_send_decisions_ignore_wall_clock_jumps() {
    let anchors = [
        TimeAnchor::valid(
            Duration::from_millis(2),
            2_000,
            Duration::from_millis(2) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::valid(
            Duration::from_millis(2),
            9_999_999_999,
            Duration::from_millis(2) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::valid(
            Duration::from_millis(2),
            -9_999_999_999,
            Duration::from_millis(2) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::unavailable(
            Duration::from_millis(2),
            "wall_read_failed",
            Duration::from_millis(2) + Duration::from_nanos(100),
        )
        .unwrap(),
    ];
    let mut traces = Vec::new();
    for (case, anchor) in anchors.into_iter().enumerate() {
        let path = temporary_database();
        let boot_id = format!("{:032x}", case + 101);
        let boot_anchor =
            TimeAnchor::valid(Duration::ZERO, 123, Duration::from_nanos(100)).unwrap();
        let mut store = SqliteStore::open_with_boot_anchor(&path, &boot_id, boot_anchor).unwrap();
        store.start_run("wall-independent M3 send").unwrap();
        let (mut runtime, actuator, resource, wire) = m3_time_fixture();
        let at = Duration::from_millis(1);
        let CommandResult::Output(OutputResult::Lease(lease)) = runtime
            .command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Acquire {
                    owner: OutputOwner::Manual(1),
                    lifetime: Duration::from_millis(100),
                },
            })
            .unwrap()
        else {
            panic!("manual lease absent")
        };
        runtime
            .command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Propose(OutputProposal {
                    lease,
                    value: Value::Float(75.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(100),
                }),
            })
            .unwrap();
        runtime
            .command(Command::QueueMetakonOutput {
                actuator,
                at,
                queue_ttl: Duration::from_millis(100),
                timeout: Duration::from_millis(2),
            })
            .unwrap();

        let capture = |runtime: &Runtime, wire: &Rc<RefCell<DeterministicWire>>, now_ms: u64| {
            let QueryResult::Transport(transport) =
                runtime.query(Query::Transport(resource)).unwrap()
            else {
                panic!("transport query changed kind")
            };
            let QueryResult::Output(output) = runtime.query(Query::Output(actuator)).unwrap()
            else {
                panic!("output query changed kind")
            };
            let normalized_output = (
                output.state,
                output
                    .lease
                    .map(|lease| (lease.owner(), lease.epoch(), lease.expires())),
                output.epoch,
                output.fault_latched,
                output.safe_confirmed,
                output.pending,
                output
                    .in_flight
                    .map(|dispatch| (dispatch.value(), dispatch.is_safe(), dispatch.epoch())),
                output.requested,
                output.sent,
                output.acknowledged,
                output.readback,
                output.outcome,
            );
            let wire = wire.borrow();
            (
                now_ms,
                transport,
                normalized_output,
                wire.written.clone(),
                wire.recoveries,
            )
        };
        let mut trace = Vec::new();
        for now_ms in [1, 3, 4] {
            let now = Duration::from_millis(now_ms);
            runtime
                .command(Command::PollTransports { at: now })
                .unwrap();
            if now_ms == 3 {
                store.append_clock_anchor("periodic", &anchor).unwrap();
            }
            trace.push(capture(&runtime, &wire, now_ms));
        }
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::from_millis(5),
                command: OutputCommand::RequestSafe,
            })
            .unwrap();
        runtime
            .command(Command::QueueMetakonOutput {
                actuator,
                at: Duration::from_millis(5),
                queue_ttl: Duration::from_millis(100),
                timeout: Duration::from_millis(20),
            })
            .unwrap();
        for now_ms in [5, 6, 7] {
            runtime
                .command(Command::PollTransports {
                    at: Duration::from_millis(now_ms),
                })
                .unwrap();
            trace.push(capture(&runtime, &wire, now_ms));
        }
        assert_eq!(wire.borrow().written.len(), 9);
        assert_eq!(wire.borrow().recoveries, 1);
        let QueryResult::Output(output) = runtime.query(Query::Output(actuator)).unwrap() else {
            panic!("output query changed kind")
        };
        assert!(output.safe_confirmed);
        traces.push(trace);

        let seal = TimeAnchor::valid(
            Duration::from_millis(8),
            8_000,
            Duration::from_millis(8) + Duration::from_nanos(100),
        )
        .unwrap();
        store.stop_run_with_anchor(&seal).unwrap();
        store.finish_boot(Duration::from_millis(8)).unwrap();
        store.close().unwrap();
        std::fs::remove_file(path).unwrap();
    }
    for changed in &traces[1..] {
        assert_eq!(
            changed, &traces[0],
            "M3 first-byte, recovery or safe-send decision changed with UTC"
        );
    }
}

#[test]
fn deterministic_native_control_is_identical_across_forward_backward_and_failed_wall_reads() {
    let anchors = [
        TimeAnchor::valid(
            Duration::from_secs(1),
            1_000_000,
            Duration::from_secs(1) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::valid(
            Duration::from_secs(1),
            9_999_999_999,
            Duration::from_secs(1) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::valid(
            Duration::from_secs(1),
            -9_999_999_999,
            Duration::from_secs(1) + Duration::from_nanos(100),
        )
        .unwrap(),
        TimeAnchor::unavailable(
            Duration::from_secs(1),
            "wall_read_failed",
            Duration::from_secs(1) + Duration::from_nanos(100),
        )
        .unwrap(),
    ];
    let mut traces = Vec::new();
    for (case, anchor) in anchors.into_iter().enumerate() {
        let path = temporary_database();
        let boot_id = format!("{:032x}", case + 1);
        let boot_anchor =
            TimeAnchor::valid(Duration::ZERO, -1_000_000, Duration::from_nanos(200)).unwrap();
        let mut store = SqliteStore::open_with_boot_anchor(&path, &boot_id, boot_anchor).unwrap();
        store.start_run("wall-independent native control").unwrap();
        let mut host = HostCore::virtual_demo().unwrap();
        let clock = DeterministicClock(Cell::new(Duration::ZERO));
        host.service(&clock).unwrap();
        host.command(Command::StartController {
            controller: host.controller_id(),
            at: Duration::ZERO,
        })
        .unwrap();
        let mut trace = Vec::new();
        for tenth in 1..=20 {
            clock.0.set(Duration::from_millis(tenth * 100));
            let report = host.service(&clock).unwrap();
            if tenth == 10 {
                store.append_clock_anchor("periodic", &anchor).unwrap();
            }
            let QueryResult::Controller(controller) =
                host.query(Query::Controller(host.controller_id())).unwrap()
            else {
                panic!("controller query changed kind")
            };
            let QueryResult::Reference(reference) =
                host.query(Query::Reference(host.reference_id())).unwrap()
            else {
                panic!("reference query changed kind")
            };
            let QueryResult::Latest(sample) = host
                .query(Query::GetLatestSignal(host.temperature()))
                .unwrap()
            else {
                panic!("sample query changed kind")
            };
            let QueryResult::Output(output) = host
                .query(Query::Output(lab_core::output::ActuatorId::new(
                    host.plant_id(),
                    lab_core::HEATER_POWER,
                )))
                .unwrap()
            else {
                panic!("output query changed kind")
            };
            // Lease instance numbers are process-wide allocation identities.
            // Compare their safety deadlines and every control/output decision.
            trace.push((
                report,
                (
                    controller.state,
                    controller.ema,
                    controller.pid,
                    controller.lease.map(|lease| lease.expires()),
                    controller.last_tick,
                    controller.latest_output,
                ),
                (reference, sample),
                (
                    output.state,
                    output.epoch,
                    output.fault_latched,
                    output.safe_confirmed,
                    output.lease.map(|lease| lease.expires()),
                    output.requested,
                    output.sent,
                    output.acknowledged,
                    output.readback,
                    output.outcome,
                ),
            ));
        }
        let seal = TimeAnchor::valid(
            Duration::from_secs(2),
            2_000_000,
            Duration::from_secs(2) + Duration::from_nanos(100),
        )
        .unwrap();
        store.stop_run_with_anchor(&seal).unwrap();
        store.finish_boot(Duration::from_secs(2)).unwrap();
        store.close().unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        let observed: (Option<i64>, Option<String>, i64, Vec<u8>) = db
            .query_row(
                "SELECT c.wall_us,c.unavailable_reason,r.wall_estimate_us,r.published_at
             FROM clock_anchors c JOIN records r
             ON r.boot_id=c.boot_id AND r.record_seq=c.record_seq
             WHERE c.kind='periodic'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(observed.0, anchor.wall_us());
        assert_eq!(observed.1, anchor.unavailable_reason().map(str::to_owned));
        assert_eq!(observed.2, -1, "boot mapping must not follow later UTC");
        assert_eq!(observed.3, (1_000_000_100u64).to_be_bytes());
        drop(db);
        std::fs::remove_file(path).unwrap();
        traces.push(trace);
    }
    for changed in &traces[1..] {
        assert_eq!(
            changed, &traces[0],
            "PID/EMA, Reference, lease, sample and output decisions changed with UTC"
        );
    }
}

#[test]
fn boot_mapping_remains_stable_across_later_utc_jump_and_supports_pre_epoch_time() {
    let boot = TimeAnchor::valid(
        Duration::from_nanos(100),
        -1_000_000,
        Duration::from_nanos(300),
    )
    .unwrap();
    let jumped = TimeAnchor::valid(
        Duration::from_secs(2),
        9_999_999_999,
        Duration::from_secs(2) + Duration::from_nanos(100),
    )
    .unwrap();
    assert_eq!(boot.uncertainty_ns(), 200);
    assert_eq!(
        boot.estimate_us(Duration::from_nanos(300)).unwrap(),
        -1_000_000
    );
    assert_eq!(boot.estimate_us(Duration::from_secs(2)).unwrap(), 999_999);
    assert_ne!(
        boot.estimate_us(Duration::from_secs(2)).unwrap(),
        jumped.wall_us().unwrap()
    );
    assert_eq!(
        boot.estimate_us(Duration::from_nanos(100)).unwrap(),
        -1_000_001
    );
}

#[test]
fn invalid_bracket_or_overflow_fails_as_metadata_without_changing_a_fact_time() {
    assert!(TimeAnchor::valid(Duration::from_nanos(2), 0, Duration::from_nanos(1)).is_err());
    let anchor = TimeAnchor::valid(Duration::ZERO, i64::MAX, Duration::ZERO).unwrap();
    assert!(anchor.estimate_us(Duration::from_secs(1)).is_err());
}

#[test]
fn real_file_persists_boot_bracket_and_independent_actual_anchors_across_reopen() {
    let path = temporary_database();
    let boot = "f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";
    let initial = TimeAnchor::valid(Duration::ZERO, -1_000_000, Duration::from_nanos(200)).unwrap();
    let jumped = TimeAnchor::valid(
        Duration::from_secs(2),
        9_999_999_999,
        Duration::from_secs(2) + Duration::from_nanos(100),
    )
    .unwrap();
    let failed = TimeAnchor::unavailable(
        Duration::from_secs(3),
        "wall_read_failed",
        Duration::from_secs(3) + Duration::from_nanos(50),
    )
    .unwrap();
    let mut store = SqliteStore::open_with_boot_anchor(&path, boot, initial.clone()).unwrap();
    store.append_clock_anchor("periodic", &jumped).unwrap();
    store.append_clock_anchor("interval_end", &failed).unwrap();
    store.finish_boot(Duration::from_secs(3)).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let bracket: (Vec<u8>, Vec<u8>, i64) = db
        .query_row(
            "SELECT anchor_before,anchor_after,started_wall_us FROM runtime_boots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(bracket.0, 0u64.to_be_bytes());
    assert_eq!(bracket.1, 200u64.to_be_bytes());
    assert_eq!(bracket.2, -1_000_000);
    let anchors: Vec<(String, Option<i64>, Option<String>)> = {
        let mut statement = db
            .prepare("SELECT kind,wall_us,unavailable_reason FROM clock_anchors ORDER BY anchor_no")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(anchors[0].0, "boot");
    assert_eq!(anchors[1], ("periodic".into(), Some(9_999_999_999), None));
    assert_eq!(
        anchors[2],
        ("interval_end".into(), None, Some("wall_read_failed".into()))
    );
    let projected: Vec<(String, Option<i64>, i64)> = {
        let mut statement = db
            .prepare(
                "SELECT c.kind,c.wall_us,count(e.record_seq) FROM clock_anchors c
                 JOIN records r ON r.boot_id=c.boot_id AND r.record_seq=c.record_seq
                 JOIN runtime_events e ON e.boot_id=r.boot_id AND e.record_seq=r.record_seq
                 WHERE c.kind!='boot' GROUP BY c.kind,c.wall_us ORDER BY c.anchor_no",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        projected,
        vec![
            ("periodic".into(), Some(9_999_999_999), 1),
            ("interval_end".into(), None, 1),
        ]
    );
    drop(db);
    let reopened = SqliteStore::open_with_boot(&path, "11111111111111111111111111111111").unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn fact_reopen_keeps_monotonic_observation_publication_and_capture_separate_from_boot_utc() {
    let path = temporary_database();
    let boot = TimeAnchor::valid(Duration::ZERO, -1_000_000, Duration::from_nanos(200)).unwrap();
    let mut store =
        SqliteStore::open_with_boot_anchor(&path, "22222222222222222222222222222222", boot)
            .unwrap();
    store.start_run("time domains").unwrap();
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(222);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "time fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let published = Duration::from_secs(2);
    let captured = published + Duration::from_millis(50);
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: published,
        })
        .unwrap();
    store
        .append_facts_with_capture(&runtime.take_recording_facts(), captured)
        .unwrap();
    store.stop_run().unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let boundary_wall: Option<i64> = db
        .query_row(
            "SELECT wall_estimate_us FROM records WHERE kind='boundary_snapshot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(boundary_wall, Some(-1_000_001));
    let times: (Vec<u8>, Vec<u8>, Vec<u8>, i64) = db
        .query_row(
            "SELECT observed_at,published_at,captured_at,wall_estimate_us
             FROM records WHERE kind='measurement'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(times.0, (2_000_000_000u64).to_be_bytes());
    assert_eq!(times.1, times.0);
    assert_eq!(times.2, (2_050_000_000u64).to_be_bytes());
    assert_eq!(times.3, 999_999);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn idle_storage_worker_samples_periodic_utc_without_a_runtime_control_tick() {
    let path = temporary_database();
    let clock = SystemClock::new();
    let boot_anchor =
        TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let mut worker = RecorderWorker::open_with_boot_clock(
        &path,
        RecorderLimits::default(),
        "33333333333333333333333333333333",
        boot_anchor,
        clock,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while worker.poll().persisted_through_sequence == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(worker.poll().persisted_through_sequence > 0);
    worker.request_finish().unwrap();
    while worker.poll().state != RecordingState::Closed && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM records WHERE kind='clock_anchor'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn live_worker_preserves_failed_later_wall_read_as_nullable_periodic_anchor() {
    let path = temporary_database();
    let barrier = WriterBarrier::fail_next_periodic_wall_read();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while worker.poll().persisted_through_sequence == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(worker.poll().persisted_through_sequence > 0);
    assert_eq!(worker.poll().state, RecordingState::Idle);
    worker.request_finish().unwrap();
    while worker.poll().state != RecordingState::Closed && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let actual: (Option<i64>, Option<String>, i64) = db
        .query_row(
            "SELECT c.wall_us,c.unavailable_reason,r.wall_estimate_us
             FROM clock_anchors c JOIN records r
             ON r.boot_id=c.boot_id AND r.record_seq=c.record_seq
             WHERE c.kind='periodic' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(actual.0, None);
    assert_eq!(actual.1.as_deref(), Some("wall_read_failed"));
    assert!(actual.2 > 0, "boot-mapped wall estimate remains available");
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn worker_start_and_stop_commit_actual_utc_anchors_for_the_exact_interval() {
    let path = temporary_database();
    let clock = SystemClock::new();
    let boot_anchor =
        TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
    let mut worker = RecorderWorker::open_with_boot_clock(
        &path,
        RecorderLimits::default(),
        "44444444444444444444444444444444",
        boot_anchor,
        clock,
    )
    .unwrap();
    worker
        .request_start_at("anchored interval", clock.now())
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while worker.poll().state == RecordingState::Starting && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    worker.request_stop().unwrap();
    while worker.poll().state == RecordingState::Stopping && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Idle);
    worker.request_finish().unwrap();
    while worker.poll().state != RecordingState::Closed && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let actual: (Option<i64>, Option<i64>) = db
        .query_row(
            "SELECT started_wall_us,ended_wall_us FROM runs WHERE label='anchored interval'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(actual.0.is_some());
    assert!(actual.1.is_some());
    let kinds: Vec<String> = {
        let mut statement = db
            .prepare(
                "SELECT kind FROM clock_anchors WHERE kind LIKE 'interval_%' ORDER BY anchor_no",
            )
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(kinds, ["interval_start", "interval_end"]);
    let anchors: Vec<u64> = db
        .prepare("SELECT anchor_no FROM clock_anchors ORDER BY anchor_no")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| u64::from_be_bytes(row.unwrap().try_into().unwrap()))
        .collect();
    assert!(
        anchors.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "boot/start/end/final anchor identities must be contiguous: {anchors:?}"
    );
    let boot_end: i64 = db
        .query_row(
            "SELECT count(*) FROM clock_anchors WHERE kind='boot_end'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(boot_end, 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-time-{suffix}.sqlite"))
}
