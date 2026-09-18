//! Process-level acceptance for bounded best-effort diagnostic logging.

use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temporary_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "lab-runtime-diagnostic-process-{}-{label}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn log_path(root: &Path) -> PathBuf {
    root.join("lab-runtime")
        .join("logs")
        .join("lab-runtime.log")
}

#[test]
fn default_windows_local_data_path_contains_bounded_process_lifecycle() {
    let root = temporary_root("lifecycle");
    let output = Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
        .env("LOCALAPPDATA", &root)
        .env_remove("LAB_RUNTIME_LOG_DIRECTORY")
        .env_remove("LAB_RUNTIME_LOG_LEVEL")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = fs::read_to_string(log_path(&root)).unwrap();
    assert!(text.contains("process_start"));
    assert!(text.contains("process_exit"));
    assert!(text.contains("protocol_version=1"));
    assert!(fs::metadata(log_path(&root)).unwrap().len() <= 4 * 1024 * 1024);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_log_directory_falls_back_without_killing_runtime() {
    let root = temporary_root("unavailable");
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::write(&root, b"not a directory").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
        .env("LOCALAPPDATA", &root)
        .env_remove("LAB_RUNTIME_LOG_DIRECTORY")
        .env_remove("LAB_RUNTIME_LOG_LEVEL")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("diagnostic_file_unavailable"));
    fs::remove_file(root).unwrap();
}

#[test]
fn malformed_client_log_is_bounded_and_does_not_copy_payload() {
    let root = temporary_root("malformed");
    let mut child = Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
        .args(["--serve", "--profile", "virtual-demo", "--port", "0"])
        .env("LOCALAPPDATA", &root)
        .env_remove("LAB_RUNTIME_LOG_DIRECTORY")
        .env("LAB_RUNTIME_LOG_LEVEL", "DEBUG")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut lines = BufReader::new(stdout);
        let mut line = String::new();
        let result = lines.read_line(&mut line);
        ready_tx.send((result, line)).ok();
    });
    let (_, line) = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("readiness deadline");
    let ready: Value = serde_json::from_str(&line).unwrap();
    let port = ready["port"].as_u64().unwrap() as u16;

    let secret = "DO_NOT_COPY_THIS_CLIENT_PAYLOAD";
    let mut malformed = TcpStream::connect(("127.0.0.1", port)).unwrap();
    malformed
        .write_all(format!("{{not-json:{secret}}}\n").as_bytes())
        .unwrap();
    let mut rejection = String::new();
    BufReader::new(malformed).read_line(&mut rejection).unwrap();
    let rejection: Value = serde_json::from_str(&rejection).unwrap();
    assert_eq!(rejection["accepted"], false);
    assert_eq!(rejection["category"], "protocol_error");

    let mut control = TcpStream::connect(("127.0.0.1", port)).unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut replies = BufReader::new(control.try_clone().unwrap());
    control
        .write_all(
            &lab_runtime::wire::encode_frame(
                &json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
            )
            .unwrap(),
        )
        .unwrap();
    let mut response = String::new();
    replies.read_line(&mut response).unwrap();
    let hello: Value = serde_json::from_str(&response).unwrap();
    let scope = hello["result"]["scope"].as_str().unwrap();
    control
        .write_all(
            &lab_runtime::wire::encode_frame(
                &json!({"v":1,"msg_id":"stop","op":"runtime_shutdown",
                    "request_id":{"scope":scope,"seq":"1"},"args":{}}),
            )
            .unwrap(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "shutdown process deadline");
        thread::yield_now();
    }
    reader.join().unwrap();

    let text = fs::read_to_string(log_path(&root)).unwrap();
    assert!(text.contains("client_request_malformed"));
    assert!(text.contains("client_accepted"));
    assert!(text.contains("frame_bytes="));
    assert!(!text.contains(secret));
    assert!(text.len() <= 4 * 1024 * 1024);
    fs::remove_dir_all(root).unwrap();
}
