//! Bounded IPv4-loopback TCP/NDJSON adapter for the private Workbench dispatcher.

use crate::{
    client::types::MutationIdentity,
    dispatcher::{
        CallOrigin, CallerId, EventRoute, LabClient, PresentationExpectation, RecoveryExpectation,
        RecoveryGetArgs, RecoveryKind, RecoveryStatusTarget, RoutedWorkbenchEvent,
        WorkbenchDispatchError, WorkbenchDispatcher, WorkbenchErrorCode, WorkbenchRequest,
        WorkbenchResult,
    },
    presentation::{Plot, RuntimeRef, Trace},
};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) const JSON_BODY_BYTES: usize = 2_097_152;
pub(crate) const FRAME_BYTES: usize = 2_097_153;
pub(crate) const JSON_DEPTH: usize = 16;
pub(crate) const JSON_VALUES: usize = 16_384;
pub(crate) const JSON_STRING_BYTES: usize = 512;
pub(crate) const NAME_BYTES: usize = 64;
pub(crate) const ERROR_MESSAGE_BYTES: usize = 256;
pub(crate) const CALLERS: usize = 8;
pub(crate) const CALLER_INPUT_MESSAGES: usize = 4;
pub(crate) const CALLER_INPUT_BYTES: usize = 4_194_304;
pub(crate) const OWNER_MAILBOX_MESSAGES: usize = 32;
pub(crate) const OWNER_MAILBOX_BYTES: usize = 16_777_216;
pub(crate) const CALLER_OUTPUT_MESSAGES: usize = 4;
pub(crate) const CALLER_OUTPUT_BYTES: usize = 4_194_304;
pub(crate) const RESERVED_CALL_IDS_PER_CALLER: usize = 8;
pub(crate) const RESERVED_CALL_IDS_TOTAL: usize = 32;
pub(crate) const OUTSTANDING_LAB_PER_CALLER: usize = 8;
pub(crate) const OUTSTANDING_LAB_TOTAL: usize = 32;
pub(crate) const SOCKET_BYTES_PER_TURN: usize = 8 * 1024;
pub(crate) const FRAMES_PER_CALLER_TURN: usize = 4;
pub(crate) const OWNER_CALLERS_PER_TURN: usize = 4;
pub(crate) const CLIENT_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const SHUTDOWN_DELIVERY: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireErrorCode {
    InvalidShape,
    InvalidArgs,
    InvalidUtf8,
    InvalidJson,
    FrameTooLarge,
    VersionMismatch,
    HelloRequired,
    AlreadyHello,
    DuplicateCallId,
    UnsupportedOperation,
    Busy,
    ClientNotReady,
    RecoveryUnavailable,
    RevisionConflict,
    UnknownItem,
    InvalidPresentation,
    WorkerStopped,
    ResponseTooLarge,
    InternalError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodeError {
    code: WireErrorCode,
    call_id: Option<String>,
    message: &'static str,
    fatal: bool,
}

impl DecodeError {
    fn fatal(code: WireErrorCode, message: &'static str) -> Self {
        Self {
            code,
            call_id: None,
            message,
            fatal: true,
        }
    }

    fn correlated(code: WireErrorCode, call_id: String, message: &'static str) -> Self {
        Self {
            code,
            call_id: Some(call_id),
            message,
            fatal: false,
        }
    }
}

#[derive(Debug)]
struct DecodedRequest {
    call_id: String,
    request: WorkbenchRequest,
    lab: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEnvelope {
    v: u32,
    #[serde(rename = "type")]
    message_type: String,
    call_id: String,
    op: String,
    args: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabArgs {
    op: String,
    args: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedWire {
    workbench_id: String,
    revision: String,
}

impl ExpectedWire {
    fn into_dispatch(self) -> Result<PresentationExpectation, ()> {
        Ok(PresentationExpectation {
            workbench_id: self.workbench_id,
            revision: canonical_u64(&self.revision)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryExpectedWire {
    workbench_id: String,
    recovery_generation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryGetWire {
    kind: String,
    index: String,
    expected: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestIdentityWire {
    scope: String,
    seq: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusTargetWire {
    workbench_id: String,
    recovery_generation: String,
    boot_id: String,
    request_id: RequestIdentityWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusWire {
    target: StatusTargetWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddPlotWire {
    expected: ExpectedWire,
    plot: Plot,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovePlotWire {
    expected: ExpectedWire,
    plot_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTraceWire {
    expected: ExpectedWire,
    plot_id: String,
    trace: Trace,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveTraceWire {
    expected: ExpectedWire,
    plot_id: String,
    trace_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceVisibilityWire {
    expected: ExpectedWire,
    plot_id: String,
    trace_id: String,
    visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeWindowWire {
    expected: ExpectedWire,
    plot_id: String,
    seconds: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameWire {
    expected: ExpectedWire,
    item_id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceSourceWire {
    expected: ExpectedWire,
    plot_id: String,
    trace_id: String,
    source: RuntimeRef,
}

fn canonical_u64(text: &str) -> Result<u64, ()> {
    if text == "0" {
        return Ok(0);
    }
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    text.parse().map_err(|_| ())
}

fn strict_args<T: for<'de> Deserialize<'de>>(args: Value, call_id: &str) -> Result<T, DecodeError> {
    serde_json::from_value(args).map_err(|_| {
        DecodeError::correlated(
            WireErrorCode::InvalidArgs,
            call_id.to_owned(),
            "Workbench operation arguments are invalid",
        )
    })
}

fn decode_request(body: &[u8]) -> Result<DecodedRequest, DecodeError> {
    if body.len() > JSON_BODY_BYTES {
        return Err(DecodeError::fatal(
            WireErrorCode::FrameTooLarge,
            "Workbench JSON body is too large",
        ));
    }
    std::str::from_utf8(body).map_err(|_| {
        DecodeError::fatal(WireErrorCode::InvalidUtf8, "Workbench frame is not UTF-8")
    })?;
    let bounded = bounded_json(body)?;
    if !bounded.is_object() {
        return Err(DecodeError::fatal(
            WireErrorCode::InvalidShape,
            "Workbench request root must be an object",
        ));
    }
    let envelope: RequestEnvelope = serde_json::from_value(bounded).map_err(|_| {
        DecodeError::fatal(
            WireErrorCode::InvalidShape,
            "Workbench request envelope is invalid",
        )
    })?;
    let call_id = envelope.call_id;
    if call_id.is_empty() || call_id.len() > NAME_BYTES {
        return Err(DecodeError::fatal(
            WireErrorCode::InvalidShape,
            "Workbench call_id is invalid",
        ));
    }
    if envelope.v != 1 {
        return Err(DecodeError::correlated(
            WireErrorCode::VersionMismatch,
            call_id,
            "Workbench protocol version is unsupported",
        ));
    }
    if envelope.message_type != "request" {
        return Err(DecodeError::correlated(
            WireErrorCode::InvalidShape,
            call_id,
            "Workbench message type must be request",
        ));
    }
    if envelope.op.is_empty() || envelope.op.len() > NAME_BYTES {
        return Err(DecodeError::correlated(
            WireErrorCode::UnsupportedOperation,
            call_id,
            "Workbench operation is unsupported",
        ));
    }
    if !envelope.args.is_object() {
        return Err(DecodeError::correlated(
            WireErrorCode::InvalidArgs,
            call_id,
            "Workbench operation arguments must be an object",
        ));
    }

    let op = envelope.op.as_str();
    let request = match op {
        "hello" => {
            let _: EmptyArgs = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::Hello
        }
        "client_status" => {
            let _: EmptyArgs = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::ClientStatus
        }
        "recovery_get" => {
            let args: RecoveryGetWire = strict_args(envelope.args, &call_id)?;
            let kind = match args.kind.as_str() {
                "active" => RecoveryKind::Active,
                "quarantined" => RecoveryKind::Quarantined,
                _ => return invalid_args(call_id),
            };
            let index = canonical_u64(&args.index).map_err(|_| invalid_args_value(&call_id))?;
            let expected = if args.expected.is_null() {
                None
            } else {
                let expected: RecoveryExpectedWire = serde_json::from_value(args.expected)
                    .map_err(|_| invalid_args_value(&call_id))?;
                Some(RecoveryExpectation {
                    workbench_id: expected.workbench_id,
                    recovery_generation: canonical_u64(&expected.recovery_generation)
                        .map_err(|_| invalid_args_value(&call_id))?,
                })
            };
            WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                kind,
                index,
                expected,
            })
        }
        "lab_query" | "lab_mutation" => {
            let args: LabArgs = strict_args(envelope.args, &call_id)?;
            if args.op.is_empty() || args.op.len() > NAME_BYTES || !args.args.is_object() {
                return invalid_args(call_id);
            }
            if op == "lab_query" {
                WorkbenchRequest::LabQuery {
                    op: args.op,
                    args: args.args,
                }
            } else {
                WorkbenchRequest::LabMutation {
                    op: args.op,
                    args: args.args,
                }
            }
        }
        "lab_operation_status" => {
            let args: StatusWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::LabOperationStatus {
                target: RecoveryStatusTarget {
                    workbench_id: args.target.workbench_id,
                    recovery_generation: canonical_u64(&args.target.recovery_generation)
                        .map_err(|_| invalid_args_value(&call_id))?,
                    boot_id: args.target.boot_id,
                    request_id: MutationIdentity {
                        scope: args.target.request_id.scope,
                        seq: canonical_u64(&args.target.request_id.seq)
                            .map_err(|_| invalid_args_value(&call_id))?,
                    },
                },
            }
        }
        "presentation_get" => {
            let _: EmptyArgs = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::PresentationGet
        }
        "ui_add_plot" => {
            let args: AddPlotWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiAddPlot {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot: args.plot,
            }
        }
        "ui_remove_plot" => {
            let args: RemovePlotWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiRemovePlot {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
            }
        }
        "ui_add_trace" => {
            let args: AddTraceWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiAddTrace {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
                trace: args.trace,
            }
        }
        "ui_remove_trace" => {
            let args: RemoveTraceWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiRemoveTrace {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
                trace_id: args.trace_id,
            }
        }
        "ui_set_trace_visibility" => {
            let args: TraceVisibilityWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiSetTraceVisibility {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
                trace_id: args.trace_id,
                visible: args.visible,
            }
        }
        "ui_set_time_window" => {
            let args: TimeWindowWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiSetTimeWindow {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
                seconds: args.seconds,
            }
        }
        "ui_rename_item" => {
            let args: RenameWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiRenameItem {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                item_id: args.item_id,
                label: args.label,
            }
        }
        "ui_set_trace_source" => {
            let args: TraceSourceWire = strict_args(envelope.args, &call_id)?;
            WorkbenchRequest::UiSetTraceSource {
                expected: args
                    .expected
                    .into_dispatch()
                    .map_err(|_| invalid_args_value(&call_id))?,
                plot_id: args.plot_id,
                trace_id: args.trace_id,
                source: args.source,
            }
        }
        _ => {
            return Err(DecodeError::correlated(
                WireErrorCode::UnsupportedOperation,
                call_id,
                "Workbench operation is unsupported",
            ));
        }
    };
    let lab = matches!(
        request,
        WorkbenchRequest::LabQuery { .. }
            | WorkbenchRequest::LabMutation { .. }
            | WorkbenchRequest::LabOperationStatus { .. }
    );
    Ok(DecodedRequest {
        call_id,
        request,
        lab,
    })
}

fn invalid_args<T>(call_id: String) -> Result<T, DecodeError> {
    Err(invalid_args_value(&call_id))
}

fn invalid_args_value(call_id: &str) -> DecodeError {
    DecodeError::correlated(
        WireErrorCode::InvalidArgs,
        call_id.to_owned(),
        "Workbench operation arguments are invalid",
    )
}

struct JsonBounds {
    count: Cell<usize>,
}

impl JsonBounds {
    fn charge<E: de::Error>(&self, amount: usize) -> Result<(), E> {
        let count = self
            .count
            .get()
            .checked_add(amount)
            .ok_or_else(|| E::custom("JSON value bound exceeded"))?;
        if count > JSON_VALUES {
            return Err(E::custom("JSON value bound exceeded"));
        }
        self.count.set(count);
        Ok(())
    }
}

struct JsonSeed<'a> {
    bounds: &'a JsonBounds,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.bounds.charge(1)?;
        deserializer.deserialize_any(JsonVisitor {
            bounds: self.bounds,
            depth: self.depth,
        })
    }
}

struct JsonVisitor<'a> {
    bounds: &'a JsonBounds,
    depth: usize,
}

impl<'de> Visitor<'de> for JsonVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded JSON")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        if !value.is_finite() {
            return Err(E::custom("nonfinite JSON number"));
        }
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite JSON number"))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        if value.len() > JSON_STRING_BYTES {
            return Err(E::custom("JSON string bound exceeded"));
        }
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        if value.len() > JSON_STRING_BYTES {
            return Err(E::custom("JSON string bound exceeded"));
        }
        Ok(Value::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        if self.depth > JSON_DEPTH {
            return Err(de::Error::custom("JSON depth bound exceeded"));
        }
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(JsonSeed {
            bounds: self.bounds,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        if self.depth > JSON_DEPTH {
            return Err(de::Error::custom("JSON depth bound exceeded"));
        }
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if key.len() > JSON_STRING_BYTES {
                return Err(de::Error::custom("JSON key bound exceeded"));
            }
            self.bounds.charge(1)?;
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON key"));
            }
            let value = object.next_value_seed(JsonSeed {
                bounds: self.bounds,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

fn bounded_json(body: &[u8]) -> Result<Value, DecodeError> {
    let bounds = JsonBounds {
        count: Cell::new(0),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let value = JsonSeed {
        bounds: &bounds,
        depth: 1,
    }
    .deserialize(&mut deserializer)
    .map_err(|_| {
        DecodeError::fatal(
            WireErrorCode::InvalidJson,
            "Workbench frame is not valid bounded JSON",
        )
    })?;
    deserializer.end().map_err(|_| {
        DecodeError::fatal(
            WireErrorCode::InvalidJson,
            "Workbench frame contains trailing JSON data",
        )
    })?;
    Ok(value)
}

fn bounded_message(message: &'static str) -> &'static str {
    debug_assert!(message.len() <= ERROR_MESSAGE_BYTES);
    message
}

fn error_frame(call_id: Option<&str>, code: WireErrorCode, message: &'static str) -> Vec<u8> {
    encode_frame(&json!({
        "v": 1,
        "type": "error",
        "call_id": call_id,
        "error": {
            "domain": "workbench",
            "code": code,
            "message": bounded_message(message),
            "retryable": code == WireErrorCode::Busy,
            "resync_required": code == WireErrorCode::RevisionConflict,
        }
    }))
    .expect("fixed bounded error envelope serializes")
}

fn result_frame(call_id: &str, result: &WorkbenchResult) -> Result<Vec<u8>, WireErrorCode> {
    encode_frame(&json!({"v":1,"type":"result","call_id":call_id,"result":result}))
}

fn event_frame(event: &RoutedWorkbenchEvent) -> Result<Vec<u8>, WireErrorCode> {
    let Value::Object(event_fields) =
        serde_json::to_value(&event.event).map_err(|_| WireErrorCode::InternalError)?
    else {
        return Err(WireErrorCode::InternalError);
    };
    let mut envelope = Map::new();
    envelope.insert("v".into(), Value::from(1));
    envelope.insert("type".into(), Value::String("event".into()));
    envelope.insert(
        "workbench_id".into(),
        Value::String(event.workbench_id.clone()),
    );
    envelope.extend(event_fields);
    encode_frame(&Value::Object(envelope))
}

fn encode_frame(value: &Value) -> Result<Vec<u8>, WireErrorCode> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| WireErrorCode::InternalError)?;
    if bytes.len() > JSON_BODY_BYTES {
        return Err(WireErrorCode::ResponseTooLarge);
    }
    bytes.push(b'\n');
    if bytes.len() > FRAME_BYTES {
        return Err(WireErrorCode::ResponseTooLarge);
    }
    Ok(bytes)
}

fn dispatch_error(error: &WorkbenchDispatchError) -> WireErrorCode {
    match error.code {
        WorkbenchErrorCode::InvalidArgs => WireErrorCode::InvalidArgs,
        WorkbenchErrorCode::Busy => WireErrorCode::Busy,
        WorkbenchErrorCode::ClientNotReady => WireErrorCode::ClientNotReady,
        WorkbenchErrorCode::RecoveryUnavailable => WireErrorCode::RecoveryUnavailable,
        WorkbenchErrorCode::RevisionConflict => WireErrorCode::RevisionConflict,
        WorkbenchErrorCode::UnknownItem => WireErrorCode::UnknownItem,
        WorkbenchErrorCode::InvalidPresentation => WireErrorCode::InvalidPresentation,
        WorkbenchErrorCode::WorkerStopped => WireErrorCode::WorkerStopped,
        WorkbenchErrorCode::InternalError => WireErrorCode::InternalError,
    }
}

/// A loopback listener bound before the GUI owner starts its fixed network thread.
#[derive(Debug)]
pub(crate) struct PreparedEndpoint {
    listener: TcpListener,
    address: SocketAddrV4,
}

impl PreparedEndpoint {
    pub(crate) fn bind(address: SocketAddrV4) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Workbench endpoint requires numeric IPv4 loopback",
            ));
        }
        let listener = TcpListener::bind(SocketAddr::V4(address))?;
        listener.set_nonblocking(true)?;
        let SocketAddr::V4(actual) = listener.local_addr()? else {
            return Err(io::Error::other("Workbench listener was not IPv4"));
        };
        if !actual.ip().is_loopback() {
            return Err(io::Error::other("Workbench listener was not loopback"));
        }
        Ok(Self {
            listener,
            address: actual,
        })
    }

    pub(crate) fn address(&self) -> SocketAddrV4 {
        self.address
    }

    pub(crate) fn readiness_line(&self) -> String {
        json!({"workbench_endpoint":self.address().to_string()}).to_string()
    }
}

#[derive(Debug)]
struct OwnerRequest {
    caller_id: CallerId,
    call_id: String,
    request: WorkbenchRequest,
    frame_bytes: usize,
    lab: bool,
}

#[derive(Clone, Debug)]
struct NetworkOutput {
    caller_id: CallerId,
    bytes: Vec<u8>,
    terminal_call_id: Option<String>,
    hello_completion: Option<bool>,
    input_release_bytes: usize,
}

#[derive(Default)]
struct SharedCaller {
    connected: bool,
    detach_requested: bool,
    detach_pending: bool,
    owner_detached: bool,
    hello_complete: bool,
    input_messages: usize,
    input_bytes: usize,
    output_messages: usize,
    output_bytes: usize,
}

#[derive(Default)]
struct SharedState {
    callers: BTreeMap<CallerId, SharedCaller>,
    network_to_owner_messages: usize,
    network_to_owner_bytes: usize,
    owner_to_network_messages: usize,
    owner_to_network_bytes: usize,
}

impl SharedState {
    fn register(&mut self, caller_id: CallerId) -> bool {
        if self.callers.len() == CALLERS {
            return false;
        }
        self.callers.insert(
            caller_id,
            SharedCaller {
                connected: true,
                ..SharedCaller::default()
            },
        );
        true
    }

    fn reserve_input(&mut self, caller_id: CallerId, bytes: usize) -> bool {
        let Some(caller) = self.callers.get_mut(&caller_id) else {
            return false;
        };
        if !caller.connected
            || caller.input_messages == CALLER_INPUT_MESSAGES
            || caller.input_bytes.saturating_add(bytes) > CALLER_INPUT_BYTES
            || self.network_to_owner_messages == OWNER_MAILBOX_MESSAGES
            || self.network_to_owner_bytes.saturating_add(bytes) > OWNER_MAILBOX_BYTES
        {
            return false;
        }
        caller.input_messages += 1;
        caller.input_bytes += bytes;
        self.network_to_owner_messages += 1;
        self.network_to_owner_bytes += bytes;
        true
    }

    fn release_owner_input(&mut self, bytes: usize) {
        self.network_to_owner_messages = self.network_to_owner_messages.saturating_sub(1);
        self.network_to_owner_bytes = self.network_to_owner_bytes.saturating_sub(bytes);
    }

    fn release_caller_input(&mut self, caller_id: CallerId, bytes: usize) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.input_messages = caller.input_messages.saturating_sub(1);
            caller.input_bytes = caller.input_bytes.saturating_sub(bytes);
        }
        self.cleanup(caller_id);
    }

    fn reserve_owner_output(&mut self, caller_id: CallerId, bytes: usize) -> bool {
        let Some(caller) = self.callers.get_mut(&caller_id) else {
            return false;
        };
        if !caller.connected
            || caller.output_messages == CALLER_OUTPUT_MESSAGES
            || caller.output_bytes.saturating_add(bytes) > CALLER_OUTPUT_BYTES
            || self.owner_to_network_messages == OWNER_MAILBOX_MESSAGES
            || self.owner_to_network_bytes.saturating_add(bytes) > OWNER_MAILBOX_BYTES
        {
            return false;
        }
        caller.output_messages += 1;
        caller.output_bytes += bytes;
        self.owner_to_network_messages += 1;
        self.owner_to_network_bytes += bytes;
        true
    }

    fn reserve_network_output(&mut self, caller_id: CallerId, bytes: usize) -> bool {
        let Some(caller) = self.callers.get_mut(&caller_id) else {
            return false;
        };
        if !caller.connected
            || caller.output_messages == CALLER_OUTPUT_MESSAGES
            || caller.output_bytes.saturating_add(bytes) > CALLER_OUTPUT_BYTES
        {
            return false;
        }
        caller.output_messages += 1;
        caller.output_bytes += bytes;
        true
    }

    fn undo_owner_output(&mut self, caller_id: CallerId, bytes: usize) {
        self.owner_to_network_messages = self.owner_to_network_messages.saturating_sub(1);
        self.owner_to_network_bytes = self.owner_to_network_bytes.saturating_sub(bytes);
        self.release_output(caller_id, bytes);
    }

    fn receive_owner_output(&mut self, bytes: usize) {
        self.owner_to_network_messages = self.owner_to_network_messages.saturating_sub(1);
        self.owner_to_network_bytes = self.owner_to_network_bytes.saturating_sub(bytes);
    }

    fn release_output(&mut self, caller_id: CallerId, bytes: usize) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.output_messages = caller.output_messages.saturating_sub(1);
            caller.output_bytes = caller.output_bytes.saturating_sub(bytes);
        }
        self.cleanup(caller_id);
    }

    fn request_detach(&mut self, caller_id: CallerId, owner_already_detached: bool) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.detach_requested = true;
            caller.owner_detached |= owner_already_detached;
        }
    }

    fn network_detached(&mut self, caller_id: CallerId) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.connected = false;
            caller.detach_pending = true;
            caller.input_messages = 0;
            caller.input_bytes = 0;
            caller.output_messages = 0;
            caller.output_bytes = 0;
        }
        self.cleanup(caller_id);
    }

    fn detach_requests(&self) -> Vec<CallerId> {
        self.callers
            .iter()
            .filter_map(|(id, caller)| {
                (caller.detach_pending && !caller.owner_detached).then_some(*id)
            })
            .take(CALLERS)
            .collect()
    }

    fn acknowledge_detach(&mut self, caller_id: CallerId) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.owner_detached = true;
        }
        self.cleanup(caller_id);
    }

    fn active(&self, caller_id: CallerId) -> bool {
        self.callers
            .get(&caller_id)
            .is_some_and(|caller| caller.connected && !caller.detach_requested)
    }

    fn broadcast_callers(&self) -> Vec<CallerId> {
        self.callers
            .iter()
            .filter_map(|(id, caller)| {
                (caller.connected && !caller.detach_requested && caller.hello_complete)
                    .then_some(*id)
            })
            .take(CALLERS)
            .collect()
    }

    fn mark_hello_complete(&mut self, caller_id: CallerId) {
        if let Some(caller) = self.callers.get_mut(&caller_id) {
            caller.hello_complete = true;
        }
    }

    fn cleanup(&mut self, caller_id: CallerId) {
        let remove = self.callers.get(&caller_id).is_some_and(|caller| {
            !caller.connected
                && caller.owner_detached
                && caller.output_messages == 0
                && caller.input_messages == 0
        });
        if remove {
            self.callers.remove(&caller_id);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelloGate {
    Required,
    Pending,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExchangeKind {
    Local,
    Lab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CorrelationReserveError {
    Duplicate,
    PerCallerReserved,
    GlobalReserved,
    PerCallerLab,
    GlobalLab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CorrelationCompletion {
    Terminal(String),
    Related(String),
}

struct LiveCorrelation {
    kind: ExchangeKind,
    terminal_written: bool,
    related_writes: usize,
}

#[derive(Default)]
struct GlobalCorrelation {
    reserved: usize,
    lab: usize,
}

struct CorrelationState {
    calls: BTreeMap<String, LiveCorrelation>,
}

impl CorrelationState {
    fn new() -> Self {
        Self {
            calls: BTreeMap::new(),
        }
    }

    fn contains(&self, call_id: &str) -> bool {
        self.calls.contains_key(call_id)
    }

    fn reserve(
        &mut self,
        call_id: &str,
        lab: bool,
        global: &mut GlobalCorrelation,
    ) -> Result<(), CorrelationReserveError> {
        if self.contains(call_id) {
            return Err(CorrelationReserveError::Duplicate);
        }
        if lab && self.lab_count() == OUTSTANDING_LAB_PER_CALLER {
            return Err(CorrelationReserveError::PerCallerLab);
        }
        if lab && global.lab == OUTSTANDING_LAB_TOTAL {
            return Err(CorrelationReserveError::GlobalLab);
        }
        if self.calls.len() == RESERVED_CALL_IDS_PER_CALLER {
            return Err(CorrelationReserveError::PerCallerReserved);
        }
        if global.reserved == RESERVED_CALL_IDS_TOTAL {
            return Err(CorrelationReserveError::GlobalReserved);
        }
        self.calls.insert(
            call_id.to_owned(),
            LiveCorrelation {
                kind: if lab {
                    ExchangeKind::Lab
                } else {
                    ExchangeKind::Local
                },
                terminal_written: false,
                related_writes: 0,
            },
        );
        global.reserved += 1;
        if lab {
            global.lab += 1;
        }
        Ok(())
    }

    fn hold_related_write(&mut self, call_id: &str) {
        let call = self
            .calls
            .get_mut(call_id)
            .expect("duplicate response retains an existing correlation");
        call.related_writes = call
            .related_writes
            .checked_add(1)
            .expect("bounded related-write count cannot overflow");
    }

    fn complete(&mut self, completion: CorrelationCompletion, global: &mut GlobalCorrelation) {
        match completion {
            CorrelationCompletion::Terminal(call_id) => {
                if let Some(call) = self.calls.get_mut(&call_id) {
                    call.terminal_written = true;
                    if call.related_writes == 0 {
                        self.release_live(&call_id, global);
                    }
                }
            }
            CorrelationCompletion::Related(call_id) => {
                if let Some(call) = self.calls.get_mut(&call_id) {
                    call.related_writes = call
                        .related_writes
                        .checked_sub(1)
                        .expect("related live-correlation write completes exactly once");
                    if call.terminal_written && call.related_writes == 0 {
                        self.release_live(&call_id, global);
                    }
                }
            }
        }
    }

    fn lab_count(&self) -> usize {
        self.calls
            .values()
            .filter(|call| call.kind == ExchangeKind::Lab)
            .count()
    }

    fn release_live(&mut self, call_id: &str, global: &mut GlobalCorrelation) {
        let Some(call) = self.calls.remove(call_id) else {
            return;
        };
        global.reserved = global
            .reserved
            .checked_sub(1)
            .expect("live correlation releases global credit exactly once");
        if call.kind == ExchangeKind::Lab {
            global.lab = global
                .lab
                .checked_sub(1)
                .expect("lab correlation releases global credit exactly once");
        }
    }

    fn detach(&mut self, global: &mut GlobalCorrelation) {
        for (_, call) in std::mem::take(&mut self.calls) {
            global.reserved = global
                .reserved
                .checked_sub(1)
                .expect("detached correlation releases global credit exactly once");
            if call.kind == ExchangeKind::Lab {
                global.lab = global
                    .lab
                    .checked_sub(1)
                    .expect("detached lab correlation releases global credit exactly once");
            }
        }
    }
}

struct QueuedWrite {
    bytes: Vec<u8>,
    offset: usize,
    correlation_completion: Option<CorrelationCompletion>,
    hello_completion: Option<bool>,
    input_release_bytes: usize,
    deadline: Instant,
}

struct Connection {
    id: CallerId,
    stream: TcpStream,
    input: Vec<u8>,
    partial_deadline: Option<Instant>,
    hello_deadline: Option<Instant>,
    hello: HelloGate,
    writes: VecDeque<QueuedWrite>,
    correlation: CorrelationState,
}

impl Connection {
    fn new(id: CallerId, stream: TcpStream, now: Instant) -> Self {
        Self {
            id,
            stream,
            input: Vec::with_capacity(SOCKET_BYTES_PER_TURN),
            partial_deadline: None,
            hello_deadline: Some(now + CLIENT_DEADLINE),
            hello: HelloGate::Required,
            writes: VecDeque::with_capacity(CALLER_OUTPUT_MESSAGES),
            correlation: CorrelationState::new(),
        }
    }

    fn timed_out(&self, now: Instant) -> bool {
        self.partial_deadline
            .is_some_and(|deadline| now >= deadline)
            || self.hello_deadline.is_some_and(|deadline| now >= deadline)
            || self
                .writes
                .front()
                .is_some_and(|write| now >= write.deadline)
    }
}

/// GUI-owner half of the fixed one-thread external endpoint.
pub(crate) struct WorkbenchEndpoint {
    request_rx: Receiver<OwnerRequest>,
    output_tx: SyncSender<NetworkOutput>,
    shared: Arc<Mutex<SharedState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    owner_wake: Arc<dyn Fn() + Send + Sync>,
    pending: VecDeque<OwnerRequest>,
    fairness_cursor: u64,
}

impl WorkbenchEndpoint {
    pub(crate) fn start(
        prepared: PreparedEndpoint,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let (output_tx, output_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_shared = Arc::clone(&shared);
        let worker_stop = Arc::clone(&stop);
        let owner_wake = Arc::clone(&wake);
        let worker = thread::Builder::new()
            .name("lab-workbench-external".into())
            .spawn(move || {
                network_loop(
                    prepared.listener,
                    request_tx,
                    output_rx,
                    worker_shared,
                    worker_stop,
                    wake,
                );
            })?;
        Ok(Self {
            request_rx,
            output_tx,
            shared,
            stop,
            worker: Some(worker),
            owner_wake,
            pending: VecDeque::with_capacity(OWNER_MAILBOX_MESSAGES),
            fairness_cursor: 0,
        })
    }

    pub(crate) fn service_owner<C: LabClient>(
        &mut self,
        dispatcher: &mut WorkbenchDispatcher<C>,
    ) -> usize {
        self.service_detaches(dispatcher);
        while self.pending.len() < OWNER_MAILBOX_MESSAGES {
            match self.request_rx.try_recv() {
                Ok(request) => self.pending.push_back(request),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        let mut caller_ids = self
            .pending
            .iter()
            .map(|request| request.caller_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        caller_ids.sort_by_key(|id| (id.0 <= self.fairness_cursor, id.0));
        caller_ids.truncate(OWNER_CALLERS_PER_TURN);
        let mut serviced = 0;
        for caller_id in caller_ids {
            let Some(index) = self
                .pending
                .iter()
                .position(|request| request.caller_id == caller_id)
            else {
                continue;
            };
            let request = self
                .pending
                .remove(index)
                .expect("located bounded owner request remains queued");
            self.shared
                .lock()
                .expect("external admission lock")
                .release_owner_input(request.frame_bytes);
            if !self
                .shared
                .lock()
                .expect("external admission lock")
                .active(caller_id)
            {
                continue;
            }
            self.dispatch_one(dispatcher, request);
            self.fairness_cursor = caller_id.0;
            serviced += 1;
        }
        self.service_detaches(dispatcher);
        // Input admission charges this counter before publishing to `request_rx`.
        // Reading it after the bounded dispatch turn therefore also covers work
        // that arrived after the initial channel drain. An admission that races
        // after this check supplies its own network-thread wake.
        let admitted_owner_work_remains = self
            .shared
            .lock()
            .expect("external admission lock")
            .network_to_owner_messages
            != 0;
        if !self.pending.is_empty() || admitted_owner_work_remains {
            (self.owner_wake)();
        }
        serviced
    }

    pub(crate) fn route_events<C: LabClient>(
        &mut self,
        dispatcher: &mut WorkbenchDispatcher<C>,
        events: impl IntoIterator<Item = RoutedWorkbenchEvent>,
    ) {
        for event in events {
            let frame = match event_frame(&event) {
                Ok(frame) => frame,
                Err(_) => continue,
            };
            match event.route {
                EventRoute::Caller(caller_id) => {
                    let terminal_call_id = event
                        .lab_correlation()
                        .and_then(|(call_id, terminal)| terminal.then(|| call_id.to_owned()));
                    self.send_owner_frame(
                        dispatcher,
                        NetworkOutput {
                            caller_id,
                            bytes: frame,
                            terminal_call_id,
                            hello_completion: None,
                            input_release_bytes: 0,
                        },
                    );
                }
                EventRoute::Broadcast => {
                    let callers = self
                        .shared
                        .lock()
                        .expect("external admission lock")
                        .broadcast_callers();
                    for caller_id in callers {
                        self.send_owner_frame(
                            dispatcher,
                            NetworkOutput {
                                caller_id,
                                bytes: frame.clone(),
                                terminal_call_id: None,
                                hello_completion: None,
                                input_release_bytes: 0,
                            },
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn shutdown<C: LabClient>(&mut self, dispatcher: &mut WorkbenchDispatcher<C>) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let callers = self
            .shared
            .lock()
            .expect("external admission lock")
            .callers
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for caller_id in callers {
            dispatcher.detach_caller(caller_id);
        }
        self.pending.clear();
        self.shared
            .lock()
            .expect("external admission lock")
            .callers
            .clear();
    }

    fn dispatch_one<C: LabClient>(
        &mut self,
        dispatcher: &mut WorkbenchDispatcher<C>,
        request: OwnerRequest,
    ) {
        let caller_id = request.caller_id;
        let call_id = request.call_id;
        let is_hello = matches!(request.request, WorkbenchRequest::Hello);
        match dispatcher.dispatch(
            CallOrigin {
                caller_id,
                call_id: call_id.clone(),
            },
            request.request,
        ) {
            Ok(outcome) => {
                let terminal = !request.lab;
                let frame = result_frame(&call_id, &outcome.result).unwrap_or_else(|code| {
                    error_frame(
                        Some(&call_id),
                        code,
                        "Workbench response exceeded its bound",
                    )
                });
                self.send_owner_frame(
                    dispatcher,
                    NetworkOutput {
                        caller_id,
                        bytes: frame,
                        terminal_call_id: terminal.then_some(call_id),
                        hello_completion: is_hello.then_some(true),
                        input_release_bytes: request.frame_bytes,
                    },
                );
                self.route_events(dispatcher, outcome.events);
            }
            Err(error) => {
                let code = dispatch_error(&error);
                let frame = error_frame(Some(&call_id), code, error.message);
                self.send_owner_frame(
                    dispatcher,
                    NetworkOutput {
                        caller_id,
                        bytes: frame,
                        terminal_call_id: Some(call_id),
                        hello_completion: is_hello.then_some(false),
                        input_release_bytes: request.frame_bytes,
                    },
                );
            }
        }
    }

    fn send_owner_frame<C: LabClient>(
        &mut self,
        dispatcher: &mut WorkbenchDispatcher<C>,
        output: NetworkOutput,
    ) {
        let bytes = output.bytes.len();
        let caller_id = output.caller_id;
        if !self
            .shared
            .lock()
            .expect("external admission lock")
            .reserve_owner_output(caller_id, bytes)
        {
            self.detach_owner(dispatcher, caller_id);
            return;
        }
        if let Err(error) = self.output_tx.try_send(output) {
            self.shared
                .lock()
                .expect("external admission lock")
                .undo_owner_output(caller_id, bytes);
            match error {
                TrySendError::Full(_) | TrySendError::Disconnected(_) => {
                    self.detach_owner(dispatcher, caller_id)
                }
            }
        }
    }

    fn detach_owner<C: LabClient>(
        &mut self,
        dispatcher: &mut WorkbenchDispatcher<C>,
        caller_id: CallerId,
    ) {
        self.shared
            .lock()
            .expect("external admission lock")
            .request_detach(caller_id, true);
        dispatcher.detach_caller(caller_id);
    }

    fn service_detaches<C: LabClient>(&mut self, dispatcher: &mut WorkbenchDispatcher<C>) {
        let callers = self
            .shared
            .lock()
            .expect("external admission lock")
            .detach_requests();
        for caller_id in callers {
            dispatcher.detach_caller(caller_id);
            self.shared
                .lock()
                .expect("external admission lock")
                .acknowledge_detach(caller_id);
        }
    }
}

impl Drop for WorkbenchEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn network_loop(
    listener: TcpListener,
    request_tx: SyncSender<OwnerRequest>,
    output_rx: Receiver<NetworkOutput>,
    shared: Arc<Mutex<SharedState>>,
    stop: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut connections = BTreeMap::new();
    let mut next_caller_id = 1u64;
    let mut correlation = GlobalCorrelation::default();
    let mut shutdown_deadline = None;

    loop {
        let now = Instant::now();
        if stop.load(Ordering::Acquire) && shutdown_deadline.is_none() {
            shutdown_deadline = Some(now + SHUTDOWN_DELIVERY);
        }
        let shutting_down = shutdown_deadline.is_some();

        if !shutting_down {
            accept_callers(
                &listener,
                &mut connections,
                &mut next_caller_id,
                &shared,
                now,
            );
        }
        receive_owner_outputs(&output_rx, &mut connections, &shared, now);

        let caller_ids = connections.keys().copied().collect::<Vec<_>>();
        for caller_id in caller_ids {
            let detach_requested = shared
                .lock()
                .expect("external admission lock")
                .callers
                .get(&caller_id)
                .is_some_and(|caller| caller.detach_requested);
            let detach = if detach_requested {
                true
            } else if let Some(connection) = connections.get_mut(&caller_id) {
                if connection.timed_out(now) {
                    true
                } else {
                    let write_failed = write_caller(connection, &shared, &mut correlation, now);
                    let read_failed = !shutting_down
                        && !write_failed
                        && read_caller(
                            connection,
                            &request_tx,
                            &shared,
                            &mut correlation,
                            &wake,
                            now,
                        );
                    write_failed || read_failed
                }
            } else {
                false
            };
            if detach {
                detach_connection(caller_id, &mut connections, &shared, &mut correlation);
                wake();
            }
        }

        if shutdown_deadline.is_some_and(|deadline| now >= deadline)
            || (shutting_down
                && connections
                    .values()
                    .all(|connection| connection.writes.is_empty()))
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }

    for caller_id in connections.keys().copied().collect::<Vec<_>>() {
        detach_connection(caller_id, &mut connections, &shared, &mut correlation);
    }
    wake();
}

fn accept_callers(
    listener: &TcpListener,
    connections: &mut BTreeMap<CallerId, Connection>,
    next_caller_id: &mut u64,
    shared: &Arc<Mutex<SharedState>>,
    now: Instant,
) {
    for _ in 0..CALLERS {
        let (stream, peer) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        };
        if !matches!(peer, SocketAddr::V4(address) if address.ip().is_loopback()) {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        let caller_id = CallerId(*next_caller_id);
        let Some(next) = next_caller_id.checked_add(1) else {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        };
        if !shared
            .lock()
            .expect("external admission lock")
            .register(caller_id)
        {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        *next_caller_id = next;
        if stream.set_nonblocking(true).is_err() {
            shared
                .lock()
                .expect("external admission lock")
                .network_detached(caller_id);
            continue;
        }
        let _ = stream.set_nodelay(true);
        connections.insert(caller_id, Connection::new(caller_id, stream, now));
    }
}

fn receive_owner_outputs(
    output_rx: &Receiver<NetworkOutput>,
    connections: &mut BTreeMap<CallerId, Connection>,
    shared: &Arc<Mutex<SharedState>>,
    now: Instant,
) {
    for _ in 0..OWNER_MAILBOX_MESSAGES {
        let output = match output_rx.try_recv() {
            Ok(output) => output,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        };
        let bytes = output.bytes.len();
        shared
            .lock()
            .expect("external admission lock")
            .receive_owner_output(bytes);
        let Some(connection) = connections.get_mut(&output.caller_id) else {
            let mut state = shared.lock().expect("external admission lock");
            state.release_output(output.caller_id, bytes);
            state.release_caller_input(output.caller_id, output.input_release_bytes);
            continue;
        };
        debug_assert!(connection.writes.len() < CALLER_OUTPUT_MESSAGES);
        connection.writes.push_back(QueuedWrite {
            bytes: output.bytes,
            offset: 0,
            correlation_completion: output.terminal_call_id.map(CorrelationCompletion::Terminal),
            hello_completion: output.hello_completion,
            input_release_bytes: output.input_release_bytes,
            deadline: now + CLIENT_DEADLINE,
        });
    }
}

fn write_caller(
    connection: &mut Connection,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
    _now: Instant,
) -> bool {
    let mut budget = SOCKET_BYTES_PER_TURN;
    while budget > 0 {
        let Some(write) = connection.writes.front_mut() else {
            break;
        };
        let remaining = &write.bytes[write.offset..];
        let amount = remaining.len().min(budget);
        match connection.stream.write(&remaining[..amount]) {
            Ok(0) => return true,
            Ok(written) => {
                write.offset += written;
                budget -= written;
                if write.offset == write.bytes.len() {
                    let completed = connection
                        .writes
                        .pop_front()
                        .expect("current bounded write remains queued");
                    let mut state = shared.lock().expect("external admission lock");
                    state.release_output(connection.id, completed.bytes.len());
                    if completed.input_release_bytes != 0 {
                        state.release_caller_input(connection.id, completed.input_release_bytes);
                    }
                    if let Some(success) = completed.hello_completion {
                        if success {
                            connection.hello = HelloGate::Complete;
                            connection.hello_deadline = None;
                            state.mark_hello_complete(connection.id);
                        } else {
                            connection.hello = HelloGate::Required;
                        }
                    }
                    drop(state);
                    if let Some(completion) = completed.correlation_completion {
                        connection.correlation.complete(completion, correlation);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => return true,
        }
    }
    false
}

fn read_caller(
    connection: &mut Connection,
    request_tx: &SyncSender<OwnerRequest>,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
    wake: &Arc<dyn Fn() + Send + Sync>,
    now: Instant,
) -> bool {
    let mut frames = 0;
    while frames < FRAMES_PER_CALLER_TURN {
        match admit_buffered_frame(connection, request_tx, shared, correlation, wake, now) {
            None => break,
            Some(true) => return true,
            Some(false) => frames += 1,
        }
    }
    if frames == FRAMES_PER_CALLER_TURN {
        return false;
    }

    let mut buffer = [0u8; SOCKET_BYTES_PER_TURN];
    let read = match connection.stream.read(&mut buffer) {
        Ok(0) => return true,
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
        Err(_) => return true,
    };

    let mut cursor = 0;
    while cursor < read {
        if frames == FRAMES_PER_CALLER_TURN {
            debug_assert!(connection.input.is_empty());
            let remaining = &buffer[cursor..read];
            if remaining.len() > FRAME_BYTES - 1 {
                return true;
            }
            connection.partial_deadline = Some(now + CLIENT_DEADLINE);
            connection.input.extend_from_slice(remaining);
            break;
        }

        let remaining = &buffer[cursor..read];
        let Some(newline) = remaining.iter().position(|byte| *byte == b'\n') else {
            if connection.input.len().saturating_add(remaining.len()) > FRAME_BYTES - 1 {
                return true;
            }
            if connection.input.is_empty() {
                connection.partial_deadline = Some(now + CLIENT_DEADLINE);
            }
            connection.input.extend_from_slice(remaining);
            break;
        };
        let segment_len = newline + 1;
        if connection.input.len().saturating_add(segment_len) > FRAME_BYTES {
            return true;
        }
        if connection.input.is_empty() {
            connection.partial_deadline = Some(now + CLIENT_DEADLINE);
        }
        connection
            .input
            .extend_from_slice(&remaining[..segment_len]);
        cursor += segment_len;
        match admit_buffered_frame(connection, request_tx, shared, correlation, wake, now) {
            Some(false) => frames += 1,
            Some(true) | None => return true,
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn admit_buffered_frame(
    connection: &mut Connection,
    request_tx: &SyncSender<OwnerRequest>,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
    wake: &Arc<dyn Fn() + Send + Sync>,
    now: Instant,
) -> Option<bool> {
    let newline = connection.input.iter().position(|byte| *byte == b'\n')?;
    let frame_len = newline + 1;
    if frame_len > FRAME_BYTES {
        return Some(true);
    }
    let mut frame = connection.input.drain(..frame_len).collect::<Vec<_>>();
    let delimiter = frame.pop();
    debug_assert_eq!(delimiter, Some(b'\n'));
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
    if frame.len() > JSON_BODY_BYTES {
        return Some(true);
    }
    if connection.input.is_empty() {
        connection.partial_deadline = None;
    }
    Some(admit_frame(
        connection,
        frame,
        frame_len,
        request_tx,
        shared,
        correlation,
        wake,
        now,
    ))
}

#[allow(clippy::too_many_arguments)]
fn admit_frame(
    connection: &mut Connection,
    body: Vec<u8>,
    frame_bytes: usize,
    request_tx: &SyncSender<OwnerRequest>,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
    wake: &Arc<dyn Fn() + Send + Sync>,
    now: Instant,
) -> bool {
    let decoded = match decode_request(&body) {
        Ok(decoded) => decoded,
        Err(error) if error.fatal => return true,
        Err(error) => {
            let Some(call_id) = error.call_id else {
                return true;
            };
            return queue_correlated_error(
                connection,
                call_id,
                false,
                error.code,
                error.message,
                0,
                shared,
                correlation,
                now,
            );
        }
    };

    if connection.correlation.contains(&decoded.call_id) {
        return queue_duplicate_error(connection, &decoded.call_id, shared, now);
    }
    match connection.hello {
        HelloGate::Required if !matches!(decoded.request, WorkbenchRequest::Hello) => {
            return queue_correlated_error(
                connection,
                decoded.call_id,
                false,
                WireErrorCode::HelloRequired,
                "Workbench hello is required first",
                0,
                shared,
                correlation,
                now,
            );
        }
        HelloGate::Pending | HelloGate::Complete
            if matches!(decoded.request, WorkbenchRequest::Hello) =>
        {
            return queue_correlated_error(
                connection,
                decoded.call_id,
                false,
                WireErrorCode::AlreadyHello,
                "Workbench hello was already submitted",
                0,
                shared,
                correlation,
                now,
            );
        }
        HelloGate::Pending if !matches!(decoded.request, WorkbenchRequest::Hello) => {
            return queue_correlated_error(
                connection,
                decoded.call_id,
                false,
                WireErrorCode::HelloRequired,
                "Workbench hello result is not complete",
                0,
                shared,
                correlation,
                now,
            );
        }
        _ => {}
    }

    if let Err(error) = connection
        .correlation
        .reserve(&decoded.call_id, decoded.lab, correlation)
    {
        if error == CorrelationReserveError::Duplicate {
            return queue_duplicate_error(connection, &decoded.call_id, shared, now);
        }
        return true;
    }

    if !shared
        .lock()
        .expect("external admission lock")
        .reserve_input(connection.id, frame_bytes)
    {
        let frame = error_frame(
            Some(&decoded.call_id),
            WireErrorCode::Busy,
            "Workbench admission is full",
        );
        if !queue_write(
            connection,
            frame,
            Some(CorrelationCompletion::Terminal(decoded.call_id)),
            None,
            0,
            shared,
            now,
        ) {
            return true;
        }
        return false;
    }

    let owner_request = OwnerRequest {
        caller_id: connection.id,
        call_id: decoded.call_id.clone(),
        request: decoded.request,
        frame_bytes,
        lab: decoded.lab,
    };
    match request_tx.try_send(owner_request) {
        Ok(()) => {
            if matches!(connection.hello, HelloGate::Required) {
                connection.hello = HelloGate::Pending;
            }
            wake();
            false
        }
        Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
            shared
                .lock()
                .expect("external admission lock")
                .release_owner_input(frame_bytes);
            let frame = error_frame(
                Some(&decoded.call_id),
                WireErrorCode::Busy,
                "Workbench owner mailbox is full",
            );
            !queue_write(
                connection,
                frame,
                Some(CorrelationCompletion::Terminal(decoded.call_id)),
                None,
                frame_bytes,
                shared,
                now,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_correlated_error(
    connection: &mut Connection,
    call_id: String,
    lab: bool,
    code: WireErrorCode,
    message: &'static str,
    input_release_bytes: usize,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
    now: Instant,
) -> bool {
    match connection.correlation.reserve(&call_id, lab, correlation) {
        Ok(()) => {}
        Err(CorrelationReserveError::Duplicate) => {
            return queue_duplicate_error(connection, &call_id, shared, now);
        }
        Err(
            CorrelationReserveError::PerCallerReserved
            | CorrelationReserveError::GlobalReserved
            | CorrelationReserveError::PerCallerLab
            | CorrelationReserveError::GlobalLab,
        ) => return true,
    }
    let frame = error_frame(Some(&call_id), code, message);
    !queue_write(
        connection,
        frame,
        Some(CorrelationCompletion::Terminal(call_id)),
        None,
        input_release_bytes,
        shared,
        now,
    )
}

fn queue_duplicate_error(
    connection: &mut Connection,
    call_id: &str,
    shared: &Arc<Mutex<SharedState>>,
    now: Instant,
) -> bool {
    let frame = error_frame(
        Some(call_id),
        WireErrorCode::DuplicateCallId,
        "Workbench call_id is already reserved",
    );
    if !queue_write(
        connection,
        frame,
        Some(CorrelationCompletion::Related(call_id.to_owned())),
        None,
        0,
        shared,
        now,
    ) {
        return true;
    }
    connection.correlation.hold_related_write(call_id);
    false
}

fn queue_write(
    connection: &mut Connection,
    bytes: Vec<u8>,
    correlation_completion: Option<CorrelationCompletion>,
    hello_completion: Option<bool>,
    input_release_bytes: usize,
    shared: &Arc<Mutex<SharedState>>,
    now: Instant,
) -> bool {
    if !shared
        .lock()
        .expect("external admission lock")
        .reserve_network_output(connection.id, bytes.len())
    {
        return false;
    }
    connection.writes.push_back(QueuedWrite {
        bytes,
        offset: 0,
        correlation_completion,
        hello_completion,
        input_release_bytes,
        deadline: now + CLIENT_DEADLINE,
    });
    true
}

fn detach_connection(
    caller_id: CallerId,
    connections: &mut BTreeMap<CallerId, Connection>,
    shared: &Arc<Mutex<SharedState>>,
    correlation: &mut GlobalCorrelation,
) {
    let Some(mut connection) = connections.remove(&caller_id) else {
        return;
    };
    connection.correlation.detach(correlation);
    let _ = connection.stream.shutdown(Shutdown::Both);
    shared
        .lock()
        .expect("external admission lock")
        .network_detached(caller_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::{
            ClientUpdate,
            types::{
                CommandSendError, ConnectionState, EventCursor, HelloState, KnownAdmission,
                QuarantinedRecoveryRecord, RecoveryQuarantineReason, RecoveryRecord, ReplyKind,
            },
        },
        dispatcher::WorkbenchLimits,
        model::WorkbenchModel,
        presentation::PresentationDocument,
    };
    use std::{cell::RefCell, net::Ipv4Addr, sync::atomic::AtomicUsize};

    #[derive(Default)]
    struct FakeClient {
        submitted: RefCell<Vec<String>>,
        next: Cell<u64>,
    }

    impl LabClient for FakeClient {
        fn query(&self, op: &str, _args: Value) -> Result<u64, CommandSendError> {
            self.submitted.borrow_mut().push(format!("query:{op}"));
            Ok(self.next_id())
        }

        fn mutation(&self, op: &str, _args: Value) -> Result<u64, CommandSendError> {
            self.submitted.borrow_mut().push(format!("mutation:{op}"));
            Ok(self.next_id())
        }

        fn operation_status(&self, _identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.submitted.borrow_mut().push("status".into());
            Ok(self.next_id())
        }
    }

    impl FakeClient {
        fn next_id(&self) -> u64 {
            let next = self.next.get() + 1;
            self.next.set(next);
            next
        }
    }

    fn dispatcher() -> WorkbenchDispatcher<FakeClient> {
        let mut dispatcher = WorkbenchDispatcher::new(
            WorkbenchModel::new(PresentationDocument::empty("main")),
            FakeClient::default(),
        );
        dispatcher.apply_client_update(ClientUpdate::Hello(HelloState {
            boot_id: "0123456789abcdef0123456789abcdef".into(),
            scope: "scope".into(),
            next_seq: 1,
            operations: vec!["reference".into(), "reference_retune".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "0123456789abcdef0123456789abcdef".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "0123456789abcdef0123456789abcdef".into(),
                seq: 0,
            },
        }));
        dispatcher
    }

    fn envelope(call_id: &str, op: &str, args: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "v":1,
            "type":"request",
            "call_id":call_id,
            "op":op,
            "args":args
        }))
        .unwrap()
    }

    fn test_connection(caller_id: CallerId) -> (TcpStream, Connection) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let peer = TcpStream::connect(address).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        (peer, Connection::new(caller_id, stream, Instant::now()))
    }

    fn expected() -> Value {
        json!({
            "workbench_id":"0123456789abcdef0123456789abcdef",
            "revision":"1"
        })
    }

    fn operation_samples() -> Vec<(&'static str, Value)> {
        vec![
            ("hello", json!({})),
            ("client_status", json!({})),
            (
                "recovery_get",
                json!({"kind":"active","index":"0","expected":null}),
            ),
            (
                "lab_query",
                json!({"op":"reference","args":{"reference":"1"}}),
            ),
            (
                "lab_mutation",
                json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
            ),
            (
                "lab_operation_status",
                json!({"target":{"workbench_id":"0123456789abcdef0123456789abcdef","recovery_generation":"1","boot_id":"0123456789abcdef0123456789abcdef","request_id":{"scope":"scope","seq":"1"}}}),
            ),
            ("presentation_get", json!({})),
            (
                "ui_add_plot",
                json!({"expected":expected(),"plot":{"id":"plot","title":"Plot","time_window_seconds":60.0,"axes":{"y_min":null,"y_max":null},"traces":[]}}),
            ),
            (
                "ui_remove_plot",
                json!({"expected":expected(),"plot_id":"plot"}),
            ),
            (
                "ui_add_trace",
                json!({"expected":expected(),"plot_id":"plot","trace":{"id":"trace","source":{"kind":"reference","reference":"1"},"display_label":"Trace","visible":true,"style":{"color":"cyan","width":1.0},"display_unit":null}}),
            ),
            (
                "ui_remove_trace",
                json!({"expected":expected(),"plot_id":"plot","trace_id":"trace"}),
            ),
            (
                "ui_set_trace_visibility",
                json!({"expected":expected(),"plot_id":"plot","trace_id":"trace","visible":false}),
            ),
            (
                "ui_set_time_window",
                json!({"expected":expected(),"plot_id":"plot","seconds":120.0}),
            ),
            (
                "ui_rename_item",
                json!({"expected":expected(),"item_id":"plot","label":"Renamed"}),
            ),
            (
                "ui_set_trace_source",
                json!({"expected":expected(),"plot_id":"plot","trace_id":"trace","source":{"kind":"reference","reference":"2"}}),
            ),
        ]
    }

    #[test]
    fn all_fifteen_operations_decode_to_the_accepted_typed_surface() {
        let samples = operation_samples();
        assert_eq!(samples.len(), 15);
        for (index, (op, args)) in samples.into_iter().enumerate() {
            let decoded = decode_request(&envelope(&format!("call-{index}"), op, args)).unwrap();
            assert_eq!(decoded.request.operation(), op);
        }
    }

    #[test]
    fn endpoint_bounds_match_the_frozen_dispatcher_hello_limits() {
        let limits = serde_json::to_value(WorkbenchLimits::default()).unwrap();
        assert_eq!(limits["json_body_bytes"], JSON_BODY_BYTES);
        assert_eq!(limits["frame_bytes"], FRAME_BYTES);
        assert_eq!(limits["json_depth"], JSON_DEPTH);
        assert_eq!(limits["json_values"], JSON_VALUES);
        assert_eq!(limits["json_string_bytes"], JSON_STRING_BYTES);
        assert_eq!(limits["callers"], CALLERS);
        assert_eq!(limits["caller_input_messages"], CALLER_INPUT_MESSAGES);
        assert_eq!(limits["caller_input_bytes"], CALLER_INPUT_BYTES);
        assert_eq!(limits["owner_mailbox_messages"], OWNER_MAILBOX_MESSAGES);
        assert_eq!(limits["owner_mailbox_bytes"], OWNER_MAILBOX_BYTES);
        assert_eq!(limits["caller_output_messages"], CALLER_OUTPUT_MESSAGES);
        assert_eq!(limits["caller_output_bytes"], CALLER_OUTPUT_BYTES);
        assert_eq!(
            limits["reserved_call_ids_per_caller"],
            RESERVED_CALL_IDS_PER_CALLER
        );
        assert_eq!(limits["reserved_call_ids_total"], RESERVED_CALL_IDS_TOTAL);
        assert_eq!(
            limits["outstanding_lab_calls_per_caller"],
            OUTSTANDING_LAB_PER_CALLER
        );
        assert_eq!(limits["outstanding_lab_calls_total"], OUTSTANDING_LAB_TOTAL);
        assert_eq!(limits["client_deadline_ms"], 2_000);
    }

    #[test]
    fn strict_envelope_and_operation_args_reject_unknown_fields() {
        let envelope_unknown =
            br#"{"v":1,"type":"request","call_id":"x","op":"hello","args":{},"extra":1}"#;
        assert_eq!(
            decode_request(envelope_unknown).unwrap_err().code,
            WireErrorCode::InvalidShape
        );
        let args_unknown = envelope("x", "hello", json!({"extra":1}));
        assert_eq!(
            decode_request(&args_unknown).unwrap_err().code,
            WireErrorCode::InvalidArgs
        );
    }

    #[test]
    fn recovery_get_expected_is_required_but_explicitly_nullable() {
        let missing = envelope(
            "missing",
            "recovery_get",
            json!({
                "kind":"active","index":"0"
            }),
        );
        assert_eq!(
            decode_request(&missing).unwrap_err().code,
            WireErrorCode::InvalidArgs
        );

        let explicit_null = decode_request(&envelope(
            "null",
            "recovery_get",
            json!({"kind":"active","index":"0","expected":null}),
        ))
        .unwrap();
        assert!(matches!(
            explicit_null.request,
            WorkbenchRequest::RecoveryGet(RecoveryGetArgs { expected: None, .. })
        ));

        let expected = decode_request(&envelope(
            "object",
            "recovery_get",
            json!({
                "kind":"quarantined",
                "index":"1",
                "expected":{
                    "workbench_id":"0123456789abcdef0123456789abcdef",
                    "recovery_generation":"7"
                }
            }),
        ))
        .unwrap();
        assert!(matches!(
            expected.request,
            WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                expected: Some(RecoveryExpectation {
                    recovery_generation: 7,
                    ..
                }),
                ..
            })
        ));

        let nested_unknown = envelope(
            "unknown",
            "recovery_get",
            json!({
                "kind":"active",
                "index":"0",
                "expected":{
                    "workbench_id":"0123456789abcdef0123456789abcdef",
                    "recovery_generation":"1",
                    "extra":true
                }
            }),
        );
        assert_eq!(
            decode_request(&nested_unknown).unwrap_err().code,
            WireErrorCode::InvalidArgs
        );
    }

    #[test]
    fn malformed_duplicate_utf8_root_and_trailing_json_are_rejected() {
        for (body, code) in [
            (b"{".as_slice(), WireErrorCode::InvalidJson),
            (
                br#"{"v":1,"v":1,"type":"request","call_id":"x","op":"hello","args":{}}"#,
                WireErrorCode::InvalidJson,
            ),
            (&[0xff][..], WireErrorCode::InvalidUtf8),
            (b"[]".as_slice(), WireErrorCode::InvalidShape),
            (b"{} {}".as_slice(), WireErrorCode::InvalidJson),
        ] {
            assert_eq!(decode_request(body).unwrap_err().code, code);
        }
    }

    #[test]
    fn lexical_depth_value_string_frame_and_number_bounds_are_enforced() {
        let long = "x".repeat(JSON_STRING_BYTES + 1);
        assert_eq!(
            decode_request(&envelope(
                "x",
                "lab_query",
                json!({"op":"reference","args":{"value":long}}),
            ))
            .unwrap_err()
            .code,
            WireErrorCode::InvalidJson
        );

        let mut nested = Value::Null;
        for _ in 0..=JSON_DEPTH {
            nested = Value::Array(vec![nested]);
        }
        let deep = envelope(
            "x",
            "lab_query",
            json!({"op":"reference","args":{"x":nested}}),
        );
        assert_eq!(
            decode_request(&deep).unwrap_err().code,
            WireErrorCode::InvalidJson
        );

        let many = vec![Value::Null; JSON_VALUES];
        assert_eq!(
            decode_request(&envelope(
                "x",
                "lab_query",
                json!({"op":"reference","args":{"many":many}}),
            ))
            .unwrap_err()
            .code,
            WireErrorCode::InvalidJson
        );
        assert_eq!(
            decode_request(&vec![b' '; JSON_BODY_BYTES + 1])
                .unwrap_err()
                .code,
            WireErrorCode::FrameTooLarge
        );
        let mut exact_body = envelope("exact", "hello", json!({}));
        exact_body.resize(JSON_BODY_BYTES, b' ');
        assert_eq!(
            decode_request(&exact_body).unwrap().request.operation(),
            "hello"
        );
        assert_eq!(
            decode_request(br#"{"v":1,"type":"request","call_id":"x","op":"ui_set_time_window","args":{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"1"},"plot_id":"p","seconds":1e9999}}"#)
                .unwrap_err()
                .code,
            WireErrorCode::InvalidJson
        );
        assert_eq!(FRAME_BYTES, JSON_BODY_BYTES + 1);
    }

    #[test]
    fn exact_physical_lf_and_crlf_frame_boundaries_are_consistent() {
        for (call_id, body_len, delimiter) in [
            ("max-lf", JSON_BODY_BYTES, b"\n".as_slice()),
            ("max-crlf", JSON_BODY_BYTES - 1, b"\r\n".as_slice()),
        ] {
            let caller_id = CallerId(1);
            let (_peer, mut connection) = test_connection(caller_id);
            connection.hello = HelloGate::Complete;
            connection.hello_deadline = None;
            let shared = Arc::new(Mutex::new(SharedState::default()));
            assert!(shared.lock().unwrap().register(caller_id));
            let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
            let mut global = GlobalCorrelation::default();
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let mut body = envelope(call_id, "client_status", json!({}));
            body.resize(body_len, b' ');
            connection.input = body;
            connection.input.extend_from_slice(delimiter);
            connection.partial_deadline = Some(Instant::now() + CLIENT_DEADLINE);
            assert_eq!(connection.input.len(), FRAME_BYTES);
            let outcome = admit_buffered_frame(
                &mut connection,
                &request_tx,
                &shared,
                &mut global,
                &wake,
                Instant::now(),
            );
            assert_eq!(
                outcome,
                Some(false),
                "{call_id}: writes={}, correlations={}, global_calls={}",
                connection.writes.len(),
                connection.correlation.calls.len(),
                global.reserved,
            );
            let request = request_rx.try_recv().unwrap();
            assert_eq!(request.call_id, call_id);
            assert_eq!(request.frame_bytes, FRAME_BYTES);
            assert!(connection.input.is_empty());
        }

        let caller_id = CallerId(1);
        let (_peer, mut connection) = test_connection(caller_id);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let (request_tx, _request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let mut global = GlobalCorrelation::default();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut oversized_crlf = envelope("too-large-crlf", "client_status", json!({}));
        oversized_crlf.resize(JSON_BODY_BYTES, b' ');
        oversized_crlf.extend_from_slice(b"\r\n");
        connection.input = oversized_crlf;
        assert_eq!(connection.input.len(), FRAME_BYTES + 1);
        assert_eq!(
            admit_buffered_frame(
                &mut connection,
                &request_tx,
                &shared,
                &mut global,
                &wake,
                Instant::now(),
            ),
            Some(true)
        );
    }

    #[test]
    fn maximum_frame_and_next_prefix_may_share_one_tcp_read() {
        let caller_id = CallerId(1);
        let (mut peer, mut connection) = test_connection(caller_id);
        connection.hello = HelloGate::Complete;
        connection.hello_deadline = None;
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let mut global = GlobalCorrelation::default();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut body = envelope("max", "client_status", json!({}));
        body.resize(JSON_BODY_BYTES, b' ');
        let split = body.len() - 2;
        connection.input.extend_from_slice(&body[..split]);
        connection.partial_deadline = Some(Instant::now() + CLIENT_DEADLINE);
        let next_prefix = b"{\"v\":1";
        peer.write_all(&[&body[split..], b"\n", next_prefix].concat())
            .unwrap();

        assert!(!read_caller(
            &mut connection,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        let request = request_rx.try_recv().unwrap();
        assert_eq!(request.call_id, "max");
        assert_eq!(request.frame_bytes, FRAME_BYTES);
        assert_eq!(connection.input, next_prefix);
        assert!(connection.input.len() < FRAME_BYTES);
    }

    #[test]
    fn oversized_single_frame_detaches_before_admission() {
        let caller_id = CallerId(1);
        let (mut peer, mut connection) = test_connection(caller_id);
        connection.hello = HelloGate::Complete;
        connection.hello_deadline = None;
        connection.input = vec![b' '; FRAME_BYTES - 1];
        connection.partial_deadline = Some(Instant::now() + CLIENT_DEADLINE);
        peer.write_all(b"x\n").unwrap();
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let mut global = GlobalCorrelation::default();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        assert!(read_caller(
            &mut connection,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn coalesced_frames_process_at_most_four_per_caller_turn() {
        let caller_id = CallerId(1);
        let (mut peer, mut connection) = test_connection(caller_id);
        connection.hello = HelloGate::Complete;
        connection.hello_deadline = None;
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let mut global = GlobalCorrelation::default();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut coalesced = Vec::new();
        for index in 0..6 {
            coalesced.extend_from_slice(&envelope(
                &format!("coalesced-{index}"),
                "client_status",
                json!({}),
            ));
            coalesced.push(b'\n');
        }
        assert!(coalesced.len() < SOCKET_BYTES_PER_TURN);
        peer.write_all(&coalesced).unwrap();

        let readiness_deadline = Instant::now() + CLIENT_DEADLINE;
        let mut ready = [0_u8; 1];
        loop {
            match connection.stream.peek(&mut ready) {
                Ok(1..) => break,
                Ok(0) => panic!("coalesced test peer closed before readiness"),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < readiness_deadline,
                        "coalesced frames did not become readable"
                    );
                    thread::yield_now();
                }
                Err(error) => panic!("coalesced frame readiness failed: {error}"),
            }
        }

        assert!(!read_caller(
            &mut connection,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        assert_eq!(request_rx.try_iter().count(), FRAMES_PER_CALLER_TURN);
        assert_eq!(
            connection
                .input
                .iter()
                .filter(|byte| **byte == b'\n')
                .count(),
            2
        );

        assert!(!read_caller(
            &mut connection,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        assert!(connection.input.is_empty());
        assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(connection.writes.len(), 2);
        for write in &connection.writes {
            assert_eq!(
                serde_json::from_slice::<Value>(&write.bytes[..write.bytes.len() - 1]).unwrap()["error"]
                    ["code"],
                "busy"
            );
        }
    }

    #[test]
    fn decimal_strings_are_canonical_and_runtime_args_remain_opaque() {
        for invalid in ["", "00", "01", "+1", "-1", " 1", "1.0", "1e0"] {
            let body = envelope(
                "x",
                "recovery_get",
                json!({"kind":"active","index":invalid,"expected":null}),
            );
            assert_eq!(
                decode_request(&body).unwrap_err().code,
                WireErrorCode::InvalidArgs
            );
        }
        let decoded = decode_request(&envelope(
            "x",
            "lab_query",
            json!({"op":"runtime_op","args":{"unknown_to_workbench":true}}),
        ))
        .unwrap();
        let WorkbenchRequest::LabQuery { args, .. } = decoded.request else {
            panic!("expected lab query");
        };
        assert_eq!(args, json!({"unknown_to_workbench":true}));
    }

    #[test]
    fn duplicate_related_writes_extend_call_id_lifetime_through_the_last_byte() {
        let caller_id = CallerId(1);
        let (_peer, mut connection) = test_connection(caller_id);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let mut global = GlobalCorrelation::default();
        connection
            .correlation
            .reserve("local", false, &mut global)
            .unwrap();
        assert_eq!(
            connection.correlation.reserve("local", false, &mut global),
            Err(CorrelationReserveError::Duplicate)
        );
        assert!(queue_write(
            &mut connection,
            b"terminal\n".to_vec(),
            Some(CorrelationCompletion::Terminal("local".into())),
            None,
            0,
            &shared,
            Instant::now(),
        ));
        assert!(!queue_duplicate_error(
            &mut connection,
            "local",
            &shared,
            Instant::now(),
        ));
        for _ in 0..2 {
            assert!(!queue_duplicate_error(
                &mut connection,
                "local",
                &shared,
                Instant::now(),
            ));
        }
        assert_eq!(connection.writes.len(), CALLER_OUTPUT_MESSAGES);

        let terminal = connection.writes.pop_front().unwrap();
        connection
            .correlation
            .complete(terminal.correlation_completion.unwrap(), &mut global);
        assert!(connection.correlation.contains("local"));

        let duplicate = connection.writes.front_mut().unwrap();
        duplicate.offset = duplicate.bytes.len() - 1;
        assert!(connection.correlation.contains("local"));
        assert_eq!(
            connection.correlation.reserve("local", false, &mut global),
            Err(CorrelationReserveError::Duplicate)
        );
        while let Some(duplicate) = connection.writes.pop_front() {
            connection
                .correlation
                .complete(duplicate.correlation_completion.unwrap(), &mut global);
            if !connection.writes.is_empty() {
                assert!(connection.correlation.contains("local"));
            }
        }
        assert!(!connection.correlation.contains("local"));
        assert_eq!(global.reserved, 0);

        connection
            .correlation
            .reserve("local", false, &mut global)
            .unwrap();
        assert!(connection.correlation.contains("local"));
        connection.correlation.detach(&mut global);
    }

    #[test]
    fn detach_releases_held_correlation_once_without_cancelling_admitted_work() {
        let mut global = GlobalCorrelation::default();
        let mut calls = CorrelationState::new();
        calls.reserve("a", true, &mut global).unwrap();
        calls.reserve("b", false, &mut global).unwrap();
        calls.hold_related_write("a");
        calls.detach(&mut global);
        assert!(calls.calls.is_empty());
        assert_eq!(global.reserved, 0);
        assert_eq!(global.lab, 0);
        calls.detach(&mut global);
        assert_eq!(global.reserved, 0);
        assert_eq!(global.lab, 0);

        let mut dispatcher = dispatcher();
        dispatcher
            .dispatch(
                CallOrigin {
                    caller_id: CallerId(7),
                    call_id: "a".into(),
                },
                WorkbenchRequest::LabMutation {
                    op: "reference_retune".into(),
                    args: json!({"reference":"1","target":2.0,"rate":1.0}),
                },
            )
            .unwrap();
        dispatcher.detach_caller(CallerId(7));
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );
    }

    #[test]
    fn caller_mailbox_count_and_byte_bounds_are_atomic() {
        let caller = CallerId(1);
        let mut state = SharedState::default();
        assert!(state.register(caller));
        for _ in 0..CALLER_INPUT_MESSAGES {
            assert!(state.reserve_input(caller, 1));
        }
        assert!(!state.reserve_input(caller, 1));
        for _ in 0..CALLER_INPUT_MESSAGES {
            state.release_owner_input(1);
            state.release_caller_input(caller, 1);
        }
        assert!(!state.reserve_input(caller, CALLER_INPUT_BYTES + 1));

        for _ in 0..CALLER_OUTPUT_MESSAGES {
            assert!(state.reserve_owner_output(caller, 1));
        }
        assert!(!state.reserve_owner_output(caller, 1));
        assert_eq!(state.owner_to_network_messages, CALLER_OUTPUT_MESSAGES);
    }

    #[test]
    fn per_caller_reserved_saturation_detaches_without_a_ninth_held_id() {
        let caller_id = CallerId(1);
        let (_peer, mut connection) = test_connection(caller_id);
        connection.hello = HelloGate::Complete;
        connection.hello_deadline = None;
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let mut global = GlobalCorrelation::default();
        for index in 0..RESERVED_CALL_IDS_PER_CALLER {
            connection
                .correlation
                .reserve(&format!("held-{index}"), false, &mut global)
                .unwrap();
        }
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let body = envelope("ninth", "client_status", json!({}));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        assert!(admit_frame(
            &mut connection,
            body.clone(),
            body.len() + 1,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        assert!(connection.writes.is_empty());
        assert!(!connection.correlation.contains("ninth"));
        assert_eq!(
            connection.correlation.calls.len(),
            RESERVED_CALL_IDS_PER_CALLER
        );
        assert_eq!(global.reserved, RESERVED_CALL_IDS_PER_CALLER);
        assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn global_reserved_saturation_detaches_without_a_thirty_third_held_id() {
        let caller_id = CallerId(1);
        let (_peer, mut connection) = test_connection(caller_id);
        connection.hello = HelloGate::Complete;
        connection.hello_deadline = None;
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(caller_id));
        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let mut global = GlobalCorrelation {
            reserved: RESERVED_CALL_IDS_TOTAL,
            lab: 0,
        };
        let body = envelope("thirty-third", "client_status", json!({}));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        assert!(admit_frame(
            &mut connection,
            body.clone(),
            body.len() + 1,
            &request_tx,
            &shared,
            &mut global,
            &wake,
            Instant::now(),
        ));
        assert!(connection.writes.is_empty());
        assert!(connection.correlation.calls.is_empty());
        assert_eq!(global.reserved, RESERVED_CALL_IDS_TOTAL);
        assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn lab_saturation_cannot_create_hidden_rejection_ids() {
        for global_saturation in [false, true] {
            let caller_id = CallerId(1);
            let (_peer, mut connection) = test_connection(caller_id);
            connection.hello = HelloGate::Complete;
            connection.hello_deadline = None;
            let shared = Arc::new(Mutex::new(SharedState::default()));
            assert!(shared.lock().unwrap().register(caller_id));
            let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
            let mut global = GlobalCorrelation::default();
            if global_saturation {
                global.reserved = RESERVED_CALL_IDS_TOTAL;
                global.lab = OUTSTANDING_LAB_TOTAL;
            } else {
                for index in 0..OUTSTANDING_LAB_PER_CALLER {
                    connection
                        .correlation
                        .reserve(&format!("lab-{index}"), true, &mut global)
                        .unwrap();
                }
            }
            let before_calls = connection.correlation.calls.len();
            let before_reserved = global.reserved;
            let before_lab = global.lab;
            let body = envelope(
                "lab-overflow",
                "lab_query",
                json!({"op":"reference","args":{}}),
            );
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            assert!(admit_frame(
                &mut connection,
                body.clone(),
                body.len() + 1,
                &request_tx,
                &shared,
                &mut global,
                &wake,
                Instant::now(),
            ));
            assert_eq!(connection.correlation.calls.len(), before_calls);
            assert_eq!(global.reserved, before_reserved);
            assert_eq!(global.lab, before_lab);
            assert!(connection.writes.is_empty());
            assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
        }
    }

    #[test]
    fn admitted_correlation_returns_busy_for_input_and_owner_mailbox_pressure() {
        for mailbox_pressure in [false, true] {
            let caller_id = CallerId(1);
            let (_peer, mut connection) = test_connection(caller_id);
            connection.hello = HelloGate::Complete;
            connection.hello_deadline = None;
            let shared = Arc::new(Mutex::new(SharedState::default()));
            assert!(shared.lock().unwrap().register(caller_id));
            let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
            if mailbox_pressure {
                for index in 0..OWNER_MAILBOX_MESSAGES {
                    request_tx
                        .try_send(OwnerRequest {
                            caller_id,
                            call_id: format!("mailbox-{index}"),
                            request: WorkbenchRequest::ClientStatus,
                            frame_bytes: 1,
                            lab: false,
                        })
                        .unwrap();
                }
            } else {
                let mut state = shared.lock().unwrap();
                state.callers.get_mut(&caller_id).unwrap().input_messages = CALLER_INPUT_MESSAGES;
            }
            let mut global = GlobalCorrelation::default();
            let call_id = if mailbox_pressure {
                "mailbox-busy"
            } else {
                "input-busy"
            };
            let body = envelope(call_id, "client_status", json!({}));
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            assert!(!admit_frame(
                &mut connection,
                body.clone(),
                body.len() + 1,
                &request_tx,
                &shared,
                &mut global,
                &wake,
                Instant::now(),
            ));
            assert_eq!(connection.correlation.calls.len(), 1);
            assert_eq!(global.reserved, 1);
            let busy = connection.writes.front().unwrap();
            assert!(matches!(
                busy.correlation_completion,
                Some(CorrelationCompletion::Terminal(ref held)) if held == call_id
            ));
            assert_eq!(
                serde_json::from_slice::<Value>(&busy.bytes[..busy.bytes.len() - 1]).unwrap()["error"]
                    ["code"],
                "busy"
            );
            if !mailbox_pressure {
                assert!(matches!(request_rx.try_recv(), Err(TryRecvError::Empty)));
            }
        }
    }

    #[test]
    fn aggregate_mailbox_byte_credits_saturate_without_partial_admission() {
        let mut state = SharedState::default();
        for id in 1..=5 {
            assert!(state.register(CallerId(id)));
        }
        for id in 1..=4 {
            assert!(state.reserve_input(CallerId(id), CALLER_INPUT_BYTES));
        }
        assert_eq!(state.network_to_owner_bytes, OWNER_MAILBOX_BYTES);
        assert!(!state.reserve_input(CallerId(5), 1));
        assert_eq!(state.callers[&CallerId(5)].input_messages, 0);

        let mut output = SharedState::default();
        for id in 1..=5 {
            assert!(output.register(CallerId(id)));
        }
        for id in 1..=4 {
            assert!(output.reserve_owner_output(CallerId(id), CALLER_OUTPUT_BYTES));
        }
        assert_eq!(output.owner_to_network_bytes, OWNER_MAILBOX_BYTES);
        assert!(!output.reserve_owner_output(CallerId(5), 1));
        assert_eq!(output.callers[&CallerId(5)].output_messages, 0);
    }

    #[test]
    fn eight_callers_include_detach_pending_and_global_correlation_is_bounded() {
        let mut state = SharedState::default();
        for id in 1..=CALLERS as u64 {
            assert!(state.register(CallerId(id)));
        }
        state.network_detached(CallerId(1));
        assert!(!state.register(CallerId(9)));
        state.acknowledge_detach(CallerId(1));
        assert!(state.register(CallerId(9)));

        let mut global = GlobalCorrelation::default();
        let mut callers = (0..CALLERS)
            .map(|_| CorrelationState::new())
            .collect::<Vec<_>>();
        for index in 0..RESERVED_CALL_IDS_TOTAL {
            callers[index % CALLERS]
                .reserve(&format!("call-{index}"), false, &mut global)
                .unwrap();
        }
        assert_eq!(global.reserved, RESERVED_CALL_IDS_TOTAL);
        assert_eq!(
            callers[0].reserve("overflow", false, &mut global),
            Err(CorrelationReserveError::GlobalReserved)
        );

        let mut lab_global = GlobalCorrelation::default();
        let mut lab_callers = (0..5).map(|_| CorrelationState::new()).collect::<Vec<_>>();
        for index in 0..OUTSTANDING_LAB_TOTAL {
            lab_callers[index / OUTSTANDING_LAB_PER_CALLER]
                .reserve(&format!("lab-{index}"), true, &mut lab_global)
                .unwrap();
        }
        assert_eq!(lab_global.lab, OUTSTANDING_LAB_TOTAL);
        assert_eq!(
            lab_callers[4].reserve("lab-overflow", true, &mut lab_global),
            Err(CorrelationReserveError::GlobalLab)
        );
    }

    #[test]
    fn bounded_mailboxes_reject_the_thirty_third_message() {
        let (sender, _receiver) = sync_channel(OWNER_MAILBOX_MESSAGES);
        for index in 0..OWNER_MAILBOX_MESSAGES {
            sender.try_send(index).unwrap();
        }
        assert!(matches!(
            sender.try_send(OWNER_MAILBOX_MESSAGES),
            Err(TrySendError::Full(_))
        ));
    }

    #[test]
    fn partial_hello_and_blocked_output_deadlines_are_absolute() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let _peer = TcpStream::connect(address).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let now = Instant::now();
        let mut connection = Connection::new(CallerId(1), stream, now);
        assert!(!connection.timed_out(now + CLIENT_DEADLINE - Duration::from_millis(1)));
        assert!(connection.timed_out(now + CLIENT_DEADLINE));

        connection.hello_deadline = None;
        connection.partial_deadline = Some(now + CLIENT_DEADLINE);
        assert!(connection.timed_out(now + CLIENT_DEADLINE));
        connection.partial_deadline = None;
        connection.writes.push_back(QueuedWrite {
            bytes: vec![b'x'; SOCKET_BYTES_PER_TURN + 1],
            offset: 1,
            correlation_completion: None,
            hello_completion: None,
            input_release_bytes: 0,
            deadline: now + CLIENT_DEADLINE,
        });
        assert!(connection.timed_out(now + CLIENT_DEADLINE));
    }

    #[test]
    fn owner_turn_services_at_most_four_distinct_callers_once_each() {
        let (_request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let (output_tx, _output_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        for id in 1..=5 {
            assert!(shared.lock().unwrap().register(CallerId(id)));
        }
        let mut pending = VecDeque::new();
        for caller in [1, 1, 2, 3, 4, 5] {
            pending.push_back(OwnerRequest {
                caller_id: CallerId(caller),
                call_id: format!("call-{caller}-{}", pending.len()),
                request: WorkbenchRequest::ClientStatus,
                frame_bytes: 1,
                lab: false,
            });
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let counted_wakes = Arc::clone(&wakes);
        let mut endpoint = WorkbenchEndpoint {
            request_rx,
            output_tx,
            shared,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            owner_wake: Arc::new(move || {
                counted_wakes.fetch_add(1, Ordering::Relaxed);
            }),
            pending,
            fairness_cursor: 0,
        };
        let mut dispatcher = dispatcher();
        assert_eq!(
            endpoint.service_owner(&mut dispatcher),
            OWNER_CALLERS_PER_TURN
        );
        assert_eq!(endpoint.pending.len(), 2);
        let remaining = endpoint
            .pending
            .iter()
            .map(|request| request.caller_id)
            .collect::<Vec<_>>();
        assert!(remaining.contains(&CallerId(1)));
        assert!(remaining.contains(&CallerId(5)));
        assert_eq!(wakes.swap(0, Ordering::Relaxed), 1);

        assert_eq!(endpoint.service_owner(&mut dispatcher), 2);
        assert!(endpoint.pending.is_empty());
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn same_caller_pending_burst_reschedules_each_bounded_owner_turn() {
        let (_request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let (output_tx, _output_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        assert!(shared.lock().unwrap().register(CallerId(1)));
        let pending = (0..3)
            .map(|index| OwnerRequest {
                caller_id: CallerId(1),
                call_id: format!("same-{index}"),
                request: WorkbenchRequest::ClientStatus,
                frame_bytes: 1,
                lab: false,
            })
            .collect();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counted_wakes = Arc::clone(&wakes);
        let mut endpoint = WorkbenchEndpoint {
            request_rx,
            output_tx,
            shared,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            owner_wake: Arc::new(move || {
                counted_wakes.fetch_add(1, Ordering::Relaxed);
            }),
            pending,
            fairness_cursor: 0,
        };
        let mut dispatcher = dispatcher();

        for remaining in [2, 1] {
            assert_eq!(endpoint.service_owner(&mut dispatcher), 1);
            assert_eq!(endpoint.pending.len(), remaining);
            assert_eq!(wakes.swap(0, Ordering::Relaxed), 1);
        }
        assert_eq!(endpoint.service_owner(&mut dispatcher), 1);
        assert!(endpoint.pending.is_empty());
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn request_admitted_during_dispatch_reschedules_after_the_initial_drain() {
        type DuringDispatchInjection = (
            SyncSender<OwnerRequest>,
            Arc<Mutex<SharedState>>,
            OwnerRequest,
        );

        struct DuringDispatchClient {
            injection: RefCell<Option<DuringDispatchInjection>>,
        }

        impl LabClient for DuringDispatchClient {
            fn query(&self, _op: &str, _args: Value) -> Result<u64, CommandSendError> {
                let (request_tx, shared, request) = self
                    .injection
                    .borrow_mut()
                    .take()
                    .expect("the final dispatch injects exactly one admitted request");
                assert!(
                    shared
                        .lock()
                        .unwrap()
                        .reserve_input(request.caller_id, request.frame_bytes)
                );
                request_tx
                    .try_send(request)
                    .expect("bounded owner channel has room for injected request");
                // Deliberately omit the network wake: this models a wake that was
                // coalesced with the owner turn currently in progress.
                Ok(1)
            }

            fn mutation(&self, _op: &str, _args: Value) -> Result<u64, CommandSendError> {
                unreachable!("the progress regression submits only a query")
            }

            fn operation_status(
                &self,
                _identity: MutationIdentity,
            ) -> Result<u64, CommandSendError> {
                unreachable!("the progress regression submits no status request")
            }
        }

        let (request_tx, request_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let (output_tx, _output_rx) = sync_channel(OWNER_MAILBOX_MESSAGES);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        for caller in [CallerId(1), CallerId(2)] {
            assert!(shared.lock().unwrap().register(caller));
        }
        assert!(shared.lock().unwrap().reserve_input(CallerId(1), 1));
        let injected = OwnerRequest {
            caller_id: CallerId(2),
            call_id: "during-dispatch".into(),
            request: WorkbenchRequest::ClientStatus,
            frame_bytes: 1,
            lab: false,
        };
        let client = DuringDispatchClient {
            injection: RefCell::new(Some((request_tx, Arc::clone(&shared), injected))),
        };
        let mut dispatcher = WorkbenchDispatcher::new(
            WorkbenchModel::new(PresentationDocument::empty("main")),
            client,
        );
        dispatcher.apply_client_update(ClientUpdate::Hello(HelloState {
            boot_id: "0123456789abcdef0123456789abcdef".into(),
            scope: "scope".into(),
            next_seq: 1,
            operations: vec!["reference".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "0123456789abcdef0123456789abcdef".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "0123456789abcdef0123456789abcdef".into(),
                seq: 0,
            },
        }));
        let wakes = Arc::new(AtomicUsize::new(0));
        let counted_wakes = Arc::clone(&wakes);
        let mut endpoint = WorkbenchEndpoint {
            request_rx,
            output_tx,
            shared: Arc::clone(&shared),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            owner_wake: Arc::new(move || {
                counted_wakes.fetch_add(1, Ordering::Relaxed);
            }),
            pending: VecDeque::from([OwnerRequest {
                caller_id: CallerId(1),
                call_id: "initial".into(),
                request: WorkbenchRequest::LabQuery {
                    op: "reference".into(),
                    args: json!({"reference":"1"}),
                },
                frame_bytes: 1,
                lab: true,
            }]),
            fairness_cursor: 0,
        };

        assert_eq!(endpoint.service_owner(&mut dispatcher), 1);
        assert!(endpoint.pending.is_empty());
        assert_eq!(shared.lock().unwrap().network_to_owner_messages, 1);
        assert_eq!(wakes.swap(0, Ordering::Relaxed), 1);

        assert_eq!(endpoint.service_owner(&mut dispatcher), 1);
        assert!(endpoint.pending.is_empty());
        assert_eq!(shared.lock().unwrap().network_to_owner_messages, 0);
        assert_eq!(wakes.load(Ordering::Relaxed), 0);

        assert_eq!(endpoint.service_owner(&mut dispatcher), 0);
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn endpoint_binding_is_numeric_ipv4_loopback_only_and_readiness_is_bounded() {
        for address in [
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
            SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 1), 0),
        ] {
            assert_eq!(
                PreparedEndpoint::bind(address).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let endpoint = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert!(endpoint.address().ip().is_loopback());
        let readiness: Value = serde_json::from_str(&endpoint.readiness_line()).unwrap();
        assert_eq!(readiness.as_object().unwrap().len(), 1);
        assert_eq!(
            readiness["workbench_endpoint"],
            endpoint.address().to_string()
        );
        assert!(endpoint.readiness_line().len() <= JSON_STRING_BYTES);
    }

    fn read_frame_with_pump(
        stream: &mut TcpStream,
        endpoint: &mut WorkbenchEndpoint,
        dispatcher: &mut WorkbenchDispatcher<FakeClient>,
    ) -> Value {
        stream.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut bytes = Vec::new();
        while Instant::now() < deadline {
            endpoint.service_owner(dispatcher);
            let mut buffer = [0u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
                        return serde_json::from_slice(&bytes[..newline]).unwrap();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("read failed: {error}"),
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("timed out waiting for Workbench frame")
    }

    fn endpoint_pair() -> (
        WorkbenchEndpoint,
        TcpStream,
        WorkbenchDispatcher<FakeClient>,
    ) {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let stream = TcpStream::connect(address).unwrap();
        (endpoint, stream, dispatcher())
    }

    struct AcceptanceCaller {
        stream: TcpStream,
        input: Vec<u8>,
        frames: VecDeque<Value>,
    }

    impl AcceptanceCaller {
        fn connect(address: SocketAddrV4) -> Self {
            let stream = TcpStream::connect(address).unwrap();
            stream.set_nonblocking(true).unwrap();
            Self {
                stream,
                input: Vec::new(),
                frames: VecDeque::new(),
            }
        }

        fn send(&mut self, call_id: &str, op: &str, args: Value) {
            let mut frame = envelope(call_id, op, args);
            frame.push(b'\n');
            self.stream.write_all(&frame).unwrap();
        }

        fn receive_available(&mut self) -> bool {
            let mut progressed = false;
            let mut buffer = [0_u8; 4096];
            loop {
                match self.stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        progressed = true;
                        self.input.extend_from_slice(&buffer[..read]);
                        while let Some(newline) = self.input.iter().position(|byte| *byte == b'\n')
                        {
                            let line = self.input.drain(..=newline).collect::<Vec<_>>();
                            self.frames.push_back(
                                serde_json::from_slice(&line[..line.len() - 1]).unwrap(),
                            );
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("acceptance caller read failed: {error}"),
                }
            }
            progressed
        }

        fn take(&mut self, predicate: impl Fn(&Value) -> bool) -> Option<Value> {
            let index = self.frames.iter().position(predicate)?;
            self.frames.remove(index)
        }
    }

    fn wait_for_frame(
        caller: &mut AcceptanceCaller,
        endpoint: &mut WorkbenchEndpoint,
        dispatcher: &mut WorkbenchDispatcher<FakeClient>,
        predicate: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            endpoint.service_owner(dispatcher);
            caller.receive_available();
            if let Some(frame) = caller.take(&predicate) {
                return frame;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for acceptance frame; queued={:?}",
                caller.frames
            );
            thread::yield_now();
        }
    }

    fn complete_acceptance_hello(
        caller: &mut AcceptanceCaller,
        endpoint: &mut WorkbenchEndpoint,
        dispatcher: &mut WorkbenchDispatcher<FakeClient>,
        call_id: &str,
    ) -> Value {
        caller.send(call_id, "hello", json!({}));
        wait_for_frame(caller, endpoint, dispatcher, |frame| {
            frame["call_id"] == call_id && frame["type"] == "result"
        })
    }

    fn recovery_record(seq: u64, admission: KnownAdmission) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "0123456789abcdef0123456789abcdef".into(),
            identity: MutationIdentity {
                scope: "scope".into(),
                seq,
            },
            op: "reference_retune".into(),
            args: json!({"reference":"1","target":2.0,"rate":1.0}),
            admission,
        }
    }

    #[test]
    fn hello_gate_duplicate_hello_and_crlf_framing_are_exact() {
        let (mut endpoint, mut stream, mut dispatcher) = endpoint_pair();
        stream
            .write_all(
                &[
                    envelope("before", "client_status", json!({})),
                    b"\n".to_vec(),
                ]
                .concat(),
            )
            .unwrap();
        let required = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(required["error"]["code"], "hello_required");

        stream
            .write_all(&[envelope("hello", "hello", json!({})), b"\r\n".to_vec()].concat())
            .unwrap();
        let hello = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(hello["type"], "result");
        assert_eq!(hello["result"]["operations"].as_array().unwrap().len(), 15);

        stream
            .write_all(&[envelope("again", "hello", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let again = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(again["error"]["code"], "already_hello");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn submitted_and_accepted_lab_updates_retain_call_id_until_final_write() {
        let (mut endpoint, mut stream, mut dispatcher) = endpoint_pair();
        stream
            .write_all(&[envelope("hello", "hello", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        assert_eq!(
            read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher)["type"],
            "result"
        );

        let mutation = [
            envelope(
                "lab",
                "lab_mutation",
                json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
            ),
            b"\n".to_vec(),
        ]
        .concat();
        stream.write_all(&mutation).unwrap();
        let submitted = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(submitted["result"]["state"], "submitted");

        stream
            .write_all(&[envelope("lab", "client_status", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let duplicate = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(duplicate["error"]["code"], "duplicate_call_id");

        let accepted = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationAccepted,
            envelope: json!({"type":"operation_accepted"}),
            recovery: None,
        });
        endpoint.route_events(&mut dispatcher, accepted);
        let accepted = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(accepted["event"], "lab_update");
        assert_eq!(accepted["data"]["kind"], "mutation_accepted");

        stream
            .write_all(&[envelope("lab", "client_status", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let duplicate = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(duplicate["error"]["code"], "duplicate_call_id");

        let completed = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation_completed"}),
            recovery: None,
        });
        endpoint.route_events(&mut dispatcher, completed);
        let completed = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(completed["data"]["kind"], "mutation_completed");

        stream
            .write_all(&[envelope("lab", "client_status", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let reused = read_frame_with_pump(&mut stream, &mut endpoint, &mut dispatcher);
        assert_eq!(reused["type"], "result");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn malformed_caller_isolated_and_caller_death_does_not_cancel_lab_work() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut malformed = TcpStream::connect(address).unwrap();
        malformed.write_all(b"{not-json}\n").unwrap();

        let mut good = TcpStream::connect(address).unwrap();
        good.write_all(&[envelope("hello", "hello", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let mut dispatcher = dispatcher();
        let hello = read_frame_with_pump(&mut good, &mut endpoint, &mut dispatcher);
        assert_eq!(hello["type"], "result");

        good.write_all(
            &[
                envelope(
                    "mutation",
                    "lab_mutation",
                    json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
                ),
                b"\n".to_vec(),
            ]
            .concat(),
        )
        .unwrap();
        let submitted = read_frame_with_pump(&mut good, &mut endpoint, &mut dispatcher);
        assert_eq!(submitted["result"]["state"], "submitted");
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );
        drop(good);
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            endpoint.service_owner(&mut dispatcher);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn partial_and_nonreading_callers_do_not_starve_a_healthy_caller() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut partial = TcpStream::connect(address).unwrap();
        partial.write_all(b"{").unwrap();
        let mut nonreading = TcpStream::connect(address).unwrap();
        nonreading
            .write_all(&[envelope("slow", "hello", json!({})), b"\n".to_vec()].concat())
            .unwrap();

        let mut healthy = TcpStream::connect(address).unwrap();
        healthy
            .write_all(&[envelope("hello", "hello", json!({})), b"\n".to_vec()].concat())
            .unwrap();
        let mut dispatcher = dispatcher();
        let hello = read_frame_with_pump(&mut healthy, &mut endpoint, &mut dispatcher);
        assert_eq!(hello["type"], "result");

        healthy
            .write_all(
                &[
                    envelope("status", "client_status", json!({})),
                    b"\n".to_vec(),
                ]
                .concat(),
            )
            .unwrap();
        let status = read_frame_with_pump(&mut healthy, &mut endpoint, &mut dispatcher);
        assert_eq!(status["type"], "result");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn m17_4_multi_caller_mixed_workload_uses_one_dispatch_lane() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut dispatcher = dispatcher();
        let mut callers = (0..4)
            .map(|_| AcceptanceCaller::connect(address))
            .collect::<Vec<_>>();
        for (index, caller) in callers.iter_mut().enumerate() {
            caller.send(&format!("hello-{index}"), "hello", json!({}));
        }
        let mut hello = None;
        for (index, caller) in callers.iter_mut().enumerate() {
            let call_id = format!("hello-{index}");
            let frame = wait_for_frame(caller, &mut endpoint, &mut dispatcher, |frame| {
                frame["call_id"] == call_id && frame["type"] == "result"
            });
            hello.get_or_insert(frame);
        }
        let workbench_id = hello.unwrap()["result"]["workbench_id"]
            .as_str()
            .unwrap()
            .to_owned();

        callers[0].send(
            "query",
            "lab_query",
            json!({"op":"reference","args":{"reference":"1"}}),
        );
        callers[1].send(
            "mutation",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
        );
        callers[2].send(
            "plot",
            "ui_add_plot",
            json!({
                "expected":{"workbench_id":workbench_id,"revision":"1"},
                "plot":{"id":"plot","title":"Plot","time_window_seconds":60.0,
                    "axes":{"y_min":null,"y_max":null},"traces":[]}
            }),
        );
        callers[3].send("status", "client_status", json!({}));

        for (index, call_id) in ["query", "mutation", "plot", "status"]
            .into_iter()
            .enumerate()
        {
            let frame = wait_for_frame(
                &mut callers[index],
                &mut endpoint,
                &mut dispatcher,
                |frame| frame["call_id"] == call_id,
            );
            assert_eq!(frame["type"], "result", "{frame:?}");
        }
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["query:reference", "mutation:reference_retune"]
        );
        assert_eq!(dispatcher.presentation_revision(), 2);

        let query_events = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "query-message".into(),
            op: "reference".into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":{"reference":"1"}}),
            recovery: None,
        });
        endpoint.route_events(&mut dispatcher, query_events);
        let mutation_accepted = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 2,
            msg_id: "mutation-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationAccepted,
            envelope: json!({"type":"operation_accepted","request_id":{"scope":"scope","seq":"1"}}),
            recovery: None,
        });
        endpoint.route_events(&mut dispatcher, mutation_accepted);
        let query = wait_for_frame(&mut callers[0], &mut endpoint, &mut dispatcher, |frame| {
            frame["event"] == "lab_update" && frame["data"]["call_id"] == "query"
        });
        assert_eq!(query["data"]["kind"], "result");
        let accepted = wait_for_frame(&mut callers[1], &mut endpoint, &mut dispatcher, |frame| {
            frame["event"] == "lab_update" && frame["data"]["call_id"] == "mutation"
        });
        assert_eq!(accepted["data"]["kind"], "mutation_accepted");

        let state_events =
            dispatcher.apply_client_update(ClientUpdate::State(ConnectionState::Disconnected));
        endpoint.route_events(&mut dispatcher, state_events);
        for caller in &mut callers {
            let state = wait_for_frame(caller, &mut endpoint, &mut dispatcher, |frame| {
                frame["event"] == "client_state"
            });
            assert_eq!(state["data"]["connection"], "disconnected");
        }

        drop(callers.remove(2));
        let completed = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 2,
            msg_id: "mutation-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation_completed","request_id":{"scope":"scope","seq":"1"}}),
            recovery: None,
        });
        endpoint.route_events(&mut dispatcher, completed);
        let terminal = wait_for_frame(&mut callers[1], &mut endpoint, &mut dispatcher, |frame| {
            frame["event"] == "lab_update" && frame["data"]["kind"] == "mutation_completed"
        });
        assert_eq!(terminal["data"]["call_id"], "mutation");
        callers[2].send("still-healthy", "presentation_get", json!({}));
        let healthy = wait_for_frame(&mut callers[2], &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "still-healthy"
        });
        assert_eq!(healthy["result"]["presentation_revision"], "2");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn m17_4_all_ui_operations_are_runtime_silent_and_atomic_across_disconnect() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut dispatcher = dispatcher();
        let mut caller = AcceptanceCaller::connect(address);
        let hello = complete_acceptance_hello(&mut caller, &mut endpoint, &mut dispatcher, "hello");
        let workbench_id = hello["result"]["workbench_id"].as_str().unwrap();
        let operations = [
            (
                "add-plot",
                "ui_add_plot",
                json!({"plot":{"id":"plot","title":"Plot","time_window_seconds":60.0,
                    "axes":{"y_min":null,"y_max":null},"traces":[]}}),
            ),
            (
                "window",
                "ui_set_time_window",
                json!({"plot_id":"plot","seconds":120.0}),
            ),
            (
                "add-trace",
                "ui_add_trace",
                json!({"plot_id":"plot","trace":{"id":"trace",
                    "source":{"kind":"reference","reference":"1"},"display_label":"Trace",
                    "visible":true,"style":{"color":"cyan","width":1.0},"display_unit":null}}),
            ),
            (
                "visibility",
                "ui_set_trace_visibility",
                json!({"plot_id":"plot","trace_id":"trace","visible":false}),
            ),
            (
                "rename",
                "ui_rename_item",
                json!({"item_id":"plot","label":"Renamed"}),
            ),
            (
                "source",
                "ui_set_trace_source",
                json!({"plot_id":"plot","trace_id":"trace",
                    "source":{"kind":"reference","reference":"2"}}),
            ),
            (
                "remove-trace",
                "ui_remove_trace",
                json!({"plot_id":"plot","trace_id":"trace"}),
            ),
            ("remove-plot", "ui_remove_plot", json!({"plot_id":"plot"})),
        ];
        for (index, (call_id, operation, fields)) in operations.into_iter().enumerate() {
            let mut args = fields.as_object().unwrap().clone();
            args.insert(
                "expected".into(),
                json!({"workbench_id":workbench_id,"revision":(index + 1).to_string()}),
            );
            caller.send(call_id, operation, Value::Object(args));
            let result = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
                frame["call_id"] == call_id
            });
            assert_eq!(
                result["result"]["presentation_revision"],
                (index + 2).to_string()
            );
        }
        assert_eq!(dispatcher.presentation_revision(), 9);
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());

        caller.send(
            "disconnect-race",
            "ui_add_plot",
            json!({"expected":{"workbench_id":workbench_id,"revision":"9"},
                "plot":{"id":"disconnect-race","title":"Disconnect race","time_window_seconds":60.0,
                    "axes":{"y_min":null,"y_max":null},"traces":[]}}),
        );
        drop(caller);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            endpoint.service_owner(&mut dispatcher);
            if endpoint.shared.lock().unwrap().callers.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "disconnected presentation caller was not fully detached"
            );
            thread::yield_now();
        }
        let mut observer = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut observer,
            &mut endpoint,
            &mut dispatcher,
            "observer-hello",
        );
        observer.send("presentation", "presentation_get", json!({}));
        let presentation = wait_for_frame(&mut observer, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "presentation"
        });
        let observed_revision = presentation["result"]["presentation_revision"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let plots = &presentation["result"]["document"]["plots"];
        match observed_revision {
            9 => assert_eq!(plots, &json!([]), "unadmitted request changed the document"),
            10 => assert_eq!(
                plots,
                &json!([{
                    "id":"disconnect-race",
                    "title":"Disconnect race",
                    "time_window_seconds":60.0,
                    "axes":{"y_min":null,"y_max":null},
                    "traces":[]
                }]),
                "admitted request was not committed atomically"
            ),
            revision => panic!("caller death produced an invalid presentation revision {revision}"),
        }
        assert_eq!(dispatcher.presentation_revision(), observed_revision);
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn m17_4_recovery_generation_restart_quarantine_and_reconnect_are_exact() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut dispatcher = dispatcher();
        let mut caller = AcceptanceCaller::connect(address);
        let hello = complete_acceptance_hello(&mut caller, &mut endpoint, &mut dispatcher, "hello");
        let workbench_id = hello["result"]["workbench_id"].as_str().unwrap().to_owned();

        let pending = recovery_record(7, KnownAdmission::Pending);
        let quarantined = QuarantinedRecoveryRecord {
            record: RecoveryRecord {
                boot_id: "fedcba9876543210fedcba9876543210".into(),
                identity: MutationIdentity {
                    scope: "old-scope".into(),
                    seq: 3,
                },
                op: "reference_retune".into(),
                args: json!({"reference":"1","target":1.0,"rate":1.0}),
                admission: KnownAdmission::Ambiguous,
            },
            reason: RecoveryQuarantineReason::InstanceChanged,
        };
        let changed = dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![pending.clone()],
            quarantined: vec![quarantined.clone()],
        });
        endpoint.route_events(&mut dispatcher, changed);
        assert_eq!(dispatcher.recovery_generation(), 2);

        caller.send(
            "active-first",
            "recovery_get",
            json!({"kind":"active","index":"0","expected":null}),
        );
        let active = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "active-first"
        });
        assert_eq!(active["result"]["state"], "record");
        assert_eq!(active["result"]["recovery_generation"], "2");
        assert_eq!(
            active["result"]["counts"],
            json!({"active":"1","quarantined":"1"})
        );

        let accepted = recovery_record(7, KnownAdmission::Accepted);
        let changed = dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![accepted],
            quarantined: vec![quarantined],
        });
        endpoint.route_events(&mut dispatcher, changed);
        assert_eq!(dispatcher.recovery_generation(), 3);
        caller.send(
            "stale-enumeration",
            "recovery_get",
            json!({"kind":"active","index":"0","expected":{
                "workbench_id":workbench_id,"recovery_generation":"2"}}),
        );
        let restart = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "stale-enumeration"
        });
        assert_eq!(restart["result"]["state"], "restart");
        assert_eq!(restart["result"]["recovery_generation"], "3");

        caller.send(
            "wrong-status",
            "lab_operation_status",
            json!({"target":{"workbench_id":workbench_id,"recovery_generation":"3",
                "boot_id":"fedcba9876543210fedcba9876543210",
                "request_id":{"scope":"scope","seq":"7"}}}),
        );
        let rejected = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "wrong-status"
        });
        assert_eq!(rejected["error"]["code"], "recovery_unavailable");
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());

        drop(caller);
        let detach_deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < detach_deadline {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }
        let mut reconnected = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut reconnected,
            &mut endpoint,
            &mut dispatcher,
            "active-first",
        );
        reconnected.send("active-first", "client_status", json!({}));
        let reused = wait_for_frame(&mut reconnected, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "active-first" && frame["type"] == "result"
        });
        assert_eq!(reused["result"]["recovery"]["recovery_generation"], "3");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn m17_4_continuity_loss_keeps_ambiguity_without_retry_or_local_rejection() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut dispatcher = dispatcher();
        let mut caller = AcceptanceCaller::connect(address);
        complete_acceptance_hello(&mut caller, &mut endpoint, &mut dispatcher, "hello");
        caller.send(
            "mutation",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
        );
        let submitted = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "mutation"
        });
        assert_eq!(submitted["result"]["state"], "submitted");
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );

        let ambiguous = recovery_record(1, KnownAdmission::Ambiguous);
        let recovery = dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![ambiguous],
            quarantined: Vec::new(),
        });
        endpoint.route_events(&mut dispatcher, recovery);
        let continuity = dispatcher.apply_client_update(ClientUpdate::ResnapshotRequired {
            reason: "test continuity loss".into(),
            envelope: None,
            connection_lost: true,
        });
        endpoint.route_events(&mut dispatcher, continuity);
        let notice = wait_for_frame(&mut caller, &mut endpoint, &mut dispatcher, |frame| {
            frame["event"] == "client_notice" && frame["data"]["kind"] == "resnapshot_required"
        });
        assert_eq!(notice["data"]["detail"], "test continuity loss");
        caller.receive_available();
        assert!(!caller.frames.iter().any(|frame| {
            frame["event"] == "lab_update" && frame["data"]["kind"] == "local_rejected"
        }));
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );

        drop(caller);
        let detach_deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < detach_deadline {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }
        let terminal = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "mutation-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation_completed","request_id":{"scope":"scope","seq":"1"}}),
            recovery: Some(recovery_record(1, KnownAdmission::Completed)),
        });
        assert!(
            terminal.is_empty(),
            "detached caller must not retain correlation"
        );
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &["mutation:reference_retune"]
        );

        let mut observer = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut observer,
            &mut endpoint,
            &mut dispatcher,
            "observer-hello",
        );
        observer.send(
            "recovery",
            "recovery_get",
            json!({"kind":"active","index":"0","expected":null}),
        );
        let evidence = wait_for_frame(&mut observer, &mut endpoint, &mut dispatcher, |frame| {
            frame["call_id"] == "recovery"
        });
        assert_eq!(evidence["result"]["record"]["admission"], "ambiguous");
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn m17_4_caller_loss_at_admission_accepted_and_terminal_stages_is_local_only() {
        let prepared = PreparedEndpoint::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = prepared.address();
        let mut endpoint = WorkbenchEndpoint::start(prepared, Arc::new(|| {})).unwrap();
        let mut dispatcher = dispatcher();

        // Stage A: socket death races admission. Either zero or one admission is
        // valid; a second submission, status request, or retry is never valid.
        let mut before_admission = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut before_admission,
            &mut endpoint,
            &mut dispatcher,
            "before-hello",
        );
        before_admission.send(
            "before-admission",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":2.0,"rate":1.0}}),
        );
        drop(before_admission);
        let settle = Instant::now() + Duration::from_millis(250);
        while Instant::now() < settle {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }
        let stage_a_commands = dispatcher.client().unwrap().submitted.borrow().len();
        assert!(stage_a_commands <= 1);

        // Stage B: local submission is visible, but no Runtime admission
        // evidence has arrived when the caller dies.
        let mut after_submission = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut after_submission,
            &mut endpoint,
            &mut dispatcher,
            "submitted-hello",
        );
        after_submission.send(
            "submitted",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":2.5,"rate":1.0}}),
        );
        let submitted = wait_for_frame(
            &mut after_submission,
            &mut endpoint,
            &mut dispatcher,
            |frame| frame["call_id"] == "submitted",
        );
        assert_eq!(submitted["result"]["state"], "submitted");
        drop(after_submission);
        let detach = Instant::now() + Duration::from_millis(250);
        while Instant::now() < detach {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }

        // Stage C: Runtime acceptance is observed, then the caller dies. The
        // terminal worker evidence has no route, but work is neither cancelled
        // nor resubmitted.
        let mut after_acceptance = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut after_acceptance,
            &mut endpoint,
            &mut dispatcher,
            "accepted-hello",
        );
        after_acceptance.send(
            "accepted",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":3.0,"rate":1.0}}),
        );
        let submitted = wait_for_frame(
            &mut after_acceptance,
            &mut endpoint,
            &mut dispatcher,
            |frame| frame["call_id"] == "accepted",
        );
        let accepted_command = submitted["result"]["command_id"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let accepted = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: accepted_command,
            msg_id: "accepted-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationAccepted,
            envelope: json!({"type":"operation","state":"accepted",
                "request_id":{"scope":"scope","seq":"2"}}),
            recovery: Some(recovery_record(2, KnownAdmission::Accepted)),
        });
        endpoint.route_events(&mut dispatcher, accepted);
        let accepted = wait_for_frame(
            &mut after_acceptance,
            &mut endpoint,
            &mut dispatcher,
            |frame| frame["data"]["kind"] == "mutation_accepted",
        );
        assert_eq!(accepted["data"]["call_id"], "accepted");
        drop(after_acceptance);
        let detach = Instant::now() + Duration::from_millis(250);
        while Instant::now() < detach {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }
        let terminal = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: accepted_command,
            msg_id: "accepted-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation","state":"completed",
                "request_id":{"scope":"scope","seq":"2"}}),
            recovery: Some(recovery_record(2, KnownAdmission::Completed)),
        });
        assert!(terminal.is_empty());

        // Stage D: terminal delivery may already be queued/current when the
        // socket dies. Detach releases only wire correlation.
        let mut terminal_delivery = AcceptanceCaller::connect(address);
        complete_acceptance_hello(
            &mut terminal_delivery,
            &mut endpoint,
            &mut dispatcher,
            "terminal-hello",
        );
        terminal_delivery.send(
            "terminal",
            "lab_mutation",
            json!({"op":"reference_retune","args":{"reference":"1","target":4.0,"rate":1.0}}),
        );
        let submitted = wait_for_frame(
            &mut terminal_delivery,
            &mut endpoint,
            &mut dispatcher,
            |frame| frame["call_id"] == "terminal",
        );
        let terminal_command = submitted["result"]["command_id"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let terminal = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: terminal_command,
            msg_id: "terminal-message".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation","state":"completed",
                "request_id":{"scope":"scope","seq":"3"}}),
            recovery: Some(recovery_record(3, KnownAdmission::Completed)),
        });
        endpoint.route_events(&mut dispatcher, terminal);
        drop(terminal_delivery);
        let detach = Instant::now() + Duration::from_millis(250);
        while Instant::now() < detach {
            endpoint.service_owner(&mut dispatcher);
            thread::yield_now();
        }

        let commands = dispatcher.client().unwrap().submitted.borrow();
        assert_eq!(commands.len(), stage_a_commands + 3);
        assert!(
            commands
                .iter()
                .all(|command| command == "mutation:reference_retune")
        );
        drop(commands);
        endpoint.shutdown(&mut dispatcher);
    }

    #[test]
    fn shutdown_is_bounded_and_does_not_submit_runtime_work() {
        let (mut endpoint, _stream, mut dispatcher) = endpoint_pair();
        let started = Instant::now();
        endpoint.shutdown(&mut dispatcher);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());
    }
}
