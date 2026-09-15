//! Process death before a confirmed SQLite commit leaves an honest archive tail.

use lab_core::{Command as DomainCommand, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::recorder::{
    HistoryFilter, RecorderLimits, RecorderWorker, RecordingState, SqliteStore, WriterBarrier,
};
use std::{
    io::{BufRead, BufReader, Write},
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
    while !barrier.reached() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(
        barrier.reached(),
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
fn killed_process_a_reopens_in_b_without_promoting_uncommitted_work() {
    let path = temporary_database();
    let (old_boot, database_id) = kill_held_child(&path, "before_commit", "M7_PRECOMMIT_REACHED ");
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
    std::fs::remove_file(path).unwrap();
}

fn kill_held_child(path: &PathBuf, stage: &str, marker: &str) -> (String, String) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_holds_a_real_batch_at_selected_commit_stage",
            "--nocapture",
        ])
        .env("LAB_M7_PROCESS_REOPEN_DB", path)
        .env("LAB_M7_PROCESS_REOPEN_STAGE", stage)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
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
    let observed = rx.recv_timeout(Duration::from_secs(4));
    child.kill().unwrap();
    child.wait().unwrap();
    reader.join().unwrap();
    let marker = observed.expect("child did not reach held SQLite batch");
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
