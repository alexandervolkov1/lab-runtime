//! Bounded transport-local NDJSON input and partial-write state.

use super::types::{APPLICATION_JSON_LIMIT, FRAME_LIMIT};
use serde_json::Value;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

/// Connection-local framing failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameError {
    FrameTooLarge,
    InvalidUtf8,
    InvalidJson,
    IncompleteTimeout,
    EncodeFailed,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FrameTooLarge => "NDJSON frame exceeds 16384 bytes",
            Self::InvalidUtf8 => "NDJSON body is not UTF-8",
            Self::InvalidJson => "NDJSON body is not one JSON value",
            Self::IncompleteTimeout => "partial NDJSON frame exceeded its deadline",
            Self::EncodeFailed => "Application JSON could not be encoded within its bound",
        })
    }
}

impl std::error::Error for FrameError {}

/// One bounded input frame under construction.
pub(crate) struct FrameDecoder {
    bytes: Vec<u8>,
    partial_since: Option<Instant>,
}

impl FrameDecoder {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(FRAME_LIMIT),
            partial_since: None,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8], now: Instant) -> Result<Vec<Value>, FrameError> {
        let mut values = Vec::new();
        for &byte in chunk {
            if self.bytes.len() == FRAME_LIMIT {
                return Err(FrameError::FrameTooLarge);
            }
            if self.bytes.is_empty() {
                self.partial_since = Some(now);
            }
            self.bytes.push(byte);
            if byte == b'\n' {
                values.push(decode_frame(&self.bytes)?);
                self.bytes.clear();
                self.partial_since = None;
            }
        }
        Ok(values)
    }

    pub(crate) fn check_deadline(
        &self,
        now: Instant,
        deadline: Duration,
    ) -> Result<(), FrameError> {
        if self
            .partial_since
            .is_some_and(|since| now.saturating_duration_since(since) >= deadline)
        {
            Err(FrameError::IncompleteTimeout)
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    pub(crate) fn is_partial(&self) -> bool {
        !self.bytes.is_empty()
    }
}

fn decode_frame(frame: &[u8]) -> Result<Value, FrameError> {
    if frame.len() > FRAME_LIMIT || frame.last() != Some(&b'\n') {
        return Err(FrameError::FrameTooLarge);
    }
    let body = if frame.len() >= 2 && frame[frame.len() - 2] == b'\r' {
        &frame[..frame.len() - 2]
    } else {
        &frame[..frame.len() - 1]
    };
    if body.len() > APPLICATION_JSON_LIMIT {
        return Err(FrameError::FrameTooLarge);
    }
    std::str::from_utf8(body).map_err(|_| FrameError::InvalidUtf8)?;
    serde_json::from_slice(body).map_err(|_| FrameError::InvalidJson)
}

pub(crate) fn encode_frame(value: &Value) -> Result<Vec<u8>, FrameError> {
    struct Limited {
        bytes: Vec<u8>,
        overflowed: bool,
    }
    impl Write for Limited {
        fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
            if chunk.len() > APPLICATION_JSON_LIMIT.saturating_sub(self.bytes.len()) {
                self.overflowed = true;
                return Err(io::Error::other("Application JSON limit"));
            }
            self.bytes.extend_from_slice(chunk);
            Ok(chunk.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Limited {
        bytes: Vec::with_capacity(1024),
        overflowed: false,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return Err(if writer.overflowed {
            FrameError::FrameTooLarge
        } else {
            FrameError::EncodeFailed
        });
    }
    writer.bytes.push(b'\n');
    Ok(writer.bytes)
}

/// One frame and exact byte offset until the socket accepts it completely.
pub(crate) struct PendingWrite {
    bytes: Vec<u8>,
    offset: usize,
    blocked_since: Option<Instant>,
}

impl PendingWrite {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            offset: 0,
            blocked_since: None,
        }
    }

    pub(crate) fn advance<W: Write>(&mut self, writer: &mut W, now: Instant) -> io::Result<bool> {
        while self.offset < self.bytes.len() {
            match writer.write(&self.bytes[self.offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "socket write zero",
                    ));
                }
                Ok(written) => {
                    self.offset += written;
                    self.blocked_since = None;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.blocked_since.get_or_insert(now);
                    return Ok(false);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    pub(crate) fn timed_out(&self, now: Instant, deadline: Duration) -> bool {
        self.blocked_since
            .is_some_and(|since| now.saturating_duration_since(since) >= deadline)
    }

    pub(crate) fn wrote_any(&self) -> bool {
        self.offset != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;

    #[test]
    fn lf_crlf_partial_and_exact_body_bound_are_compatible() {
        let now = Instant::now();
        let mut decoder = FrameDecoder::new();
        assert!(decoder.push(br#"{"a":1}"#, now).unwrap().is_empty());
        assert!(decoder.is_partial());
        assert_eq!(decoder.push(b"\n", now).unwrap(), vec![json!({"a":1})]);
        assert_eq!(
            decoder.push(b"{\"b\":2}\r\n", now).unwrap(),
            vec![json!({"b":2})]
        );

        let padding = APPLICATION_JSON_LIMIT - 8;
        let value = json!({"x":"x".repeat(padding)});
        let frame = encode_frame(&value).unwrap();
        assert_eq!(frame.len(), FRAME_LIMIT);
        assert_eq!(decoder.push(&frame, now).unwrap(), vec![value]);
    }

    #[test]
    fn oversized_body_and_incomplete_deadline_fail_locally() {
        let value = json!({"x":"x".repeat(APPLICATION_JSON_LIMIT)});
        assert_eq!(encode_frame(&value), Err(FrameError::FrameTooLarge));

        let start = Instant::now();
        let mut decoder = FrameDecoder::new();
        decoder.push(b"{", start).unwrap();
        assert_eq!(
            decoder.check_deadline(start + Duration::from_secs(2), Duration::from_secs(2)),
            Err(FrameError::IncompleteTimeout)
        );
    }

    #[test]
    fn malformed_utf8_json_and_oversized_stream_are_rejected() {
        let now = Instant::now();
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            decoder.push(&[0xff, b'\n'], now),
            Err(FrameError::InvalidUtf8)
        );

        let mut decoder = FrameDecoder::new();
        assert_eq!(
            decoder.push(b"not-json\n", now),
            Err(FrameError::InvalidJson)
        );

        let mut decoder = FrameDecoder::new();
        assert!(
            decoder
                .push(&vec![b'x'; FRAME_LIMIT], now)
                .unwrap()
                .is_empty()
        );
        assert_eq!(decoder.push(b"\n", now), Err(FrameError::FrameTooLarge));
    }

    struct ScriptedWriter {
        actions: VecDeque<Result<usize, io::ErrorKind>>,
        bytes: Vec<u8>,
    }

    impl Write for ScriptedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self.actions.pop_front().expect("scripted write") {
                Ok(limit) => {
                    let written = limit.min(bytes.len());
                    self.bytes.extend_from_slice(&bytes[..written]);
                    Ok(written)
                }
                Err(kind) => Err(io::Error::from(kind)),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_write_identity_is_retained_until_all_bytes_are_accepted() {
        let expected = encode_frame(&json!({"v":1})).unwrap();
        let mut pending = PendingWrite::new(expected.clone());
        let start = Instant::now();
        let mut writer = ScriptedWriter {
            actions: VecDeque::from([Ok(2), Err(io::ErrorKind::WouldBlock), Ok(usize::MAX)]),
            bytes: Vec::new(),
        };
        assert!(!pending.advance(&mut writer, start).unwrap());
        assert!(pending.wrote_any());
        assert!(!pending.timed_out(start + Duration::from_secs(1), Duration::from_secs(2)));
        assert!(pending.timed_out(start + Duration::from_secs(2), Duration::from_secs(2)));
        assert!(
            pending
                .advance(&mut writer, start + Duration::from_secs(3))
                .unwrap()
        );
        assert_eq!(writer.bytes, expected);
    }
}
