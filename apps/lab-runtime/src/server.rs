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
    wire::encode_frame(value).unwrap_or_else(|_| {
        let fallback = rejection(
            value.get("msg_id").and_then(serde_json::Value::as_str),
            "response_too_large",
        );
        wire::encode_frame(&fallback).expect("fixed bounded protocol rejection")
    })
}
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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
        frame: Vec<u8>,
        consumed: bool,
        hello: bool,
    },
    Event {
        connection: u64,
        frame: Vec<u8>,
    },
}

struct Peer {
    stream: TcpStream,
    input: Vec<u8>,
    partial_since: Option<Instant>,
    handshake_since: Instant,
    replied_hello: bool,
    replies: VecDeque<Vec<u8>>,
    events: VecDeque<Vec<u8>>,
    writing: Option<(Vec<u8>, usize, bool)>,
    last_reply: bool,
    last_write: Instant,
    pending: usize,
    pending_ids: BTreeSet<String>,
    rejection: Option<Vec<u8>>,
    closing: bool,
}
impl Peer {
    fn new(stream: TcpStream) -> Self {
        let now = Instant::now();
        Self {
            stream,
            input: Vec::with_capacity(wire::FRAME_LIMIT),
            partial_since: None,
            handshake_since: now,
            replied_hello: false,
            replies: VecDeque::new(),
            events: VecDeque::new(),
            writing: None,
            last_reply: false,
            last_write: now,
            pending: 0,
            pending_ids: BTreeSet::new(),
            rejection: None,
            closing: false,
        }
    }
    fn queued(&self) -> usize {
        self.replies.len()
            + self.events.len()
            + usize::from(self.writing.is_some())
            + usize::from(self.rejection.is_some())
    }
    fn reply_queued(&self) -> usize {
        self.replies.len() + usize::from(self.writing.as_ref().is_some_and(|(_, _, reply)| *reply))
    }
    fn event_queued(&self) -> usize {
        self.events.len() + usize::from(self.writing.as_ref().is_some_and(|(_, _, reply)| !*reply))
    }
    fn read(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> io::Result<bool> {
        if self.pending >= CLIENT_IN {
            return Ok(true);
        }
        if !self.dispatch_buffered(id, to_owner) {
            return Ok(false);
        }
        if self.pending >= CLIENT_IN {
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
            if self.pending >= CLIENT_IN {
                break;
            }
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                break;
            };
            let frame = self.input[..=end].to_vec();
            let request = match wire::decode_frame(&frame) {
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
                    self.rejection = Some(encode_outgoing(&rejection));
                    self.closing = true;
                    return true;
                }
            };
            if self.pending_ids.contains(&request.msg_id) {
                let rejection = rejection(Some(&request.msg_id), "duplicate_msg_id");
                self.rejection = Some(encode_outgoing(&rejection));
                self.closing = true;
                return true;
            }
            let request_msg_id = request.msg_id.clone();
            match to_owner.try_send(Incoming::Request(id, request)) {
                Ok(()) => {
                    self.pending_ids.insert(request_msg_id);
                    self.input.drain(..=end);
                    self.pending += 1;
                    self.partial_since = (!self.input.is_empty()).then(Instant::now);
                }
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
        true
    }
    fn write(&mut self) -> io::Result<bool> {
        if self.pending_ids.is_empty()
            && self.reply_queued() < CLIENT_OUT
            && let Some(rejection) = self.rejection.take()
        {
            self.replies.push_back(rejection);
        }
        let mut budget = SWEEP_BYTES;
        for _ in 0..4 {
            if self.writing.is_none() {
                let pick_event = self.last_reply && !self.events.is_empty();
                self.writing = if pick_event {
                    self.events.pop_front().map(|bytes| (bytes, 0, false))
                } else {
                    self.replies
                        .pop_front()
                        .map(|bytes| (bytes, 0, true))
                        .or_else(|| self.events.pop_front().map(|bytes| (bytes, 0, false)))
                };
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
                        self.last_reply = *reply;
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
        (!self.replied_hello && now.duration_since(self.handshake_since) >= CLIENT_DEADLINE)
            || self
                .partial_since
                .is_some_and(|t| now.duration_since(t) >= CLIENT_DEADLINE)
            || (self.queued() > 0 && now.duration_since(self.last_write) >= CLIENT_DEADLINE)
    }
}

fn reactor(
    listener: TcpListener,
    to_owner: SyncSender<Incoming>,
    from_owner: Receiver<Outgoing>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut peers = BTreeMap::<u64, Peer>::new();
    // A full owner mailbox must never erase a detach. In-flight generations
    // count against the same eight-slot budget until their detach is delivered.
    let mut pending_detach = VecDeque::<u64>::new();
    let mut next_id = 1u64;
    let mut accepting = true;
    while !stop.load(Ordering::Acquire) {
        while let Some(id) = pending_detach.front().copied() {
            match to_owner.try_send(Incoming::Detach(id)) {
                Ok(()) => {
                    pending_detach.pop_front();
                }
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
        }
        for _ in 0..if accepting { 8 } else { 0 } {
            match listener.accept() {
                Ok((stream, _)) => {
                    if peers.len() + pending_detach.len() >= MAX_CLIENTS {
                        drop(stream);
                        continue;
                    }
                    stream.set_nonblocking(true)?;
                    let id = next_id;
                    next_id = next_id
                        .checked_add(1)
                        .ok_or(io::Error::other("connection ID exhausted"))?;
                    peers.insert(id, Peer::new(stream));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        for _ in 0..QUEUE {
            match from_owner.try_recv() {
                Ok(Outgoing::StopAccept) => accepting = false,
                Ok(Outgoing::Reply {
                    connection: id,
                    frame,
                    consumed,
                    hello,
                }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        if peer.reply_queued() >= CLIENT_OUT {
                            peers.remove(&id);
                            pending_detach.push_back(id);
                        } else {
                            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&frame)
                                && let Some(msg_id) = value["msg_id"].as_str()
                                && !(value["type"] == "operation" && value["state"] == "accepted")
                            {
                                peer.pending_ids.remove(msg_id);
                            }
                            if peer.queued() == 0 {
                                peer.last_write = Instant::now();
                            }
                            peer.replied_hello |= hello;
                            peer.replies.push_back(frame);
                            if consumed {
                                peer.pending = peer.pending.saturating_sub(1);
                            }
                        }
                    }
                }
                Ok(Outgoing::Event {
                    connection: id,
                    frame,
                }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        if peer.event_queued() >= CLIENT_EVENTS {
                            peers.remove(&id);
                            pending_detach.push_back(id);
                        } else {
                            if peer.queued() == 0 {
                                peer.last_write = Instant::now();
                            }
                            peer.events.push_back(frame);
                        }
                    }
                }
                Ok(Outgoing::Close { connection: id }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        peer.closing = true;
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
                    && (peer.closing || peer.read(id, &to_owner).unwrap_or(false))
                    && peer.write().unwrap_or(false)
                    && !(peer.closing && peer.queued() == 0)
            } else {
                false
            };
            if !alive {
                peers.remove(&id);
                pending_detach.push_back(id);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    for id in peers.keys().copied() {
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
    let listener = service.listener().try_clone()?;
    listener.set_nonblocking(true)?;
    let (incoming_tx, incoming_rx) = mpsc::sync_channel::<Incoming>(QUEUE);
    let (outgoing_tx, outgoing_rx) = mpsc::sync_channel::<Outgoing>(QUEUE);
    let net_stop = Arc::new(AtomicBool::new(false));
    let net_flag = net_stop.clone();
    let reactor_thread =
        thread::spawn(move || reactor(listener, incoming_tx, outgoing_rx, net_flag));
    let mut app =
        Application::new(service.boot_id()).map_err(|_| io::Error::other("invalid boot"))?;
    let mut queued = BTreeMap::<u64, VecDeque<WireRequest>>::new();
    let mut closing = std::collections::BTreeSet::<u64>::new();
    let mut close_sent = std::collections::BTreeSet::<u64>::new();
    let mut rotation = 0usize;
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
        for (id, value) in app.poll_recording(&mut service) {
            let frame = encode_outgoing(&value);
            if outgoing_tx
                .try_send(Outgoing::Reply {
                    connection: id,
                    frame,
                    consumed: false,
                    hello: false,
                })
                .is_err()
            {
                app.detach(&service, id);
                queued.remove(&id);
                closing.insert(id);
            }
        }
        for (id, value) in app.poll_history(&mut service) {
            let frame = encode_outgoing(&value);
            if outgoing_tx
                .try_send(Outgoing::Reply {
                    connection: id,
                    frame,
                    consumed: false,
                    hello: false,
                })
                .is_err()
            {
                app.detach(&service, id);
                queued.remove(&id);
                closing.insert(id);
            }
        }
        for _ in 0..16 {
            match incoming_rx.try_recv() {
                Ok(Incoming::Request(id, req)) => {
                    if closing.contains(&id) {
                        continue;
                    }
                    let q = queued.entry(id).or_default();
                    if q.len() < CLIENT_IN {
                        q.push_back(req);
                    }
                }
                Ok(Incoming::Detach(id)) => {
                    queued.remove(&id);
                    closing.remove(&id);
                    close_sent.remove(&id);
                    app.detach(&service, id);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    service.request_fatal_shutdown();
                    break;
                }
            }
        }
        let ids: Vec<_> = queued.keys().copied().collect();
        if !ids.is_empty() {
            for n in 0..ids.len().min(4) {
                let id = ids[(rotation + n) % ids.len()];
                let Some(req) = queued.get_mut(&id).and_then(VecDeque::pop_front) else {
                    continue;
                };
                let hello = req.op == "hello";
                for (index, value) in app.handle(&mut service, id, req).into_iter().enumerate() {
                    let frame = encode_outgoing(&value);
                    let hello = hello && value["type"] == "result";
                    if outgoing_tx
                        .try_send(Outgoing::Reply {
                            connection: id,
                            frame,
                            consumed: index == 0,
                            hello,
                        })
                        .is_err()
                    {
                        app.detach(&service, id);
                        queued.remove(&id);
                        closing.insert(id);
                        break;
                    }
                }
            }
            rotation = (rotation + 1) % ids.len();
        }
        for id in queued.keys().copied() {
            for event in app.pump_events(&service, id) {
                let gap = event["code"] == "event_gap";
                if gap {
                    app.detach(&service, id);
                    closing.insert(id);
                }
                let frame = encode_outgoing(&event);
                if outgoing_tx
                    .try_send(Outgoing::Event {
                        connection: id,
                        frame,
                    })
                    .is_err()
                {
                    app.detach(&service, id);
                    closing.insert(id);
                    break;
                }
                if gap {
                    break;
                }
            }
        }
        for id in closing.iter().copied().collect::<Vec<_>>() {
            if !close_sent.contains(&id)
                && outgoing_tx
                    .try_send(Outgoing::Close { connection: id })
                    .is_ok()
            {
                queued.remove(&id);
                close_sent.insert(id);
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
                    let frame = encode_outgoing(&value);
                    let _ = outgoing_tx.try_send(Outgoing::Reply {
                        connection: id,
                        frame,
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
                    return Ok(());
                } else {
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
    use std::io::{BufRead, BufReader};

    fn peer() -> (Peer, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_nonblocking(true).unwrap();
        (Peer::new(server), client)
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
        peer.replied_hello = true;
        peer.partial_since = Some(past);
        assert!(peer.timed_out());
        peer.partial_since = None;
        peer.replies.push_back(vec![b'X'; wire::FRAME_LIMIT]);
        peer.last_write = past;
        assert!(peer.timed_out());
    }

    #[test]
    fn one_sweep_partial_write_resumes_at_the_exact_byte_offset() {
        let (mut peer, mut client) = peer();
        let frame = vec![b'x'; wire::FRAME_LIMIT];
        peer.replies.push_back(frame.clone());
        assert!(peer.write().unwrap());
        assert_eq!(peer.writing.as_ref().unwrap().1, SWEEP_BYTES);
        assert!(peer.write().unwrap());
        assert!(peer.writing.is_none());
        let mut received = vec![0u8; wire::FRAME_LIMIT];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.read_exact(&mut received).unwrap();
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
        let join = thread::spawn(move || reactor(listener, to_owner, from_owner, flag).unwrap());
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let gap = wire::encode_frame(&serde_json::json!({"v":1,"type":"error","code":"event_gap"}))
            .unwrap();
        to_net
            .send(Outgoing::Event {
                connection: 1,
                frame: gap,
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
        let join = thread::spawn(move || reactor(listener, to_owner, from_owner, flag).unwrap());
        let nonreader = TcpStream::connect(addr).unwrap();
        for _ in 0..=CLIENT_EVENTS {
            to_net
                .send(Outgoing::Event {
                    connection: 1,
                    frame: vec![b'x'; wire::FRAME_LIMIT],
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
        let stale =
            wire::encode_frame(&serde_json::json!({"v":1,"type":"result","msg_id":"stale"}))
                .unwrap();
        let live = wire::encode_frame(&serde_json::json!({"v":1,"type":"result","msg_id":"live"}))
            .unwrap();
        to_net
            .send(Outgoing::Reply {
                connection: 1,
                frame: stale,
                consumed: false,
                hello: false,
            })
            .unwrap();
        to_net
            .send(Outgoing::Reply {
                connection: 2,
                frame: live,
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
    fn full_owner_mailbox_cannot_drop_detach_or_reuse_its_connection_generation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_owner, from_net) = mpsc::sync_channel(1);
        let probe = to_owner.clone();
        let (to_net, from_owner) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || reactor(listener, to_owner, from_owner, flag).unwrap());
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
                    frame: vec![b'x'; 1024],
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
        let frame = encode_outgoing(&value);
        assert!(frame.len() <= wire::FRAME_LIMIT);
        let response: serde_json::Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(response["msg_id"], "large-result");
        assert_eq!(response["code"], "response_too_large");
        assert_eq!(response["category"], "protocol_error");
        assert_eq!(response["accepted"], false);
    }
}
