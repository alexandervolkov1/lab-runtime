//! C20 configuration activation uses M7's unchanged four-group ingress credit.

use lab_core::{
    Command, InstrumentId, Sample, SignalId, Unit, Value, plant::ThermalPlantConfig,
    recording::RecordingFact,
};
use lab_runtime::recorder::{
    ConfigurationLifecycleRecord, OperationRecord, ProvenanceEntry, RecorderLimits, RecorderWorker,
    RecordingState, WriterBarrier,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-reload-budget-{suffix}.sqlite"))
}

fn provenance(content: &[u8]) -> Vec<ProvenanceEntry> {
    vec![ProvenanceEntry {
        kind: "runtime_toml".into(),
        encoding: "toml_utf8_v1".into(),
        content: content.to_vec(),
    }]
}

fn await_state(worker: &mut RecorderWorker, expected: RecordingState) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != expected {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

fn lifecycle(operation_id: u64) -> ConfigurationLifecycleRecord {
    ConfigurationLifecycleRecord {
        operation_id,
        operation_kind: "reconnect_resource",
        base_revision: 1,
        committed_revision: 1,
        toml_hash: [0x5a; 32],
        affected: vec!["resource:7:binding_generation:2".into()],
        reason: None,
        at: Duration::from_millis(10),
    }
}

fn measurement_fact(sequence: u64) -> RecordingFact {
    let signal = SignalId::new(InstrumentId::new(7), lab_core::TEMPERATURE);
    RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(
            signal,
            Unit::CELSIUS,
            Duration::from_millis(10 + sequence),
            Value::Float(20.0 + sequence as f64),
        )
        .unwrap(),
        generation: 2,
        revision: 2,
        state_revision: None,
        lineage: None,
    }
}

fn unavailable_fact() -> RecordingFact {
    let mut runtime = lab_core::Runtime::new();
    let instrument = InstrumentId::new(8);
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: instrument,
            name: "unavailable reconnect probe".into(),
            history_capacity: 1,
            ambient_temperature: 20.0,
            gain_per_percent: 1.0,
            initial_temperature: 20.0,
            time_constant: Duration::from_secs(1),
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(Command::InjectPlantMeasurementFailure {
            instrument,
            at: Duration::from_millis(11),
        })
        .unwrap();
    let mut facts = runtime.take_recording_facts();
    assert_eq!(facts.len(), 1);
    facts.pop().unwrap()
}

fn start_activated_worker(path: &std::path::Path) -> RecorderWorker {
    let mut worker = RecorderWorker::open(path, RecorderLimits::default()).unwrap();
    worker
        .request_activation(provenance(b"old=true"), vec![])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 1 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    worker.request_start("live activation ordering").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    worker
}

fn close_worker(worker: &mut RecorderWorker) {
    worker.request_stop().unwrap();
    await_state(worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(worker, RecordingState::Closed);
}

fn record_sequences(db: &rusqlite::Connection) -> Vec<u64> {
    let mut statement = db
        .prepare("SELECT record_seq FROM records ORDER BY record_seq")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|value| {
            let bytes: [u8; 8] = value.unwrap().try_into().unwrap();
            u64::from_be_bytes(bytes)
        })
        .collect()
}

#[test]
fn live_activation_capacity_allows_fifo_fact_before_commit_without_identity_hole() {
    let path = temporary_database();
    let mut worker = start_activated_worker(&path);
    let lifecycle = lifecycle(41);
    let generation = worker
        .try_reserve_live_activation(&lifecycle)
        .unwrap()
        .unwrap();

    let mut fact_ranges = Vec::new();
    for sequence in 1..=3 {
        fact_ranges.push(worker.try_admit(vec![measurement_fact(sequence)]).unwrap());
    }
    worker
        .commit_reserved_live_activation(generation, provenance(b"new=true"), vec![], lifecycle)
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    while (worker.poll().activation_generation != 2 || worker.poll().outstanding_groups != 0)
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    let status = worker.poll();
    assert_eq!(status.state, RecordingState::Recording, "{status:?}");
    assert_eq!(status.activation_generation, 2);
    close_worker(&mut worker);
    drop(worker);

    let db = rusqlite::Connection::open(&path).unwrap();
    let kinds: Vec<String> = {
        let mut statement = db
            .prepare(
                "SELECT kind FROM records WHERE kind IN ('measurement','configuration_lifecycle') ORDER BY record_seq",
            )
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        kinds,
        [
            "measurement",
            "measurement",
            "measurement",
            "configuration_lifecycle"
        ]
    );
    let sequences = record_sequences(&db);
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    let lifecycle_sequence: Vec<u8> = db
        .query_row(
            "SELECT record_seq FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        u64::from_be_bytes(lifecycle_sequence.try_into().unwrap()),
        *fact_ranges.last().unwrap().end() + 1
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancellation_after_intervening_fact_groups_releases_credit_without_sequence_rewind() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker
        .request_activation(provenance(b"old=true"), vec![])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 1 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    worker.request_start("cancel ordered activation").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let generation = worker
        .try_reserve_live_activation(&lifecycle(42))
        .unwrap()
        .unwrap();
    assert!(worker.try_reserve_live_activation(&lifecycle(43)).is_err());
    for sequence in 1..=3 {
        worker.try_admit(vec![measurement_fact(sequence)]).unwrap();
    }
    let saturated = worker.poll();
    assert_eq!(saturated.limits, RecorderLimits::default());
    assert_eq!(saturated.outstanding_groups, 4);
    assert_eq!(saturated.outstanding_records, 4);

    worker
        .cancel_live_activation_reservation(generation)
        .unwrap();
    let cancelled = worker.poll();
    assert_eq!(cancelled.outstanding_groups, 3);
    assert_eq!(cancelled.outstanding_records, 3);
    barrier.release();
    while worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline, "{:?}", worker.poll());
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    close_worker(&mut worker);
    drop(worker);

    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM measurements", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let sequences = record_sequences(&db);
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unavailable_fact_before_lifecycle_cancellation_remains_contiguous_and_recorded() {
    let path = temporary_database();
    let mut worker = start_activated_worker(&path);
    let generation = worker
        .try_reserve_live_activation(&lifecycle(44))
        .unwrap()
        .unwrap();
    worker.try_admit(vec![unavailable_fact()]).unwrap();
    worker
        .cancel_live_activation_reservation(generation)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline, "{:?}", worker.poll());
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    close_worker(&mut worker);
    drop(worker);

    let db = rusqlite::Connection::open(&path).unwrap();
    let row: (String, Option<f64>) = db
        .query_row("SELECT quality,float_value FROM measurements", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(row, ("unavailable".into(), None));
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let sequences = record_sequences(&db);
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn c20_live_activation_reserves_one_of_the_same_four_groups_and_commits_one_fact() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker
        .request_activation(provenance(b"old=true"), vec![])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 1 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    worker.request_start("reload budget").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    for seq in 1..=4 {
        worker
            .try_admit_operation(OperationRecord {
                scope: "reload-budget".into(),
                request_seq: seq,
                command: "reference_retune",
                phase: "accepted",
                data: "{}".into(),
                outcome_basis: "application_admission",
                at: Duration::from_millis(seq),
            })
            .unwrap();
    }
    assert_eq!(worker.poll().outstanding_groups, 4);
    let lifecycle = ConfigurationLifecycleRecord {
        operation_id: 7,
        operation_kind: "reload_configuration",
        base_revision: 1,
        committed_revision: 2,
        toml_hash: [0x5a; 32],
        affected: vec!["instrument:41:generation:1".into()],
        reason: None,
        at: Duration::from_millis(10),
    };
    assert_eq!(
        worker.try_reserve_live_activation(&lifecycle).unwrap(),
        None
    );
    assert_eq!(worker.poll().state, RecordingState::Recording);

    barrier.release();
    while worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let reservation = worker
        .try_reserve_live_activation(&lifecycle)
        .unwrap()
        .expect("one ordinary group must now be reserved");
    assert_eq!(worker.poll().outstanding_groups, 1);
    worker
        .commit_reserved_live_activation(reservation, provenance(b"new=true"), vec![], lifecycle)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 2 || worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline, "{:?}", worker.poll());
        std::thread::yield_now();
    }
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);

    let db = rusqlite::Connection::open(&path).unwrap();
    let payload: Vec<u8> = db
        .query_row(
            "SELECT payload FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["base_revision"], "1");
    assert_eq!(payload["committed_revision"], "2");
    assert_eq!(payload["activation_generation"], "2");
    assert_eq!(payload["activation_root"].as_str().unwrap().len(), 64);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
