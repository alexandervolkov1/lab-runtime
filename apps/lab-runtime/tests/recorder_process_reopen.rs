//! Process death before a confirmed SQLite commit leaves an honest archive tail.

use lab_core::{
    Command as DomainCommand, InstrumentId, Query, QueryResult, Runtime, Sample, SignalId, Unit,
    Value, VirtualInstrumentConfig, recording::RecordingFact,
};
use lab_runtime::recorder::{
    HistoryFilter, RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore,
    WriterBarrier,
};
use lab_runtime::{
    application::Application,
    host::{Clock, HostCore},
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-m7-process-reopen-{suffix}.sqlite"))
}

fn assert_integrity_ok(path: &PathBuf) {
    let archive = rusqlite::Connection::open(path).unwrap();
    let result: String = archive
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(result, "ok", "offline SQLite integrity check failed");
}

fn assert_crashed_lifecycle_is_unsealed(path: &PathBuf, boot: &str) {
    let archive = rusqlite::Connection::open(path).unwrap();
    let boot_id = boot_bytes(boot);
    let boot_state: String = archive
        .query_row(
            "SELECT state FROM runtime_boots WHERE boot_id=?1",
            [&boot_id],
            |row| row.get(0),
        )
        .unwrap();
    let run: (String, String) = archive
        .query_row(
            "SELECT state,coverage FROM runs WHERE boot_id=?1",
            [&boot_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let interval: (String, String) = archive
        .query_row(
            "SELECT state,coverage FROM recording_intervals WHERE boot_id=?1",
            [&boot_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(boot_state, "active");
    assert_eq!(run, ("recording".into(), "complete".into()));
    assert_eq!(interval, ("recording".into(), "complete".into()));
    // Coverage has not observed a known gap, but the active lifecycle rows are
    // unsealed: structural readability is not semantic completeness.
}

#[test]
fn child_holds_real_retune_terminal_after_durable_acceptance() {
    let Some(path) = std::env::var_os("LAB_M7_ACCEPTED_ONLY_DB") else {
        return;
    };
    let path = PathBuf::from(path);
    let barrier = WriterBarrier::held_terminal_operation_after_acceptance();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(
        worker,
        lab_runtime::recorder::RecordingPolicy::BestEffort,
        Duration::ZERO,
    )
    .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("accepted only", now)
        .unwrap();
    let ready_by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < ready_by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        thread::yield_now();
    }
    let start_watermark = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let mut app = Application::new(service.boot_id()).unwrap();
    let frame = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"h","op":"hello","args":{"scope":null}
        })),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let reference = service.owner().reference_id().get().to_string();
    let outcome = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"retune","op":"reference_retune",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":reference,"expected_revision":"1","target":53.0,"rate":1.0}
        })),
    );
    assert_eq!(outcome[0]["state"], "accepted");
    assert_eq!(outcome[1]["state"], "completed");
    assert_eq!(outcome[1]["result"]["revision"], "2");
    assert!(
        barrier.wait_until_reached(Duration::from_secs(2)),
        "terminal operation did not reach the authoritative SQLite barrier"
    );
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .persisted_through_sequence
            > start_watermark,
        "acceptance must commit before terminal barrier"
    );
    println!("M7_ACCEPTED_ONLY_REACHED {} {}", service.boot_id(), scope);
    std::io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(10));
    panic!("parent did not kill accepted-only child");
}

#[test]
fn killed_retune_after_durable_acceptance_has_unknown_terminal_and_is_not_replayed() {
    let path = temporary_database();
    let (old_boot, scope) = kill_selected_child(
        &path,
        "child_holds_real_retune_terminal_after_durable_acceptance",
        "LAB_M7_ACCEPTED_ONLY_DB",
        None,
        "M7_ACCEPTED_ONLY_REACHED ",
    );
    let new_boot = "bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc";
    let reopened = SqliteStore::open_with_boot(&path, new_boot).unwrap();
    reopened.close().unwrap();
    let archive = rusqlite::Connection::open(&path).unwrap();
    let rows: Vec<(String, String, String)> = archive
        .prepare(
            "SELECT phase,request_scope,request_seq FROM operation_events
         WHERE command='reference_retune' ORDER BY record_seq",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(rows, [("accepted".into(), scope, "1".into())]);
    let coverage: String = archive
        .query_row(
            "SELECT coverage FROM runs WHERE boot_id=?1 AND run_no=?2",
            rusqlite::params![boot_bytes(&old_boot), 1u64.to_be_bytes().to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(coverage, "unknown_tail");
    drop(archive);
    // Reopen loads durable history; it never dispatches the old client mutation.
    let fresh = HostCore::virtual_demo().unwrap();
    let QueryResult::Reference(reference) =
        fresh.query(Query::Reference(fresh.reference_id())).unwrap()
    else {
        panic!("fresh reference query returned another domain kind")
    };
    let revision = match reference {
        lab_core::reference::ReferenceSnapshot::Fixed { revision, .. }
        | lab_core::reference::ReferenceSnapshot::Ramp { revision, .. } => revision,
    };
    assert_eq!(revision, 1);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn child_holds_a_real_batch_at_selected_commit_stage() {
    let Some(path) = std::env::var_os("LAB_M7_PROCESS_REOPEN_DB") else {
        return;
    };
    let path = PathBuf::from(path);
    let after_commit =
        std::env::var("LAB_M7_PROCESS_REOPEN_STAGE").is_ok_and(|stage| stage == "after_commit");
    let barrier = if after_commit {
        WriterBarrier::held_after_fact_commit()
    } else {
        WriterBarrier::held()
    };
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("process A").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    let instrument = InstrumentId::new(701);
    let mut runtime = Runtime::new();
    runtime
        .command(DomainCommand::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "process A source".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(DomainCommand::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    worker.try_admit(runtime.take_recording_facts()).unwrap();
    assert!(
        barrier.wait_until_reached(Duration::from_secs(2)),
        "writer never reached the held batch stage"
    );
    assert_eq!(worker.poll().outstanding_records, 1);
    assert_eq!(worker.poll().persisted_through_sequence, start_watermark);
    let marker = if after_commit {
        "M7_POSTCOMMIT_REACHED"
    } else {
        "M7_PRECOMMIT_REACHED"
    };
    println!("{marker} {} {}", worker.boot_id(), worker.database_id());
    std::io::stdout().flush().unwrap();
    // The parent kills this real process while its single writer remains held.
    thread::sleep(Duration::from_secs(10));
    panic!("parent did not kill the held child process");
}

#[test]
fn child_commits_three_equal_time_history_rows_before_parent_kills_its_receipt() {
    let Some(path) = std::env::var_os("LAB_M7_PROCESS_PAGE_DB") else {
        return;
    };
    let barrier = WriterBarrier::held_after_fact_commit();
    let mut worker = RecorderWorker::open_with_barrier(
        &PathBuf::from(path),
        RecorderLimits::default(),
        barrier.clone(),
    )
    .unwrap();
    worker.request_start("killed page A").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != RecordingState::Recording && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(worker.poll().state, RecordingState::Recording);
    let signal = SignalId::new(InstrumentId::new(704), lab_core::TEMPERATURE);
    let facts = (1..=3u64)
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
        .collect();
    worker.try_admit(facts).unwrap();
    assert!(
        barrier.wait_until_reached(Duration::from_secs(2)),
        "three-row SQL commit never reached receipt barrier"
    );
    println!(
        "M7_KILLED_PAGE_COMMIT {} {}",
        worker.boot_id(),
        worker.database_id()
    );
    std::io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(10));
    panic!("parent did not kill the paged-history child");
}

#[test]
fn killed_equal_time_archive_pages_exactly_the_committed_prefix_under_a_new_boot() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_selected_child(
        &path,
        "child_commits_three_equal_time_history_rows_before_parent_kills_its_receipt",
        "LAB_M7_PROCESS_PAGE_DB",
        None,
        "M7_KILLED_PAGE_COMMIT ",
    );
    let new_boot = "dddddddddddddddddddddddddddddddd";
    let reopened = SqliteStore::open_with_boot(&path, new_boot).unwrap();
    assert_eq!(reopened.database_id(), database_id);
    assert_ne!(new_boot, old_boot);
    let filter = HistoryFilter {
        boot_id: old_boot.clone(),
        run_no: 1,
        instrument: InstrumentId::new(704),
        parameter: lab_core::TEMPERATURE,
        from: Duration::ZERO,
        to: Duration::from_secs(2),
    };
    let mut cursor = None;
    let mut ids = Vec::new();
    let mut values = Vec::new();
    let mut watermark = None;
    loop {
        let page = reopened
            .read_history_measurements(&filter, cursor.as_ref(), 1)
            .unwrap();
        assert_eq!(page.coverage, "unknown_tail");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.watermark, *watermark.get_or_insert(page.watermark));
        ids.push(page.rows[0].record_sequence);
        values.push(page.rows[0].value.clone());
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
        assert!(ids.len() < 4, "keyset cursor did not finish");
    }
    assert_eq!(ids, [3, 4, 5]);
    assert_eq!(
        values,
        [
            Some(Value::Float(21.0)),
            Some(Value::Float(22.0)),
            Some(Value::Float(23.0))
        ]
    );
    reopened.close().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let sql_ids: Vec<u64> = db
        .prepare("SELECT record_seq FROM measurements WHERE boot_id=?1 ORDER BY record_seq")
        .unwrap()
        .query_map([boot_bytes(&old_boot)], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            Ok(u64::from_be_bytes(bytes.try_into().unwrap()))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, sql_ids);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn killed_process_a_reopens_in_b_without_promoting_uncommitted_work() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_held_child(&path, "before_commit", "M7_PRECOMMIT_REACHED ");
    assert_integrity_ok(&path);
    assert_crashed_lifecycle_is_unsealed(&path, &old_boot);
    let new_boot = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert_ne!(old_boot, new_boot);
    let mut reopened = SqliteStore::open_with_boot(&path, new_boot).unwrap();
    assert_eq!(reopened.database_id(), database_id);
    let old_filter = HistoryFilter {
        boot_id: old_boot.clone(),
        run_no: 1,
        instrument: InstrumentId::new(701),
        parameter: lab_core::TEMPERATURE,
        from: Duration::ZERO,
        to: Duration::from_secs(2),
    };
    let old_page = reopened
        .read_history_measurements(&old_filter, None, 8)
        .unwrap();
    assert!(old_page.rows.is_empty(), "held A row was never committed");
    assert_eq!(old_page.coverage, "unknown_tail");
    reopened.start_run("process B").unwrap();
    let instrument = InstrumentId::new(702);
    let mut runtime = Runtime::new();
    runtime
        .command(DomainCommand::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "process B source".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(DomainCommand::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    reopened
        .append_facts(&runtime.take_recording_facts())
        .unwrap();
    reopened.stop_run().unwrap();
    let fresh = reopened
        .read_history_measurements(
            &HistoryFilter {
                boot_id: new_boot.into(),
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
    assert_eq!(fresh.rows.len(), 1);
    assert_eq!(fresh.coverage, "complete");
    reopened.finish_boot(Duration::from_secs(2)).unwrap();
    reopened.close().unwrap();
    let archive = rusqlite::Connection::open(&path).unwrap();
    let old_state: String = archive
        .query_row(
            "SELECT state FROM runtime_boots WHERE boot_id=?1",
            [boot_bytes(&old_boot)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_state, "interrupted");
    let boot_count: i64 = archive
        .query_row("SELECT COUNT(*) FROM runtime_boots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(boot_count, 2);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn killed_process_after_commit_reopens_committed_row_without_a_receipt() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_held_child(&path, "after_commit", "M7_POSTCOMMIT_REACHED ");
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    assert!(
        std::fs::metadata(PathBuf::from(wal_path)).unwrap().len() > 0,
        "a killed post-commit writer must leave its committed WAL available to SQLite"
    );
    assert_integrity_ok(&path);
    assert_crashed_lifecycle_is_unsealed(&path, &old_boot);
    let new_boot = "abababababababababababababababab";
    assert_ne!(old_boot, new_boot);
    let mut reopened = SqliteStore::open_with_boot(&path, new_boot).unwrap();
    assert_eq!(reopened.database_id(), database_id);
    let old_page = reopened
        .read_history_measurements(
            &HistoryFilter {
                boot_id: old_boot.clone(),
                run_no: 1,
                instrument: InstrumentId::new(701),
                parameter: lab_core::TEMPERATURE,
                from: Duration::ZERO,
                to: Duration::from_secs(2),
            },
            None,
            8,
        )
        .unwrap();
    assert_eq!(old_page.rows.len(), 1, "committed WAL row survives A death");
    assert_eq!(old_page.coverage, "unknown_tail");
    let committed = old_page.rows[0].record_sequence;
    reopened.finish_boot(Duration::from_secs(2)).unwrap();
    reopened.close().unwrap();
    let archive = rusqlite::Connection::open(&path).unwrap();
    let checkpoint: Vec<u8> = archive
        .query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints WHERE boot_id=?1",
            [boot_bytes(&old_boot)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        u64::from_be_bytes(checkpoint.try_into().unwrap()),
        committed
    );
    drop(archive);
    assert_integrity_ok(&path);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn killed_commit_before_receipt_is_readable_via_public_history_without_control_replay() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_held_child(&path, "after_commit", "M7_POSTCOMMIT_REACHED ");
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
    let mut b = ServiceHost::startup(options).unwrap();
    assert_ne!(b.boot_id(), old_boot);
    assert_eq!(
        b.owner().recording_database_id(),
        Some(database_id.as_str())
    );
    let QueryResult::Controller(controller) = b
        .owner()
        .query(Query::Controller(b.owner().controller_id()))
        .unwrap()
    else {
        panic!("new boot native controller unavailable")
    };
    assert_eq!(controller.state, lab_core::control::ControllerState::Ready);
    let QueryResult::Output(output) = b
        .owner()
        .query(Query::Output(lab_core::output::ActuatorId::new(
            b.owner().plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!("new boot output unavailable")
    };
    assert!(
        output.lease.is_none(),
        "archive reopen cannot restore old output authority"
    );
    let mut app = Application::new(b.boot_id()).unwrap();
    let frame = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = app.handle(
        &mut b,
        1,
        frame(json!({"v":1,"msg_id":"hello-b",
        "op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let read = app.handle(
        &mut b,
        1,
        frame(json!({"v":1,"msg_id":"old-run",
        "op":"history_read","request_id":{"scope":scope,"seq":"1"},
        "args":{"mode":"measurements","database_id":database_id,"boot_id":old_boot,
            "run_id":{"boot_id":old_boot,"run_no":"1"},
            "signal":{"instrument":"701","parameter":"1"},
            "from_ns":"0","to_ns":"2000000000","max_records":8,"cursor":null}})),
    );
    assert_eq!(read[0]["state"], "accepted");
    let by = Instant::now() + Duration::from_secs(2);
    let completion = loop {
        let completion = app.poll_history(&mut b);
        if !completion.is_empty() {
            break completion;
        }
        assert!(Instant::now() < by);
        thread::yield_now();
    };
    assert_eq!(completion[0].1["state"], "completed", "{completion:?}");
    let token = completion[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut b,
        1,
        frame(json!({"v":1,"msg_id":"old-page",
        "op":"history_page","args":{"page_token":token}})),
    );
    assert_eq!(page[0]["result"]["coverage"], "unknown_tail");
    let rows = page[0]["result"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["quality"], "good");
    assert_eq!(rows[0]["value"]["value"], 21.0);
    let stale_event = app.handle(
        &mut b,
        1,
        frame(json!({"v":1,"msg_id":"old-event",
        "op":"subscribe","args":{"after":{"boot_id":old_boot,"seq":"0"},
            "filter":{"kinds":[],"targets":[]}}})),
    );
    assert_eq!(stale_event[0]["code"], "instance_changed");
    b.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while b.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        thread::yield_now();
    }
    drop(app);
    drop(b);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn killed_active_native_authority_reopens_ready_without_restoring_lease_or_replaying_output() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_selected_child(
        &path,
        "child_holds_active_native_authority_after_fact_commit",
        "LAB_M7_ACTIVE_AUTHORITY_DB",
        None,
        "M7_ACTIVE_AUTHORITY_REACHED ",
    );
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
    let mut b = ServiceHost::startup(options).unwrap();
    assert_ne!(b.boot_id(), old_boot);
    assert_eq!(
        b.owner().recording_database_id(),
        Some(database_id.as_str())
    );
    let QueryResult::Controller(controller) = b
        .owner()
        .query(Query::Controller(b.owner().controller_id()))
        .unwrap()
    else {
        panic!("B controller unavailable")
    };
    assert_eq!(controller.state, lab_core::control::ControllerState::Ready);
    let QueryResult::Output(output) = b
        .owner()
        .query(Query::Output(lab_core::output::ActuatorId::new(
            b.owner().plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!("B output unavailable")
    };
    assert!(output.lease.is_none());
    assert!(!output.pending);
    assert!(output.in_flight.is_none());
    assert_eq!(
        b.owner().recording_status().unwrap().state,
        RecordingState::Idle
    );
    b.request_shutdown().unwrap();
    let by = Instant::now() + Duration::from_secs(3);
    while b.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < by);
        thread::yield_now();
    }
    drop(b);
    let db = rusqlite::Connection::open(&path).unwrap();
    let old_state: (String, String) = db
        .query_row(
            "SELECT state,coverage FROM runs WHERE boot_id=?1",
            [boot_bytes(&old_boot)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(old_state, ("interrupted".into(), "unknown_tail".into()));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn child_holds_active_native_authority_after_fact_commit() {
    let Some(path) = std::env::var_os("LAB_M7_ACTIVE_AUTHORITY_DB") else {
        return;
    };
    let path = PathBuf::from(path);
    let barrier = WriterBarrier::held_after_fact_commit();
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
    service
        .owner_mut()
        .start_recording("active authority process A", clock.now())
        .unwrap();
    let by = Instant::now() + Duration::from_secs(3);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by, "A Start did not commit");
        service.owner_mut().service(&clock).unwrap();
        thread::yield_now();
    }
    let controller = service.owner().controller_id();
    service
        .owner_mut()
        .command(DomainCommand::StartController {
            controller,
            at: clock.now(),
        })
        .unwrap();
    let actuator =
        lab_core::output::ActuatorId::new(service.owner().plant_id(), lab_core::HEATER_POWER);
    loop {
        service.owner_mut().service(&clock).unwrap();
        let QueryResult::Controller(state) = service
            .owner()
            .query(Query::Controller(controller))
            .unwrap()
        else {
            panic!("A controller unavailable")
        };
        let QueryResult::Output(output) = service.owner().query(Query::Output(actuator)).unwrap()
        else {
            panic!("A output unavailable")
        };
        if state.state == lab_core::control::ControllerState::Running
            && output.lease.is_some()
            && barrier.reached()
        {
            break;
        }
        assert!(
            Instant::now() < by,
            "A did not hold active output with a committed fact"
        );
        thread::yield_now();
    }
    println!(
        "M7_ACTIVE_AUTHORITY_REACHED {} {}",
        service.boot_id(),
        service.owner().recording_database_id().unwrap()
    );
    std::io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(10));
    panic!("parent did not kill the active output owner");
}

fn kill_held_child(path: &PathBuf, stage: &str, marker: &str) -> (String, String) {
    kill_selected_child(
        path,
        "child_holds_a_real_batch_at_selected_commit_stage",
        "LAB_M7_PROCESS_REOPEN_DB",
        Some(stage),
        marker,
    )
}

fn kill_selected_child(
    path: &PathBuf,
    test_name: &str,
    database_env: &str,
    stage: Option<&str>,
    marker: &str,
) -> (String, String) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test_name, "--nocapture"])
        .env(database_env, path);
    if let Some(stage) = stage {
        command.env("LAB_M7_PROCESS_REOPEN_STAGE", stage);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let marker = marker.to_owned();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(after_marker) = line.split(&marker).nth(1) {
                let _ = tx.send(after_marker.to_owned());
                break;
            }
        }
    });
    let error_reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    // The child owns finite state/barrier watchdogs and either emits this exact
    // marker or exits, which disconnects the channel. The parent therefore
    // waits on process evidence instead of a second scheduler-dependent timer.
    let observed = rx.recv();
    let _ = child.kill();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    let stderr = error_reader.join().unwrap();
    let marker = observed.unwrap_or_else(|error| {
        panic!("child did not reach held SQLite batch: {error:?}; status={status}; stderr={stderr}")
    });
    let mut identities = marker.split_whitespace();
    let old_boot = identities.next().unwrap().to_owned();
    let database_id = identities.next().unwrap().to_owned();
    assert!(identities.next().is_none());
    (old_boot, database_id)
}

fn boot_bytes(value: &str) -> Vec<u8> {
    (0..32)
        .step_by(2)
        .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
        .collect()
}
