//! Safe shutdown and durable flush have separate, honest terminal evidence.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore, WriterBarrier,
    },
    service::{ServiceHost, ServiceOptions},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-shutdown-{suffix}.sqlite"))
}

#[test]
fn shutdown_seals_active_interval_and_closes_worker_before_reporting_flush_success() {
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
    let mut service =
        ServiceHost::startup_from_trusted_host(options, HostCore::virtual_demo().unwrap()).unwrap();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    service
        .owner_mut()
        .start_recording("shutdown fixture", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(terminal.safe_confirmed);
    assert!(
        terminal.recorder_flushed,
        "terminal={terminal:?}, recorder={:?}",
        service.owner().recording_status()
    );
    assert!(terminal.exit_success);
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Closed
    );
    drop(service);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let sealed_boots: i64 = archive
        .query_row(
            "SELECT COUNT(*) FROM runtime_boots WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        sealed_boots, 1,
        "worker close alone is not a durable boot seal"
    );
    let sealed_intervals: i64 = archive
        .query_row(
            "SELECT COUNT(*) FROM recording_intervals WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sealed_intervals, 1);
    let (exit_summary, ended_wall): (String, Option<i64>) = archive
        .query_row(
            "SELECT exit_summary,ended_wall_us FROM runtime_boots WHERE state='sealed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&exit_summary).unwrap();
    assert_eq!(summary["outputs"][0]["safe_confirmed"], true);
    assert_eq!(summary["outputs"][0]["lease_present"], false);
    assert_eq!(summary["unfinished_managed_workers"], 0);
    assert!(ended_wall.is_some(), "final worker anchor must observe UTC");
    let shutdown_at: Vec<u8> = archive
        .query_row(
            "SELECT published_at FROM records WHERE kind='shutdown'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(
        shutdown_at,
        vec![0u8; 16],
        "finish must use owner monotonic time"
    );
    drop(archive);
    let reopened = SqliteStore::open(&path).unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn blocked_writer_expires_finite_flush_without_falsifying_safe_output_evidence() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    service
        .owner_mut()
        .start_recording("stalled flush", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    let at = clock.now();
    let plant = service.owner().plant_id();
    service
        .owner_mut()
        .command(lab_core::Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at,
        })
        .unwrap();
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_records
            > 0
    );
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(terminal.safe_confirmed);
    assert!(!terminal.recorder_flushed);
    assert!(terminal.recorder_unfinished);
    assert!(!terminal.exit_success);
    barrier.release();
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn stop_and_finish_drain_all_four_accepted_groups_with_full_normal_credit() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("full credit shutdown").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while worker.poll().state != RecordingState::Recording && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    let instrument = InstrumentId::new(811);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "full credit source".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    for n in 1..=4u64 {
        let at = Duration::from_millis(n);
        runtime
            .command(Command::RefreshMeasurement {
                instrument,
                parameter: lab_core::TEMPERATURE,
                at,
            })
            .unwrap();
        worker
            .try_admit_at(runtime.take_recording_facts(), at)
            .unwrap();
        if n == 1 {
            while !barrier.reached() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert!(
                barrier.reached(),
                "first actual SQLite batch never reached barrier"
            );
        }
    }
    assert_eq!(worker.poll().outstanding_groups, 4);
    worker.request_stop().unwrap();
    assert_eq!(worker.poll().state, RecordingState::Stopping);
    barrier.release();
    while worker.poll().state != RecordingState::Idle && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Idle);
    assert_eq!(worker.poll().outstanding_groups, 0);
    worker.request_finish().unwrap();
    while worker.poll().state != RecordingState::Closed && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    let sealed: i64 = db
        .query_row(
            "SELECT count(*) FROM runtime_boots WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 4,
        "every accepted group belongs to the terminal prefix"
    );
    assert_eq!(sealed, 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn terminal_sqlite_insert_failure_rolls_back_boot_seal_and_keeps_safe_outcome_honest() {
    let path = temporary_database();
    let mut initial = SqliteStore::open(&path).unwrap();
    initial.finish_boot(Duration::ZERO).unwrap();
    drop(initial);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_terminal_shutdown BEFORE INSERT ON records
        WHEN NEW.kind='shutdown' BEGIN SELECT RAISE(ABORT,'injected terminal insert failure'); END;")
        .unwrap();
    drop(db);
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
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "finite Recorder flush grace was exceeded"
        );
        std::thread::yield_now();
    };
    assert!(terminal.safe_confirmed);
    assert!(!terminal.recorder_flushed);
    assert!(terminal.recorder_error);
    assert!(!terminal.exit_success);
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let seals:i64=db.query_row("SELECT count(*) FROM records WHERE kind='shutdown' AND boot_id=(SELECT boot_id FROM runtime_boots WHERE state='active')",[],
        |row|row.get(0)).unwrap();
    let active: i64 = db
        .query_row(
            "SELECT count(*) FROM runtime_boots WHERE state='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seals, 0);
    assert_eq!(
        active, 1,
        "failed terminal transaction cannot update boot state"
    );
    drop(db);
    let reopened = SqliteStore::open(&path).unwrap();
    drop(reopened);
    let db = rusqlite::Connection::open(&path).unwrap();
    let interrupted: i64 = db
        .query_row(
            "SELECT count(*) FROM runtime_boots WHERE state='interrupted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(interrupted, 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
