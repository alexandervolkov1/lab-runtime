//! Safe shutdown and durable flush have separate, honest terminal evidence.

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
