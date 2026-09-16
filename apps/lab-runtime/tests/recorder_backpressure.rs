//! Recorder owner-to-storage handoff remains finite and nonblocking.

use lab_core::{
    Command, InstrumentId, Runtime, Sample, SignalId, Unit, Value, VirtualInstrumentConfig,
    managed::CapturedInput, recording::RecordingFact,
};
use lab_runtime::recorder::{RecorderLimits, RecorderWorker, RecordingState, WriterBarrier};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-worker-{suffix}.sqlite"))
}

#[test]
fn exhausted_group_and_history_credits_still_deliver_one_reserved_gap_and_terminal_seal() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("reserved fault seal").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let signal = SignalId::new(InstrumentId::new(185), lab_core::TEMPERATURE);
    let fact = |sequence: u64| RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(
            signal,
            Unit::CELSIUS,
            Duration::from_millis(sequence * 10),
            Value::Float(sequence as f64),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: None,
        lineage: None,
    };
    worker.try_admit(vec![fact(1)]).unwrap();
    let held_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < held_by);
        std::thread::yield_now();
    }
    for sequence in 2..=4 {
        worker.try_admit(vec![fact(sequence)]).unwrap();
    }
    let mut history_jobs = Vec::new();
    for _ in 0..8 {
        history_jobs.push(worker.request_runs(None, 1).unwrap());
    }
    assert_eq!(history_jobs.len(), 8);
    let saturated = worker.poll();
    assert_eq!(saturated.outstanding_groups, 4);
    assert_eq!(saturated.outstanding_records, 4);
    assert!(worker.try_admit(vec![fact(5)]).is_err());
    let failed = worker.poll();
    assert_eq!(failed.state, RecordingState::Failed);
    assert_eq!(failed.first_missing_fact, Some(5));
    assert_eq!(
        failed.outstanding_groups, 4,
        "rejected fifth group must not consume ordinary ingress credit"
    );
    worker.request_finish().unwrap();
    barrier.release();
    let close_by = Instant::now() + Duration::from_secs(3);
    while !worker.poll().worker_closed {
        assert!(Instant::now() < close_by, "{:?}", worker.poll());
        std::thread::yield_now();
    }
    let closed = worker.poll();
    assert_eq!(
        closed.state,
        RecordingState::Failed,
        "a sticky recording failure remains visible after worker close"
    );
    assert!(closed.failure_persisted, "{:?}", closed);
    assert!(closed.terminal_seal_committed, "{:?}", closed);
    assert_eq!(closed.outstanding_records, 0);
    drop(worker);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let fact_count: i64 = archive
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    let gap_count: i64 = archive
        .query_row("SELECT COUNT(*) FROM gaps", [], |row| row.get(0))
        .unwrap();
    let run: (String, String) = archive
        .query_row(
            "SELECT state,coverage FROM runs WHERE label='reserved fault seal'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(fact_count, 4);
    assert_eq!(gap_count, 1);
    assert_eq!(run, ("failed".into(), "gap".into()));
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn escaped_measurement_charges_owned_and_encoded_bytes_before_group_admission() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(
        &path,
        RecorderLimits {
            records: 4,
            bytes: 2500,
            groups: 2,
        },
    )
    .unwrap();
    worker.request_start("encoded charge").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let fact = RecordingFact::Measurement {
        sequence: 1,
        sample: Sample::validated_good(
            SignalId::new(InstrumentId::new(177), lab_core::TEMPERATURE),
            Unit::CELSIUS,
            Duration::from_secs(1),
            Value::Text("\0".repeat(512)),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: None,
        lineage: None,
    };
    assert!(worker.try_admit(vec![fact]).is_err());
    let status = worker.poll();
    assert_eq!(status.state, RecordingState::Failed);
    assert_eq!(status.outstanding_records, 0);
    assert_eq!(status.outstanding_bytes, 0);
    assert_eq!(status.first_missing_fact, Some(1));
    worker.request_finish().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.poll().worker_closed && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(worker.poll().worker_closed);
    drop(worker);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = archive
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn transform_lineage_encoder_scratch_is_charged_before_group_admission() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(
        &path,
        RecorderLimits {
            records: 4,
            bytes: std::mem::size_of::<RecordingFact>() + 900,
            groups: 2,
        },
    )
    .unwrap();
    worker.request_start("lineage scratch charge").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let fact = RecordingFact::Measurement {
        sequence: 1,
        sample: Sample::validated_good(
            SignalId::new(InstrumentId::new(179), lab_core::TEMPERATURE),
            Unit::CELSIUS,
            Duration::from_secs(1),
            Value::Float(84.0),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: Some(1),
        lineage: Some(CapturedInput {
            signal: SignalId::new(InstrumentId::new(178), lab_core::TEMPERATURE),
            value: 42.0,
            unit: Unit::CELSIUS,
            at: Duration::from_secs(1),
            freshness_at: Duration::from_secs(1),
            source_generation: 1,
            source_revision: 1,
            source_state_revision: Some(1),
        }),
    };
    assert!(worker.try_admit(vec![fact]).is_err());
    let status = worker.poll();
    assert_eq!(status.state, RecordingState::Failed);
    assert_eq!(status.outstanding_bytes, 0);
    assert_eq!(status.first_missing_fact, Some(1));
    worker.request_finish().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.poll().worker_closed && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(worker.poll().worker_closed);
    drop(worker);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = archive
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn overallocated_fact_vector_is_charged_even_when_it_contains_one_small_fact() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(
        &path,
        RecorderLimits {
            records: 4,
            bytes: 1024,
            groups: 2,
        },
    )
    .unwrap();
    worker.request_start("owned Vec charge").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let mut facts = Vec::with_capacity(32);
    facts.push(RecordingFact::Measurement {
        sequence: 1,
        sample: Sample::validated_good(
            SignalId::new(InstrumentId::new(178), lab_core::TEMPERATURE),
            Unit::CELSIUS,
            Duration::from_secs(1),
            Value::Float(20.0),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: None,
        lineage: None,
    });
    assert!(worker.try_admit(facts).is_err());
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_finish().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.poll().worker_closed && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(worker.poll().worker_closed);
    drop(worker);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn owner_assigns_contiguous_record_ids_to_a_whole_group_before_sqlite_commit() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("assigned range").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    let signal = SignalId::new(InstrumentId::new(179), lab_core::TEMPERATURE);
    let facts = (1..=2u64)
        .map(|sequence| RecordingFact::Measurement {
            sequence,
            sample: Sample::validated_good(
                signal,
                Unit::CELSIUS,
                Duration::from_secs(1),
                Value::Float(20.0 + sequence as f64),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
            state_revision: None,
            lineage: None,
        })
        .collect::<Vec<_>>();
    let assigned = worker.try_admit_at(facts, Duration::from_secs(1)).unwrap();
    assert_eq!(*assigned.start(), start_watermark + 1);
    assert_eq!(*assigned.end(), start_watermark + 2);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(barrier.reached(), "writer did not hold the assigned group");
    assert_eq!(worker.poll().persisted_through_sequence, start_watermark);
    barrier.release();
    while worker.poll().outstanding_records != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let identities = archive
        .prepare("SELECT record_seq FROM measurements ORDER BY record_seq")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| u64::from_be_bytes(row.unwrap().try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(identities, vec![*assigned.start(), *assigned.end()]);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn owner_clock_reservation_preserves_group_order_during_a_real_writer_hold() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("owner clock order").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let signal = SignalId::new(InstrumentId::new(180), lab_core::TEMPERATURE);
    let fact = |sequence, at| RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(signal, Unit::CELSIUS, at, Value::Float(sequence as f64))
            .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: None,
        lineage: None,
    };
    let first = worker
        .try_admit_at(
            vec![fact(1, Duration::from_secs(1))],
            Duration::from_secs(1),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !barrier.reached() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(barrier.reached());
    // Time passing only triggers the next owner poll. The stored clock row
    // and ID order below are the actual oracle for this periodic stimulus.
    std::thread::sleep(Duration::from_millis(1100));
    worker.poll();
    let second = worker
        .try_admit_at(
            vec![fact(2, Duration::from_secs(2))],
            Duration::from_secs(2),
        )
        .unwrap();
    assert_eq!(*second.start(), *first.end() + 2);
    barrier.release();
    while worker.poll().outstanding_records != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let middle = archive
        .prepare("SELECT record_seq,kind FROM records WHERE record_seq>=?1 AND record_seq<=?2 ORDER BY record_seq")
        .unwrap()
        .query_map(
            rusqlite::params![first.start().to_be_bytes(), second.end().to_be_bytes()],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap()
        .map(|row| {
            let (id, kind) = row.unwrap();
            (u64::from_be_bytes(id.try_into().unwrap()), kind)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        middle,
        vec![
            (*first.start(), "measurement".into()),
            (*first.end() + 1, "clock_anchor".into()),
            (*second.end(), "measurement".into())
        ]
    );
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

fn await_state(worker: &mut RecorderWorker, state: RecordingState) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != state && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let status = worker.poll();
    assert_eq!(status.state, state, "{status:?}");
}

#[test]
fn bounded_worker_admits_a_group_without_waiting_for_its_sqlite_commit() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    worker.request_start("worker fixture").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(76);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "worker fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
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
    worker.try_admit(facts).unwrap();
    assert_eq!(worker.poll().outstanding_records, 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().persisted_through_sequence <= start_watermark && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        worker.poll().persisted_through_sequence,
        start_watermark + 1
    );
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn quiet_recording_progress_requires_a_committed_sqlite_probe() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    worker.request_start("quiet fixture").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    worker.request_probe_at(Duration::from_millis(250)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().confirmed_submission != Some(Duration::from_millis(250))
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        worker.poll().confirmed_submission,
        Some(Duration::from_millis(250))
    );
    assert_eq!(worker.poll().persisted_through_sequence, start_watermark);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn two_causal_fact_groups_under_one_held_writer_use_one_bounded_batch_commit() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("batch fixture").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(176);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "batch source".into(),
            history_capacity: 2,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(10),
        })
        .unwrap();
    worker
        .try_admit_at(runtime.take_recording_facts(), Duration::from_millis(10))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(barrier.reached());
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(20),
        })
        .unwrap();
    worker
        .try_admit_at(runtime.take_recording_facts(), Duration::from_millis(20))
        .unwrap();
    barrier.release();
    while worker.poll().outstanding_records > 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let commits: Vec<u64> = db
        .prepare("SELECT commit_no FROM durable_checkpoints")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| u64::from_be_bytes(row.unwrap().try_into().unwrap()))
        .collect();
    assert_eq!(
        commits,
        [4],
        "start, one batch, stop and boot seal each commit once"
    );
    let facts: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(facts, 2);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn four_full_causal_groups_commit_exactly_the_256_record_batch_limit() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("full batch cap").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let signal = SignalId::new(InstrumentId::new(186), lab_core::TEMPERATURE);
    let deadline = Instant::now() + Duration::from_secs(4);
    for group in 0..4u64 {
        let facts = (1..=64u64)
            .map(|position| {
                let sequence = group * 64 + position;
                RecordingFact::Measurement {
                    sequence,
                    sample: Sample::validated_good(
                        signal,
                        Unit::CELSIUS,
                        Duration::from_millis(sequence),
                        Value::Float(sequence as f64),
                    )
                    .unwrap(),
                    generation: 1,
                    revision: 1,
                    state_revision: None,
                    lineage: None,
                }
            })
            .collect();
        worker
            .try_admit_at(facts, Duration::from_millis(group + 1))
            .unwrap();
        if group == 0 {
            while !barrier.reached() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert!(barrier.reached(), "first SQL stage was never held");
        }
    }
    let pending = worker.poll();
    assert_eq!(pending.outstanding_records, 256);
    assert_eq!(pending.outstanding_groups, 4);
    assert!(pending.outstanding_bytes <= 4 * 1024 * 1024);
    barrier.release();
    while worker.poll().outstanding_records > 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().outstanding_records, 0);
    assert_eq!(worker.poll().outstanding_groups, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    let commits: Vec<u8> = db
        .query_row("SELECT commit_no FROM durable_checkpoints", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 256);
    assert_eq!(u64::from_be_bytes(commits.try_into().unwrap()), 4);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
