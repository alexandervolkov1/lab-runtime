//! Real loopback connections exercise framed requests and autonomous ownership.

use lab_runtime::{
    server::run,
    service::{ServiceHost, ServiceOptions},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

// One process-wide fixed Lua supervisor is intentionally shared by this test
// binary; each real-host case owns it through its network shutdown.
static TEST_SERVICE_GATE: Mutex<()> = Mutex::new(());

fn start() -> (
    std::net::SocketAddr,
    Arc<AtomicBool>,
    thread::JoinHandle<()>,
) {
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let join = thread::spawn(move || {
        let host = ServiceHost::startup(
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap(),
        )
        .unwrap();
        ready_tx.send(host.bound_address()).unwrap();
        run(host, flag).unwrap();
    });
    let addr = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    (addr, stop, join)
}
fn send(reader: &mut BufReader<TcpStream>, value: Value) -> Value {
    let bytes = lab_runtime::wire::encode_frame(&value).unwrap();
    reader.get_mut().write_all(&bytes).unwrap();
    read(reader)
}
fn read(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(!line.is_empty(), "connection closed before response");
    serde_json::from_str(&line).unwrap()
}
fn connect(addr: std::net::SocketAddr) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    BufReader::new(stream)
}
fn hello(reader: &mut BufReader<TcpStream>) -> String {
    let reply = send(
        reader,
        json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
    );
    assert_eq!(reply["type"], "result");
    reply["result"]["scope"].as_str().unwrap().into()
}

#[test]
fn fragmented_and_coalesced_frames_preserve_connection_order_and_hello_gate() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut a = connect(addr);
    let gate = send(
        &mut a,
        json!({"v":1,"msg_id":"early","op":"discover","args":{}}),
    );
    assert_eq!(gate["code"], "hello_required");
    let hello_bytes = lab_runtime::wire::encode_frame(
        &json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
    )
    .unwrap();
    a.get_mut().write_all(&hello_bytes[..5]).unwrap();
    a.get_mut().write_all(&hello_bytes[5..]).unwrap();
    assert_eq!(read(&mut a)["type"], "result");
    let first = lab_runtime::wire::encode_frame(
        &json!({"v":1,"msg_id":"q1","op":"controller","args":{"controller":"1"}}),
    )
    .unwrap();
    let second = lab_runtime::wire::encode_frame(
        &json!({"v":1,"msg_id":"q2","op":"reference","args":{"reference":"1"}}),
    )
    .unwrap();
    a.get_mut().write_all(&[first, second].concat()).unwrap();
    assert_eq!(read(&mut a)["msg_id"], "q1");
    assert_eq!(read(&mut a)["msg_id"], "q2");
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn client_disconnect_does_not_stop_native_owner_or_other_client_queries() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut a = connect(addr);
    hello(&mut a);
    let mut b = connect(addr);
    hello(&mut b);
    drop(a);
    let before = send(
        &mut b,
        json!({"v":1,"msg_id":"m1","op":"latest","args":{"signal":{"instrument":"1","parameter":lab_core::TEMPERATURE.get().to_string()}}}),
    );
    let started = Instant::now();
    let mut later = before.clone();
    while started.elapsed() < Duration::from_secs(2) {
        later = send(
            &mut b,
            json!({"v":1,"msg_id":"m2","op":"latest","args":{"signal":{"instrument":"1","parameter":lab_core::TEMPERATURE.get().to_string()}}}),
        );
        if later["result"]["observed_at"] != before["result"]["observed_at"] {
            break;
        }
        thread::yield_now();
    }
    assert_ne!(
        before["result"]["observed_at"],
        later["result"]["observed_at"]
    );
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn partial_frame_trickler_closes_at_absolute_first_byte_deadline_while_healthy_peer_progresses() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut trickler = connect(addr);
    trickler.get_mut().write_all(b"{\"v\":1").unwrap();
    let mut healthy = connect(addr);
    hello(&mut healthy);
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(2100) {
        let progress = send(
            &mut healthy,
            json!({"v":1,"msg_id":"q","op":"reference","args":{"reference":"1"}}),
        );
        assert_eq!(progress["type"], "result");
        thread::sleep(Duration::from_millis(40));
    }
    let mut line = String::new();
    assert_eq!(
        trickler.read_line(&mut line).unwrap(),
        0,
        "trickled incomplete frame stayed attached"
    );
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn incompatible_version_receives_bounded_error_then_only_that_socket_closes() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut incompatible = connect(addr);
    incompatible
        .get_mut()
        .write_all(b"{\"v\":2,\"msg_id\":\"bad\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
        .unwrap();
    let error = read(&mut incompatible);
    assert_eq!(error["code"], "version_mismatch");
    assert_eq!(error["category"], "protocol_error");
    assert!(error["message"].as_str().unwrap().len() <= 256);
    assert_eq!(error["accepted"], false);
    let mut eof = String::new();
    assert_eq!(incompatible.read_line(&mut eof).unwrap(), 0);
    let mut healthy = connect(addr);
    hello(&mut healthy);
    assert_eq!(
        send(
            &mut healthy,
            json!({"v":1,"msg_id":"q","op":"discover","args":{}})
        )["type"],
        "result"
    );
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn unknown_operation_is_rejected_without_closing_the_healthy_session() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut peer = connect(addr);
    hello(&mut peer);
    let rejected = send(
        &mut peer,
        json!({"v":1,"msg_id":"unknown","op":"not_a_runtime_operation","args":{}}),
    );
    assert_eq!(rejected["code"], "unsupported_operation");
    assert_eq!(rejected["category"], "unsupported_operation");
    assert_eq!(rejected["accepted"], false);
    let healthy = send(
        &mut peer,
        json!({"v":1,"msg_id":"later","op":"discover","args":{}}),
    );
    assert_eq!(healthy["type"], "result");
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn duplicate_msg_id_while_first_exchange_is_pending_is_rejected_without_second_dispatch() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut peer = connect(addr);
    hello(&mut peer);
    let one =
        lab_runtime::wire::encode_frame(&json!({"v":1,"msg_id":"same","op":"discover","args":{}}))
            .unwrap();
    peer.get_mut()
        .write_all(&[one.clone(), one].concat())
        .unwrap();
    let first = read(&mut peer);
    let second = read(&mut peer);
    assert_eq!(first["type"], "result");
    assert_eq!(second["code"], "duplicate_msg_id");
    let mut eof = String::new();
    assert_eq!(peer.read_line(&mut eof).unwrap(), 0);
    let mut fresh = connect(addr);
    hello(&mut fresh);
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}

#[test]
fn completed_exchange_releases_msg_id_for_later_use_on_the_same_socket() {
    let _gate = TEST_SERVICE_GATE.lock().unwrap();
    let (addr, stop, join) = start();
    let mut peer = connect(addr);
    hello(&mut peer);
    for _ in 0..2 {
        let reply = send(
            &mut peer,
            json!({"v":1,"msg_id":"reused","op":"discover","args":{}}),
        );
        assert_eq!(reply["type"], "result");
    }
    stop.store(true, Ordering::SeqCst);
    join.join().unwrap();
}
