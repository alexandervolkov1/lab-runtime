//! One Runtime owner and one bounded nonblocking loopback reactor.
//!
//! The reactor owns sockets and never borrows Runtime. The owner services safety
//! before client requests; full mailboxes detach peers rather than wait on I/O.

use crate::{
    application::Application,
    service::ServiceHost,
    wire::{self, WireRequest},
};
use std::{
    collections::{BTreeMap, VecDeque},
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

const MAX_CLIENTS: usize = 8;
const QUEUE: usize = 64;
const CLIENT_IN: usize = 8;
const CLIENT_OUT: usize = 8;
const SWEEP_BYTES: usize = 8192;

enum Incoming {
    Request(u64, WireRequest),
    Detach(u64),
}
enum Outgoing {
    Reply {
        connection: u64,
        frame: Vec<u8>,
        consumed: bool,
        hello: bool,
    },
}

struct Peer {
    stream: TcpStream,
    input: Vec<u8>,
    partial_since: Option<Instant>,
    handshake_since: Instant,
    replied_hello: bool,
    replies: VecDeque<Vec<u8>>,
    writing: Option<(Vec<u8>, usize)>,
    last_write: Instant,
    pending: usize,
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
            writing: None,
            last_write: now,
            pending: 0,
        }
    }
    fn queued(&self) -> usize {
        self.replies.len() + usize::from(self.writing.is_some())
    }
    fn read(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> io::Result<bool> {
        if self.pending >= CLIENT_IN {
            return Ok(true);
        }
        let mut scratch = [0u8; SWEEP_BYTES];
        match self.stream.read(&mut scratch) {
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
                Err(_) => return Ok(false),
            };
            match to_owner.try_send(Incoming::Request(id, request)) {
                Ok(()) => {
                    self.input.drain(..=end);
                    self.pending += 1;
                    self.partial_since = (!self.input.is_empty()).then(Instant::now);
                }
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return Ok(false),
            }
        }
        Ok(true)
    }
    fn write(&mut self) -> io::Result<bool> {
        let mut budget = SWEEP_BYTES;
        for _ in 0..4 {
            if self.writing.is_none() {
                self.writing = self.replies.pop_front().map(|bytes| (bytes, 0));
            }
            let Some((bytes, offset)) = &mut self.writing else {
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
        (!self.replied_hello && now.duration_since(self.handshake_since) >= Duration::from_secs(2))
            || self
                .partial_since
                .is_some_and(|t| now.duration_since(t) >= Duration::from_secs(2))
            || (self.queued() > 0 && now.duration_since(self.last_write) >= Duration::from_secs(2))
    }
}

fn reactor(
    listener: TcpListener,
    to_owner: SyncSender<Incoming>,
    from_owner: Receiver<Outgoing>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut peers = BTreeMap::<u64, Peer>::new();
    let mut next_id = 1u64;
    while !stop.load(Ordering::Acquire) {
        for _ in 0..8 {
            match listener.accept() {
                Ok((stream, _)) => {
                    if peers.len() >= MAX_CLIENTS {
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
                Ok(Outgoing::Reply {
                    connection: id,
                    frame,
                    consumed,
                    hello,
                }) => {
                    if let Some(peer) = peers.get_mut(&id) {
                        if peer.queued() >= CLIENT_OUT {
                            peers.remove(&id);
                            let _ = to_owner.try_send(Incoming::Detach(id));
                        } else {
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
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        let ids: Vec<_> = peers.keys().copied().collect();
        for id in ids {
            let alive = if let Some(peer) = peers.get_mut(&id) {
                !peer.timed_out()
                    && peer.read(id, &to_owner).unwrap_or(false)
                    && peer.write().unwrap_or(false)
            } else {
                false
            };
            if !alive {
                peers.remove(&id);
                let _ = to_owner.try_send(Incoming::Detach(id));
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    let mut rotation = 0usize;
    loop {
        if stop.load(Ordering::Acquire) {
            service.request_shutdown()?;
        }
        let clock = service.clock_copy();
        service.owner_mut().service(&clock)?;
        for _ in 0..16 {
            match incoming_rx.try_recv() {
                Ok(Incoming::Request(id, req)) => {
                    let q = queued.entry(id).or_default();
                    if q.len() < CLIENT_IN {
                        q.push_back(req);
                    }
                }
                Ok(Incoming::Detach(id)) => {
                    queued.remove(&id);
                    app.detach(&service, id);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    service.request_shutdown()?;
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
                    let frame = wire::encode_frame(&value)?;
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
                        break;
                    }
                }
            }
            rotation = (rotation + 1) % ids.len();
        }
        if let Some(status) = service.shutdown_step()? {
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
        thread::sleep(Duration::from_millis(5));
    }
}
