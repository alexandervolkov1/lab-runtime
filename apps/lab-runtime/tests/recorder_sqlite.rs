//! Real-file acceptance for the storage adapter, including reopen from WAL.

use lab_core::{Command, InstrumentId, Runtime, Value, VirtualInstrumentConfig};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore, WriterBarrier,
    },
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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
