//! Version-one JSON is a bounded transport adapter, never a Core domain type.
//!
//! Every frame is checked before recursion/owned values. Duplicate keys fail at
//! all levels, so JSON spelling cannot change command identity invisibly.

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Write},
};

/// Maximum complete NDJSON frame including its line feed.
pub const FRAME_LIMIT: usize = 16_384;
/// Maximum nested JSON arrays/objects admitted from any client.
pub const DEPTH_LIMIT: usize = 16;
/// Maximum lexical JSON values/object members in one frame.
pub const VALUE_LIMIT: usize = 1_024;

/// Stable bounded protocol error, never a Rust enum discriminant on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireError {
    /// Fixed machine-readable error identifier.
    pub code: &'static str,
    /// Fixed bounded explanation with no untrusted frame echo.
    pub message: &'static str,
}
impl WireError {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}
impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for WireError {}

/// Parsed connection correlation and strict arguments, owned by the network layer.
#[derive(Clone, Debug, PartialEq)]
pub struct WireRequest {
    /// Explicit supported protocol version, always one.
    pub v: u8,
    /// Connection-local exchange correlation, not a deduplication key.
    pub msg_id: String,
    /// Fixed advertised operation name.
    pub op: String,
    /// Bounded strict JSON arguments; service mapping converts them to domain data.
    pub args: Value,
    /// Mutating requests additionally carry a bounded process-local operation ID.
    pub request_id: Option<WireRequestId>,
}

/// Scope plus consecutive operation number for process-local deduplication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireRequestId {
    /// Server-issued boot identity and session scope counter.
    pub scope: String,
    /// Canonical decimal u64 sequence (never numeric JSON with precision loss).
    pub seq: u64,
}

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("unique-key JSON")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(Value::Number(Number::from(value))))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(Value::Number(Number::from(value))))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(Unique)
                    .ok_or_else(|| E::custom("nonfinite_number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                if value.len() > 512 {
                    return Err(E::custom("string_too_large"));
                }
                Ok(Unique(Value::String(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Unique, E> {
                if value.len() > 512 {
                    return Err(E::custom("string_too_large"));
                }
                Ok(Unique(Value::String(value)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(Unique(item)) = access.next_element::<Unique>()? {
                    if out.len() >= VALUE_LIMIT {
                        return Err(de::Error::custom("json_values"));
                    }
                    out.push(item);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Unique, A::Error> {
                let mut seen = BTreeSet::new();
                let mut out = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if key.len() > 512 {
                        return Err(de::Error::custom("string_too_large"));
                    }
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate_key"));
                    }
                    if seen.len() > VALUE_LIMIT {
                        return Err(de::Error::custom("json_values"));
                    }
                    let Unique(value) = access.next_value::<Unique>()?;
                    out.insert(key, value);
                }
                Ok(Unique(Value::Object(out)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

/// Validate one complete UTF-8 NDJSON frame and map its fixed v1 operation shape.
pub fn decode_frame(frame: &[u8]) -> Result<WireRequest, WireError> {
    if frame.len() > FRAME_LIMIT {
        return Err(WireError::new(
            "frame_too_large",
            "frame exceeds 16384 bytes",
        ));
    }
    if frame.last() != Some(&b'\n') {
        return Err(WireError::new("incomplete_frame", "frame needs LF"));
    }
    let body = if frame.len() >= 2 && frame[frame.len() - 2] == b'\r' {
        &frame[..frame.len() - 2]
    } else {
        &frame[..frame.len() - 1]
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| WireError::new("invalid_utf8", "frame is not UTF-8"))?;
    lexical_limits(body)?;
    let mut parser = serde_json::Deserializer::from_str(text);
    let Unique(root) =
        Unique::deserialize(&mut parser).map_err(|error| parse_error(&error.to_string()))?;
    parser
        .end()
        .map_err(|_| WireError::new("invalid_json", "trailing JSON data"))?;
    let object = root
        .as_object()
        .ok_or(WireError::new("invalid_shape", "request must be an object"))?;
    allowed_keys(object, &["v", "msg_id", "op", "args", "request_id"])?;
    let v = object
        .get("v")
        .and_then(Value::as_u64)
        .ok_or(WireError::new("invalid_shape", "v must be integer"))?;
    if v != 1 {
        return Err(WireError::new(
            "version_mismatch",
            "only protocol version 1 is supported",
        ));
    }
    let msg_id = object
        .get("msg_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 64)
        .ok_or(WireError::new("invalid_id", "msg_id must be 1..64 bytes"))?
        .to_owned();
    let op = object
        .get("op")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= 64)
        .ok_or(WireError::new(
            "invalid_shape",
            "op must be a bounded string",
        ))?;
    let Some((mutating, keys)) = operation_keys(op) else {
        return Err(WireError::new(
            "unknown_operation",
            "operation is not advertised in v1",
        ));
    };
    let args = object
        .get("args")
        .ok_or(WireError::new("invalid_args", "args object is required"))?;
    let arg_object = args
        .as_object()
        .ok_or(WireError::new("invalid_args", "args must be an object"))?;
    if allowed_keys(arg_object, keys).is_err() {
        return Err(WireError::new("invalid_args", "unknown args field"));
    }
    validate_nested_args(op, arg_object)?;
    let request_id = if mutating {
        let id = object
            .get("request_id")
            .and_then(Value::as_object)
            .ok_or(WireError::new(
                "missing_request_id",
                "commands need request_id",
            ))?;
        allowed_keys(id, &["scope", "seq"])
            .map_err(|_| WireError::new("invalid_id", "request_id fields"))?;
        let scope = id
            .get("scope")
            .and_then(Value::as_str)
            .filter(|scope| !scope.is_empty() && scope.len() <= 64)
            .ok_or(WireError::new("invalid_id", "invalid operation scope"))?
            .to_owned();
        let seq = id
            .get("seq")
            .and_then(Value::as_str)
            .and_then(decimal_u64)
            .ok_or(WireError::new(
                "invalid_id",
                "operation seq must be canonical decimal",
            ))?;
        Some(WireRequestId { scope, seq })
    } else {
        if object.contains_key("request_id") {
            return Err(WireError::new("invalid_args", "queries have no request_id"));
        }
        None
    };
    Ok(WireRequest {
        v: 1,
        msg_id,
        op: op.to_owned(),
        args: args.clone(),
        request_id,
    })
}

/// Encode one owned response/event with the same hard outgoing frame limit.
pub fn encode_frame(value: &Value) -> Result<Vec<u8>, WireError> {
    // serde writes incrementally into a hard-cap writer, so an oversized owned
    // result never creates an unbounded second transport allocation.
    struct Limited {
        bytes: Vec<u8>,
        overflowed: bool,
    }
    impl Write for Limited {
        fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
            if chunk.len() > (FRAME_LIMIT - 1).saturating_sub(self.bytes.len()) {
                self.overflowed = true;
                return Err(io::Error::other("outgoing frame limit"));
            }
            self.bytes.extend_from_slice(chunk);
            Ok(chunk.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut limited = Limited {
        bytes: Vec::with_capacity(1024),
        overflowed: false,
    };
    if serde_json::to_writer(&mut limited, value).is_err() {
        return Err(if limited.overflowed {
            WireError::new("frame_too_large", "response exceeds frame limit")
        } else {
            WireError::new("encode_failed", "response could not be encoded")
        });
    }
    limited.bytes.push(b'\n');
    Ok(limited.bytes)
}

/// Parse u64 without signs, leading zeros or JSON-number precision loss.
pub fn decimal_u64(text: &str) -> Option<u64> {
    if text.is_empty()
        || text.len() > 20
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    text.parse().ok()
}

fn allowed_keys(object: &Map<String, Value>, keys: &[&str]) -> Result<(), WireError> {
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        Err(WireError::new("unknown_field", "unknown field"))
    } else {
        Ok(())
    }
}

fn operation_keys(op: &str) -> Option<(bool, &'static [&'static str])> {
    Some(match op {
        "hello" => (false, &["scope"]),
        "discover" | "runtime_snapshot" => (false, &[]),
        "describe" => (false, &["instrument"]),
        "latest" => (false, &["signal"]),
        "controller" | "controller_start" | "controller_pause" | "controller_resume" => {
            (op != "controller", &["controller"])
        }
        "reference" => (false, &["reference"]),
        "component" => (false, &["component"]),
        "output" => (false, &["actuator"]),
        "operation_status" => (false, &["request_id"]),
        "snapshot_page" => (false, &["snapshot", "index"]),
        "snapshot_release" => (false, &["snapshot"]),
        "subscribe" => (false, &["after", "filter"]),
        "unsubscribe" => (false, &["subscription"]),
        "reference_retune" => (true, &["reference", "expected_revision", "target", "rate"]),
        "controller_configure_pid" => (true, &["controller", "expected_revision", "pid"]),
        "runtime_shutdown" => (true, &[]),
        "recording_status" => (false, &[]),
        "recording_start" => (true, &["label"]),
        "recording_stop" => (true, &["run_id"]),
        "history_read" => (
            true,
            &[
                "mode",
                "database_id",
                "boot_id",
                "run_id",
                "signal",
                "from_ns",
                "to_ns",
                "max_records",
                "cursor",
            ],
        ),
        "history_page" | "history_release" => (false, &["page_token"]),
        _ => return None,
    })
}

fn validate_nested_args(op: &str, args: &Map<String, Value>) -> Result<(), WireError> {
    let mut nested = Vec::new();
    match op {
        "latest" => nested.push(("signal", &["instrument", "parameter"] as &[&str])),
        "output" => nested.push(("actuator", &["instrument", "parameter"])),
        "operation_status" => nested.push(("request_id", &["scope", "seq"])),
        "subscribe" => {
            nested.push(("after", &["boot_id", "seq"]));
            nested.push(("filter", &["kinds", "targets"]));
        }
        "controller_configure_pid" => {
            nested.push(("pid", &["kp", "ki", "kd", "output_min", "output_max"]))
        }
        "recording_stop" => nested.push(("run_id", &["boot_id", "run_no"])),
        "history_read" => {
            if args.get("mode").and_then(Value::as_str) == Some("runs") {
                allowed_keys(args, &["mode", "database_id", "max_records", "cursor"])
                    .map_err(|_| WireError::new("invalid_args", "unknown runs field"))?;
            } else if args.get("mode").and_then(Value::as_str) == Some("measurements") {
                allowed_keys(
                    args,
                    &[
                        "mode",
                        "database_id",
                        "boot_id",
                        "run_id",
                        "signal",
                        "from_ns",
                        "to_ns",
                        "max_records",
                        "cursor",
                    ],
                )
                .map_err(|_| WireError::new("invalid_args", "unknown measurement field"))?;
                nested.push(("run_id", &["boot_id", "run_no"]));
                nested.push(("signal", &["instrument", "parameter"]));
            }
        }
        _ => {}
    }
    for (field, keys) in nested {
        let obj = args
            .get(field)
            .and_then(Value::as_object)
            .ok_or(WireError::new(
                "invalid_args",
                "nested args must be objects",
            ))?;
        allowed_keys(obj, keys)
            .map_err(|_| WireError::new("invalid_args", "unknown nested args field"))?;
    }
    Ok(())
}

fn parse_error(error: &str) -> WireError {
    for (fragment, code, message) in [
        ("duplicate_key", "duplicate_key", "duplicate JSON key"),
        ("json_values", "json_values", "JSON value limit"),
        ("string_too_large", "string_too_large", "JSON string limit"),
        ("nonfinite_number", "invalid_number", "nonfinite number"),
    ] {
        if error.contains(fragment) {
            return WireError::new(code, message);
        }
    }
    WireError::new("invalid_json", "invalid JSON")
}

fn lexical_limits(body: &[u8]) -> Result<(), WireError> {
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0usize;
    // Colons count object member values; each nonempty array contributes its
    // first element and each array comma another. This is a total-frame budget,
    // including singleton objects that contain no comma.
    let mut values = 1usize;
    let mut containers: Vec<(u8, bool)> = Vec::with_capacity(DEPTH_LIMIT);
    for &byte in body {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if byte == b'\\' {
                escape = true
            } else if byte == b'"' {
                in_string = false
            }
            continue;
        }
        if let Some((b'[', first_seen)) = containers.last_mut()
            && !*first_seen
            && !byte.is_ascii_whitespace()
            && byte != b']'
        {
            *first_seen = true;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > DEPTH_LIMIT {
                    return Err(WireError::new("json_depth", "JSON depth exceeds 16"));
                }
                containers.push((byte, false));
            }
            b'}' | b']' => {
                if let Some((b'[', true)) = containers.pop() {
                    values += 1;
                }
                depth = depth.saturating_sub(1);
            }
            b':' => values += 1,
            b',' if containers.last().is_some_and(|(kind, _)| *kind == b'[') => values += 1,
            _ => {}
        }
        if values > VALUE_LIMIT {
            return Err(WireError::new("json_values", "JSON value limit"));
        }
    }
    Ok(())
}
