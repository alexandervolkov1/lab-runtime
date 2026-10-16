//! Bounded WebSocket transport, browser policy, and mixed-transport isolation.

use lab_runtime::{
    server::run,
    service::{ServiceHost, ServiceOptions},
    websocket::{APPLICATION_PATH, APPLICATION_SUBPROTOCOL, HANDSHAKE_BYTES},
    wire::APPLICATION_JSON_LIMIT,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    ClientRequestBuilder, Error as WebSocketError, Message, WebSocket, client,
    protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    },
};

const ORIGIN: &str = "http://127.0.0.1:3000";
static SERVICE_GATE: Mutex<()> = Mutex::new(());

struct Running {
    tcp: SocketAddr,
    websocket: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Running {
    fn start() -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            let options = ServiceOptions::parse(&[
                "--serve",
                "--profile",
                "virtual-demo",
                "--port",
                "0",
                "--ws-port",
                "0",
                "--ws-origin",
                ORIGIN,
            ])
            .unwrap();
            let service = ServiceHost::startup(options).unwrap();
            let tcp = service.bound_address();
            let websocket = service.websocket_bound_address().unwrap();
            let ready: Value = serde_json::from_str(&service.ready_line()).unwrap();
            assert_eq!(ready["port"], tcp.port());
            assert_eq!(ready["websocket"]["port"], websocket.port());
            assert_eq!(ready["websocket"]["path"], APPLICATION_PATH);
            assert_eq!(ready["websocket"]["subprotocol"], APPLICATION_SUBPROTOCOL);
            ready_tx.send((tcp, websocket)).unwrap();
            run(service, flag).unwrap();
        });
        let (tcp, websocket) = ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        Self {
            tcp,
            websocket,
            stop,
            join: Some(join),
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

fn connect_websocket(address: SocketAddr) -> WebSocket<TcpStream> {
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let uri = format!("ws://{address}{APPLICATION_PATH}").parse().unwrap();
    let request = ClientRequestBuilder::new(uri)
        .with_header("Origin", ORIGIN)
        .with_sub_protocol(APPLICATION_SUBPROTOCOL);
    let (socket, response) = client(request, stream).unwrap();
    assert_eq!(response.status(), 101);
    assert_eq!(
        response.headers()["Sec-WebSocket-Protocol"],
        APPLICATION_SUBPROTOCOL
    );
    socket
}

fn send(socket: &mut WebSocket<TcpStream>, value: Value) -> Value {
    socket
        .write(Message::Text(value.to_string().into()))
        .unwrap();
    socket.flush().unwrap();
    read_application(socket)
}

fn read_application(socket: &mut WebSocket<TcpStream>) -> Value {
    loop {
        match socket.read().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected Application text response, got {other:?}"),
        }
    }
}

fn hello(socket: &mut WebSocket<TcpStream>) -> String {
    let reply = send(
        socket,
        json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
    );
    assert_eq!(reply["type"], "result");
    reply["result"]["scope"].as_str().unwrap().to_owned()
}

fn raw_upgrade(address: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut response = Vec::new();
    let mut scratch = [0u8; 512];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(count) => {
                response.extend_from_slice(&scratch[..count]);
                if response.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("upgrade response read failed: {error}"),
        }
    }
    response
}

fn request(
    address: SocketAddr,
    method: &str,
    target: &str,
    host: &str,
    origin_headers: &[&str],
    subprotocol: Option<&str>,
    extra: &str,
) -> Vec<u8> {
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    for origin in origin_headers {
        request.push_str(&format!("Origin: {origin}\r\n"));
    }
    if let Some(subprotocol) = subprotocol {
        request.push_str(&format!("Sec-WebSocket-Protocol: {subprotocol}\r\n"));
    }
    request.push_str(extra);
    request.push_str("\r\n");
    debug_assert_eq!(address.ip().to_string(), "127.0.0.1");
    request.into_bytes()
}

fn rejected(response: &[u8]) -> bool {
    response.is_empty() || !response.starts_with(b"HTTP/1.1 101")
}

#[test]
fn browser_upgrade_policy_is_exact_and_does_not_negotiate_extensions() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let host = format!("127.0.0.1:{}", server.websocket.port());
    let accepted = raw_upgrade(
        server.websocket,
        &request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "Sec-WebSocket-Extensions: permessage-deflate\r\n",
        ),
    );
    assert!(accepted.starts_with(b"HTTP/1.1 101"), "{accepted:?}");
    assert!(
        !String::from_utf8_lossy(&accepted)
            .to_ascii_lowercase()
            .contains("sec-websocket-extensions")
    );

    let cases = [
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &["http://foreign.example:3000"],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &["null"],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN, ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            "/wrong",
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            "/application/v1?query=forbidden",
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            "localhost:1",
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            None,
            "",
        ),
        request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            Some("wrong"),
            "",
        ),
        request(
            server.websocket,
            "POST",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ),
    ];
    for case in cases {
        assert!(rejected(&raw_upgrade(server.websocket, &case)));
    }
}

#[test]
fn delegated_rfc6455_upgrade_requirements_are_rejected() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let host = format!("127.0.0.1:{}", server.websocket.port());
    let valid = String::from_utf8(request(
        server.websocket,
        "GET",
        APPLICATION_PATH,
        &host,
        &[ORIGIN],
        Some(APPLICATION_SUBPROTOCOL),
        "",
    ))
    .unwrap();
    let cases = [
        valid.replace("HTTP/1.1", "HTTP/1.0"),
        valid.replace("Connection: Upgrade\r\n", ""),
        valid.replace("Connection: Upgrade", "Connection: keep-alive"),
        valid.replace("Upgrade: websocket\r\n", ""),
        valid.replace("Upgrade: websocket", "Upgrade: h2c"),
        valid.replace("Sec-WebSocket-Version: 13", "Sec-WebSocket-Version: 12"),
        valid.replace("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n", ""),
        valid.replace(
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
            "Sec-WebSocket-Key: !!!!!!!!!!!!!!!!!!!!!!!!",
        ),
        valid.replace(
            &format!("Host: {host}\r\n"),
            &format!("Host: {host}\r\nHost: {host}\r\n"),
        ),
    ];
    for case in cases {
        assert!(
            rejected(&raw_upgrade(server.websocket, case.as_bytes())),
            "invalid Upgrade admitted: {case:?}"
        );
    }
}

#[test]
fn websocket_uses_the_same_application_for_hello_query_and_mutation() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut socket = connect_websocket(server.websocket);
    let scope = hello(&mut socket);
    let query = send(
        &mut socket,
        json!({"v":1,"msg_id":"q","op":"reference","args":{"reference":"1"}}),
    );
    assert_eq!(query["type"], "result");
    let revision = query["result"]["revision"].clone();
    socket
        .write(Message::Text(
            json!({"v":1,"msg_id":"m","op":"reference_retune",
                "request_id":{"scope":scope,"seq":"1"},
                "args":{"reference":"1","expected_revision":revision,"target":55.0,"rate":2.0}})
            .to_string()
            .into(),
        ))
        .unwrap();
    socket.flush().unwrap();
    let accepted = read_application(&mut socket);
    let completed = read_application(&mut socket);
    assert_eq!(accepted["state"], "accepted");
    assert_eq!(completed["state"], "completed");
}

#[test]
fn shared_json_rejections_are_text_responses_and_binary_is_never_dispatched() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    for (body, code) in [
        (
            "{\"v\":1,\"v\":1,\"msg_id\":\"x\",\"op\":\"hello\",\"args\":{\"scope\":null}}"
                .to_owned(),
            "duplicate_key",
        ),
        ("not-json".to_owned(), "invalid_json"),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"x\",\"op\":\"discover\",\"args\":{{\"x\":{}}}}}",
                "[".repeat(17) + &"]".repeat(17)
            ),
            "json_depth",
        ),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"x\",\"op\":\"discover\",\"args\":{{\"x\":[{}]}}}}",
                vec!["0"; 1025].join(",")
            ),
            "json_values",
        ),
        (
            format!(
                "{{\"v\":1,\"msg_id\":\"x\",\"op\":\"discover\",\"args\":{{\"x\":\"{}\"}}}}",
                "x".repeat(513)
            ),
            "string_too_large",
        ),
    ] {
        let mut socket = connect_websocket(server.websocket);
        socket.write(Message::Text(body.into())).unwrap();
        socket.flush().unwrap();
        assert_eq!(read_application(&mut socket)["code"], code);
    }

    let mut binary = connect_websocket(server.websocket);
    binary
        .write(Message::Binary(b"{}".to_vec().into()))
        .unwrap();
    binary.flush().unwrap();
    assert!(matches!(binary.read(), Ok(Message::Close(_)) | Err(_)));
}

#[test]
fn fragmented_text_is_dispatched_once_and_oversized_message_is_closed() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut socket = connect_websocket(server.websocket);
    let hello_body = br#"{"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}"#;
    let mut exact = vec![b' '; APPLICATION_JSON_LIMIT - hello_body.len()];
    exact.extend_from_slice(hello_body);
    socket
        .write(Message::Text(String::from_utf8(exact).unwrap().into()))
        .unwrap();
    socket.flush().unwrap();
    assert_eq!(read_application(&mut socket)["msg_id"], "h");

    let mut socket = connect_websocket(server.websocket);
    let body = br#"{"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}"#;
    let middle = body.len() / 2;
    socket
        .write(Message::Frame(Frame::message(
            body[..middle].to_vec(),
            OpCode::Data(Data::Text),
            false,
        )))
        .unwrap();
    socket
        .write(Message::Frame(Frame::message(
            body[middle..].to_vec(),
            OpCode::Data(Data::Continue),
            true,
        )))
        .unwrap();
    socket.flush().unwrap();
    assert_eq!(read_application(&mut socket)["msg_id"], "h");
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert!(
        matches!(socket.read(), Err(WebSocketError::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
    );

    let mut oversized = connect_websocket(server.websocket);
    oversized
        .write(Message::Text(" ".repeat(APPLICATION_JSON_LIMIT + 1).into()))
        .unwrap();
    oversized.flush().unwrap();
    assert!(matches!(oversized.read(), Ok(Message::Close(_)) | Err(_)));

    let mut fragmented_oversized = connect_websocket(server.websocket);
    let half = vec![b' '; APPLICATION_JSON_LIMIT / 2 + 1];
    fragmented_oversized
        .write(Message::Frame(Frame::message(
            half.clone(),
            OpCode::Data(Data::Text),
            false,
        )))
        .unwrap();
    fragmented_oversized
        .write(Message::Frame(Frame::message(
            half,
            OpCode::Data(Data::Continue),
            true,
        )))
        .unwrap();
    fragmented_oversized.flush().unwrap();
    assert!(matches!(
        fragmented_oversized.read(),
        Ok(Message::Close(_)) | Err(_)
    ));
}

#[test]
fn ping_close_and_incomplete_upgrade_are_bounded_without_harming_tcp() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut socket = connect_websocket(server.websocket);
    hello(&mut socket);
    socket
        .write(Message::Ping(b"bounded".to_vec().into()))
        .unwrap();
    socket.flush().unwrap();
    assert!(
        matches!(socket.read().unwrap(), Message::Pong(payload) if payload.as_ref() == b"bounded")
    );
    socket.close(None).unwrap();
    assert!(matches!(
        socket.read(),
        Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed)
    ));

    let mut incomplete = TcpStream::connect(server.websocket).unwrap();
    incomplete
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    incomplete
        .write_all(b"GET /application/v1 HTTP/1.1\r\n")
        .unwrap();

    let mut tcp = TcpStream::connect(server.tcp).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.write_all(b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
        .unwrap();
    let mut response = String::new();
    BufReader::new(tcp).read_line(&mut response).unwrap();
    assert!(response.contains("\"type\":\"result\""));

    let began = Instant::now();
    let mut byte = [0u8; 1];
    assert_eq!(incomplete.read(&mut byte).unwrap(), 0);
    assert!(began.elapsed() < Duration::from_secs(3));
}

#[test]
fn incomplete_websocket_message_deadline_is_absolute_and_isolated() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut slow = connect_websocket(server.websocket);
    slow.write(Message::Frame(Frame::message(
        br#"{"v":1,"msg_id":"slow""#.to_vec(),
        OpCode::Data(Data::Text),
        false,
    )))
    .unwrap();
    slow.flush().unwrap();

    let mut healthy = BufReader::new(TcpStream::connect(server.tcp).unwrap());
    healthy
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    healthy
        .get_mut()
        .write_all(b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
        .unwrap();
    let mut line = String::new();
    healthy.read_line(&mut line).unwrap();
    assert!(line.contains("\"type\":\"result\""));

    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut sequence = 0;
    while Instant::now() < deadline {
        sequence += 1;
        healthy
            .get_mut()
            .write_all(
                format!(
                    "{{\"v\":1,\"msg_id\":\"q{sequence}\",\"op\":\"reference\",\"args\":{{\"reference\":\"1\"}}}}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        line.clear();
        healthy.read_line(&mut line).unwrap();
        assert!(line.contains("\"type\":\"result\""));
        thread::sleep(Duration::from_millis(100));
    }
    assert!(matches!(slow.read(), Ok(Message::Close(_)) | Err(_)));
}

#[test]
fn fragmented_text_with_interleaved_ping_keeps_the_absolute_message_deadline() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut slow = connect_websocket(server.websocket);
    hello(&mut slow);
    let mut fragment = Frame::message(
        br#"{"v":1,"msg_id":"slow""#.to_vec(),
        OpCode::Data(Data::Text),
        false,
    );
    fragment.header_mut().mask = Some([1, 2, 3, 4]);
    let mut ping = Frame::ping(b"between-fragments".to_vec());
    ping.header_mut().mask = Some([5, 6, 7, 8]);
    let mut coalesced = Vec::new();
    fragment.format(&mut coalesced).unwrap();
    ping.format(&mut coalesced).unwrap();
    slow.get_mut().write_all(&coalesced).unwrap();

    let mut healthy = connect_websocket(server.websocket);
    hello(&mut healthy);
    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut sequence = 0;
    while Instant::now() < deadline {
        sequence += 1;
        let reply = send(
            &mut healthy,
            json!({"v":1,"msg_id":format!("q{sequence}"),"op":"reference","args":{"reference":"1"}}),
        );
        assert_eq!(reply["type"], "result");
        thread::sleep(Duration::from_millis(100));
    }

    assert!(matches!(slow.read(), Ok(Message::Pong(_))));
    match slow.read() {
        Ok(Message::Close(_))
        | Err(
            WebSocketError::ConnectionClosed
            | WebSocketError::AlreadyClosed
            | WebSocketError::Protocol(_),
        ) => {}
        Err(WebSocketError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("fragmented message did not reach its deadline: {other:?}"),
    }
}

#[test]
fn idle_ping_does_not_create_an_incomplete_application_message_deadline() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut socket = connect_websocket(server.websocket);
    hello(&mut socket);
    socket
        .write(Message::Ping(b"idle".to_vec().into()))
        .unwrap();
    socket.flush().unwrap();
    assert!(matches!(socket.read(), Ok(Message::Pong(_))));

    thread::sleep(Duration::from_millis(2200));
    let reply = send(
        &mut socket,
        json!({"v":1,"msg_id":"still-open","op":"reference","args":{"reference":"1"}}),
    );
    assert_eq!(reply["type"], "result");
}

#[test]
fn incomplete_tcp_frame_does_not_delay_healthy_websocket_progress() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut slow = TcpStream::connect(server.tcp).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    slow.write_all(b"{\"v\":1").unwrap();

    let mut healthy = connect_websocket(server.websocket);
    hello(&mut healthy);
    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut sequence = 0;
    while Instant::now() < deadline {
        sequence += 1;
        let reply = send(
            &mut healthy,
            json!({"v":1,"msg_id":format!("q{sequence}"),"op":"reference","args":{"reference":"1"}}),
        );
        assert_eq!(reply["type"], "result");
        thread::sleep(Duration::from_millis(100));
    }
    let mut byte = [0u8; 1];
    assert_eq!(slow.read(&mut byte).unwrap(), 0);
}

#[test]
fn unacknowledged_protocol_close_releases_capacity_at_absolute_deadline() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut closing = connect_websocket(server.websocket);
    closing
        .write(Message::Binary(b"not application json".to_vec().into()))
        .unwrap();
    closing.flush().unwrap();

    let mut retained = Vec::new();
    for index in 0..7 {
        let mut stream = TcpStream::connect(server.tcp).unwrap();
        stream
            .write_all(
                format!(
                    "{{\"v\":1,\"msg_id\":\"h{index}\",\"op\":\"hello\",\"args\":{{\"scope\":null}}}}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        retained.push(stream);
    }
    thread::sleep(Duration::from_millis(100));
    let mut ninth = TcpStream::connect(server.tcp).unwrap();
    ninth
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    ninth
        .write_all(b"{\"v\":1,\"msg_id\":\"ninth\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
        .unwrap();
    let mut byte = [0u8; 1];
    match ninth.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("ninth client was not rejected: {other:?}"),
    }

    thread::sleep(Duration::from_millis(2100));
    let mut replacement = connect_websocket(server.websocket);
    assert!(!hello(&mut replacement).is_empty());
}

#[test]
fn mixed_tcp_and_websocket_connections_share_the_global_eight_slot_budget() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let mut tcp = Vec::new();
    for _ in 0..4 {
        let stream = TcpStream::connect(server.tcp).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        tcp.push(stream);
    }
    let mut websocket = Vec::new();
    for _ in 0..4 {
        websocket.push(connect_websocket(server.websocket));
    }

    let mut ninth = TcpStream::connect(server.websocket).unwrap();
    ninth
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let host = format!("127.0.0.1:{}", server.websocket.port());
    ninth
        .write_all(&request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            "",
        ))
        .unwrap();
    let mut response = [0u8; 1];
    match ninth.read(&mut response) {
        Ok(0) => {}
        Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset) => {}
        other => panic!("ninth mixed client was not rejected: {other:?}"),
    }

    drop(tcp.pop());
    thread::sleep(Duration::from_millis(100));
    let mut replacement = connect_websocket(server.websocket);
    assert!(!hello(&mut replacement).is_empty());
}

#[test]
fn upgrade_byte_and_header_limits_are_project_owned() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = Running::start();
    let host = format!("127.0.0.1:{}", server.websocket.port());
    let oversized = request(
        server.websocket,
        "GET",
        APPLICATION_PATH,
        &host,
        &[ORIGIN],
        Some(APPLICATION_SUBPROTOCOL),
        &format!("X-Fill: {}\r\n", "x".repeat(HANDSHAKE_BYTES)),
    );
    assert!(oversized.len() > HANDSHAKE_BYTES);
    assert!(rejected(&raw_upgrade(server.websocket, &oversized)));

    let headers = (0..33)
        .map(|index| format!("X-{index}: x\r\n"))
        .collect::<String>();
    assert!(rejected(&raw_upgrade(
        server.websocket,
        &request(
            server.websocket,
            "GET",
            APPLICATION_PATH,
            &host,
            &[ORIGIN],
            Some(APPLICATION_SUBPROTOCOL),
            &headers,
        ),
    )));
}
