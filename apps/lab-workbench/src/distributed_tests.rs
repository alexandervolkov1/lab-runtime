//! Real Runtime transport parity and protected local TLS proxy acceptance.

use crate::client::{
    ClientHandle, ClientUpdate,
    endpoint::RuntimeEndpoint,
    types::{CommandSendError, EventCursor, HelloState, MutationIdentity, ReplyKind},
};
use lab_runtime::{
    server,
    service::{ServiceHost, ServiceOptions},
};
use serde_json::{Value, json};
use std::{
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    Error, Message,
    handshake::server::{ErrorResponse, Request, Response},
    http::StatusCode,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const ORIGIN: &str = "http://127.0.0.1:3000";
static SERVICE_GATE: Mutex<()> = Mutex::new(());
static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

#[allow(
    clippy::result_large_err,
    reason = "Tungstenite fixes the callback ErrorResponse type"
)]
fn authorize_proxy(request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    if request
        .headers()
        .get("X-Token")
        .and_then(|value| value.to_str().ok())
        != Some("synthetic-test-key")
    {
        let mut denied = ErrorResponse::new(Some("unauthorized".into()));
        *denied.status_mut() = StatusCode::UNAUTHORIZED;
        return Err(denied);
    }
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        tungstenite::http::HeaderValue::from_static("lab-runtime.application.v1"),
    );
    Ok(response)
}

struct ProxyAuthorization {
    accepted: Arc<AtomicUsize>,
    denied: Arc<AtomicUsize>,
}
impl tungstenite::handshake::server::Callback for ProxyAuthorization {
    #[allow(
        clippy::result_large_err,
        reason = "Tungstenite fixes the callback ErrorResponse type"
    )]
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let result = authorize_proxy(request, response);
        if result.is_ok() {
            self.accepted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.denied.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}

struct Running {
    tcp: SocketAddr,
    ws: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Running {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (tx, rx) = mpsc::sync_channel(1);
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
            tx.send((
                service.bound_address(),
                service.websocket_bound_address().unwrap(),
            ))
            .unwrap();
            server::run(service, flag).unwrap();
        });
        let (tcp, ws) = rx.recv_timeout(TIMEOUT).unwrap();
        Self {
            tcp,
            ws,
            stop,
            join: Some(join),
        }
    }
    fn endpoint(&self, ws: bool) -> RuntimeEndpoint {
        RuntimeEndpoint::parse(
            &if ws {
                format!("ws://{}/application/v1", self.ws)
            } else {
                self.tcp.to_string()
            },
            ORIGIN,
            false,
            None,
            None,
        )
        .unwrap()
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let deadline = Instant::now() + TIMEOUT;
            while !join.is_finished() {
                assert!(Instant::now() < deadline, "Runtime shutdown deadline");
                thread::yield_now();
            }
            join.join().unwrap();
        }
    }
}

fn wait_update(
    client: &ClientHandle,
    mut predicate: impl FnMut(&ClientUpdate) -> bool,
) -> ClientUpdate {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let update = client
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("bounded client update");
        assert!(
            !matches!(
                update,
                ClientUpdate::LocalRejected { .. }
                    | ClientUpdate::RecoveryJournalProblem { .. }
                    | ClientUpdate::TransportFailure { .. }
            ),
            "unexpected failure: {update:?}"
        );
        if predicate(&update) {
            return update;
        }
    }
}
fn attach(client: &ClientHandle, scope: Option<String>) -> HelloState {
    client.connect(scope).unwrap();
    let ClientUpdate::Hello(hello) =
        wait_update(client, |update| matches!(update, ClientUpdate::Hello(_)))
    else {
        unreachable!()
    };
    hello
}
fn reply(client: &ClientHandle, id: u64, kind: ReplyKind) -> Value {
    let ClientUpdate::Reply { envelope, .. } = wait_update(
        client,
        |update| matches!(update, ClientUpdate::Reply {command_id, kind: actual, ..} if *command_id == id && *actual == kind),
    ) else {
        unreachable!()
    };
    envelope
}
fn query(client: &ClientHandle, op: &str, args: Value) -> Value {
    let id = client.query(op, args).unwrap();
    reply(client, id, ReplyKind::Result)["result"].clone()
}

#[test]
fn tcp_and_websocket_share_queries_operations_exact_retry_and_subscription_authority() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = Running::start();
    let tcp = ClientHandle::spawn_configured(runtime.endpoint(false), false, None, None).unwrap();
    let ws = ClientHandle::spawn_configured(runtime.endpoint(true), false, None, None).unwrap();
    let tcp_hello = attach(&tcp, None);
    let ws_hello = attach(&ws, None);
    assert_eq!(tcp_hello.boot_id, ws_hello.boot_id);
    assert_eq!(tcp_hello.operations, ws_hello.operations);
    assert_ne!(tcp_hello.scope, ws_hello.scope);
    for (op, args) in [
        ("discover", json!({})),
        ("reference", json!({"reference":"1"})),
        ("recording_status", json!({})),
    ] {
        let left = query(&tcp, op, args.clone());
        let right = query(&ws, op, args);
        // Separate committed snapshots can advance acquisition time. Compare
        // discovery identity/capabilities and Reference configuration, not time.
        let comparable = |mut value: Value| {
            if op == "discover" {
                value["records"] = Value::Array(value["records"].as_array().unwrap().iter().map(|record| {
                    json!({"kind":record["kind"], "id":record["id"], "capabilities":record["capabilities"], "generation":record["generation"]})
                }).collect());
                value.as_object_mut().unwrap().remove("projection");
                value.as_object_mut().unwrap().remove("revision");
            } else if op == "reference" {
                for field in ["value", "last_at", "last_evaluated_at_ns"] {
                    value.as_object_mut().unwrap().remove(field);
                }
            }
            value
        };
        assert_eq!(
            comparable(left),
            comparable(right),
            "transport parity for {op}"
        );
    }
    let signal = json!({"instrument":"1", "parameter":"1"});
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let current = query(&tcp, "latest", json!({"signal":signal}));
        if current["status"] == "available" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "virtual acquisition did not publish"
        );
        thread::yield_now();
    }
    let sample_semantics = |sample: &Value| {
        json!({"signal":sample["signal"], "value":sample["value"],
            "unit":sample["unit"], "quality":sample["quality"],
            "status":sample["status"], "failure":sample["failure"],
            "generation":sample["generation"]})
    };
    let mut expected_sample = None;
    for client in [&tcp, &ws] {
        let latest = query(client, "latest", json!({"signal":signal}));
        assert_eq!(latest["value"], 20.0);
        let comparable = sample_semantics(&latest);
        if let Some(expected) = &expected_sample {
            assert_eq!(&comparable, expected, "current observation parity");
        } else {
            expected_sample = Some(comparable.clone());
        }
        let current = query(client, "measurements_current", json!({}));
        let row = current["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["signal"] == signal)
            .unwrap();
        assert_eq!(sample_semantics(row), comparable);
        let window = query(
            client,
            "measurement_window",
            json!({"signal":signal,"max_records":8}),
        );
        assert_eq!(window["ordering"], "oldest_first");
        assert_eq!(window["source"], "runtime_recent");
        let rows = window["rows"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() <= 8);
        assert!(rows.iter().all(|row| sample_semantics(row) == comparable));
        assert!(rows.windows(2).all(|pair| {
            pair[0]["observed_at_ns"]
                .as_str()
                .unwrap()
                .parse::<u128>()
                .unwrap()
                <= pair[1]["observed_at_ns"]
                    .as_str()
                    .unwrap()
                    .parse::<u128>()
                    .unwrap()
        }));
    }
    for client in [&tcp, &ws] {
        let bootstrap = client.bootstrap_reference("1").unwrap();
        wait_update(
            client,
            |update| matches!(update, ClientUpdate::ReferenceBootstrap {command_id, ..} if *command_id == bootstrap),
        );
    }
    let retune = tcp
        .mutation(
            "reference_retune",
            json!({"reference":"1", "expected_revision":"1", "target":1.0, "rate":2.0}),
        )
        .unwrap();
    let accepted = reply(&tcp, retune, ReplyKind::MutationAccepted);
    let completed = reply(&tcp, retune, ReplyKind::MutationCompleted);
    assert_eq!(accepted["request_id"], completed["request_id"]);
    let identity = MutationIdentity {
        scope: tcp_hello.scope.clone(),
        seq: 1,
    };
    let status = tcp.operation_status(identity.clone()).unwrap();
    let status_result = reply(&tcp, status, ReplyKind::Result);
    assert_eq!(status_result["result"]["state"], "completed");
    let retry = tcp.retry_mutation(identity).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let update = tcp
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if let ClientUpdate::LocalRejected { command_id, reason } = update {
            assert_eq!(command_id, retry);
            assert_eq!(reason, "recovery_terminal");
            break;
        }
    }
    assert_eq!(
        query(&tcp, "reference", json!({"reference":"1"}))["revision"],
        "2"
    );
    assert_eq!(
        query(&ws, "reference", json!({"reference":"1"}))["revision"],
        "2"
    );
    let fail = ws
        .mutation(
            "reference_retune",
            json!({"reference":"1", "expected_revision":"1", "target":3.0, "rate":2.0}),
        )
        .unwrap();
    reply(&ws, fail, ReplyKind::MutationAccepted);
    reply(&ws, fail, ReplyKind::MutationFailed);
    let after = EventCursor {
        boot_id: tcp_hello.boot_id,
        seq: 0,
    };
    let unsubscribe = ws.unsubscribe().unwrap();
    reply(&ws, unsubscribe, ReplyKind::Result);
    let subscription = ws
        .subscribe(after, json!({"kinds":[], "targets":[]}))
        .unwrap();
    reply(&ws, subscription, ReplyKind::Result);
    wait_update(&ws, |update| matches!(update, ClientUpdate::Event { .. }));
    let unsubscribe = ws.unsubscribe().unwrap();
    reply(&ws, unsubscribe, ReplyKind::Result);
    tcp.shutdown().unwrap();
    ws.shutdown().unwrap();
}

#[test]
fn observation_client_rejects_mutations_and_exact_retry_without_identity_or_journal_changes() {
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = Running::start();
    for websocket in [false, true] {
        let client =
            ClientHandle::spawn_configured(runtime.endpoint(websocket), true, None, None).unwrap();
        let hello = attach(&client, None);
        assert_eq!(
            client.mutation("reference_retune", json!({})),
            Err(CommandSendError::ObservationOnly)
        );
        assert_eq!(
            client.retry_mutation(MutationIdentity {
                scope: hello.scope,
                seq: 1
            }),
            Err(CommandSendError::ObservationOnly)
        );
        assert_eq!(
            query(&client, "reference", json!({"reference":"1"}))["revision"],
            "1"
        );
        assert!(query(&client, "discover", json!({})).is_object());
        client.shutdown().unwrap();
    }
}

// A test-only reverse proxy proves normal certificate/hostname verification and
// handshake authorization. It is not a required deployment bridge or Runtime API.
struct TlsProxy {
    accepted: Arc<AtomicUsize>,
    denied: Arc<AtomicUsize>,
    address: SocketAddr,
    ca: PathBuf,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl TlsProxy {
    fn start(upstream: SocketAddr) -> Self {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate = certified.cert.der().clone();
        let key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key.into())
        .unwrap();
        let config = Arc::new(config);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/m18-distributed-workbench-20261008/tls-fixtures");
        std::fs::create_dir_all(&root).unwrap();
        let ca = root.join(format!(
            "{}-{}.pem",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&ca, certified.cert.pem()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let accepted = Arc::new(AtomicUsize::new(0));
        let denied = Arc::new(AtomicUsize::new(0));
        let proxy_accepted = Arc::clone(&accepted);
        let proxy_denied = Arc::clone(&denied);
        let join = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::park_timeout(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("proxy accept: {error}"),
                };
                // Winsock accepted sockets inherit the listener's nonblocking
                // mode; the bounded TLS/HTTP fixture handshake is blocking.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(Arc::clone(&config)).unwrap(),
                    stream,
                );
                let callback = ProxyAuthorization {
                    accepted: Arc::clone(&proxy_accepted),
                    denied: Arc::clone(&proxy_denied),
                };
                let Ok(mut remote) = tungstenite::accept_hdr(tls, callback) else {
                    continue;
                };
                let request = RuntimeEndpoint::parse(
                    &format!("ws://{upstream}/application/v1"),
                    ORIGIN,
                    false,
                    None,
                    None,
                )
                .unwrap();
                let RuntimeEndpoint::WebSocket(request) = request else {
                    unreachable!()
                };
                let stream = TcpStream::connect(upstream).unwrap();
                stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                let (mut local, _) =
                    tungstenite::client(request.request().unwrap(), stream).unwrap();
                remote.get_mut().sock.set_nonblocking(true).unwrap();
                local.get_mut().set_nonblocking(true).unwrap();
                let mut connected = true;
                while connected && !flag.load(Ordering::Acquire) {
                    match remote.read() {
                        Ok(Message::Text(text)) => {
                            if local.write(Message::Text(text)).is_err() {
                                connected = false;
                            }
                        }
                        Ok(Message::Close(_))
                        | Err(Error::ConnectionClosed | Error::AlreadyClosed) => connected = false,
                        Ok(_) => {}
                        Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(_) => connected = false,
                    }
                    match local.read() {
                        Ok(Message::Text(text)) => {
                            if remote.write(Message::Text(text)).is_err() {
                                connected = false;
                            }
                        }
                        Ok(Message::Close(_))
                        | Err(Error::ConnectionClosed | Error::AlreadyClosed) => connected = false,
                        Ok(_) => {}
                        Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(_) => connected = false,
                    }
                    let _ = remote.flush();
                    let _ = local.flush();
                    thread::park_timeout(Duration::from_millis(2));
                }
                let _ = remote.flush();
                let _ = local.close(None);
            }
        });
        Self {
            accepted,
            denied,
            address,
            ca,
            stop,
            join: Some(join),
        }
    }
    fn endpoint(&self, host: &str, token: Option<&str>, trust_ca: bool) -> RuntimeEndpoint {
        RuntimeEndpoint::parse(
            &format!("wss://{host}:{}/application/v1", self.address.port()),
            ORIGIN,
            false,
            token.map(str::to_owned),
            trust_ca.then(|| self.ca.clone()),
        )
        .unwrap()
    }
}
impl Drop for TlsProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
        let _ = std::fs::remove_file(&self.ca);
    }
}

#[test]
fn wss_proxy_requires_key_trusted_certificate_and_matching_hostname() {
    // Explicit desktop acceptance helper mode. Ordinary gates always run the
    // complete negative/positive test below, without this environment variable.
    if let Some(upstream) = std::env::var_os("LAB_M18_DESKTOP_PROXY_UPSTREAM") {
        let upstream: SocketAddr = upstream.to_str().unwrap().parse().unwrap();
        assert!(upstream.ip().is_loopback());
        let proxy = TlsProxy::start(upstream);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/m18-distributed-workbench-20261008");
        let done = root.join("desktop-proxy.done");
        assert!(!done.exists(), "stale desktop helper completion marker");
        std::fs::write(
            root.join("desktop-proxy-ready.json"),
            json!({
                "endpoint":format!("wss://localhost:{}/application/v1", proxy.address.port()),
                "ca_file":proxy.ca,"pid":std::process::id()
            })
            .to_string(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !done.exists() {
            assert!(Instant::now() < deadline, "desktop proxy fixture deadline");
            thread::park_timeout(Duration::from_millis(10));
        }
        assert!(proxy.accepted.load(Ordering::Relaxed) >= 1);
        return;
    }
    let _gate = SERVICE_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = Running::start();
    let proxy = TlsProxy::start(runtime.ws);
    for (case, host, token, ca, expected_failure) in [
        (
            "missing key",
            "localhost",
            None,
            true,
            "authorization rejected",
        ),
        (
            "wrong key",
            "localhost",
            Some("wrong-key"),
            true,
            "authorization rejected",
        ),
        (
            "untrusted CA",
            "localhost",
            Some("synthetic-test-key"),
            false,
            "TLS validation failed",
        ),
        (
            "wrong hostname",
            "127.0.0.1",
            Some("synthetic-test-key"),
            true,
            "TLS validation failed",
        ),
    ] {
        let client =
            ClientHandle::spawn_configured(proxy.endpoint(host, token, ca), false, None, None)
                .unwrap();
        client.connect(None).unwrap();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert!(
                !matches!(update, ClientUpdate::Hello(_)),
                "unauthorized/TLS-invalid connection succeeded"
            );
            if let ClientUpdate::TransportFailure { reason } = update {
                assert!(!reason.contains("synthetic-test-key") && !reason.contains("wrong-key"));
                assert!(
                    reason.contains(expected_failure),
                    "{case}: expected {expected_failure}, observed {reason}"
                );
                break;
            }
        }
        client.shutdown().unwrap();
    }
    assert_eq!(
        proxy.denied.load(Ordering::Relaxed),
        2,
        "missing and wrong keys must reach the authorization check"
    );
    assert_eq!(
        proxy.accepted.load(Ordering::Relaxed),
        0,
        "invalid TLS/keys must never pass authorization"
    );
    let client = ClientHandle::spawn_configured(
        proxy.endpoint("localhost", Some("synthetic-test-key"), true),
        true,
        None,
        None,
    )
    .unwrap();
    attach(&client, None);
    assert_eq!(proxy.accepted.load(Ordering::Relaxed), 1);
    assert!(query(&client, "discover", json!({})).is_object());
    assert_eq!(
        query(&client, "reference", json!({"reference":"1"}))["revision"],
        "1"
    );
    assert_eq!(
        log::STATIC_MAX_LEVEL,
        log::LevelFilter::Info,
        "raw Upgrade trace logging must be compiled out"
    );
    client.shutdown().unwrap();
}

#[allow(
    clippy::result_large_err,
    reason = "Tungstenite fixes the callback ErrorResponse type"
)]
fn scripted_upgrade(_: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        tungstenite::http::HeaderValue::from_static("lab-runtime.application.v1"),
    );
    Ok(response)
}

fn accept_ws(listener: &TcpListener) -> tungstenite::WebSocket<TcpStream> {
    let deadline = Instant::now() + TIMEOUT;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "scripted accept deadline");
                thread::yield_now();
            }
            Err(error) => panic!("scripted accept: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
    tungstenite::accept_hdr(stream, scripted_upgrade).unwrap()
}
fn ws_read(socket: &mut tungstenite::WebSocket<TcpStream>) -> Value {
    loop {
        match socket.read().unwrap() {
            Message::Text(text) => return serde_json::from_slice(text.as_bytes()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            message => panic!("unexpected scripted message: {message:?}"),
        }
    }
}
fn ws_write(socket: &mut tungstenite::WebSocket<TcpStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}

fn ws_fragmented_hello(socket: &mut tungstenite::WebSocket<TcpStream>, value: Value) {
    use tungstenite::protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    };
    let bytes = value.to_string().into_bytes();
    let split = bytes.len() / 2;
    socket
        .send(Message::Frame(Frame::message(
            bytes[..split].to_vec(),
            OpCode::Data(Data::Text),
            false,
        )))
        .unwrap();
    socket
        .send(Message::Ping(b"fragment-control".to_vec().into()))
        .unwrap();
    socket
        .send(Message::Frame(Frame::message(
            bytes[split..].to_vec(),
            OpCode::Data(Data::Continue),
            true,
        )))
        .unwrap();
}
fn scripted_hello(msg_id: &Value, boot: &str, scope: &str, seq: u64) -> Value {
    json!({"v":1,"msg_id":msg_id,"type":"result","result":{
        "boot_id":boot,"scope":scope,"next_seq":seq.to_string(),
        "operations":["hello","reference","reference_retune","operation_status","subscribe","unsubscribe"],
        "capabilities":[],"limits":{"client_pending_requests":8},
        "event_oldest":{"boot_id":boot,"seq":"0"},"event_latest":{"boot_id":boot,"seq":"0"}}})
}
fn journal_fixture(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/m18-distributed-workbench-20261008/recovery-fixtures");
    std::fs::create_dir_all(&root).unwrap();
    root.join(format!(
        "{name}-{}-{}.json",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn websocket_fault_retains_ambiguous_exact_payload_and_never_replays_before_manual_retry() {
    use crate::client::types::KnownAdmission;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let path = journal_fixture("ambiguous");
    let args = json!({"reference":"1","expected_revision":"1","target":41.5,"rate":2.25});
    let original = args.clone();
    let (pong_tx, pong_rx) = mpsc::sync_channel(1);
    let (probe_tx, probe_rx) = mpsc::sync_channel(1);
    let (probed_tx, probed_rx) = mpsc::sync_channel(1);
    let peer = thread::spawn(move || {
        let mut first = accept_ws(&listener);
        let hello = ws_read(&mut first);
        assert_eq!(hello["args"]["scope"], Value::Null);
        ws_fragmented_hello(
            &mut first,
            scripted_hello(&hello["msg_id"], "boot", "scope", 1),
        );
        first
            .send(Message::Ping(b"bounded-ping".to_vec().into()))
            .unwrap();
        let mut fragment_pong = false;
        loop {
            match first.read().unwrap() {
                Message::Pong(bytes) if bytes.as_ref() == b"fragment-control" => {
                    fragment_pong = true
                }
                Message::Pong(bytes) if bytes.as_ref() == b"bounded-ping" => {
                    assert!(fragment_pong);
                    break;
                }
                other => panic!("unexpected control response: {other:?}"),
            }
        }
        pong_tx.send(()).unwrap();
        let mutation = ws_read(&mut first);
        assert_eq!(mutation["args"], original);
        assert_eq!(mutation["request_id"], json!({"scope":"scope","seq":"1"}));
        drop(first); // no acceptance/terminal evidence: mutation remains ambiguous.
        let mut second = accept_ws(&listener);
        let hello = ws_read(&mut second);
        assert_eq!(hello["op"], "hello");
        assert_eq!(hello["args"]["scope"], "scope");
        ws_write(
            &mut second,
            scripted_hello(&hello["msg_id"], "boot", "scope", 1),
        );
        probe_rx.recv_timeout(TIMEOUT).unwrap();
        second.get_mut().set_nonblocking(true).unwrap();
        assert!(
            matches!(second.read(), Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock),
            "no automatic mutation/status/retry"
        );
        second.get_mut().set_nonblocking(false).unwrap();
        probed_tx.send(()).unwrap();
        let retry = ws_read(&mut second);
        assert_eq!(retry["op"], mutation["op"]);
        assert_eq!(retry["args"], mutation["args"]);
        assert_eq!(retry["request_id"], mutation["request_id"]);
        ws_write(
            &mut second,
            json!({"v":1,"msg_id":retry["msg_id"],"type":"operation","request_id":retry["request_id"],"state":"completed","result":{"revision":"2"}}),
        );
        // A bounded graceful Close is a transport action, not Runtime shutdown.
        assert!(matches!(second.read().unwrap(), Message::Close(_)));
        assert!(matches!(
            second.flush(),
            Ok(()) | Err(Error::ConnectionClosed)
        ));
    });
    let endpoint = RuntimeEndpoint::parse(
        &format!("ws://{address}/application/v1"),
        ORIGIN,
        false,
        None,
        None,
    )
    .unwrap();
    let client = ClientHandle::spawn_configured(endpoint, false, Some(path.clone()), None).unwrap();
    attach(&client, None);
    pong_rx.recv_timeout(TIMEOUT).unwrap();
    client.mutation("reference_retune", args.clone()).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    let mut reattached = false;
    let mut ambiguous = false;
    while !reattached || !ambiguous {
        let update = client
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if let ClientUpdate::Hello(hello) = &update {
            assert_eq!(hello.scope, "scope");
            reattached = true;
        }
        if let ClientUpdate::RecoveryProjection { active, .. } = &update {
            ambiguous |= active
                .iter()
                .any(|record| record.args == args && record.admission == KnownAdmission::Ambiguous);
        }
    }
    let before = std::fs::read(&path).unwrap();
    probe_tx.send(()).unwrap();
    probed_rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let retry = client
        .retry_mutation(MutationIdentity {
            scope: "scope".into(),
            seq: 1,
        })
        .unwrap();
    reply(&client, retry, ReplyKind::MutationCompleted);
    let journal = crate::recovery::load_journal(&path).unwrap();
    assert_eq!(journal.records.len(), 1);
    assert_eq!(journal.records[0].args, args);
    assert_eq!(
        journal.records[0].admission,
        KnownAdmission::Completed.into()
    );
    client.shutdown().unwrap();
    peer.join().unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn websocket_instance_change_requires_explicit_scope_and_preserves_empty_or_unresolved_journal() {
    use crate::{
        client::types::{KnownAdmission, RecoveryRecord},
        model::WorkbenchModel,
        presentation::PresentationDocument,
        recovery::{RecoveryJournal, save_journal},
    };
    for unresolved in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let path = journal_fixture("instance");
        let records = if unresolved {
            vec![RecoveryRecord {
                boot_id: "old-boot".into(),
                identity: MutationIdentity {
                    scope: "old-scope".into(),
                    seq: 1,
                },
                op: "reference_retune".into(),
                args: json!({"reference":"1","target":41.5}),
                admission: KnownAdmission::Ambiguous,
            }]
        } else {
            Vec::new()
        };
        save_journal(
            &path,
            &RecoveryJournal::from_records("old-boot".into(), "old-scope".into(), 1, &records)
                .unwrap(),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let (probe_tx, probe_rx) = mpsc::sync_channel(1);
        let (probed_tx, probed_rx) = mpsc::sync_channel(1);
        let peer = thread::spawn(move || {
            let mut first = accept_ws(&listener);
            let hello = ws_read(&mut first);
            assert_eq!(hello["args"]["scope"], "old-scope");
            ws_write(
                &mut first,
                json!({"v":1,"msg_id":hello["msg_id"],"type":"error","code":"instance_changed"}),
            );
            drop(first);
            probe_rx.recv_timeout(TIMEOUT).unwrap();
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
                "new scope must never be selected automatically"
            );
            probed_tx.send(()).unwrap();
            let mut second = accept_ws(&listener);
            let hello = ws_read(&mut second);
            assert_eq!(hello["args"]["scope"], Value::Null);
            ws_write(
                &mut second,
                scripted_hello(&hello["msg_id"], "new-boot", "new-scope", 1),
            );
            assert!(
                matches!(second.read().unwrap(), Message::Close(_)),
                "no old mutation replay"
            );
            assert!(matches!(
                second.flush(),
                Ok(()) | Err(Error::ConnectionClosed)
            ));
        });
        let endpoint = RuntimeEndpoint::parse(
            &format!("ws://{address}/application/v1"),
            ORIGIN,
            false,
            None,
            None,
        )
        .unwrap();
        let client =
            ClientHandle::spawn_configured(endpoint, false, Some(path.clone()), None).unwrap();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("same-workspace"));
        client.connect(Some("old-scope".into())).unwrap();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            let rejected = matches!(&update, ClientUpdate::Reply {kind:ReplyKind::PublicError,envelope,..} if envelope["code"] == "instance_changed");
            model.apply_client_update(update);
            if rejected {
                break;
            }
        }
        assert!(model.connection_requires_new_scope());
        assert_eq!(model.recovery.quarantined.len(), usize::from(unresolved));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        probe_tx.send(()).unwrap();
        probed_rx.recv_timeout(TIMEOUT).unwrap();
        client.connect(None).unwrap(); // one explicit user action; never retries Busy.
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            let ready = matches!(update, ClientUpdate::Hello(_));
            model.apply_client_update(update);
            if ready {
                break;
            }
        }
        assert_eq!(model.hello.as_ref().unwrap().scope, "new-scope");
        assert_eq!(model.quarantine_blocks_mutations(), unresolved);
        if unresolved {
            assert_eq!(model.recovery.quarantined[0].record, records[0]);
            let command = client.mutation("reference_retune", json!({})).unwrap();
            loop {
                let update = client
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if let ClientUpdate::LocalRejected { command_id, reason } = update {
                    assert_eq!(command_id, command);
                    assert_eq!(reason, "recovery_quarantine_unresolved");
                    break;
                }
            }
        }
        if unresolved {
            assert_eq!(std::fs::read(&path).unwrap(), before);
        } else {
            assert!(
                !path.exists(),
                "empty old-scope journal is retired only after explicit new-scope hello"
            );
        }
        client.shutdown().unwrap();
        peer.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
