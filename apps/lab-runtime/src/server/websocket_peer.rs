//! Bounded RFC 6455 mechanics for one connection in the shared network reactor.

use super::{
    CLIENT_DEADLINE, Incoming, SWEEP_BYTES, coordination::ClientDelivery, encode_outgoing,
    rejection,
};
use crate::{
    websocket::{
        APPLICATION_PATH, APPLICATION_SUBPROTOCOL, HANDSHAKE_BYTES, HANDSHAKE_HEADERS,
        READ_BUFFER_BYTES, WRITE_BUFFER_BYTES,
    },
    wire,
};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    sync::mpsc::{SyncSender, TrySendError},
    time::Instant,
};
use tungstenite::{
    Error as WebSocketError, Message, WebSocket,
    handshake::{HandshakeError, MidHandshake, server::ServerHandshake},
    http::{
        HeaderValue, Method, StatusCode, Version,
        header::{HOST, ORIGIN, SEC_WEBSOCKET_PROTOCOL},
    },
    protocol::{
        CloseFrame, WebSocketConfig,
        frame::{
            FrameHeader,
            coding::{CloseCode, Data, OpCode},
        },
    },
};

#[derive(Debug)]
struct ObservedPayload {
    remaining: u64,
    is_final: bool,
    opcode: OpCode,
}

/// Deadline-only observation of frame boundaries using Tungstenite's own header
/// parser. Payload validation and message assembly remain exclusively Tungstenite's.
#[derive(Debug)]
struct FragmentProgress {
    header: Vec<u8>,
    payload: Option<ObservedPayload>,
    incomplete_since: Option<Instant>,
    enabled: bool,
}

impl FragmentProgress {
    fn new() -> Self {
        Self {
            header: Vec::with_capacity(14),
            payload: None,
            incomplete_since: None,
            enabled: false,
        }
    }

    fn start(&mut self) {
        self.header.clear();
        self.payload = None;
        self.incomplete_since = None;
        self.enabled = true;
    }

    fn observe(&mut self, mut bytes: &[u8]) {
        if !self.enabled {
            return;
        }
        while !bytes.is_empty() {
            if let Some(payload) = &mut self.payload {
                let consumed = payload.remaining.min(bytes.len() as u64) as usize;
                payload.remaining -= consumed as u64;
                bytes = &bytes[consumed..];
                if payload.remaining == 0 {
                    let payload = self.payload.take().expect("observed payload exists");
                    self.finish_frame(payload);
                }
                continue;
            }

            self.header.push(bytes[0]);
            bytes = &bytes[1..];
            let mut cursor = std::io::Cursor::new(self.header.as_slice());
            match FrameHeader::parse(&mut cursor) {
                Ok(Some((header, remaining))) => {
                    self.header.clear();
                    let payload = ObservedPayload {
                        remaining,
                        is_final: header.is_final,
                        opcode: header.opcode,
                    };
                    if remaining == 0 {
                        self.finish_frame(payload);
                    } else {
                        self.payload = Some(payload);
                    }
                }
                Ok(None) if self.header.len() < 14 => {}
                Ok(None) | Err(_) => {
                    // Tungstenite remains the sole protocol validator. Invalid
                    // framing will terminate there; this observer simply retires.
                    self.enabled = false;
                    self.header.clear();
                    self.payload = None;
                    return;
                }
            }
        }
    }

    fn finish_frame(&mut self, payload: ObservedPayload) {
        match payload.opcode {
            OpCode::Data(Data::Text | Data::Binary) if !payload.is_final => {
                self.incomplete_since.get_or_insert_with(Instant::now);
            }
            OpCode::Data(Data::Continue) if payload.is_final => {
                self.incomplete_since = None;
            }
            // A final unfragmented data frame is not an incomplete message.
            // If it follows an open fragment Tungstenite rejects it as protocol
            // invalid, so clearing here cannot admit a valid stalled peer.
            OpCode::Data(Data::Text | Data::Binary) => self.incomplete_since = None,
            _ => {}
        }
    }
}

/// Socket wrapper enforcing project-owned handshake and per-turn I/O budgets.
#[derive(Debug)]
struct BoundedStream {
    stream: TcpStream,
    handshaking: bool,
    handshake_read: usize,
    read_budget: usize,
    write_budget: usize,
    total_read: u64,
    total_written: u64,
    fragment_progress: FragmentProgress,
}

impl BoundedStream {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            handshaking: true,
            handshake_read: 0,
            read_budget: SWEEP_BYTES,
            write_budget: SWEEP_BYTES,
            total_read: 0,
            total_written: 0,
            fragment_progress: FragmentProgress::new(),
        }
    }

    fn begin_turn(&mut self) {
        self.read_budget = SWEEP_BYTES;
        self.write_budget = SWEEP_BYTES;
    }

    fn handshake_complete(&mut self) {
        self.handshaking = false;
        self.fragment_progress.start();
    }
}

impl Read for BoundedStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.read_budget == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let handshake_remaining = if self.handshaking {
            HANDSHAKE_BYTES
                .saturating_add(1)
                .saturating_sub(self.handshake_read)
        } else {
            usize::MAX
        };
        let limit = bytes.len().min(self.read_budget).min(handshake_remaining);
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WebSocket handshake exceeds byte limit",
            ));
        }
        let count = self.stream.read(&mut bytes[..limit])?;
        self.fragment_progress.observe(&bytes[..count]);
        self.read_budget -= count;
        self.total_read = self.total_read.saturating_add(count as u64);
        if self.handshaking {
            self.handshake_read += count;
            if self.handshake_read > HANDSHAKE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WebSocket handshake exceeds byte limit",
                ));
            }
        }
        Ok(count)
    }
}

impl Write for BoundedStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.write_budget == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = self
            .stream
            .write(&bytes[..bytes.len().min(self.write_budget)])?;
        self.write_budget -= count;
        self.total_written = self.total_written.saturating_add(count as u64);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[derive(Debug)]
struct HandshakePolicy {
    expected_host: String,
    allowed_origins: Vec<String>,
}

impl HandshakePolicy {
    fn reject(status: StatusCode) -> tungstenite::handshake::server::ErrorResponse {
        tungstenite::http::Response::builder()
            .status(status)
            .body(None)
            .expect("fixed WebSocket handshake rejection")
    }
}

impl tungstenite::handshake::server::Callback for HandshakePolicy {
    fn on_request(
        self,
        request: &tungstenite::handshake::server::Request,
        mut response: tungstenite::handshake::server::Response,
    ) -> Result<
        tungstenite::handshake::server::Response,
        tungstenite::handshake::server::ErrorResponse,
    > {
        if request.method() != Method::GET
            || request.version() != Version::HTTP_11
            || request.uri().path() != APPLICATION_PATH
            || request.uri().query().is_some()
            || request.headers().len() > HANDSHAKE_HEADERS
        {
            return Err(Self::reject(StatusCode::BAD_REQUEST));
        }
        let hosts = request.headers().get_all(HOST).iter().collect::<Vec<_>>();
        if hosts.len() != 1 || hosts[0].as_bytes() != self.expected_host.as_bytes() {
            return Err(Self::reject(StatusCode::BAD_REQUEST));
        }
        let origins = request.headers().get_all(ORIGIN).iter().collect::<Vec<_>>();
        if origins.len() != 1
            || origins[0].as_bytes() == b"null"
            || !self
                .allowed_origins
                .iter()
                .any(|origin| origin.as_bytes() == origins[0].as_bytes())
        {
            return Err(Self::reject(StatusCode::FORBIDDEN));
        }
        let protocols = request
            .headers()
            .get_all(SEC_WEBSOCKET_PROTOCOL)
            .iter()
            .collect::<Vec<_>>();
        if protocols.len() != 1 || protocols[0].as_bytes() != APPLICATION_SUBPROTOCOL.as_bytes() {
            return Err(Self::reject(StatusCode::BAD_REQUEST));
        }
        response.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(APPLICATION_SUBPROTOCOL),
        );
        // Extensions are deliberately never copied to the response. M12 does not
        // negotiate compression or any other WebSocket extension.
        Ok(response)
    }
}

type Handshake = MidHandshake<ServerHandshake<BoundedStream, HandshakePolicy>>;

enum State {
    Handshake(Handshake),
    Open(WebSocket<BoundedStream>),
    Empty,
}

/// One WebSocket adapter. Application delivery remains in `ClientDelivery`.
pub(super) struct WebSocketPeer {
    state: State,
    accepted_at: Instant,
    partial_since: Option<Instant>,
    output_since: Option<Instant>,
    close_since: Option<Instant>,
    close_input_readable: bool,
    staged_request: Option<wire::WireRequest>,
    writing: Option<bool>,
    delivery: ClientDelivery,
}

impl WebSocketPeer {
    pub(super) fn new(
        stream: TcpStream,
        expected_host: String,
        allowed_origins: Vec<String>,
    ) -> Self {
        let config = WebSocketConfig::default()
            .read_buffer_size(READ_BUFFER_BYTES)
            .write_buffer_size(0)
            .max_write_buffer_size(WRITE_BUFFER_BYTES)
            .max_message_size(Some(wire::APPLICATION_JSON_LIMIT))
            .max_frame_size(Some(wire::APPLICATION_JSON_LIMIT))
            .accept_unmasked_frames(false);
        let policy = HandshakePolicy {
            expected_host,
            allowed_origins,
        };
        Self {
            state: State::Handshake(ServerHandshake::start(
                BoundedStream::new(stream),
                policy,
                Some(config),
            )),
            accepted_at: Instant::now(),
            partial_since: None,
            output_since: None,
            close_since: None,
            close_input_readable: true,
            staged_request: None,
            writing: None,
            delivery: ClientDelivery::new(),
        }
    }

    pub(super) fn queued(&self) -> usize {
        self.delivery.queued(self.writing)
    }

    pub(super) fn push_reply(&mut self, message: Vec<u8>, consumed: bool, hello: bool) -> bool {
        let accepted = self
            .delivery
            .push_reply(message, consumed, hello, self.writing);
        if accepted && self.output_since.is_none() {
            self.output_since = Some(Instant::now());
        }
        accepted
    }

    pub(super) fn push_event(&mut self, message: Vec<u8>) -> bool {
        let accepted = self.delivery.push_event(message, self.writing);
        if accepted && self.output_since.is_none() {
            self.output_since = Some(Instant::now());
        }
        accepted
    }

    pub(super) fn close(&mut self) {
        self.delivery.close();
    }

    pub(super) fn service(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> bool {
        if self.timed_out() || !self.advance_handshake(id) {
            return false;
        }
        if !matches!(self.state, State::Open(_)) {
            return true;
        }
        self.begin_turn();
        if !self.delivery.closing() && !self.read_messages(id, to_owner) {
            return false;
        }
        if !self.write_messages() {
            return false;
        }
        if self.delivery.closing() && self.queued() == 0 {
            self.begin_close()
        } else {
            true
        }
    }

    fn advance_handshake(&mut self, id: u64) -> bool {
        let State::Handshake(_) = self.state else {
            return true;
        };
        let state = std::mem::replace(&mut self.state, State::Empty);
        let State::Handshake(mut handshake) = state else {
            unreachable!()
        };
        handshake.get_mut().get_mut().begin_turn();
        match handshake.handshake() {
            Ok(mut socket) => {
                socket.get_mut().handshake_complete();
                self.state = State::Open(socket);
                true
            }
            Err(HandshakeError::Interrupted(handshake)) => {
                self.state = State::Handshake(handshake);
                true
            }
            Err(HandshakeError::Failure(error)) => {
                tracing::warn!(
                    event = "websocket_upgrade_rejected",
                    connection = id,
                    error = %error,
                    "closing rejected WebSocket upgrade"
                );
                false
            }
        }
    }

    fn begin_turn(&mut self) {
        if let State::Open(socket) = &mut self.state {
            socket.get_mut().begin_turn();
        }
    }

    fn read_messages(&mut self, id: u64, to_owner: &SyncSender<Incoming>) -> bool {
        if let Some(request) = self.staged_request.take() {
            match self.deliver_request(id, request, to_owner) {
                Ok(true) => {}
                Ok(false) => return true,
                Err(()) => return false,
            }
        }
        for _ in 0..4 {
            if self.delivery.pending_full() {
                break;
            }
            let State::Open(socket) = &mut self.state else {
                return false;
            };
            let before = socket.get_ref().total_read;
            match socket.read() {
                Ok(Message::Text(text)) => {
                    self.partial_since = None;
                    let request = match wire::decode_application_json(text.as_bytes()) {
                        Ok(request) => request,
                        Err(error) => {
                            let correlation =
                                serde_json::from_slice::<serde_json::Value>(text.as_bytes())
                                    .ok()
                                    .and_then(|value| {
                                        value["msg_id"]
                                            .as_str()
                                            .filter(|msg_id| {
                                                !msg_id.is_empty() && msg_id.len() <= 64
                                            })
                                            .map(str::to_owned)
                                    });
                            self.delivery.reject_and_close(encode_outgoing(&rejection(
                                correlation.as_deref(),
                                error.code,
                            )));
                            if self.output_since.is_none() {
                                self.output_since = Some(Instant::now());
                            }
                            tracing::warn!(
                                event = "client_request_malformed",
                                connection = id,
                                code = error.code,
                                message_bytes = text.len(),
                                "closing WebSocket client after bounded malformed request"
                            );
                            break;
                        }
                    };
                    if self.delivery.request_is_in_flight(&request.msg_id) {
                        self.delivery.reject_and_close(encode_outgoing(&rejection(
                            Some(&request.msg_id),
                            "duplicate_msg_id",
                        )));
                        if self.output_since.is_none() {
                            self.output_since = Some(Instant::now());
                        }
                        break;
                    }
                    match self.deliver_request(id, request, to_owner) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(()) => return false,
                    }
                }
                Ok(Message::Binary(_)) => {
                    self.protocol_close(CloseCode::Unsupported, "text messages required", false);
                    break;
                }
                Ok(Message::Close(_)) => {
                    self.delivery.close();
                    self.close_since.get_or_insert_with(Instant::now);
                    break;
                }
                Ok(Message::Ping(_) | Message::Pong(_)) => {
                    // Tungstenite queues the required pong while reading. Flushing it
                    // below keeps transport control traffic inside the same budget.
                }
                Ok(Message::Frame(_)) => unreachable!("raw frames are not returned by read"),
                Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    if socket.get_ref().total_read > before {
                        self.partial_since.get_or_insert_with(Instant::now);
                    }
                    break;
                }
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    return false;
                }
                Err(WebSocketError::Capacity(_)) => {
                    self.protocol_close(CloseCode::Size, "message too large", true);
                    break;
                }
                Err(WebSocketError::Utf8(_)) => {
                    self.protocol_close(CloseCode::Invalid, "invalid UTF-8", true);
                    break;
                }
                Err(WebSocketError::Protocol(_)) => {
                    self.protocol_close(CloseCode::Protocol, "protocol error", true);
                    break;
                }
                Err(_) => return false,
            }
        }
        true
    }

    /// Retain one completed request while the shared bounded owner mailbox is full.
    /// Reading stops until the same request is admitted, so transport pressure never
    /// loses or reorders an already assembled Application message.
    fn deliver_request(
        &mut self,
        id: u64,
        request: wire::WireRequest,
        to_owner: &SyncSender<Incoming>,
    ) -> Result<bool, ()> {
        let msg_id = request.msg_id.clone();
        match to_owner.try_send(Incoming::Request(id, request)) {
            Ok(()) => {
                self.delivery.request_admitted(msg_id);
                self.partial_since = None;
                Ok(true)
            }
            Err(TrySendError::Full(message)) => {
                let Incoming::Request(_, request) = message else {
                    unreachable!("only requests are submitted by a WebSocket peer")
                };
                self.staged_request = Some(request);
                self.partial_since.get_or_insert_with(Instant::now);
                Ok(false)
            }
            Err(TrySendError::Disconnected(_)) => Err(()),
        }
    }

    fn application_write_accepted(&mut self, reply: bool) {
        debug_assert!(self.writing.is_none());
        self.writing = Some(reply);
    }

    fn application_flush_succeeded(&mut self) {
        if let Some(reply) = self.writing.take() {
            self.delivery.message_written(reply);
        }
    }

    fn write_messages(&mut self) -> bool {
        self.delivery.stage_rejection(self.writing);
        for _ in 0..4 {
            if self.writing.is_none()
                && let Some((body, reply)) = self.delivery.next_message()
            {
                let text = match String::from_utf8(body) {
                    Ok(text) => text,
                    Err(_) => return false,
                };
                let State::Open(socket) = &mut self.state else {
                    return false;
                };
                match socket.write(Message::Text(text.into())) {
                    Ok(()) => self.application_write_accepted(reply),
                    Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                        // Tungstenite accepted and retained this exact frame.
                        // Keep its shared queue identity until flush succeeds.
                        self.application_write_accepted(reply);
                        self.output_since.get_or_insert_with(Instant::now);
                    }
                    Err(WebSocketError::WriteBufferFull(_)) => return false,
                    Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                        return false;
                    }
                    Err(_) => return false,
                }
            }
            let State::Open(socket) = &mut self.state else {
                return false;
            };
            match socket.flush() {
                Ok(()) => {
                    self.application_flush_succeeded();
                    if self.queued() == 0 {
                        self.output_since = None;
                        break;
                    }
                }
                Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.output_since.get_or_insert_with(Instant::now);
                    return true;
                }
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    return false;
                }
                Err(_) => return false,
            }
        }
        true
    }

    fn protocol_close(&mut self, code: CloseCode, reason: &'static str, decoder_unusable: bool) {
        self.delivery.close();
        self.close_since.get_or_insert_with(Instant::now);
        // A frame decoder error can leave Tungstenite's input state positioned
        // inside the rejected frame. In that case, do not invoke the decoder
        // again; ordinary unsupported message types may still read a close ACK.
        self.close_input_readable = !decoder_unusable;
        if let State::Open(socket) = &mut self.state {
            let _ = socket.close(Some(CloseFrame {
                code,
                reason: reason.into(),
            }));
        }
    }

    fn begin_close(&mut self) -> bool {
        let first = self.close_since.is_none();
        self.close_since.get_or_insert_with(Instant::now);
        let State::Open(socket) = &mut self.state else {
            return false;
        };
        if first {
            match socket.close(None) {
                Ok(()) => {}
                Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    return false;
                }
                Err(_) => return false,
            }
        }
        if self.close_input_readable {
            match socket.read() {
                Ok(Message::Close(_)) => {}
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(Message::Text(_) | Message::Binary(_) | Message::Frame(_)) => return false,
                Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    return false;
                }
                Err(_) => return false,
            }
        }
        match socket.flush() {
            Ok(()) => true,
            Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => true,
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => false,
            Err(_) => false,
        }
    }

    fn timed_out(&self) -> bool {
        let now = Instant::now();
        (!matches!(self.state, State::Open(_))
            && now.duration_since(self.accepted_at) >= CLIENT_DEADLINE)
            || (!self.delivery.replied_hello()
                && now.duration_since(self.accepted_at) >= CLIENT_DEADLINE)
            || self
                .partial_since
                .is_some_and(|at| now.duration_since(at) >= CLIENT_DEADLINE)
            || match &self.state {
                State::Open(socket) => socket
                    .get_ref()
                    .fragment_progress
                    .incomplete_since
                    .is_some_and(|at| now.duration_since(at) >= CLIENT_DEADLINE),
                State::Handshake(_) | State::Empty => false,
            }
            || self
                .output_since
                .is_some_and(|at| now.duration_since(at) >= CLIENT_DEADLINE)
            || self
                .close_since
                .is_some_and(|at| now.duration_since(at) >= CLIENT_DEADLINE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer_without_socket() -> WebSocketPeer {
        WebSocketPeer {
            state: State::Empty,
            accepted_at: Instant::now(),
            partial_since: None,
            output_since: None,
            close_since: None,
            close_input_readable: true,
            staged_request: None,
            writing: None,
            delivery: ClientDelivery::new(),
        }
    }

    #[test]
    fn accepted_but_unflushed_application_write_keeps_its_shared_capacity_slot() {
        let mut peer = peer_without_socket();
        for index in 0..super::super::CLIENT_OUT {
            assert!(peer.push_reply(vec![index as u8], false, false));
        }

        // Model the current WouldBlock path: Tungstenite has accepted this
        // Application frame, but transport flush has not succeeded yet.
        let (_, reply) = peer.delivery.next_message().unwrap();
        peer.application_write_accepted(reply);

        assert!(!peer.push_reply(vec![b'x'], false, false));
        peer.application_flush_succeeded();
        assert!(peer.push_reply(vec![b'x'], false, false));

        let mut peer = peer_without_socket();
        for index in 0..super::super::CLIENT_EVENTS {
            assert!(peer.push_event(vec![index as u8]));
        }
        let (_, reply) = peer.delivery.next_message().unwrap();
        assert!(!reply);
        peer.application_write_accepted(reply);

        assert!(!peer.push_event(vec![b'x']));
        peer.application_flush_succeeded();
        assert!(peer.push_event(vec![b'x']));
    }
}
