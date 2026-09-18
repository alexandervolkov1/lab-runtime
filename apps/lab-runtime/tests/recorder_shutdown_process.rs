//! A blocked Recorder writer cannot keep the actual server process alive forever.

use lab_core::{Command, InstrumentId};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore, WriterBarrier,
    },
    server::run,
    service::{ServiceHost, ServiceOptions},
    wire::encode_frame,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-shutdown-process-{suffix}.sqlite"))
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn blocked_writer_child_exits_nonzero_after_the_public_flush_watchdog() {
    let path = temporary_database();
    let mut child = ChildGuard(
        ProcessCommand::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child_serves_with_a_recorder_writer_held_before_commit",
                "--nocapture",
            ])
            .env("LAB_M7_SHUTDOWN_PROCESS_DB", &path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = child.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let Ok(line) = line else { break };
            if let Some(ready) = line.split("M7_BLOCKED_SHUTDOWN_READY ").nth(1) {
                let _ = ready_tx.send(ready.to_owned());
                break;
            }
        }
    });
    let ready = ready_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    reader.join().unwrap();
    let mut parts = ready.split_whitespace();
    let address: SocketAddr = parts.next().unwrap().parse().unwrap();
    let old_boot = parts.next().unwrap().to_owned();
    let database_id = parts.next().unwrap().to_owned();
    assert!(parts.next().is_none());
    let socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(7)))
        .unwrap();
    let mut client = BufReader::new(socket);
    let mut exchange = |request: Value| -> Value {
        client
            .get_mut()
            .write_all(&encode_frame(&request).unwrap())
            .unwrap();
        let mut line = String::new();
        assert!(client.read_line(&mut line).unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    };
    let hello = exchange(json!({"v":1,"msg_id":"watchdog-hello","op":"hello",
        "args":{"scope":null}}));
    let scope = hello["result"]["scope"].as_str().unwrap();
    let began = Instant::now();
    let accepted = exchange(
        json!({"v":1,"msg_id":"watchdog-stop","op":"runtime_shutdown",
        "request_id":{"scope":scope,"seq":"1"},"args":{}}),
    );
    assert_eq!(accepted["state"], "accepted", "{accepted:?}");
    let mut terminal_line = String::new();
    assert!(client.read_line(&mut terminal_line).unwrap() > 0);
    let terminal: Value = serde_json::from_str(&terminal_line).unwrap();
    assert_eq!(terminal["state"], "failed", "{terminal:?}");
    assert_eq!(terminal["result"]["safe_confirmed"], true);
    assert_eq!(terminal["result"]["recorder_flushed"], false);
    assert_eq!(terminal["result"]["recorder_unfinished"], true);
    assert_eq!(terminal["result"]["exit_success"], false);
    let exit_by = Instant::now() + Duration::from_secs(3);
    let exit = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < exit_by,
            "blocked worker kept server process alive"
        );
        thread::yield_now();
    };
    assert!(!exit.success());
    assert!(began.elapsed() < Duration::from_secs(8));
    drop(client);
    drop(child);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_eq!(reopened.database_id(), database_id);
    assert_ne!(reopened.boot_id(), old_boot);
    let old = reopened.read_history_runs(None, 8).unwrap();
    let old_run = old.runs.iter().find(|run| run.boot_id == old_boot).unwrap();
    assert_eq!(old_run.coverage, "unknown_tail");
    reopened.close().unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn child_serves_with_a_recorder_writer_held_before_commit() {
    let Some(path) = std::env::var_os("LAB_M7_SHUTDOWN_PROCESS_DB") else {
        return;
    };
    let path = PathBuf::from(path);
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
    service
        .owner_mut()
        .start_recording("blocked process flush", clock.now())
        .unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        service.owner_mut().service(&clock).unwrap();
        thread::yield_now();
    }
    let plant: InstrumentId = service.owner().plant_id();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at: clock.now(),
        })
        .unwrap();
    assert!(barrier.wait_until_reached(Duration::from_secs(2)));
    println!(
        "M7_BLOCKED_SHUTDOWN_READY {} {} {}",
        service.bound_address(),
        service.boot_id(),
        service.owner().recording_database_id().unwrap()
    );
    std::io::stdout().flush().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    if run(service, stop).is_err() {
        std::process::exit(1);
    }
    std::process::exit(0);
}
