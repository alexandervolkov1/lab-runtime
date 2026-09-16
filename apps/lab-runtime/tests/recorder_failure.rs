//! BestEffort ingress loss leaves a durable gap without inventing lost effects.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::host::Clock;
use lab_runtime::recorder::{
    RecorderLimits, RecorderWorker, RecordingState, SqliteStore, WriterBarrier,
};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use serde_json::json;
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
fn transient_client_event_ring_overrun_does_not_create_a_durable_recorder_gap() {
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
    let started_at = service.clock_copy().now();
    service
        .owner_mut()
        .start_recording("ring independent of history", started_at)
        .unwrap();
    let clock = service.clock_copy();
    let by = Instant::now() + Duration::from_secs(2);
    while service.owner_mut().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let initial = service.owner().event_log().oldest_cursor();
    for ordinal in 1..=lab_runtime::events::EVENT_RING_LIMIT + 10 {
        service
            .owner_mut()
            .event_log_mut()
            .host_state(started_at, "client_only", json!({"ordinal":ordinal}))
            .unwrap();
    }
    assert!(service.owner().event_log().oldest_cursor() > initial);
    assert!(service.owner().event_log().scan_after(initial, 1).is_err());
    service.owner_mut().service(&clock).unwrap();
    let status = service.owner_mut().recording_status().unwrap();
    assert_eq!(status.state, RecordingState::Recording);
    assert_eq!(status.coverage, "complete");
    assert_eq!(status.first_missing_fact, None);
    let stopped_at = service.clock_copy().now();
    service.owner_mut().stop_recording_at(stopped_at).unwrap();
    service.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let (coverage, gaps): (String, i64) = db
        .query_row(
            "SELECT intervals.coverage,(SELECT COUNT(*) FROM gaps) FROM recording_intervals AS intervals \
             JOIN runs ON runs.boot_id=intervals.boot_id AND runs.run_no=intervals.run_no \
             WHERE runs.label='ring independent of history'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(coverage, "complete");
    assert_eq!(gaps, 0);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn accepted_prefix_commits_before_reserved_writable_ingress_gap_seal() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker = RecorderWorker::open_with_barrier(
        &path,
        RecorderLimits {
            records: 2,
            bytes: 4096,
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

#[test]
fn worker_panic_before_fact_sql_is_visible_without_a_fabricated_receipt() {
    let path = temporary_database();
    let barrier = WriterBarrier::panic_before_fact_sql();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let old_boot = worker.boot_id().to_owned();
    worker.request_start("panic before fact").unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        std::thread::yield_now();
    }
    let start_watermark = worker.poll().persisted_through_sequence;
    let instrument = InstrumentId::new(973);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "panic fact".into(),
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
    worker
        .try_admit_at(runtime.take_recording_facts(), Duration::from_secs(1))
        .unwrap();
    while !barrier.reached() {
        assert!(Instant::now() < by);
        std::thread::yield_now();
    }
    let failed = loop {
        let status = worker.poll();
        if status.worker_closed {
            break status;
        }
        assert!(
            Instant::now() < by,
            "panic must be detected promptly: {status:?}"
        );
        std::thread::yield_now();
    };
    assert_eq!(failed.state, RecordingState::Failed);
    assert!(
        failed
            .first_error
            .as_deref()
            .unwrap_or("")
            .contains("panicked")
    );
    assert_eq!(failed.persisted_through_sequence, start_watermark);
    assert_eq!(failed.coverage, "unknown_tail");
    assert_eq!(failed.outstanding_records, 1);
    for _ in 0..3 {
        let repeated = worker.poll();
        assert_eq!(repeated.first_error, failed.first_error);
        assert_eq!(repeated.persisted_through_sequence, start_watermark);
        assert_eq!(repeated.outstanding_records, 1);
    }
    drop(worker);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_eq!(reopened.database_id().len(), 32);
    assert_ne!(reopened.boot_id(), old_boot);
    drop(reopened);
    let db = rusqlite::Connection::open(&path).unwrap();
    let old_rows: i64 = db
        .query_row(
            "SELECT count(*) FROM measurements WHERE boot_id=(SELECT boot_id FROM runs WHERE label='panic before fact')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_rows, 0);
    let (state, coverage): (String, String) = db
        .query_row(
            "SELECT state,coverage FROM runs WHERE label='panic before fact'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "interrupted");
    assert_eq!(coverage, "unknown_tail");
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_fact_insert_error_preserves_start_receipt_and_unknown_tail() {
    let path = temporary_database();
    drop(SqliteStore::open(&path).unwrap());
    let external = rusqlite::Connection::open(&path).unwrap();
    external
        .execute_batch(
            "CREATE TRIGGER fail_worker_measurement BEFORE INSERT ON measurements
             BEGIN SELECT RAISE(ABORT,'injected worker insert failure'); END;",
        )
        .unwrap();
    drop(external);
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    worker.request_start("failing insert").unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        std::thread::yield_now();
    }
    let start_watermark = worker.poll().persisted_through_sequence;
    let instrument = InstrumentId::new(974);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "disk error fixture".into(),
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
    worker
        .try_admit_at(runtime.take_recording_facts(), Duration::from_secs(1))
        .unwrap();
    let failed = loop {
        let status = worker.poll();
        if status.worker_closed {
            break status;
        }
        assert!(
            Instant::now() < by,
            "SQL error must close writer: {status:?}"
        );
        std::thread::yield_now();
    };
    assert_eq!(failed.state, RecordingState::Failed);
    assert_eq!(failed.coverage, "unknown_tail");
    assert_eq!(failed.persisted_through_sequence, start_watermark);
    assert!(
        failed
            .first_error
            .as_deref()
            .unwrap_or("")
            .contains("injected worker insert failure")
    );
    assert!(!failed.failure_persisted);
    drop(worker);
    let reopened = SqliteStore::open(&path).unwrap();
    drop(reopened);
    let db = rusqlite::Connection::open(&path).unwrap();
    let (state, coverage): (String, String) = db
        .query_row(
            "SELECT state,coverage FROM runs WHERE label='failing insert'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "interrupted");
    assert_eq!(coverage, "unknown_tail");
    let rows: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
