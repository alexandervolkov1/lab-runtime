//! Safe shutdown and durable flush have separate, honest terminal evidence.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, VirtualInstrumentConfig,
    control::ControllerState,
};
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
fn two_host_runs_and_active_shutdown_reopen_with_distinct_fifo_seals() {
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
    let boot = service.boot_id().to_owned();
    for label in ["first host run", "second host run"] {
        let start_at = service.clock().now();
        service
            .owner_mut()
            .start_recording(label, start_at)
            .unwrap();
        let by = Instant::now() + Duration::from_secs(2);
        while service.owner().recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            let clock = service.clock_copy();
            service.owner_mut().service(&clock).unwrap();
            std::thread::yield_now();
        }
        if label == "first host run" {
            let stop_at = service.clock().now();
            service.owner_mut().stop_recording_at(stop_at).unwrap();
            while service.owner().recording_status().unwrap().state != RecordingState::Idle {
                assert!(Instant::now() < by);
                let clock = service.clock_copy();
                service.owner_mut().service(&clock).unwrap();
                std::thread::yield_now();
            }
        }
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
    assert!(terminal.recorder_flushed, "{terminal:?}");
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let runs = db
        .prepare("SELECT run_no,label,state,coverage FROM runs ORDER BY run_no")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(runs.len(), 2);
    for (index, (id, label, state, coverage)) in runs.iter().enumerate() {
        assert_eq!(
            u64::from_be_bytes(id.clone().try_into().unwrap()),
            index as u64 + 1
        );
        assert_eq!(
            label,
            if index == 0 {
                "first host run"
            } else {
                "second host run"
            }
        );
        assert_eq!(state, "sealed");
        assert_eq!(coverage, "complete");
    }
    let boundaries = db
        .prepare(
            "SELECT record_seq FROM records WHERE kind='boundary_snapshot' ORDER BY record_seq",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let seals = db
        .prepare("SELECT record_seq FROM records WHERE kind='interval_seal' ORDER BY record_seq")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let final_record: Vec<u8> = db
        .query_row(
            "SELECT record_seq FROM records WHERE kind='shutdown'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((boundaries.len(), seals.len()), (2, 2));
    assert!(
        boundaries[0] < seals[0]
            && seals[0] < boundaries[1]
            && boundaries[1] < seals[1]
            && seals[1] < final_record
    );
    let checkpoint: Vec<u8> = db
        .query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint, final_record);
    drop(db);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_ne!(reopened.boot_id(), boot);
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
fn running_controller_shutdown_drains_held_measurement_before_final_safe_seal() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    let controller = service.owner().controller_id();
    service
        .owner_mut()
        .command(Command::StartController {
            controller,
            at: clock.now(),
        })
        .unwrap();
    let ready_by = Instant::now() + Duration::from_secs(2);
    loop {
        service.owner_mut().service(&clock).unwrap();
        let QueryResult::Controller(snapshot) = service
            .owner()
            .query(Query::Controller(controller))
            .unwrap()
        else {
            panic!()
        };
        if snapshot.state == ControllerState::Running {
            break;
        }
        assert!(
            Instant::now() < ready_by,
            "native controller never reached Running"
        );
        std::thread::yield_now();
    }
    service
        .owner_mut()
        .start_recording("Running held fact shutdown", clock.now())
        .unwrap();
    let started_by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < started_by);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let plant = service.owner().plant_id();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at: clock.now(),
        })
        .unwrap();
    let held_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < held_by);
        std::thread::yield_now();
    }
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_records
            > 0
    );
    service.request_shutdown().unwrap();
    assert!(service.shutdown_step().unwrap().is_none());
    barrier.release();
    let finished_by = Instant::now() + Duration::from_secs(4);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(Instant::now() < finished_by);
        std::thread::yield_now();
    };
    assert!(terminal.safe_confirmed, "{terminal:?}");
    assert!(terminal.recorder_flushed, "{terminal:?}");
    assert!(!terminal.recorder_unfinished);
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let (measurements, safe, seals): (i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM measurements),
                    (SELECT COUNT(*) FROM output_events WHERE stage='safe_readback_verified'),
                    (SELECT COUNT(*) FROM runtime_boots WHERE state='sealed')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(
        measurements >= 1,
        "held accepted measurement must survive shutdown"
    );
    assert!(safe >= 1, "trusted virtual safe evidence must be archived");
    assert_eq!(seals, 1);
    let (measurement_record, safe_record, interval_seal, shutdown_record):
        (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT (SELECT MAX(record_seq) FROM measurements),
                    (SELECT MAX(record_seq) FROM output_events WHERE stage='safe_readback_verified'),
                    (SELECT MAX(record_seq) FROM records WHERE kind='interval_seal'),
                    (SELECT MAX(record_seq) FROM records WHERE kind='shutdown')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert!(measurement_record < safe_record);
    assert!(safe_record < interval_seal);
    assert!(interval_seal < shutdown_record);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn late_finish_commit_is_archived_after_process_flush_timeout_without_revising_terminal_result() {
    let path = temporary_database();
    let barrier = WriterBarrier::held_finish();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let boot = worker.boot_id().to_owned();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    service.request_shutdown().unwrap();
    let by = Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        if let Some(terminal) = service.shutdown_step().unwrap() {
            break terminal;
        }
        assert!(Instant::now() < by);
        std::thread::yield_now();
    };
    assert!(
        barrier.reached(),
        "Finish must have been accepted before timeout"
    );
    assert!(terminal.safe_confirmed);
    assert!(!terminal.recorder_flushed);
    assert!(terminal.recorder_unfinished);
    assert!(!terminal.exit_success);
    barrier.release();
    drop(service);
    let close_by = Instant::now() + Duration::from_secs(3);
    let recovered = loop {
        match SqliteStore::open(&path) {
            Ok(store) => break store,
            Err(_) => assert!(
                Instant::now() < close_by,
                "late Finish did not close worker"
            ),
        }
        std::thread::yield_now();
    };
    assert_ne!(recovered.boot_id(), boot);
    drop(recovered);
    let db = rusqlite::Connection::open(&path).unwrap();
    let (state, shutdown_rows): (String, i64) = (
        db.query_row("SELECT state FROM runtime_boots WHERE boot_id=(SELECT boot_id FROM records WHERE kind='shutdown')", [], |row| row.get(0)).unwrap(),
        db.query_row("SELECT count(*) FROM records WHERE kind='shutdown'", [], |row| row.get(0)).unwrap(),
    );
    assert_eq!(state, "sealed");
    assert_eq!(shutdown_rows, 1);
    drop(db);
    std::fs::remove_file(path).unwrap();
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

#[test]
fn committed_terminal_seal_with_close_error_does_not_report_a_successful_flush() {
    let path = temporary_database();
    let barrier = WriterBarrier::fail_close_after_seal();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let boot = service.boot_id().to_owned();
    service.request_shutdown().unwrap();
    let by = Instant::now() + Duration::from_secs(3);
    let terminal = loop {
        if let Some(terminal) = service.shutdown_step().unwrap() {
            break terminal;
        }
        assert!(Instant::now() < by, "close fault exceeded finite grace");
        std::thread::yield_now();
    };
    let status = service.owner().recording_status().unwrap();
    assert!(terminal.safe_confirmed);
    assert!(
        status.terminal_seal_committed,
        "SQL seal must have committed"
    );
    assert!(status.worker_closed);
    assert_eq!(status.state, RecordingState::Failed);
    assert!(
        status
            .first_error
            .as_deref()
            .unwrap_or("")
            .contains("close")
    );
    assert!(!terminal.recorder_flushed);
    assert!(terminal.recorder_error);
    assert!(!terminal.exit_success);
    drop(service);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_ne!(reopened.boot_id(), boot);
    drop(reopened);
    let db = rusqlite::Connection::open(&path).unwrap();
    let seals: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_boots WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seals, 1, "close failure cannot erase a committed seal");
    drop(db);
    std::fs::remove_file(path).unwrap();
}
