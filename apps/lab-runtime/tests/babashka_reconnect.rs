//! Actual Rust service and actual Babashka A/B reconnect acceptance.
//!
//! A missing bb executable, skipped client process or fake socket peer is failure.

use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// Two real Babashka hosts are heavyweight full-process fixtures. Serialize
// their independent tests so one cannot consume the other's bounded disk/CPU
// progress budget during a Required recording run.
static BABASHKA_PROCESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn stdout_line(child: &mut ChildGuard, deadline: Duration) -> String {
    let out = child.0.stdout.take().expect("bounded child stdout pipe");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(out);
        let mut line = String::new();
        let result = reader.read_line(&mut line);
        let _ = tx.send((result, line));
    });
    let (read, line) = rx
        .recv_timeout(deadline)
        .expect("real process checkpoint deadline");
    if read.unwrap() == 0 {
        let status = wait_exit(child, Duration::from_secs(2));
        let mut stderr = String::new();
        if let Some(mut pipe) = child.0.stderr.take() {
            pipe.read_to_string(&mut stderr).unwrap();
        }
        panic!("process exited without checkpoint: {status:?}\n{stderr}");
    }
    line
}
fn spawn_bb(exe: &str, mode: &str, port: u16, checkpoint: Option<&str>) -> ChildGuard {
    let mut command = Command::new(exe);
    command.current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    command.args([
        "--config",
        "clients/babashka/bb.edn",
        "-m",
        "lab.acceptance",
        mode,
        &port.to_string(),
    ]);
    if let Some(checkpoint) = checkpoint {
        command.arg(checkpoint);
    }
    ChildGuard(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("actual Babashka executable {exe} unavailable: {e}")),
    )
}
fn wait_exit(child: &mut ChildGuard, deadline: Duration) -> std::process::ExitStatus {
    let until = Instant::now() + deadline;
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < until,
            "process exceeded whole acceptance deadline"
        );
        thread::yield_now();
    }
}

#[test]
fn actual_recorded_babashka_a_b_reopens_autonomous_prefix_after_stop_and_shutdown() {
    let _guard = BABASHKA_PROCESS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    use lab_core::{InstrumentId, ParameterId};
    use lab_runtime::recorder::{HistoryFilter, SqliteStore};
    let whole = Instant::now() + Duration::from_secs(90);
    let bb = std::env::var("LAB_BB_EXE").unwrap_or_else(|_| "bb".into());
    let version = Command::new(&bb)
        .arg("--version")
        .output()
        .expect("actual bb missing");
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("babashka"));
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = std::env::temp_dir().join(format!("lab-runtime-m7-bb-record-{suffix}.sqlite"));
    let path_text = path.to_string_lossy();
    let mut host = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
            .args([
                "--serve",
                "--profile",
                "virtual-demo",
                "--port",
                "0",
                "--record-db",
                path_text.as_ref(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let ready: Value =
        serde_json::from_str(&stdout_line(&mut host, Duration::from_secs(4))).unwrap();
    assert_eq!(ready["state"], "ready");
    let port = ready["port"].as_u64().unwrap() as u16;
    let mut a = spawn_bb(&bb, "record-a", port, None);
    let checkpoint_text = stdout_line(&mut a, Duration::from_secs(25));
    let checkpoint: Value = serde_json::from_str(&checkpoint_text).unwrap();
    assert_eq!(checkpoint["checkpoint"], "a");
    assert_eq!(checkpoint["boot_id"], ready["boot_id"]);
    assert_eq!(checkpoint["recording_state"], "recording");
    assert_eq!(checkpoint["controller_state"], "running");
    let initial_prefix = checkpoint["recording_prefix"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    a.0.kill().unwrap();
    assert!(!wait_exit(&mut a, Duration::from_secs(3)).success());
    let disconnected = Instant::now();
    while disconnected.elapsed() < Duration::from_millis(6_100) {
        assert!(
            host.0.try_wait().unwrap().is_none(),
            "Runtime stopped with Babashka A"
        );
        assert!(Instant::now() < whole);
        thread::sleep(Duration::from_millis(50));
    }
    let mut b = spawn_bb(&bb, "record-b", port, Some(checkpoint_text.trim_end()));
    let final_text = stdout_line(&mut b, Duration::from_secs(25));
    let final_state: Value = serde_json::from_str(&final_text).unwrap();
    assert_eq!(final_state["checkpoint"], "b");
    assert_eq!(final_state["boot_id"], ready["boot_id"]);
    assert_eq!(final_state["state_before_pause"], "running");
    assert_eq!(final_state["recording_state_before_pause"], "recording");
    assert!(
        final_state["recording_prefix"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > initial_prefix
    );
    assert_eq!(final_state["recording_state_after_stop"], "idle");
    assert_eq!(final_state["safe_confirmed"], true);
    assert_eq!(final_state["shutdown_safe"], true);
    assert!(wait_exit(&mut b, Duration::from_secs(3)).success());
    assert!(wait_exit(&mut host, Duration::from_secs(4)).success());
    let archive_boot = checkpoint["boot_id"].as_str().unwrap().to_owned();
    let run_no = checkpoint["run_id"]["run_no"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let signal = &checkpoint["signal"];
    let instrument = InstrumentId::new(signal["instrument"].as_str().unwrap().parse().unwrap());
    let parameter = ParameterId::new(signal["parameter"].as_str().unwrap().parse().unwrap());
    let mut store = SqliteStore::open_with_boot(&path, "dededededededededededededededede").unwrap();
    let filter = HistoryFilter {
        boot_id: archive_boot,
        run_no,
        instrument,
        parameter,
        from: Duration::ZERO,
        to: Duration::from_secs(90),
    };
    let mut cursor = None;
    let mut seen = 0usize;
    let mut after_a = 0usize;
    for _ in 0..16 {
        let page = store
            .read_history_measurements(&filter, cursor.as_ref(), 128)
            .unwrap();
        assert_eq!(page.coverage, "complete");
        for row in &page.rows {
            assert_eq!(row.quality, "good");
            seen += 1;
            if row.record_sequence > initial_prefix {
                after_a += 1;
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(seen > 3, "no archived autonomous measurements");
    assert!(
        after_a > 2,
        "A disconnect did not leave recorded native progress"
    );
    store.finish_boot(Duration::ZERO).unwrap();
    store.close().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(Instant::now() < whole, "whole recorded A/B watchdog");
}

#[test]
fn actual_babashka_a_killed_after_running_and_b_reconciles_after_three_finite_leases() {
    let _guard = BABASHKA_PROCESS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let whole = Instant::now() + Duration::from_secs(60);
    let bb = std::env::var("LAB_BB_EXE").unwrap_or_else(|_| "bb".into());
    let version = Command::new(&bb)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("actual bb missing: {e}"));
    assert!(version.status.success(), "actual bb --version failed");
    assert!(
        String::from_utf8_lossy(&version.stdout).contains("babashka"),
        "bb version output was unusable"
    );
    let mut host = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
            .args(["--serve", "--profile", "virtual-demo", "--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let ready: Value = serde_json::from_str(&stdout_line(&mut host, Duration::from_secs(3)))
        .expect("bounded readiness JSON");
    assert_eq!(ready["state"], "ready");
    let port = ready["port"].as_u64().unwrap() as u16;
    let mut a = spawn_bb(&bb, "a", port, None);
    let checkpoint_text = stdout_line(&mut a, Duration::from_secs(25));
    let checkpoint: Value =
        serde_json::from_str(&checkpoint_text).expect("Babashka A checkpoint JSON");
    assert_eq!(checkpoint["checkpoint"], "a");
    assert_eq!(checkpoint["boot_id"], ready["boot_id"]);
    assert_eq!(checkpoint["controller_state"], "running");
    assert_eq!(checkpoint["sample_quality"], "good");
    assert!(checkpoint["sent_value"].as_f64().is_some());
    a.0.kill().unwrap();
    assert!(!wait_exit(&mut a, Duration::from_secs(3)).success());
    let disconnected = Instant::now();
    while disconnected.elapsed() < Duration::from_millis(6100) {
        assert!(
            host.0.try_wait().unwrap().is_none(),
            "host stopped when Babashka A disappeared"
        );
        assert!(
            Instant::now() < whole,
            "whole 60-second acceptance watchdog"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let client_free = disconnected.elapsed();
    assert!(client_free > Duration::from_secs(6));
    let mut b = spawn_bb(&bb, "b", port, Some(checkpoint_text.trim_end()));
    let final_text = stdout_line(&mut b, Duration::from_secs(25));
    let final_state: Value = serde_json::from_str(&final_text).expect("Babashka B evidence JSON");
    assert_eq!(final_state["checkpoint"], "b");
    assert_eq!(final_state["boot_id"], ready["boot_id"]);
    assert_eq!(final_state["resumed_scope"], checkpoint["scope"]);
    assert_eq!(final_state["state_before_pause"], "running");
    assert_eq!(final_state["instance"], checkpoint["instance"]);
    assert_eq!(final_state["owner"], checkpoint["owner"]);
    assert_eq!(final_state["epoch"], checkpoint["epoch"]);
    let prior = checkpoint["lease_expires_at"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let later = final_state["lease_expires_at"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        later > prior + 6_000_000_000,
        "private renewal did not advance across >3 leases"
    );
    assert_eq!(final_state["safe_confirmed"], true);
    assert_eq!(final_state["lease_after_pause"], Value::Null);
    assert_eq!(final_state["shutdown_safe"], true);
    eprintln!(
        "real bb A/B: boot={} scope={} A_temp={:.4} B_temp={:.4} client_free_ms={} lease_advance_ms={} safe_pause={} shutdown_safe={}",
        ready["boot_id"].as_str().unwrap(),
        checkpoint["scope"].as_str().unwrap(),
        checkpoint["temperature"].as_f64().unwrap(),
        final_state["temperature"].as_f64().unwrap(),
        client_free.as_millis(),
        (later - prior) / 1_000_000,
        final_state["safe_confirmed"],
        final_state["shutdown_safe"],
    );
    assert!(wait_exit(&mut b, Duration::from_secs(3)).success());
    assert!(wait_exit(&mut host, Duration::from_secs(3)).success());
    assert!(
        Instant::now() < whole,
        "whole 60-second acceptance watchdog"
    );
}
