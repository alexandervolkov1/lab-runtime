//! BestEffort ingress loss leaves a durable gap without inventing lost effects.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::recorder::{RecorderLimits, RecorderWorker, RecordingState, WriterBarrier};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-gap-{suffix}.sqlite"))
}

#[test]
fn accepted_prefix_commits_before_reserved_writable_ingress_gap_seal() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker = RecorderWorker::open_with_barrier(
        &path,
        RecorderLimits {
            records: 2,
            bytes: 1024,
            groups: 1,
        },
        barrier.clone(),
    )
    .unwrap();
    worker.request_start("writable gap").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(972);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "gap source".into(),
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
            at: Duration::from_millis(10),
        })
        .unwrap();
    worker
        .try_admit_at(runtime.take_recording_facts(), Duration::from_millis(10))
        .unwrap();
    while !barrier.reached() && Instant::now() < deadline {
        std::thread::yield_now()
    }
    assert!(barrier.reached());
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(20),
        })
        .unwrap();
    assert!(
        worker
            .try_admit_at(runtime.take_recording_facts(), Duration::from_millis(20))
            .is_err()
    );
    assert_eq!(worker.poll().state, RecordingState::Failed);
    barrier.release();
    worker.request_finish().unwrap();
    let finish_deadline = Instant::now() + Duration::from_secs(3);
    while !worker.poll().terminal_seal_committed && Instant::now() < finish_deadline {
        std::thread::yield_now();
    }
    assert!(
        worker.poll().terminal_seal_committed,
        "writable failure must commit prefix/gap/boot seal: {:?}",
        worker.poll()
    );
    assert_eq!(worker.poll().first_missing_fact, Some(2));
    assert!(worker.poll().failure_persisted);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let measurements: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    let gaps: i64 = db
        .query_row(
            "SELECT count(*) FROM runtime_events \
        WHERE category='recorder_gap'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let coverage: String = db
        .query_row(
            "SELECT coverage FROM recording_intervals \
        WHERE state='failed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(measurements, 1);
    assert_eq!(gaps, 1);
    assert_eq!(coverage, "gap");
    let gap_wall: (Option<i64>, Option<String>) = db
        .query_row(
            "SELECT wall_estimate_us,wall_basis FROM records WHERE kind='recorder_gap'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(gap_wall.0.is_some());
    assert_eq!(gap_wall.1.as_deref(), Some("boot_anchor"));
    let (first, known, last_confirmed): (Vec<u8>, Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT first_missing,known_count,last_confirmed FROM gaps",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(u64::from_be_bytes(first.try_into().unwrap()), 2);
    assert_eq!(u64::from_be_bytes(known.try_into().unwrap()), 1);
    assert_eq!(u64::from_be_bytes(last_confirmed.try_into().unwrap()), 3);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
