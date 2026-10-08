//! Real Linux process acceptance for both Application transports and OS shutdown.
//! No serial hardware is opened. An extracted executable can be selected explicitly.
#![cfg(target_os = "linux")]

use lab_runtime::websocket::{APPLICATION_PATH, APPLICATION_SUBPROTOCOL};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tungstenite::{ClientRequestBuilder, Message, client};

const TIMEOUT: Duration = Duration::from_secs(5);
const ORIGIN: &str = "http://127.0.0.1:3000";

struct Running {
    child: Child,
    root: PathBuf,
    ready: Value,
}

impl Running {
    fn start() -> Self {
        let mut entropy = [0; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = std::env::temp_dir().join(format!("lab-linux-process-{suffix}"));
        fs::create_dir(&root).unwrap();
        let binary = std::env::var_os("LAB_RUNTIME_SMOKE_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_lab-runtime").into());
        let child = Command::new(binary)
            .args([
                "--serve",
                "--profile",
                "virtual-demo",
                "--port",
                "0",
                "--record-db",
            ])
            .arg(root.join("history.sqlite"))
            .args(["--ws-port", "0", "--ws-origin", ORIGIN])
            .env("LAB_RUNTIME_LOG_DIRECTORY", root.join("logs"))
            .stdout(Stdio::piped())
            .stderr(fs::File::create(root.join("stderr.log")).unwrap())
            .spawn()
            .unwrap();
        // Own cleanup before any readiness assertion can fail.
        let mut running = Self {
            child,
            root,
            ready: Value::Null,
        };
        let stdout = running.child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).unwrap();
            let _ = tx.send(line);
        });
        let line = rx.recv_timeout(TIMEOUT).unwrap();
        assert!(
            !line.is_empty(),
            "Runtime failed before readiness: {}",
            fs::read_to_string(running.root.join("stderr.log")).unwrap()
        );
        running.ready = serde_json::from_str(&line).unwrap();
        reader.join().unwrap();
        assert_eq!(running.ready["state"], "ready");
        running
    }

    fn tcp(&self) -> BufReader<TcpStream> {
        BufReader::new(socket(self.ready["port"].as_u64().unwrap() as u16))
    }

    fn wait_clean(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Runtime exit: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "Runtime shutdown deadline");
            thread::sleep(Duration::from_millis(10));
        }
        let log = fs::read_to_string(self.root.join("logs/lab-runtime.log")).unwrap();
        assert!(log.contains("process_exit"));
        assert!(!log.contains("process_exit_failed"));
        let db = rusqlite::Connection::open(self.root.join("history.sqlite")).unwrap();
        let integrity: String = db
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let state: String = db
            .query_row("SELECT state FROM runtime_boots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "sealed");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn socket(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
    stream
}

fn read(client: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    assert!(client.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

fn exchange(client: &mut BufReader<TcpStream>, request: Value) -> Value {
    client
        .get_mut()
        .write_all(&lab_runtime::wire::encode_frame(&request).unwrap())
        .unwrap();
    read(client)
}

fn query(msg: &str, op: &str, args: Value) -> Value {
    json!({"v":1,"msg_id":msg,"op":op,"args":args})
}

#[test]
fn tcp_and_websocket_hello_latest_and_application_shutdown() {
    let mut process = Running::start();
    let mut tcp = process.tcp();
    let hello = exchange(&mut tcp, query("tcp-hello", "hello", json!({"scope":null})));
    assert_eq!(hello["type"], "result");
    let scope = hello["result"]["scope"].as_str().unwrap();
    let discovery = exchange(&mut tcp, query("discover", "discover", json!({})));
    let signal = discovery["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "signal")
        .unwrap()["id"]
        .clone();
    let latest = exchange(
        &mut tcp,
        query("tcp-latest", "latest", json!({"signal":signal})),
    );
    assert_eq!(latest["type"], "result", "{latest}");
    assert_eq!(latest["msg_id"], "tcp-latest");

    let port = process.ready["websocket"]["port"].as_u64().unwrap() as u16;
    let uri = format!("ws://127.0.0.1:{port}{APPLICATION_PATH}")
        .parse()
        .unwrap();
    let request = ClientRequestBuilder::new(uri)
        .with_header("Origin", ORIGIN)
        .with_sub_protocol(APPLICATION_SUBPROTOCOL);
    let (mut ws, handshake) = client(request, socket(port)).unwrap();
    assert_eq!(
        handshake.headers()["Sec-WebSocket-Protocol"],
        APPLICATION_SUBPROTOCOL
    );
    for request in [
        query("ws-hello", "hello", json!({"scope":null})),
        query("ws-latest", "latest", json!({"signal":signal})),
    ] {
        ws.send(Message::Text(request.to_string().into())).unwrap();
        let reply: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply["type"], "result", "{reply}");
        assert_eq!(reply["msg_id"], request["msg_id"]);
        if request["op"] == "hello" {
            assert_eq!(reply["result"]["boot_id"], hello["result"]["boot_id"]);
        }
    }
    ws.close(None).unwrap();
    let accepted = exchange(
        &mut tcp,
        json!({"v":1,"msg_id":"stop","op":"runtime_shutdown",
        "request_id":{"scope":scope,"seq":"1"},"args":{}}),
    );
    assert_eq!(accepted["state"], "accepted");
    let terminal = read(&mut tcp);
    assert_eq!(terminal["state"], "completed");
    assert_eq!(terminal["result"]["safe_confirmed"], true);
    assert_eq!(terminal["result"]["recorder_flushed"], true);
    process.wait_clean();
}

#[test]
fn linux_signals_use_owner_shutdown_and_seal_active_recording() {
    for signal in ["TERM", "INT", "HUP"] {
        let mut process = Running::start();
        let mut tcp = process.tcp();
        let hello = exchange(&mut tcp, query("h", "hello", json!({"scope":null})));
        let scope = hello["result"]["scope"].as_str().unwrap();
        let start = exchange(
            &mut tcp,
            json!({"v":1,"msg_id":"start","op":"recording_start",
            "request_id":{"scope":scope,"seq":"1"},"args":{"label":"Linux signal acceptance"}}),
        );
        assert_eq!(start["state"], "accepted");
        assert_eq!(read(&mut tcp)["state"], "completed");
        let status = exchange(&mut tcp, query("status", "recording_status", json!({})));
        assert_eq!(status["result"]["state"], "recording");
        assert!(
            Command::new("kill")
                .args(["-s", signal, &process.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        process.wait_clean();
        let db = rusqlite::Connection::open(process.root.join("history.sqlite")).unwrap();
        for table in ["runs", "recording_intervals"] {
            let (state, coverage): (String, String) = db
                .query_row(&format!("SELECT state,coverage FROM {table}"), [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(state, "sealed", "{signal}: {table}");
            assert_eq!(coverage, "complete", "{signal}: {table}");
        }
    }
}
