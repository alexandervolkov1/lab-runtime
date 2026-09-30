//! One Runtime owner and one bounded nonblocking loopback reactor.
//!
//! The reactor owns sockets and never borrows Runtime. The owner services safety
//! before client requests; full mailboxes detach peers rather than wait on I/O.

use crate::{
    application::Application,
    protocol::{self, PublicError},
    service::ServiceHost,
    wire::{self, WireRequest},
};

mod coordination;
mod websocket_peer;

use coordination::{AdmissionError, ClientDelivery, ConnectionCoordinator, OwnerDelivery};
use websocket_peer::WebSocketPeer;

fn rejection(msg_id: Option<&str>, code: &str) -> serde_json::Value {
    let mut value = serde_json::json!({
        "v":protocol::PROTOCOL_VERSION,
        "type":"error",
        "accepted":false,
        "msg_id":msg_id
    });
    PublicError::from_internal_code(code).apply_to(&mut value);
    value
}

fn encode_outgoing(value: &serde_json::Value) -> Vec<u8> {
    wire::encode_application_json(value).unwrap_or_else(|_| {
        let fallback = rejection(
            value.get("msg_id").and_then(serde_json::Value::as_str),
            "response_too_large",
        );
        wire::encode_application_json(&fallback).expect("fixed bounded protocol rejection")
    })
}
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

/// Simultaneous loopback client bound.
pub(crate) const MAX_CLIENTS: usize = 8;
/// Fixed owner/reactor mailbox capacity in each direction.
pub(crate) const QUEUE: usize = 64;
/// Per-client requests admitted but not yet answered.
pub(crate) const CLIENT_IN: usize = 8;
/// Per-client reply frames awaiting socket delivery.
pub(crate) const CLIENT_OUT: usize = 8;
/// Per-client event frames awaiting socket delivery.
pub(crate) const CLIENT_EVENTS: usize = 16;
/// Byte budget handled by one nonblocking network pass.
pub(crate) const SWEEP_BYTES: usize = 8192;
/// Absolute hello, partial-frame, and blocked-write deadline.
pub(crate) const CLIENT_DEADLINE: Duration = Duration::from_secs(2);

enum Incoming {
    Request(u64, WireRequest),
    Detach(u64),
}
enum Outgoing {
    StopAccept,
    Close {
        connection: u64,
    },
    Reply {
        connection: u64,
        message: Vec<u8>,
        consumed: bool,
        hello: bool,
    },
    Event {
        connection: u64,
        message: Vec<u8>,
    },
}

struct Peer {
    stream: TcpStream,
    input: Vec<u8>,
    partial_since: Option<Instant>,
    handshake_since: Instant,
    delivery: ClientDelivery,
    writing: Option<(Vec<u8>, usize, bool)>,
    last_write: Instant,
}
impl Peer {
    fn new(stream: TcpStream) -> Self {
        let now = Instant::now();
        Self {
            stream,
            input: Vec::with_capacity(wire::FRAME_LIMIT),
            partial_since: None,
            handshake_since: now,
            delivery: ClientDelivery::new(),
            writing: None,
            last_write: now,
        }
    }
    fn queued(&self) -> usize {
        self.delivery
            .queued(self.writing.as_ref().map(|(_, _, reply)| *reply))
    }
    fn read(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> io::Result<bool> {
        if self.delivery.pending_full() {
            return Ok(true);
        }
        if !self.dispatch_buffered(id, to_owner) {
            return Ok(false);
        }
        if self.delivery.pending_full() {
            return Ok(true);
        }
        let mut scratch = [0u8; SWEEP_BYTES];
        let remaining = wire::FRAME_LIMIT.saturating_sub(self.input.len());
        if remaining == 0 {
            // A full buffered frame has already been dispatched, or admission
            // is temporarily backpressured; never read into an empty slice.
            return Ok(self.input.contains(&b'\n'));
        }
        match self.stream.read(&mut scratch[..remaining.min(SWEEP_BYTES)]) {
            Ok(0) => return Ok(false),
            Ok(count) => {
                if self.input.is_empty() {
                    self.partial_since = Some(Instant::now());
                }
                if self.input.len() + count > wire::FRAME_LIMIT {
                    tracing::warn!(
                        event = "client_frame_oversized",
                        connection = id,
                        limit = wire::FRAME_LIMIT,
                        "closing client with oversized request frame"
                    );
                    return Ok(false);
                }
                self.input.extend_from_slice(&scratch[..count]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        Ok(self.dispatch_buffered(id, to_owner))
    }
    fn dispatch_buffered(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> bool {
        for _ in 0..4 {
            if self.delivery.pending_full() {
                break;
            }
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                break;
            };
            let frame = self.input[..=end].to_vec();
            let request = match wire::decode_ndjson_frame(&frame) {
                Ok(r) => r,
                Err(error) => {
                    // A complete malformed exchange can receive one bounded
                    // best-effort rejection. It never reaches the Runtime owner.
                    let correlation = serde_json::from_slice::<serde_json::Value>(&frame)
                        .ok()
                        .and_then(|value| {
                            value["msg_id"]
                                .as_str()
                                .filter(|id| !id.is_empty() && id.len() <= 64)
                                .map(str::to_owned)
                        });
                    let rejection = rejection(correlation.as_deref(), error.code);
                    self.delivery.reject_and_close(encode_outgoing(&rejection));
                    tracing::warn!(
                        event = "client_request_malformed",
                        connection = id,
                        code = error.code,
                        frame_bytes = frame.len(),
                        "closing client after bounded malformed request"
                    );
                    return true;
                }
            };
            if self.delivery.request_is_in_flight(&request.msg_id) {
                let rejection = rejection(Some(&request.msg_id), "duplicate_msg_id");
                self.delivery.reject_and_close(encode_outgoing(&rejection));
                return true;
            }
            let request_msg_id = request.msg_id.clone();
            match to_owner.try_send(Incoming::Request(id, request)) {
                Ok(()) => {
                    self.delivery.request_admitted(request_msg_id);
                    self.input.drain(..=end);
                    self.partial_since = (!self.input.is_empty()).then(Instant::now);
                }
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
        true
    }
    fn write(&mut self) -> io::Result<bool> {
        let writing = self.writing.as_ref().map(|(_, _, reply)| *reply);
        self.delivery.stage_rejection(writing);
        let mut budget = SWEEP_BYTES;
        for _ in 0..4 {
            if self.writing.is_none() {
                self.writing = self.delivery.next_message().map(|(mut body, reply)| {
                    // LF framing belongs exclusively to the TCP/NDJSON adapter.
                    body.push(b'\n');
                    (body, 0, reply)
                });
            }
            let Some((bytes, offset, reply)) = &mut self.writing else {
                break;
            };
            if budget == 0 {
                break;
            }
            match self
                .stream
                .write(&bytes[*offset..bytes.len().min(*offset + budget)])
            {
                Ok(0) => return Ok(false),
                Ok(count) => {
                    *offset += count;
                    budget -= count;
                    self.last_write = Instant::now();
                    if *offset == bytes.len() {
                        self.delivery.message_written(*reply);
                        self.writing = None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
    fn timed_out(&self) -> bool {
        let now = Instant::now();
        (!self.delivery.replied_hello()
            && now.duration_since(self.handshake_since) >= CLIENT_DEADLINE)
            || self
                .partial_since
                .is_some_and(|t| now.duration_since(t) >= CLIENT_DEADLINE)
            || (self.queued() > 0 && now.duration_since(self.last_write) >= CLIENT_DEADLINE)
    }
}

fn reactor(
    listener: TcpListener,
    websocket_listener: Option<TcpListener>,
    websocket_origins: Vec<String>,
    to_owner: SyncSender<Incoming>,
    from_owner: Receiver<Outgoing>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut peers = std::collections::BTreeMap::<u64, Peer>::new();
    let mut websocket_peers = std::collections::BTreeMap::<u64, WebSocketPeer>::new();
    let websocket_host = websocket_listener
        .as_ref()
        .map(TcpListener::local_addr)
        .transpose()?
        .map(|address| format!("127.0.0.1:{}", address.port()));
    // A full owner mailbox must never erase a detach. In-flight generations
    // count against the same eight-slot budget until their detach is delivered.
    let mut connections = ConnectionCoordinator::new();
    let mut accepting = true;
    while !stop.load(Ordering::Acquire) {
        while let Some(id) = connections.pending_detach() {
            match to_owner.try_send(Incoming::Detach(id)) {
                Ok(()) => {
                    connections.detach_delivered(id);
                }
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
        }
        let tcp_accept_budget = if websocket_listener.is_some() { 4 } else { 8 };
        for _ in 0..if accepting { tcp_accept_budget } else { 0 } {
            match listener.accept() {
                Ok((stream, _)) => {
                    if connections.capacity_full() {
                        tracing::warn!(
                            event = "client_capacity_exhausted",
                            limit = MAX_CLIENTS,
                            "dropping client at process capacity"
                        );
                        drop(stream);
                        continue;
                    }
                    stream.set_nonblocking(true)?;
                    let id = match connections.admit() {
                        Ok(id) => id,
                        Err(AdmissionError::Capacity) => {
                            drop(stream);
                            continue;
                        }
                        Err(AdmissionError::Exhausted) => {
                            return Err(io::Error::other("connection ID exhausted"));
                        }
                    };
                    peers.insert(id, Peer::new(stream));
                    tracing::debug!(
                        event = "client_accepted",
                        connection = id,
                        active_clients = connections.active_count(),
                        "loopback client accepted"
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        if accepting
            && let (Some(websocket_listener), Some(websocket_host)) =
                (&websocket_listener, &websocket_host)
        {
            for _ in 0..4 {
                match websocket_listener.accept() {
                    Ok((stream, _)) => {
                        if connections.capacity_full() {
                            tracing::warn!(
                                event = "client_capacity_exhausted",
                                limit = MAX_CLIENTS,
                                "dropping WebSocket client at process capacity"
                            );
                            drop(stream);
                            continue;
                        }
                        stream.set_nonblocking(true)?;
                        let id = match connections.admit() {
                            Ok(id) => id,
                            Err(AdmissionError::Capacity) => {
                                drop(stream);
                                continue;
                            }
                            Err(AdmissionError::Exhausted) => {
                                return Err(io::Error::other("connection ID exhausted"));
                            }
                        };
                        websocket_peers.insert(
                            id,
                            WebSocketPeer::new(
                                stream,
                                websocket_host.clone(),
                                websocket_origins.clone(),
                            ),
                        );
                        tracing::debug!(
                            event = "client_accepted",
                            connection = id,
                            active_clients = connections.active_count(),
                            transport = "websocket",
                            "loopback client accepted"
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
            }
        }
        for _ in 0..QUEUE {
            match from_owner.try_recv() {
                Ok(Outgoing::StopAccept) => accepting = false,
                Ok(Outgoing::Reply {
                    connection: id,
                    message,
                    consumed,
                    hello,
                }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        let was_idle = peer.queued() == 0;
                        let writing = peer.writing.as_ref().map(|(_, _, reply)| *reply);
                        if !peer.delivery.push_reply(message, consumed, hello, writing) {
                            tracing::warn!(
                                event = "client_reply_backpressure",
                                connection = id,
                                limit = CLIENT_OUT,
                                "dropping client with full reply queue"
                            );
                            peers.remove(&id);
                            let first = connections.begin_detach(id);
                            debug_assert!(first);
                        } else {
                            if was_idle {
                                peer.last_write = Instant::now();
                            }
                        }
                    } else if let Some(peer) = websocket_peers.get_mut(&id)
                        && !peer.push_reply(message, consumed, hello)
                    {
                        tracing::warn!(
                            event = "client_reply_backpressure",
                            connection = id,
                            limit = CLIENT_OUT,
                            transport = "websocket",
                            "dropping client with full reply queue"
                        );
                        websocket_peers.remove(&id);
                        let first = connections.begin_detach(id);
                        debug_assert!(first);
                    }
                }
                Ok(Outgoing::Event {
                    connection: id,
                    message,
                }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        let was_idle = peer.queued() == 0;
                        let writing = peer.writing.as_ref().map(|(_, _, reply)| *reply);
                        if !peer.delivery.push_event(message, writing) {
                            tracing::warn!(
                                event = "client_event_backpressure",
                                connection = id,
                                limit = CLIENT_EVENTS,
                                "dropping client with full event queue"
                            );
                            peers.remove(&id);
                            let first = connections.begin_detach(id);
                            debug_assert!(first);
                        } else {
                            if was_idle {
                                peer.last_write = Instant::now();
                            }
                        }
                    } else if let Some(peer) = websocket_peers.get_mut(&id)
                        && !peer.push_event(message)
                    {
                        tracing::warn!(
                            event = "client_event_backpressure",
                            connection = id,
                            limit = CLIENT_EVENTS,
                            transport = "websocket",
                            "dropping client with full event queue"
                        );
                        websocket_peers.remove(&id);
                        let first = connections.begin_detach(id);
                        debug_assert!(first);
                    }
                }
                Ok(Outgoing::Close { connection: id }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        peer.delivery.close();
                    } else if let Some(peer) = websocket_peers.get_mut(&id) {
                        peer.close();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        let ids: Vec<_> = peers.keys().copied().collect();
        for id in ids {
            let alive = if let Some(peer) = peers.get_mut(&id) {
                !peer.timed_out()
                    && (peer.delivery.closing() || peer.read(id, &to_owner).unwrap_or(false))
                    && peer.write().unwrap_or(false)
                    && !(peer.delivery.closing() && peer.queued() == 0)
            } else {
                false
            };
            if !alive {
                tracing::debug!(
                    event = "client_detached",
                    connection = id,
                    "loopback client detached"
                );
                peers.remove(&id);
                let first = connections.begin_detach(id);
                debug_assert!(first);
            }
        }
        let websocket_ids: Vec<_> = websocket_peers.keys().copied().collect();
        for id in websocket_ids {
            let alive = websocket_peers
                .get_mut(&id)
                .is_some_and(|peer| peer.service(id, &to_owner));
            if !alive {
                tracing::debug!(
                    event = "client_detached",
                    connection = id,
                    transport = "websocket",
                    "loopback client detached"
                );
                websocket_peers.remove(&id);
                let first = connections.begin_detach(id);
                debug_assert!(first);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    for id in connections.active_ids() {
        let _ = to_owner.try_send(Incoming::Detach(id));
    }
    Ok(())
}

/// Run one service owner until an external stop or accepted host shutdown.
/// Socket work is isolated on one fixed reactor thread and never blocks safety.
pub fn run(
    mut service: ServiceHost,
    stop: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        event = "application_server_start",
        address = %service.bound_address(),
        max_clients = MAX_CLIENTS,
        "Application server starting"
    );
    let listener = service.listener().try_clone()?;
    listener.set_nonblocking(true)?;
    let websocket_listener = service
        .websocket_listener()
        .map(TcpListener::try_clone)
        .transpose()?;
    if let Some(listener) = &websocket_listener {
        listener.set_nonblocking(true)?;
    }
    let websocket_origins = service.websocket_allowed_origins().to_vec();
    let (incoming_tx, incoming_rx) = mpsc::sync_channel::<Incoming>(QUEUE);
    let (outgoing_tx, outgoing_rx) = mpsc::sync_channel::<Outgoing>(QUEUE);
    let net_stop = Arc::new(AtomicBool::new(false));
    let net_flag = net_stop.clone();
    let reactor_thread = thread::spawn(move || {
        reactor(
            listener,
            websocket_listener,
            websocket_origins,
            incoming_tx,
            outgoing_rx,
            net_flag,
        )
    });
    let mut app =
        Application::new(service.boot_id()).map_err(|_| io::Error::other("invalid boot"))?;
    let mut delivery = OwnerDelivery::new();
    let mut terminal_since: Option<Instant> = None;
    let mut accept_stop_sent = false;
    loop {
        if stop.load(Ordering::Acquire) && service.request_shutdown().is_err() {
            service.request_fatal_shutdown();
        }
        let clock = service.clock_copy();
        if service.owner_mut().service(&clock).is_err() {
            service.request_fatal_shutdown();
        }
        for (id, value) in app.poll_configuration(&mut service) {
            let message = encode_outgoing(&value);
            if outgoing_tx
                .try_send(Outgoing::Reply {
                    connection: id,
                    message,
                    consumed: false,
                    hello: false,
                })
                .is_err()
                && delivery.begin_close(id)
            {
                app.detach(&service, id);
            }
        }
        for (id, value) in app.poll_recording(&mut service) {
            let message = encode_outgoing(&value);
            if outgoing_tx
                .try_send(Outgoing::Reply {
                    connection: id,
                    message,
                    consumed: false,
                    hello: false,
                })
                .is_err()
                && delivery.begin_close(id)
            {
                app.detach(&service, id);
            }
        }
        for (id, value) in app.poll_history(&mut service) {
            let message = encode_outgoing(&value);
            if outgoing_tx
                .try_send(Outgoing::Reply {
                    connection: id,
                    message,
                    consumed: false,
                    hello: false,
                })
                .is_err()
                && delivery.begin_close(id)
            {
                app.detach(&service, id);
            }
        }
        for _ in 0..16 {
            match incoming_rx.try_recv() {
                Ok(Incoming::Request(id, req)) => {
                    delivery.admit_request(id, req);
                }
                Ok(Incoming::Detach(id)) => {
                    if delivery.network_detached(id) {
                        app.detach(&service, id);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    tracing::error!(
                        event = "network_owner_channel_closed",
                        "network reactor owner channel closed unexpectedly"
                    );
                    service.request_fatal_shutdown();
                    break;
                }
            }
        }
        for (id, request) in delivery.next_requests() {
            let hello = request.op == "hello";
            for (index, value) in app
                .handle(&mut service, id, request)
                .into_iter()
                .enumerate()
            {
                let message = encode_outgoing(&value);
                let hello = hello && value["type"] == "result";
                if outgoing_tx
                    .try_send(Outgoing::Reply {
                        connection: id,
                        message,
                        consumed: index == 0,
                        hello,
                    })
                    .is_err()
                {
                    if delivery.begin_close(id) {
                        app.detach(&service, id);
                    }
                    break;
                }
            }
        }
        for id in delivery.connection_ids() {
            for event in app.pump_events(&service, id) {
                let gap = event["code"] == "event_gap";
                if gap && delivery.begin_close(id) {
                    app.detach(&service, id);
                }
                let message = encode_outgoing(&event);
                if outgoing_tx
                    .try_send(Outgoing::Event {
                        connection: id,
                        message,
                    })
                    .is_err()
                {
                    if delivery.begin_close(id) {
                        app.detach(&service, id);
                    }
                    break;
                }
                if gap {
                    break;
                }
            }
        }
        for id in delivery.unsent_closes() {
            if outgoing_tx
                .try_send(Outgoing::Close { connection: id })
                .is_ok()
            {
                delivery.close_sent(id);
            }
        }
        if service.is_stopping()
            && !accept_stop_sent
            && outgoing_tx.try_send(Outgoing::StopAccept).is_ok()
        {
            accept_stop_sent = true;
        }
        if let Some(status) = service.shutdown_step()? {
            if terminal_since.is_none() {
                for (id, value) in app.finish_shutdown(&mut service, status) {
                    let message = encode_outgoing(&value);
                    let _ = outgoing_tx.try_send(Outgoing::Reply {
                        connection: id,
                        message,
                        consumed: false,
                        hello: false,
                    });
                }
                terminal_since = Some(Instant::now());
            }
            if terminal_since.is_some_and(|at| at.elapsed() >= Duration::from_millis(200)) {
                net_stop.store(true, Ordering::Release);
                let net = reactor_thread
                    .join()
                    .map_err(|_| io::Error::other("network reactor panicked"))?;
                net?;
                if status.exit_success {
                    tracing::info!(
                        event = "application_server_shutdown",
                        safe_confirmed = status.safe_confirmed,
                        recorder_flushed = status.recorder_flushed,
                        "Application server stopped cleanly"
                    );
                    return Ok(());
                } else {
                    tracing::error!(
                        event = "application_server_shutdown_incomplete",
                        safe_confirmed = status.safe_confirmed,
                        unfinished_workers = status.unfinished_workers,
                        unfinished_transports = status.unfinished_transports,
                        recorder_flushed = status.recorder_flushed,
                        recorder_error = status.recorder_error,
                        "Application server shutdown incomplete"
                    );
                    return Err(io::Error::other("safe shutdown incomplete").into());
                }
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod bounded_peer_tests {
    use super::*;
    use std::{
        collections::VecDeque,
        io::{BufRead, BufReader},
    };

    fn peer() -> (Peer, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_nonblocking(true).unwrap();
        (Peer::new(server), client)
    }

    #[test]
    fn tcp_and_websocket_connections_draw_from_one_id_space() {
        use std::collections::BTreeSet;
        use tungstenite::{ClientRequestBuilder, Message, client, http::Uri};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let websocket_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        websocket_listener.set_nonblocking(true).unwrap();
        let websocket_address = websocket_listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(8);
        let (_to_net, from_owner) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(
                listener,
                Some(websocket_listener),
                vec!["http://127.0.0.1:3000".to_owned()],
                to_owner,
                from_owner,
                flag,
            )
            .unwrap()
        });

        let mut tcp = TcpStream::connect(tcp_address).unwrap();
        tcp.write_all(b"{\"v\":1,\"msg_id\":\"tcp\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
            .unwrap();
        let stream = TcpStream::connect(websocket_address).unwrap();
        let uri: Uri = format!(
            "ws://{websocket_address}{}",
            crate::websocket::APPLICATION_PATH
        )
        .parse()
        .unwrap();
        let request = ClientRequestBuilder::new(uri)
            .with_header("Origin", "http://127.0.0.1:3000")
            .with_sub_protocol(crate::websocket::APPLICATION_SUBPROTOCOL);
        let (mut websocket, _) = client(request, stream).unwrap();
        websocket
            .write(Message::Text(
                r#"{"v":1,"msg_id":"ws","op":"hello","args":{"scope":null}}"#.into(),
            ))
            .unwrap();
        websocket.flush().unwrap();

        let mut identities = BTreeSet::new();
        while identities.len() != 2 {
            if let Incoming::Request(id, _) = from_net.recv_timeout(Duration::from_secs(2)).unwrap()
            {
                identities.insert(id);
            }
        }
        assert_eq!(identities, BTreeSet::from([1, 2]));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn websocket_retains_completed_request_while_owner_mailbox_is_full() {
        use tungstenite::{ClientRequestBuilder, Message, client, http::Uri};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let websocket_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        websocket_listener.set_nonblocking(true).unwrap();
        let websocket_address = websocket_listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(1);
        let (_to_net, from_owner) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(
                listener,
                Some(websocket_listener),
                vec!["http://127.0.0.1:3000".to_owned()],
                to_owner,
                from_owner,
                flag,
            )
            .unwrap()
        });

        let stream = TcpStream::connect(websocket_address).unwrap();
        let uri: Uri = format!(
            "ws://{websocket_address}{}",
            crate::websocket::APPLICATION_PATH
        )
        .parse()
        .unwrap();
        let request = ClientRequestBuilder::new(uri)
            .with_header("Origin", "http://127.0.0.1:3000")
            .with_sub_protocol(crate::websocket::APPLICATION_SUBPROTOCOL);
        let (mut websocket, _) = client(request, stream).unwrap();
        websocket
            .write(Message::Text(
                r#"{"v":1,"msg_id":"first","op":"hello","args":{"scope":null}}"#.into(),
            ))
            .unwrap();
        websocket
            .write(Message::Text(
                r#"{"v":1,"msg_id":"second","op":"hello","args":{"scope":null}}"#.into(),
            ))
            .unwrap();
        websocket.flush().unwrap();

        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(2)).unwrap(),
            Incoming::Request(1, request) if request.msg_id == "first"
        ));
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(2)).unwrap(),
            Incoming::Request(1, request) if request.msg_id == "second"
        ));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn nonreading_websocket_event_queue_overflow_detaches_only_that_connection() {
        use tungstenite::{ClientRequestBuilder, client, http::Uri};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let websocket_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        websocket_listener.set_nonblocking(true).unwrap();
        let websocket_address = websocket_listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(4);
        let (to_net, from_owner) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(
                listener,
                Some(websocket_listener),
                vec!["http://127.0.0.1:3000".to_owned()],
                to_owner,
                from_owner,
                flag,
            )
            .unwrap()
        });
        let stream = TcpStream::connect(websocket_address).unwrap();
        let uri: Uri = format!(
            "ws://{websocket_address}{}",
            crate::websocket::APPLICATION_PATH
        )
        .parse()
        .unwrap();
        let request = ClientRequestBuilder::new(uri)
            .with_header("Origin", "http://127.0.0.1:3000")
            .with_sub_protocol(crate::websocket::APPLICATION_SUBPROTOCOL);
        let (mut websocket, _) = client(request, stream).unwrap();
        websocket
            .write(tungstenite::Message::Text(
                r#"{"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}"#.into(),
            ))
            .unwrap();
        websocket.flush().unwrap();
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Request(1, request) if request.op == "hello"
        ));
        for _ in 0..=CLIENT_EVENTS {
            to_net
                .send(Outgoing::Event {
                    connection: 1,
                    message: vec![b'x'; wire::APPLICATION_JSON_LIMIT],
                })
                .unwrap();
        }
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Detach(1)
        ));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn a_complete_frame_at_the_exact_input_cap_is_dispatched_before_further_read() {
        let (mut peer, client) = peer();
        let mut frame =
            b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n".to_vec();
        frame.splice(0..0, vec![b' '; wire::FRAME_LIMIT - frame.len()]);
        assert_eq!(frame.len(), wire::FRAME_LIMIT);
        peer.input = frame;
        drop(client);
        let (tx, rx) = mpsc::sync_channel(1);
        let _ = peer.read(9, &tx).unwrap();
        assert!(
            matches!(rx.try_recv().unwrap(), Incoming::Request(9, request) if request.op == "hello")
        );
    }

    #[test]
    fn fixed_first_byte_and_unsent_write_deadlines_close_tricklers() {
        let (mut peer, _client) = peer();
        let past = Instant::now() - Duration::from_millis(2100);
        peer.handshake_since = past;
        assert!(peer.timed_out());
        assert!(peer.delivery.push_reply(Vec::new(), false, true, None));
        peer.partial_since = Some(past);
        assert!(peer.timed_out());
        peer.partial_since = None;
        peer.last_write = past;
        assert!(peer.timed_out());
    }

    #[test]
    fn one_sweep_partial_write_resumes_at_the_exact_byte_offset() {
        let (mut peer, mut client) = peer();
        let body = vec![b'x'; wire::APPLICATION_JSON_LIMIT];
        assert!(peer.delivery.push_reply(body.clone(), false, false, None));
        assert!(peer.write().unwrap());
        assert_eq!(peer.writing.as_ref().unwrap().1, SWEEP_BYTES);
        assert!(peer.write().unwrap());
        assert!(peer.writing.is_none());
        let mut received = vec![0u8; wire::FRAME_LIMIT];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.read_exact(&mut received).unwrap();
        let mut frame = body;
        frame.push(b'\n');
        assert_eq!(received, frame);
    }

    #[test]
    fn replay_gap_is_offered_then_affected_socket_detaches_without_affecting_owner() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(4);
        let (to_net, from_owner) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(listener, None, Vec::new(), to_owner, from_owner, flag).unwrap()
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        // TCP connect completion does not prove the nonblocking reactor has assigned
        // connection 1. Observing its request is the authoritative admission barrier.
        stream
            .write_all(b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
            .unwrap();
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Request(1, request) if request.op == "hello"
        ));
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let gap = wire::encode_application_json(
            &serde_json::json!({"v":1,"type":"error","code":"event_gap"}),
        )
        .unwrap();
        to_net
            .send(Outgoing::Event {
                connection: 1,
                message: gap,
            })
            .unwrap();
        to_net.send(Outgoing::Close { connection: 1 }).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("event_gap"));
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Detach(1)
        ));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn nonreading_event_flood_detaches_and_stale_slot_frames_cannot_reach_reused_capacity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(4);
        let (to_net, from_owner) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(listener, None, Vec::new(), to_owner, from_owner, flag).unwrap()
        });
        let mut nonreader = TcpStream::connect(addr).unwrap();
        // Without this barrier an early synthetic event is correctly discarded as
        // stale, and the test can then wait forever for a detach it never caused.
        nonreader
            .write_all(b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
            .unwrap();
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Request(1, request) if request.op == "hello"
        ));
        for _ in 0..=CLIENT_EVENTS {
            to_net
                .send(Outgoing::Event {
                    connection: 1,
                    message: vec![b'x'; wire::APPLICATION_JSON_LIMIT],
                })
                .unwrap();
        }
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Detach(1)
        ));
        drop(nonreader);
        let second = TcpStream::connect(addr).unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(second);
        let stale = wire::encode_application_json(
            &serde_json::json!({"v":1,"type":"result","msg_id":"stale"}),
        )
        .unwrap();
        let live = wire::encode_application_json(
            &serde_json::json!({"v":1,"type":"result","msg_id":"live"}),
        )
        .unwrap();
        to_net
            .send(Outgoing::Reply {
                connection: 1,
                message: stale,
                consumed: false,
                hello: false,
            })
            .unwrap();
        to_net
            .send(Outgoing::Reply {
                connection: 2,
                message: live,
                consumed: false,
                hello: false,
            })
            .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("live"));
        assert!(!line.contains("stale"));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn eight_client_admission_and_repeated_disconnect_release_exact_reactor_capacity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(64);
        let (_to_net, from_owner) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(listener, None, Vec::new(), to_owner, from_owner, flag).unwrap()
        });
        let hello = b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n";
        let mut admitted = VecDeque::new();
        for expected in 1..=MAX_CLIENTS as u64 {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(hello).unwrap();
            assert!(matches!(
                from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
                Incoming::Request(id, request) if id == expected && request.op == "hello"
            ));
            admitted.push_back((expected, stream));
        }

        let mut refused = TcpStream::connect(addr).unwrap();
        refused
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        refused.write_all(hello).unwrap();
        let mut byte = [0u8; 1];
        match refused.read(&mut byte) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                ) => {}
            outcome => panic!("capacity-refused socket remained usable: {outcome:?}"),
        }

        for expected in (MAX_CLIENTS as u64 + 1)..=(MAX_CLIENTS as u64 + 16) {
            let (old_id, old) = admitted.pop_front().unwrap();
            drop(old);
            assert!(matches!(
                from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
                Incoming::Detach(id) if id == old_id
            ));
            let mut replacement = TcpStream::connect(addr).unwrap();
            replacement.write_all(hello).unwrap();
            assert!(matches!(
                from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
                Incoming::Request(id, request) if id == expected && request.op == "hello"
            ));
            admitted.push_back((expected, replacement));
        }
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn full_owner_mailbox_cannot_drop_detach_or_reuse_its_connection_generation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(1);
        let probe = to_owner.clone();
        let (to_net, from_owner) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            reactor(listener, None, Vec::new(), to_owner, from_owner, flag).unwrap()
        });
        let mut peer = TcpStream::connect(addr).unwrap();
        peer.write_all(b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n")
            .unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while probe.try_send(Incoming::Detach(999)).is_ok() {
            let _ = from_net.try_recv();
            assert!(Instant::now() < until);
            thread::yield_now();
        }
        for _ in 0..=CLIENT_EVENTS {
            to_net
                .send(Outgoing::Event {
                    connection: 1,
                    message: vec![b'x'; 1024],
                })
                .unwrap();
        }
        thread::sleep(Duration::from_millis(40));
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Request(1, _)
        ));
        assert!(matches!(
            from_net.recv_timeout(Duration::from_secs(1)).unwrap(),
            Incoming::Detach(1)
        ));
        stop.store(true, Ordering::Release);
        join.join().unwrap();
    }

    #[test]
    fn oversized_application_result_becomes_one_bounded_correlated_rejection() {
        let value = serde_json::json!({
            "v":1,
            "msg_id":"large-result",
            "type":"result",
            "result":{"records":vec!["x".repeat(512); 40]}
        });
        let message = encode_outgoing(&value);
        assert!(message.len() <= wire::APPLICATION_JSON_LIMIT);
        let response: serde_json::Value = serde_json::from_slice(&message).unwrap();
        assert_eq!(response["msg_id"], "large-result");
        assert_eq!(response["code"], "response_too_large");
        assert_eq!(response["category"], "protocol_error");
        assert_eq!(response["accepted"], false);
    }
}
