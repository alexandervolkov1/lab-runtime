//! Actual binary emits one safe readiness line before exposing client work.

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn serve_binary_reports_ephemeral_loopback_readiness_then_exits_after_shutdown() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
        .args(["--serve", "--profile", "virtual-demo", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut lines = BufReader::new(stdout);
        let mut line = String::new();
        let result = lines.read_line(&mut line);
        tx.send((result, line)).ok();
    });
    let (_, line) = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("readiness deadline");
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["state"], "ready");
    let port = ready["port"].as_u64().unwrap() as u16;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut replies = BufReader::new(stream.try_clone().unwrap());
    stream
        .write_all(
            &lab_runtime::wire::encode_frame(
                &json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
            )
            .unwrap(),
        )
        .unwrap();
    let mut response = String::new();
    replies.read_line(&mut response).unwrap();
    let scope: Value = serde_json::from_str(&response).unwrap();
    let scope = scope["result"]["scope"].as_str().unwrap();
    stream
        .write_all(
            &lab_runtime::wire::encode_frame(
                &json!({"v":1,"msg_id":"stop","op":"runtime_shutdown",
        "request_id":{"scope":scope,"seq":"1"},"args":{}}),
            )
            .unwrap(),
        )
        .unwrap();
    response.clear();
    replies.read_line(&mut response).unwrap();
    let accepted: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(accepted["state"], "accepted");
    response.clear();
    replies.read_line(&mut response).unwrap();
    let terminal: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(terminal["state"], "completed");
    assert_eq!(terminal["result"]["safe_confirmed"], true);
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "shutdown process deadline"
        );
        thread::yield_now();
    }
    reader.join().unwrap();
}
