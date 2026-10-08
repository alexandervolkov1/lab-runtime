//! Socket/framing adapter owned exclusively by the existing Application worker.
//!
//! DNS, connect, TLS and Upgrade share one absolute deadline. After Upgrade all
//! I/O is nonblocking and budgeted. A queued WebSocket message is never written
//! twice after WouldBlock; only its retained frame is flushed on later turns.

use super::{
    endpoint::{SUBPROTOCOL, WebSocketEndpoint},
    framing::{FrameDecoder, PendingWrite},
    types::APPLICATION_JSON_LIMIT,
};
use serde_json::Value;
use std::{
    cell::Cell,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tungstenite::{
    Error, Message, WebSocket,
    protocol::{
        WebSocketConfig,
        frame::{
            FrameHeader,
            coding::{Data, OpCode},
        },
    },
    stream::MaybeTlsStream,
};

const TURN_BYTES: usize = 8 * 1024;
const HANDSHAKE_BYTES: usize = 8 * 1024;
const CLOSE_DEADLINE: Duration = Duration::from_millis(200);

pub(crate) enum Transport {
    Tcp(TcpStream),
    WebSocket(Box<WebSocket<ClientIo>>),
}

impl From<TcpStream> for Transport {
    fn from(stream: TcpStream) -> Self {
        Self::Tcp(stream)
    }
}

pub(crate) struct Outbound {
    tcp: PendingWrite,
    queued: bool,
    written: bool,
    blocked_since: Option<Instant>,
    write_start: Option<u64>,
}

impl Outbound {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            tcp: PendingWrite::new(bytes),
            queued: false,
            written: false,
            blocked_since: None,
            write_start: None,
        }
    }
    pub(crate) fn wrote_any(&self) -> bool {
        self.written || self.tcp.wrote_any()
    }
    pub(crate) fn timed_out(&self, now: Instant, duration: Duration) -> bool {
        self.tcp.timed_out(now, duration)
            || self
                .blocked_since
                .is_some_and(|since| now.saturating_duration_since(since) >= duration)
    }
    pub(crate) fn advance(&mut self, transport: &mut Transport, now: Instant) -> io::Result<bool> {
        match transport {
            Transport::Tcp(stream) => self.tcp.advance(stream, now),
            Transport::WebSocket(socket) => {
                self.write_start
                    .get_or_insert(socket.get_ref().written.get());
                let before = socket.get_ref().written.get();
                let result = self.advance_websocket(socket);
                self.written |= socket.get_ref().written.get() != before;
                match result {
                    Ok(()) => {
                        self.blocked_since = None;
                        Ok(true)
                    }
                    Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.blocked_since.get_or_insert(now);
                        Ok(false)
                    }
                    Err(_) => Err(io::Error::other("WebSocket output failed")),
                }
            }
        }
    }
    pub(crate) fn observe_writes(&mut self, transport: &Transport) {
        if let Transport::WebSocket(socket) = transport {
            self.written |= self
                .write_start
                .is_some_and(|start| socket.get_ref().written.get() != start);
        }
    }
    fn advance_websocket<S: Read + Write>(
        &mut self,
        socket: &mut WebSocket<S>,
    ) -> Result<(), Error> {
        if !self.queued {
            let body = self
                .tcp
                .bytes()
                .strip_suffix(b"\n")
                .ok_or_else(|| io::Error::other("invalid encoded Application message"))?;
            let body = std::str::from_utf8(body)
                .map_err(|_| io::Error::other("invalid Application UTF-8"))?;
            // Tungstenite retains the frame on Io(WouldBlock). WriteBufferFull
            // means it was NOT retained. Never pass the mutation to write again.
            match socket.write(Message::Text(body.to_owned().into())) {
                Ok(()) => self.queued = true,
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.queued = true
                }
                Err(error) => return Err(error),
            }
        }
        socket.flush()
    }
}

pub(crate) enum Incoming {
    Idle,
    Closed,
    Values(Vec<Value>),
}

impl Transport {
    pub(crate) fn begin_turn(&mut self) {
        if let Self::WebSocket(socket) = self {
            socket.get_mut().begin_turn();
        }
    }
    pub(crate) fn read_messages(&mut self, decoder: &mut FrameDecoder) -> io::Result<Incoming> {
        match self {
            Self::Tcp(stream) => {
                let mut bytes = [0; TURN_BYTES];
                match stream.read(&mut bytes) {
                    Ok(0) => Ok(Incoming::Closed),
                    Ok(count) => decoder
                        .push(&bytes[..count], Instant::now())
                        .map(Incoming::Values)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        Ok(Incoming::Idle)
                    }
                    Err(error) => Err(error),
                }
            }
            Self::WebSocket(socket) => {
                if let Some(reason) = socket.get_mut().input_failure.take() {
                    return Err(io::Error::other(reason));
                }
                let mut values = Vec::new();
                for _ in 0..8 {
                    match socket.read() {
                        Ok(Message::Text(text)) => {
                            if text.len() > APPLICATION_JSON_LIMIT {
                                return Err(io::Error::other(
                                    "oversized WebSocket Application message",
                                ));
                            }
                            values.push(serde_json::from_slice(text.as_bytes()).map_err(|_| {
                                io::Error::other("invalid WebSocket Application JSON")
                            })?);
                        }
                        Ok(Message::Ping(_) | Message::Pong(_)) => {}
                        Ok(Message::Close(_))
                        | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                            let _ = socket.flush();
                            return if values.is_empty() {
                                Ok(Incoming::Closed)
                            } else {
                                Ok(Incoming::Values(values))
                            };
                        }
                        Ok(_) => {
                            return Err(io::Error::other(
                                "Application WebSocket requires text messages",
                            ));
                        }
                        Err(Error::Io(error))
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) =>
                        {
                            break;
                        }
                        Err(_) => {
                            if values.is_empty() {
                                return Err(io::Error::other("WebSocket input failed"));
                            }
                            // Preserve the valid ordered prefix before reporting
                            // an abrupt EOF/error on the next read turn.
                            socket.get_mut().input_failure = Some("WebSocket input failed");
                            break;
                        }
                    }
                }
                if values.is_empty() {
                    Ok(Incoming::Idle)
                } else {
                    Ok(Incoming::Values(values))
                }
            }
        }
    }
    pub(crate) fn flush_control(&mut self, now: Instant) -> io::Result<()> {
        let Self::WebSocket(socket) = self else {
            return Ok(());
        };
        match socket.flush() {
            Ok(()) => {
                socket.get_mut().control_blocked = None;
                Ok(())
            }
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                socket.get_mut().control_blocked.get_or_insert(now);
                Ok(())
            }
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => Ok(()),
            Err(_) => Err(io::Error::other("WebSocket control output failed")),
        }
    }
    pub(crate) fn deadline_expired(&self, now: Instant, duration: Duration) -> bool {
        match self {
            Self::Tcp(_) => false,
            Self::WebSocket(socket) => [
                socket.get_ref().progress.since,
                socket.get_ref().control_blocked,
            ]
            .into_iter()
            .flatten()
            .any(|since| now.saturating_duration_since(since) >= duration),
        }
    }
    pub(crate) fn graceful_close(&mut self) {
        let Self::WebSocket(socket) = self else {
            return;
        };
        let deadline = Instant::now() + CLOSE_DEADLINE;
        socket.get_mut().begin_turn();
        let _ = socket.close(None);
        while Instant::now() < deadline {
            socket.get_mut().begin_turn();
            match socket.read() {
                Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                    let _ = socket.flush();
                    break;
                }
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::park_timeout(Duration::from_millis(2))
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "connection deadline exceeded"))
}

fn resolve(host: &str, port: u16, deadline: Instant) -> io::Result<Vec<SocketAddr>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    // Cancellable async DNS on this worker's current-thread executor: no detached
    // resolver thread can outlive the connect attempt or delay shutdown indefinitely.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut builder = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|_| io::Error::other("DNS configuration failed"))?;
        builder.options_mut().cache_size = 0;
        builder.options_mut().attempts = 1;
        builder.options_mut().num_concurrent_reqs = 1;
        builder.options_mut().timeout = remaining(deadline)?;
        let resolver = builder.build();
        let answer = tokio::time::timeout(remaining(deadline)?, resolver.lookup_ip(host))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS deadline exceeded"))?
            .map_err(|_| io::Error::other("DNS lookup failed"))?;
        let addresses: Vec<_> = answer
            .iter()
            .take(8)
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        if addresses.is_empty() {
            Err(io::Error::other("DNS returned no address"))
        } else {
            Ok(addresses)
        }
    })
}

pub(crate) fn connect(endpoint: &WebSocketEndpoint, deadline: Instant) -> io::Result<Transport> {
    let host = endpoint
        .url
        .host_str()
        .expect("validated host")
        .trim_matches(['[', ']']);
    let addresses = resolve(
        host,
        endpoint.url.port_or_known_default().expect("WS port"),
        deadline,
    )?;
    let mut stream = None;
    for address in addresses {
        if let Ok(connected) = TcpStream::connect_timeout(&address, remaining(deadline)?) {
            stream = Some(connected);
            break;
        }
    }
    let stream = stream.ok_or_else(|| io::Error::other("WebSocket TCP connect failed"))?;
    stream.set_nodelay(true)?;
    let written = Rc::new(Cell::new(0));
    let tcp = DeadlineTcp {
        stream,
        deadline: Some(deadline),
        written: Rc::clone(&written),
    };
    let stream = if endpoint.url.scheme() == "wss" {
        let mut roots = rustls::RootCertStore::empty();
        for certificate in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(certificate);
        }
        if let Some(path) = &endpoint.ca_file {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(256 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 256 * 1024 {
                return Err(io::Error::other("CA file exceeds its bound"));
            }
            let certificates =
                rustls_pemfile::certs(&mut bytes.as_slice()).collect::<Result<Vec<_>, _>>()?;
            if certificates.is_empty() {
                return Err(io::Error::other("CA file contains no certificate"));
            }
            for certificate in certificates {
                roots
                    .add(certificate)
                    .map_err(|_| io::Error::other("invalid CA certificate"))?;
            }
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| io::Error::other("TLS configuration failed"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|_| io::Error::other("invalid TLS hostname"))?;
        let connection = rustls::ClientConnection::new(Arc::new(config), name)
            .map_err(|_| io::Error::other("TLS configuration failed"))?;
        MaybeTlsStream::Rustls(rustls::StreamOwned::new(connection, tcp))
    } else {
        MaybeTlsStream::Plain(tcp)
    };
    let stream = ClientIo {
        stream,
        written,
        handshaking: true,
        header_bytes: 0,
        header_tail: [0; 4],
        read_budget: TURN_BYTES,
        write_budget: TURN_BYTES,
        progress: Progress::default(),
        control_blocked: None,
        input_failure: None,
    };
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(32 * 1024)
        .max_message_size(Some(APPLICATION_JSON_LIMIT))
        .max_frame_size(Some(APPLICATION_JSON_LIMIT));
    let (mut socket, response) =
        tungstenite::client::client_with_config(endpoint.request()?, stream, Some(config))
            .map_err(|error| {
                // Never format an HTTP response/body: a proxy can reflect X-Token.
                let reason = match error {
                    tungstenite::HandshakeError::Failure(Error::Http(response)) => {
                        match response.status().as_u16() {
                            401 | 403 => "WebSocket handshake authorization rejected",
                            _ => "WebSocket HTTP Upgrade rejected",
                        }
                    }
                    tungstenite::HandshakeError::Failure(Error::Io(error))
                        if error.kind() == io::ErrorKind::InvalidData =>
                    {
                        "WebSocket TLS validation failed"
                    }
                    tungstenite::HandshakeError::Failure(Error::Protocol(_)) => {
                        "WebSocket Upgrade protocol validation failed"
                    }
                    tungstenite::HandshakeError::Failure(Error::Tls(_)) => {
                        "WebSocket TLS validation failed"
                    }
                    tungstenite::HandshakeError::Failure(Error::Io(error))
                        if error.kind() == io::ErrorKind::TimedOut =>
                    {
                        "WebSocket connection deadline exceeded"
                    }
                    tungstenite::HandshakeError::Failure(Error::Io(_)) => {
                        "WebSocket Upgrade I/O failed"
                    }
                    tungstenite::HandshakeError::Interrupted(_) => {
                        "WebSocket Upgrade unexpectedly interrupted"
                    }
                    _ => "WebSocket TLS/Upgrade failed",
                };
                io::Error::other(reason)
            })?;
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok())
        != Some(SUBPROTOCOL)
    {
        return Err(io::Error::other(
            "WebSocket Application subprotocol was not selected",
        ));
    }
    socket.get_mut().finish_handshake()?;
    Ok(Transport::WebSocket(Box::new(socket)))
}

pub(crate) struct DeadlineTcp {
    stream: TcpStream,
    deadline: Option<Instant>,
    written: Rc<Cell<u64>>,
}
impl Read for DeadlineTcp {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            self.stream.set_read_timeout(Some(remaining(deadline)?))?;
        }
        self.stream.read(bytes)
    }
}
impl Write for DeadlineTcp {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            self.stream.set_write_timeout(Some(remaining(deadline)?))?;
        }
        let count = self.stream.write(bytes)?;
        self.written
            .set(self.written.get().saturating_add(count as u64));
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub(crate) struct ClientIo {
    stream: MaybeTlsStream<DeadlineTcp>,
    written: Rc<Cell<u64>>,
    handshaking: bool,
    header_bytes: usize,
    header_tail: [u8; 4],
    read_budget: usize,
    write_budget: usize,
    progress: Progress,
    control_blocked: Option<Instant>,
    input_failure: Option<&'static str>,
}
impl ClientIo {
    fn begin_turn(&mut self) {
        self.read_budget = TURN_BYTES;
        self.write_budget = TURN_BYTES;
    }
    fn finish_handshake(&mut self) -> io::Result<()> {
        let tcp = match &mut self.stream {
            MaybeTlsStream::Plain(tcp) => tcp,
            MaybeTlsStream::Rustls(tls) => &mut tls.sock,
            _ => return Err(io::Error::other("unsupported TLS stream")),
        };
        tcp.deadline = None;
        tcp.stream.set_read_timeout(None)?;
        tcp.stream.set_write_timeout(None)?;
        tcp.stream.set_nonblocking(true)?;
        self.handshaking = false;
        self.begin_turn();
        Ok(())
    }
}
impl Read for ClientIo {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let limit = if self.handshaking {
            bytes.len().min(HANDSHAKE_BYTES)
        } else {
            bytes.len().min(self.read_budget)
        };
        if limit == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = self.stream.read(&mut bytes[..limit])?;
        if self.handshaking {
            for &byte in &bytes[..count] {
                if self.header_tail == *b"\r\n\r\n" {
                    self.progress.observe(&[byte], Instant::now());
                    continue;
                }
                self.header_bytes += 1;
                if self.header_bytes > HANDSHAKE_BYTES {
                    return Err(io::Error::other("WebSocket Upgrade exceeds its byte bound"));
                }
                self.header_tail.rotate_left(1);
                self.header_tail[3] = byte;
            }
        } else {
            self.read_budget -= count;
            self.progress.observe(&bytes[..count], Instant::now());
        }
        Ok(count)
    }
}
impl Write for ClientIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let limit = if self.handshaking {
            bytes.len()
        } else {
            bytes.len().min(self.write_budget)
        };
        if limit == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = self.stream.write(&bytes[..limit])?;
        if !self.handshaking {
            self.write_budget -= count;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

// This is a deadline observer, not another protocol parser. Tungstenite owns
// validation/assembly. Incomplete headers, payloads and fragmented messages all
// retain the first-byte deadline, including when Ping/Pong frames are interleaved.
#[derive(Default)]
struct Progress {
    header: Vec<u8>,
    payload: Option<(FrameHeader, u64)>,
    fragmented: bool,
    since: Option<Instant>,
    invalid: bool,
}
impl Progress {
    fn observe(&mut self, mut bytes: &[u8], now: Instant) {
        if self.invalid {
            return;
        }
        while !bytes.is_empty() {
            self.since.get_or_insert(now);
            if let Some((header, remaining)) = &mut self.payload {
                let count = (*remaining).min(bytes.len() as u64) as usize;
                *remaining -= count as u64;
                bytes = &bytes[count..];
                if *remaining == 0 {
                    let header = header.clone();
                    self.payload = None;
                    self.finish(header);
                }
                continue;
            }
            self.header.push(bytes[0]);
            bytes = &bytes[1..];
            match FrameHeader::parse(&mut io::Cursor::new(self.header.as_slice())) {
                Ok(Some((header, remaining))) => {
                    self.header.clear();
                    if remaining == 0 {
                        self.finish(header)
                    } else {
                        self.payload = Some((header, remaining));
                    }
                }
                Ok(None) if self.header.len() < 14 => {}
                _ => {
                    self.invalid = true;
                    self.header.clear();
                    self.payload = None;
                    return;
                }
            }
        }
    }
    fn finish(&mut self, header: FrameHeader) {
        match header.opcode {
            OpCode::Data(Data::Text | Data::Binary) => self.fragmented = !header.is_final,
            OpCode::Data(Data::Continue) if header.is_final => self.fragmented = false,
            _ => {}
        }
        if !self.fragmented {
            self.since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ScriptedIo {
        actions: std::collections::VecDeque<io::Result<usize>>,
        bytes: Vec<u8>,
    }
    impl Read for ScriptedIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    impl Write for ScriptedIo {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = self
                .actions
                .pop_front()
                .unwrap_or(Ok(bytes.len()))?
                .min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn would_block_flushes_one_retained_mutation_frame_without_requeueing() {
        let expected = serde_json::json!({"v":1,"msg_id":"2","op":"reference_retune","request_id":{"scope":"scope","seq":"1"},"args":{"target":41.5}});
        let bytes = super::super::framing::encode_frame(&expected).unwrap();
        let mut outbound = Outbound::new(bytes);
        let io = ScriptedIo {
            actions: std::collections::VecDeque::from([
                Ok(2),
                Err(io::ErrorKind::WouldBlock.into()),
                Err(io::ErrorKind::WouldBlock.into()),
                Ok(usize::MAX),
            ]),
            bytes: Vec::new(),
        };
        let mut socket = WebSocket::from_raw_socket(
            io,
            tungstenite::protocol::Role::Client,
            Some(WebSocketConfig::default().write_buffer_size(0)),
        );
        assert!(
            matches!(outbound.advance_websocket(&mut socket), Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock)
        );
        assert!(outbound.queued);
        outbound.advance_websocket(&mut socket).unwrap();
        let wire = socket.get_ref().bytes.clone();
        let mut server = WebSocket::from_raw_socket(
            io::Cursor::new(wire.clone()),
            tungstenite::protocol::Role::Server,
            None,
        );
        let Message::Text(text) = server.read().unwrap() else {
            panic!("one Application text frame")
        };
        assert_eq!(
            serde_json::from_slice::<Value>(text.as_bytes()).unwrap(),
            expected
        );
        assert_eq!(
            wire.len(),
            text.len() + 6,
            "one masked frame, no duplicate mutation"
        );
    }

    #[test]
    fn tcp_partial_error_preserves_first_byte_evidence() {
        let mut outbound = Outbound::new(b"request\n".to_vec());
        let mut io = ScriptedIo {
            actions: std::collections::VecDeque::from([
                Ok(2),
                Err(io::ErrorKind::BrokenPipe.into()),
            ]),
            bytes: Vec::new(),
        };
        assert!(outbound.tcp.advance(&mut io, Instant::now()).is_err());
        assert!(
            outbound.wrote_any(),
            "an error after send start is ambiguous"
        );
    }

    #[test]
    fn incomplete_headers_payloads_and_fragmented_messages_keep_absolute_deadline() {
        let start = Instant::now();
        let later = start + Duration::from_secs(1);
        let mut progress = Progress::default();
        progress.observe(&[0x01], start);
        assert_eq!(progress.since, Some(start));
        progress.observe(&[1, b'x', 0x89, 0], later);
        assert_eq!(progress.since, Some(start));
        progress.observe(&[0x80, 1], later);
        assert_eq!(progress.since, Some(start));
        progress.observe(b"y", later);
        assert_eq!(progress.since, None);
        progress.observe(&[0x81, 2, b'a'], start);
        assert_eq!(progress.since, Some(start));
        progress.observe(b"b", later);
        assert_eq!(progress.since, None);
    }
}
