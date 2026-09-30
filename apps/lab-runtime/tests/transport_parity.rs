//! Cross-transport acceptance for the one Application/session contract.

use lab_runtime::{
    protocol::OPERATIONS,
    recorder::SqliteStore,
    server::run,
    service::{ServiceHost, ServiceOptions},
    websocket::{APPLICATION_PATH, APPLICATION_SUBPROTOCOL},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{ClientRequestBuilder, Message, WebSocket, client};

const ORIGIN: &str = "http://127.0.0.1:3000";
const IO_TIMEOUT: Duration = Duration::from_secs(3);
static SERVICE_GATE: Mutex<()> = Mutex::new(());

struct Running {
    tcp: SocketAddr,
    websocket: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Running {
    fn start(recording: Option<&Path>) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let recording = recording.map(Path::to_path_buf);
        let join = thread::spawn(move || {
            let mut owned = vec![
                "--serve".to_owned(),
                "--profile".to_owned(),
                "virtual-demo".to_owned(),
                "--port".to_owned(),
                "0".to_owned(),
            ];
            if let Some(path) = recording {
                owned.push("--record-db".to_owned());
                owned.push(path.to_string_lossy().into_owned());
            }
            owned.extend([
                "--ws-port".to_owned(),
                "0".to_owned(),
                "--ws-origin".to_owned(),
                ORIGIN.to_owned(),
            ]);
            let args = owned.iter().map(String::as_str).collect::<Vec<_>>();
            let service = ServiceHost::startup(ServiceOptions::parse(&args).unwrap()).unwrap();
            let addresses = (
                service.bound_address(),
                service.websocket_bound_address().unwrap(),
            );
            ready_tx.send(addresses).unwrap();
            run(service, flag).unwrap();
        });
        let (tcp, websocket) = ready_rx.recv_timeout(IO_TIMEOUT).unwrap();
        Self {
            tcp,
            websocket,
            stop,
            join: Some(join),
        }
    }

    fn connect(&self, transport: Transport) -> Peer {
        Peer::connect(transport, self.tcp, self.websocket)
    }

    fn wait_for_exit(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.join.as_ref().unwrap().is_finished() {
            assert!(
                Instant::now() < deadline,
                "server reactor did not exit finitely"
            );
            thread::yield_now();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Transport {
    Tcp,
    WebSocket,
}

enum Peer {
    Tcp(BufReader<TcpStream>),
    WebSocket(Box<WebSocket<TcpStream>>),
}

impl Peer {
    fn connect(transport: Transport, tcp: SocketAddr, websocket: SocketAddr) -> Self {
        match transport {
            Transport::Tcp => {
                let stream = TcpStream::connect(tcp).unwrap();
                stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
                Self::Tcp(BufReader::new(stream))
            }
            Transport::WebSocket => {
                let stream = TcpStream::connect(websocket).unwrap();
                stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
                let uri = format!("ws://{websocket}{APPLICATION_PATH}")
                    .parse()
                    .unwrap();
                let request = ClientRequestBuilder::new(uri)
                    .with_header("Origin", ORIGIN)
                    .with_sub_protocol(APPLICATION_SUBPROTOCOL);
                let (socket, response) = client(request, stream).unwrap();
                assert_eq!(response.status(), 101);
                Self::WebSocket(Box::new(socket))
            }
        }
    }

    fn write_raw(&mut self, body: &str) {
        match self {
            Self::Tcp(reader) => {
                reader.get_mut().write_all(body.as_bytes()).unwrap();
                reader.get_mut().write_all(b"\n").unwrap();
            }
            Self::WebSocket(socket) => {
                socket.write(Message::Text(body.to_owned().into())).unwrap();
                socket.flush().unwrap();
            }
        }
    }

    fn write_value(&mut self, value: &Value) {
        self.write_raw(&value.to_string());
    }

    fn read(&mut self) -> Value {
        match self {
            Self::Tcp(reader) => {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(
                    !line.is_empty(),
                    "TCP peer closed before Application response"
                );
                serde_json::from_str(&line).unwrap()
            }
            Self::WebSocket(socket) => loop {
                match socket.read().unwrap() {
                    Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                    Message::Ping(_) | Message::Pong(_) => {}
                    other => panic!("WebSocket peer closed before Application response: {other:?}"),
                }
            },
        }
    }

    fn send(&mut self, value: Value) -> Value {
        self.write_value(&value);
        self.read()
    }

    fn read_for_msg(&mut self, msg_id: &str) -> Value {
        for _ in 0..64 {
            let value = self.read();
            if value["msg_id"] == msg_id {
                return value;
            }
        }
        panic!("no response for msg_id {msg_id}");
    }

    fn hello(&mut self, msg_id: &str, scope: Option<&str>) -> Value {
        self.send(json!({"v":1,"msg_id":msg_id,"op":"hello","args":{"scope":scope}}))
    }

    fn operation(&mut self, value: Value) -> (Value, Value) {
        let msg_id = value["msg_id"].as_str().unwrap().to_owned();
        self.write_value(&value);
        let accepted = self.read_for_msg(&msg_id);
        assert_eq!(accepted["state"], "accepted", "{accepted:?}");
        let terminal = self.read_for_msg(&msg_id);
        assert!(
            terminal["state"] == "completed" || terminal["state"] == "failed",
            "{terminal:?}"
        );
        (accepted, terminal)
    }

    fn write_many(&mut self, values: &[Value]) {
        match self {
            Self::Tcp(reader) => {
                for value in values {
                    reader
                        .get_mut()
                        .write_all(value.to_string().as_bytes())
                        .unwrap();
                    reader.get_mut().write_all(b"\n").unwrap();
                }
            }
            Self::WebSocket(socket) => {
                for value in values {
                    socket
                        .write(Message::Text(value.to_string().into()))
                        .unwrap();
                }
                socket.flush().unwrap();
            }
        }
    }
}

fn successful_hello(peer: &mut Peer, msg_id: &str, scope: Option<&str>) -> Value {
    let reply = peer.hello(msg_id, scope);
    assert_eq!(reply["type"], "result", "{reply:?}");
    reply
}

fn attach_after_detach(server: &Running, transport: Transport, scope: &str) -> (Peer, Value) {
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        let mut peer = server.connect(transport);
        let hello = peer.hello("reattach", Some(scope));
        if hello["type"] == "result" {
            return (peer, hello);
        }
        assert_eq!(hello["code"], "scope_in_use", "{hello:?}");
        assert!(
            Instant::now() < deadline,
            "detach did not reach Application owner"
        );
        drop(peer);
        thread::yield_now();
    }
}

fn without_msg_id(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("msg_id");
    value
}

fn retune(msg_id: &str, scope: &str, seq: u64, revision: u64, target: f64) -> Value {
    json!({
        "v":1,"msg_id":msg_id,"op":"reference_retune",
        "request_id":{"scope":scope,"seq":seq.to_string()},
        "args":{"reference":"1","expected_revision":revision.to_string(),
            "target":target,"rate":2.0}
    })
}

#[test]
fn representative_application_semantics_match_across_tcp_and_websocket() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let mut tcp = server.connect(Transport::Tcp);
    let mut websocket = server.connect(Transport::WebSocket);

    let tcp_gate = tcp.send(json!({"v":1,"msg_id":"tcp-gate","op":"discover","args":{}}));
    let ws_gate = websocket.send(json!({"v":1,"msg_id":"ws-gate","op":"discover","args":{}}));
    assert_eq!(without_msg_id(tcp_gate), without_msg_id(ws_gate));

    let tcp_hello = successful_hello(&mut tcp, "tcp-hello", None);
    let ws_hello = successful_hello(&mut websocket, "ws-hello", None);
    assert_eq!(OPERATIONS.len(), 43);
    assert_eq!(
        tcp_hello["result"]["operations"],
        ws_hello["result"]["operations"]
    );
    assert_eq!(
        tcp_hello["result"]["capabilities"],
        ws_hello["result"]["capabilities"]
    );
    assert_eq!(tcp_hello["result"]["limits"], ws_hello["result"]["limits"]);
    assert_eq!(
        tcp_hello["result"]["boot_id"],
        ws_hello["result"]["boot_id"]
    );

    for (tcp_msg, ws_msg, op, args) in [
        (
            "tcp-query",
            "ws-query",
            "reference",
            json!({"reference":"1"}),
        ),
        ("tcp-unknown", "ws-unknown", "not_an_operation", json!({})),
        (
            "tcp-invalid",
            "ws-invalid",
            "reference",
            json!({"reference":"bad"}),
        ),
        (
            "tcp-domain",
            "ws-domain",
            "reference",
            json!({"reference":"999"}),
        ),
    ] {
        let tcp_reply = tcp.send(json!({"v":1,"msg_id":tcp_msg,"op":op,"args":args}));
        let ws_reply = websocket.send(json!({"v":1,"msg_id":ws_msg,"op":op,"args":args}));
        assert_eq!(
            without_msg_id(tcp_reply),
            without_msg_id(ws_reply),
            "op={op}"
        );
    }
}

#[test]
fn scopes_and_dedup_survive_bidirectional_transport_migration() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let mut tcp = server.connect(Transport::Tcp);
    let opened = successful_hello(&mut tcp, "open", None);
    let scope = opened["result"]["scope"].as_str().unwrap().to_owned();

    let mut simultaneous = server.connect(Transport::WebSocket);
    assert_eq!(
        simultaneous.hello("busy", Some(&scope))["code"],
        "scope_in_use"
    );
    let old_scope = format!("{}:{}", "0".repeat(32), 1);
    let mut old_boot = server.connect(Transport::WebSocket);
    assert_eq!(
        old_boot.hello("old", Some(&old_scope))["code"],
        "instance_changed"
    );
    drop(simultaneous);
    drop(old_boot);

    let request_one = retune("shared-msg", &scope, 1, 1, 41.0);
    let (_, first_terminal) = tcp.operation(request_one.clone());
    assert_eq!(first_terminal["state"], "completed");
    let projection = tcp.send(json!({"v":1,"msg_id":"projection","op":"discover","args":{}}));
    let old_projection = projection["result"]["projection"].clone();
    drop(tcp);

    let (mut websocket, reattached_ws) = attach_after_detach(&server, Transport::WebSocket, &scope);
    assert_eq!(reattached_ws["result"]["scope"], scope);
    assert_eq!(reattached_ws["result"]["next_seq"], "2");
    let expired_projection = websocket.send(json!({"v":1,"msg_id":"old-projection",
        "op":"discovery_page","args":{"projection":old_projection,"index":"0"}}));
    assert_eq!(expired_projection["code"], "snapshot_expired");
    let retained = websocket.send(request_one.clone());
    assert_eq!(retained["state"], "completed");
    assert_eq!(retained["result"], first_terminal["result"]);
    let reference = websocket.send(
        json!({"v":1,"msg_id":"revision-after-retry","op":"reference","args":{"reference":"1"}}),
    );
    assert_eq!(reference["result"]["revision"], "2");

    let conflict = websocket.send(retune("conflict", &scope, 1, 1, 42.0));
    assert_eq!(conflict["code"], "request_conflict");
    let status = websocket.send(json!({"v":1,"msg_id":"status","op":"operation_status",
        "args":{"request_id":{"scope":scope,"seq":"1"}}}));
    assert_eq!(status["result"]["state"], "completed");

    let request_two = retune("ws-origin", &scope, 2, 2, 43.0);
    let (_, second_terminal) = websocket.operation(request_two.clone());
    drop(websocket);

    let (mut tcp, reattached_tcp) = attach_after_detach(&server, Transport::Tcp, &scope);
    assert_eq!(reattached_tcp["result"]["next_seq"], "3");
    let retained = tcp.send(request_two);
    assert_eq!(retained["state"], "completed");
    assert_eq!(retained["result"], second_terminal["result"]);
    let (_, third_terminal) = tcp.operation(retune("next", &scope, 3, 3, 45.0));
    assert_eq!(third_terminal["state"], "completed");
    for seq in 4..=34u64 {
        let (_, terminal) =
            tcp.operation(retune("evict", &scope, seq, seq, 35.0 + (seq % 3) as f64));
        assert_eq!(terminal["state"], "completed");
    }
    drop(tcp);
    let (mut websocket, hello) = attach_after_detach(&server, Transport::WebSocket, &scope);
    assert_eq!(hello["result"]["next_seq"], "35");
    assert_eq!(websocket.send(request_one)["code"], "outcome_unknown");
    assert_eq!(
        websocket
            .operation(retune("after-eviction", &scope, 35, 35, 46.0))
            .1["state"],
        "completed"
    );
}

#[test]
fn msg_id_is_connection_local_for_both_transports() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    for transport in [Transport::Tcp, Transport::WebSocket] {
        let mut peer = server.connect(transport);
        successful_hello(&mut peer, "hello", None);
        let duplicate = json!({"v":1,"msg_id":"duplicate","op":"discover","args":{}});
        peer.write_many(&[duplicate.clone(), duplicate]);
        let first = peer.read();
        let second = peer.read();
        assert_eq!(first["type"], "result", "transport={transport:?}");
        assert_eq!(
            second["code"], "duplicate_msg_id",
            "transport={transport:?}"
        );

        let mut reusable = server.connect(transport);
        successful_hello(&mut reusable, "hello", None);
        for _ in 0..2 {
            assert_eq!(
                reusable.send(json!({"v":1,"msg_id":"reused","op":"reference",
                    "args":{"reference":"1"}}))["type"],
                "result"
            );
        }
    }

    let mut tcp = server.connect(Transport::Tcp);
    let mut websocket = server.connect(Transport::WebSocket);
    successful_hello(&mut tcp, "same", None);
    successful_hello(&mut websocket, "same", None);
    assert_eq!(
        tcp.send(json!({"v":1,"msg_id":"same","op":"reference","args":{"reference":"1"}}))["type"],
        "result"
    );
    assert_eq!(
        websocket.send(json!({"v":1,"msg_id":"same","op":"reference","args":{"reference":"1"}}))["type"],
        "result"
    );
}

fn mutate_and_capture_reference_event(
    peer: &mut Peer,
    scope: &str,
    seq: u64,
    revision: u64,
) -> Value {
    let request = retune("mutation", scope, seq, revision, 40.0 + seq as f64);
    peer.write_value(&request);
    let mut terminal = false;
    let mut event = None;
    for _ in 0..32 {
        let value = peer.read();
        if value["msg_id"] == "mutation" && value["state"] == "completed" {
            terminal = true;
        }
        if value["type"] == "event" && value["kind"] == "reference" {
            event = Some(value);
        }
        if terminal && let Some(event) = event.take() {
            return event;
        }
    }
    panic!("mutation terminal/reference event pair was not delivered");
}

fn subscribe_and_receive_replay(peer: &mut Peer, msg_id: &str, cursor: Value) -> Value {
    peer.write_value(&json!({"v":1,"msg_id":msg_id,"op":"subscribe",
        "args":{"after":cursor,"filter":{"kinds":[],"targets":[]}}}));
    let mut subscribed = false;
    let mut replay = None;
    for _ in 0..32 {
        let value = peer.read();
        if value["msg_id"] == msg_id {
            assert_eq!(value["type"], "result", "{value:?}");
            subscribed = true;
        } else if value["type"] == "event" {
            replay = Some(value);
        }
        if subscribed && let Some(replay) = replay.take() {
            return replay;
        }
    }
    panic!("subscription reply and retained replay were not both delivered");
}

#[test]
fn subscriptions_replay_from_shared_event_cursors_across_migration() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let mut tcp = server.connect(Transport::Tcp);
    let hello = successful_hello(&mut tcp, "hello", None);
    let scope = hello["result"]["scope"].as_str().unwrap().to_owned();
    let cursor = hello["result"]["event_latest"].clone();
    tcp.write_value(&json!({"v":1,"msg_id":"subscribe","op":"subscribe",
        "args":{"after":cursor,"filter":{"kinds":[],"targets":[]}}}));
    let subscribed = tcp.read_for_msg("subscribe");
    let subscription = subscribed["result"]["subscription"].clone();
    let first_event = mutate_and_capture_reference_event(&mut tcp, &scope, 1, 1);
    let first_cursor = json!({"boot_id":first_event["boot_id"],"seq":first_event["seq"]});
    tcp.write_value(&json!({"v":1,"msg_id":"unsubscribe","op":"unsubscribe",
        "args":{"subscription":subscription}}));
    let unsubscribed = tcp.read_for_msg("unsubscribe");
    assert_eq!(unsubscribed["result"]["removed"], true);
    drop(tcp);

    let (mut websocket, _) = attach_after_detach(&server, Transport::WebSocket, &scope);
    let replay = subscribe_and_receive_replay(&mut websocket, "subscribe", first_cursor);
    assert_eq!(replay["type"], "event");
    assert!(
        replay["seq"].as_str().unwrap().parse::<u64>().unwrap()
            > first_event["seq"].as_str().unwrap().parse::<u64>().unwrap()
    );
    let second_event = mutate_and_capture_reference_event(&mut websocket, &scope, 2, 2);
    let second_cursor = json!({"boot_id":second_event["boot_id"],"seq":second_event["seq"]});
    drop(websocket);

    let (mut tcp, _) = attach_after_detach(&server, Transport::Tcp, &scope);
    assert_eq!(
        subscribe_and_receive_replay(&mut tcp, "subscribe-again", second_cursor)["type"],
        "event"
    );
}

#[test]
fn event_gap_details_match_across_transports() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let mut producer = server.connect(Transport::Tcp);
    let hello = successful_hello(&mut producer, "hello", None);
    let scope = hello["result"]["scope"].as_str().unwrap().to_owned();
    let boot_id = hello["result"]["boot_id"].clone();
    for seq in 1..=520u64 {
        let (_, terminal) =
            producer.operation(retune("fill", &scope, seq, seq, 30.0 + (seq % 2) as f64));
        assert_eq!(terminal["state"], "completed");
    }

    let mut tcp = server.connect(Transport::Tcp);
    let mut websocket = server.connect(Transport::WebSocket);
    successful_hello(&mut tcp, "tcp-hello", None);
    successful_hello(&mut websocket, "ws-hello", None);
    let args = json!({"after":{"boot_id":boot_id,"seq":"0"},
        "filter":{"kinds":[],"targets":[]}});
    tcp.write_value(&json!({"v":1,"msg_id":"tcp-gap","op":"subscribe","args":args}));
    websocket.write_value(&json!({"v":1,"msg_id":"ws-gap","op":"subscribe","args":args}));
    let tcp_gap = tcp.read_for_msg("tcp-gap");
    let ws_gap = websocket.read_for_msg("ws-gap");
    assert_eq!(
        without_msg_id(tcp_gap.clone()),
        without_msg_id(ws_gap.clone())
    );
    assert_eq!(tcp_gap["oldest"], ws_gap["oldest"]);
    assert_eq!(tcp_gap["latest"], ws_gap["latest"]);
    for gap in [tcp_gap, ws_gap] {
        assert_eq!(gap["code"], "event_gap");
        assert_eq!(gap["resync_required"], true);
        assert_eq!(gap["oldest"]["boot_id"], gap["latest"]["boot_id"]);
        let oldest = gap["oldest"]["seq"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let latest = gap["latest"]["seq"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!(oldest > 0);
        assert_eq!(latest - oldest, 1024);
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix = entropy
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::env::temp_dir().join(format!("lab-runtime-m12-parity-{suffix}.sqlite"))
}

fn remove_database(path: &Path) {
    let deadline = Instant::now() + IO_TIMEOUT;
    while std::fs::remove_file(path).is_err() {
        assert!(Instant::now() < deadline, "could not remove test database");
        thread::yield_now();
    }
}

fn history_runs(peer: &mut Peer, scope: &str, seq: u64, database_id: &Value) -> Value {
    let (_, terminal) = peer.operation(json!({
        "v":1,"msg_id":"history","op":"history_read",
        "request_id":{"scope":scope,"seq":seq.to_string()},
        "args":{"mode":"runs","database_id":database_id,"max_records":8,"cursor":null}
    }));
    assert_eq!(terminal["state"], "completed");
    terminal
}

#[test]
fn durable_history_is_shared_but_page_tokens_remain_connection_local() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = temporary_database();
    let mut archive =
        SqliteStore::open_with_boot(&path, "abababababababababababababababab").unwrap();
    archive.start_run("transport parity archive").unwrap();
    archive.stop_run().unwrap();
    archive.finish_boot(Duration::from_secs(1)).unwrap();
    archive.close().unwrap();

    {
        let server = Running::start(Some(&path));
        let mut tcp = server.connect(Transport::Tcp);
        let mut websocket = server.connect(Transport::WebSocket);
        let tcp_hello = successful_hello(&mut tcp, "tcp-hello", None);
        let ws_hello = successful_hello(&mut websocket, "ws-hello", None);
        let tcp_scope = tcp_hello["result"]["scope"].as_str().unwrap().to_owned();
        let ws_scope = ws_hello["result"]["scope"].as_str().unwrap().to_owned();
        let status = tcp.send(json!({"v":1,"msg_id":"status","op":"recording_status","args":{}}));
        let database_id = status["result"]["database_id"].clone();

        let tcp_history = history_runs(&mut tcp, &tcp_scope, 1, &database_id);
        let ws_history = history_runs(&mut websocket, &ws_scope, 1, &database_id);
        let tcp_token = tcp_history["result"]["page_token"].clone();
        let ws_token = ws_history["result"]["page_token"].clone();
        let tcp_page = tcp.send(json!({"v":1,"msg_id":"page","op":"history_page",
            "args":{"page_token":tcp_token}}));
        let ws_page = websocket.send(json!({"v":1,"msg_id":"page","op":"history_page",
            "args":{"page_token":ws_token}}));
        assert_eq!(tcp_page["result"], ws_page["result"]);
        assert_eq!(
            tcp_page["result"]["runs"][0]["label"],
            "transport parity archive"
        );

        drop(tcp);
        let (mut reattached, hello) =
            attach_after_detach(&server, Transport::WebSocket, &tcp_scope);
        assert_eq!(hello["result"]["next_seq"], "2");
        let expired = reattached.send(json!({"v":1,"msg_id":"old-page","op":"history_page",
            "args":{"page_token":tcp_token}}));
        assert_eq!(expired["code"], "history_page_expired");
        let fresh = history_runs(&mut reattached, &tcp_scope, 2, &database_id);
        let fresh_page = reattached.send(json!({"v":1,"msg_id":"fresh-page","op":"history_page",
            "args":{"page_token":fresh["result"]["page_token"]}}));
        assert_eq!(fresh_page["result"], ws_page["result"]);
    }
    remove_database(&path);
}

#[test]
fn admitted_recording_start_survives_tcp_loss_and_reconciles_over_websocket() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = temporary_database();
    {
        let server = Running::start(Some(&path));
        let mut tcp = server.connect(Transport::Tcp);
        let hello = successful_hello(&mut tcp, "hello", None);
        let scope = hello["result"]["scope"].as_str().unwrap().to_owned();
        let start = json!({
            "v":1,"msg_id":"start","op":"recording_start",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"label":"cross-transport retained start"}
        });
        tcp.write_value(&start);
        assert_eq!(tcp.read_for_msg("start")["state"], "accepted");
        drop(tcp);

        let (mut websocket, hello) = attach_after_detach(&server, Transport::WebSocket, &scope);
        assert_eq!(hello["result"]["next_seq"], "2");
        let deadline = Instant::now() + IO_TIMEOUT;
        let terminal = loop {
            let status = websocket.send(json!({"v":1,"msg_id":"start-status",
                "op":"operation_status",
                "args":{"request_id":{"scope":scope,"seq":"1"}}}));
            if status["result"]["state"] != "accepted" {
                break status["result"].clone();
            }
            assert!(
                Instant::now() < deadline,
                "recording_start did not reach an authoritative terminal outcome"
            );
            thread::yield_now();
        };
        assert_eq!(terminal["state"], "completed", "{terminal:?}");

        let retained = websocket.send(start);
        assert_eq!(retained["state"], "completed");
        assert_eq!(retained["result"], terminal["result"]);
        let recording =
            websocket.send(json!({"v":1,"msg_id":"recording","op":"recording_status","args":{}}));
        assert_eq!(recording["result"]["state"], "recording");
        assert_eq!(recording["result"]["run_id"], terminal["result"]["run_id"]);

        let (_, stopped) = websocket.operation(json!({
            "v":1,"msg_id":"stop","op":"recording_stop",
            "request_id":{"scope":scope,"seq":"2"},
            "args":{"run_id":terminal["result"]["run_id"]}
        }));
        assert_eq!(stopped["state"], "completed");
        assert_eq!(stopped["result"]["run_id"], terminal["result"]["run_id"]);
        assert_eq!(
            websocket.send(json!({"v":1,"msg_id":"idle","op":"recording_status","args":{}}),)["result"]
                ["state"],
            "idle"
        );

        websocket.write_value(&json!({
            "v":1,"msg_id":"shutdown","op":"runtime_shutdown",
            "request_id":{"scope":scope,"seq":"3"},"args":{}
        }));
        assert_eq!(websocket.read_for_msg("shutdown")["state"], "accepted");
        let shutdown = websocket.read_for_msg("shutdown");
        assert_eq!(shutdown["state"], "completed", "{shutdown:?}");
        assert_eq!(shutdown["result"]["recorder_flushed"], true);
        server.wait_for_exit();
    }
    remove_database(&path);
}

#[test]
fn shared_codec_rejections_are_equivalent_and_scoped() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let cases = [
        (
            r#"{"v":1,"v":1,"msg_id":"bad","op":"hello","args":{"scope":null}}"#.to_owned(),
            "duplicate_key",
        ),
        ("not-json".to_owned(), "invalid_json"),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"bad\",\"op\":\"discover\",\"args\":{{\"x\":{}}}}}",
                "[".repeat(17) + &"]".repeat(17)
            ),
            "json_depth",
        ),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"bad\",\"op\":\"discover\",\"args\":{{\"x\":[{}]}}}}",
                vec!["0"; 1025].join(",")
            ),
            "json_values",
        ),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"bad\",\"op\":\"discover\",\"args\":{{\"x\":\"{}\"}}}}",
                "x".repeat(513)
            ),
            "string_too_large",
        ),
        (
            r#"{"v":1,"msg_id":"bad","op":"discover","args":{},"extra":true}"#.to_owned(),
            "unknown_field",
        ),
        (
            r#"{"v":1,"msg_id":"bad","op":"reference_retune","request_id":{"scope":"x","seq":"01"},"args":{"reference":"1","expected_revision":"1","target":1.0,"rate":1.0}}"#.to_owned(),
            "invalid_id",
        ),
    ];
    let mut healthy_tcp = server.connect(Transport::Tcp);
    let mut healthy_ws = server.connect(Transport::WebSocket);
    successful_hello(&mut healthy_tcp, "hello", None);
    successful_hello(&mut healthy_ws, "hello", None);
    for (body, code) in cases {
        let mut tcp = server.connect(Transport::Tcp);
        let mut websocket = server.connect(Transport::WebSocket);
        tcp.write_raw(&body);
        websocket.write_raw(&body);
        assert_eq!(tcp.read()["code"], code);
        assert_eq!(websocket.read()["code"], code);
        assert_eq!(
            healthy_tcp.send(json!({"v":1,"msg_id":"healthy","op":"reference",
                "args":{"reference":"1"}}))["type"],
            "result"
        );
        assert_eq!(
            healthy_ws.send(json!({"v":1,"msg_id":"healthy","op":"reference",
                "args":{"reference":"1"}}))["type"],
            "result"
        );
    }
}

#[test]
fn pressured_peer_on_each_transport_does_not_block_the_other() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    for (pressured_transport, healthy_transport) in [
        (Transport::Tcp, Transport::WebSocket),
        (Transport::WebSocket, Transport::Tcp),
    ] {
        let mut pressured = server.connect(pressured_transport);
        successful_hello(&mut pressured, "pressure-hello", None);
        let requests = (0..32)
            .map(|index| {
                json!({"v":1,"msg_id":format!("pressure-{index}"),
                    "op":"discover","args":{}})
            })
            .collect::<Vec<_>>();
        pressured.write_many(&requests);

        let mut healthy = server.connect(healthy_transport);
        successful_hello(&mut healthy, "healthy-hello", None);
        for index in 0..8 {
            let reply = healthy.send(json!({"v":1,"msg_id":format!("healthy-{index}"),
                "op":"reference","args":{"reference":"1"}}));
            assert_eq!(reply["type"], "result");
        }
        drop(pressured);
    }
}

#[test]
fn simultaneous_tcp_and_websocket_clients_do_not_delay_authoritative_shutdown() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start(None);
    let mut tcp = server.connect(Transport::Tcp);
    let mut websocket = server.connect(Transport::WebSocket);
    let hello = successful_hello(&mut tcp, "tcp-hello", None);
    successful_hello(&mut websocket, "ws-hello", None);
    let scope = hello["result"]["scope"].as_str().unwrap();
    tcp.write_value(&json!({"v":1,"msg_id":"shutdown","op":"runtime_shutdown",
        "request_id":{"scope":scope,"seq":"1"},"args":{}}));
    assert_eq!(tcp.read_for_msg("shutdown")["state"], "accepted");
    server.wait_for_exit();

    assert!(TcpStream::connect(server.tcp).is_err());
    websocket
        .get_mut_for_test()
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut byte = [0u8; 1];
    assert!(matches!(
        websocket.read_transport_byte(&mut byte),
        Ok(0) | Err(_)
    ));
}

trait PeerTestAccess {
    fn get_mut_for_test(&mut self) -> &mut TcpStream;
    fn read_transport_byte(&mut self, byte: &mut [u8; 1]) -> std::io::Result<usize>;
}

impl PeerTestAccess for Peer {
    fn get_mut_for_test(&mut self) -> &mut TcpStream {
        match self {
            Self::Tcp(reader) => reader.get_mut(),
            Self::WebSocket(socket) => socket.get_mut(),
        }
    }

    fn read_transport_byte(&mut self, byte: &mut [u8; 1]) -> std::io::Result<usize> {
        self.get_mut_for_test().read(byte)
    }
}
