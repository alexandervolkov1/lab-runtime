//! Pure framing for the narrow Metakon profile used by the M3 proof.
//!
//! This module knows bytes, CRC, type flags and response matching. It neither owns
//! a transport nor decides whether an actuator operation is authorized. The golden
//! vectors were independently rewritten from the read-only `com_port_reader`
//! donor at commit `50d3d1e3de84c650e1aa0ffbf1625044f794d315`.

use crate::Value;

/// Largest accepted frame in the bounded M3 protocol profile.
pub const MAX_FRAME_BYTES: usize = 38;

/// Device/channel/register tuple repeated in every request and response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    device: u8,
    channel: u8,
    register: u8,
}

impl Address {
    /// Build an address. Profile-specific admission validates its meaning.
    pub const fn new(device: u8, channel: u8, register: u8) -> Self {
        Self {
            device,
            channel,
            register,
        }
    }

    /// Return the device address byte.
    pub const fn device(self) -> u8 {
        self.device
    }

    /// Return the channel byte.
    pub const fn channel(self) -> u8 {
        self.channel
    }

    /// Return the register byte.
    pub const fn register(self) -> u8 {
        self.register
    }
}

/// Scalar encodings supported by the deliberately narrow codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetakonType {
    /// One-byte Boolean, encoded as exactly zero or one.
    Bool,
    /// Unsigned eight-bit integer.
    U8,
    /// Signed eight-bit integer.
    I8,
    /// Little-endian unsigned sixteen-bit integer.
    U16,
    /// Little-endian signed sixteen-bit integer.
    I16,
}

impl MetakonType {
    fn code(self) -> u8 {
        match self {
            Self::Bool => 0,
            Self::U8 => 1,
            Self::I8 => 2,
            Self::U16 => 3,
            Self::I16 => 4,
        }
    }

    const fn width(self) -> usize {
        match self {
            Self::Bool | Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
        }
    }
}

/// Decoded scalar whose exact wire kind remains visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetakonValue {
    /// Boolean wire value.
    Bool(bool),
    /// Unsigned eight-bit wire value.
    U8(u8),
    /// Signed eight-bit wire value.
    I8(i8),
    /// Unsigned sixteen-bit wire value.
    U16(u16),
    /// Signed sixteen-bit wire value.
    I16(i16),
}

impl MetakonValue {
    fn value_type(self) -> MetakonType {
        match self {
            Self::Bool(_) => MetakonType::Bool,
            Self::U8(_) => MetakonType::U8,
            Self::I8(_) => MetakonType::I8,
            Self::U16(_) => MetakonType::U16,
            Self::I16(_) => MetakonType::I16,
        }
    }

    fn append_data(self, frame: &mut Frame) {
        match self {
            Self::Bool(value) => frame.push(u8::from(value)),
            Self::U8(value) => frame.push(value),
            Self::I8(value) => frame.push(value as u8),
            Self::U16(value) => frame.extend(&value.to_le_bytes()),
            Self::I16(value) => frame.extend(&value.to_le_bytes()),
        }
    }
}

/// Expected address, type and access flags for strict read-response matching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedRead {
    address: Address,
    value_type: MetakonType,
    readable: bool,
    writable: bool,
}

impl ExpectedRead {
    /// Describe the exact response accepted for one trusted profile operation.
    pub const fn new(
        address: Address,
        value_type: MetakonType,
        readable: bool,
        writable: bool,
    ) -> Self {
        Self {
            address,
            value_type,
            readable,
            writable,
        }
    }

    /// Return the exact response length, including CRC.
    pub const fn frame_len(self) -> usize {
        6 + self.value_type.width()
    }
}

/// Fixed-capacity frame; protocol construction cannot allocate unbounded bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    bytes: [u8; MAX_FRAME_BYTES],
    len: usize,
}

impl Frame {
    fn new() -> Self {
        Self {
            bytes: [0; MAX_FRAME_BYTES],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        self.bytes[self.len] = byte;
        self.len += 1;
    }

    fn extend(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push(byte);
        }
    }

    fn finish_crc(&mut self) {
        self.push(crc(self.as_bytes()));
    }

    /// Borrow the populated prefix; unused capacity is never exposed as wire data.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Typed codec rejection; no transport retry policy is implied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Input exceeds the profile's absolute frame bound.
    FrameTooLong,
    /// Frame length does not match the expected scalar kind.
    WrongLength,
    /// CRC does not match the preceding bytes.
    BadCrc,
    /// Device, channel, register or function differs from the request.
    WrongAddress,
    /// Type or readable/writable flags differ from trusted metadata.
    WrongType,
    /// A scalar byte pattern is invalid for its declared kind.
    InvalidValue,
    /// Domain value cannot be represented exactly by the selected mapping.
    Unrepresentable,
}

impl From<CodecError> for crate::Error {
    fn from(error: CodecError) -> Self {
        Self::Protocol(error)
    }
}

/// Calculate the Metakon one-byte CRC.
///
/// Each input byte is consumed least-significant bit first. Feedback is XORed
/// with `0x18` before the right shift and is inserted into bit seven. The initial
/// value is `0xff`; there is no final XOR.
pub fn crc(bytes: &[u8]) -> u8 {
    let mut checksum = 0xff_u8;
    for &byte in bytes {
        let mut data = byte;
        for _ in 0..8 {
            let feedback = (data ^ checksum) & 1;
            if feedback != 0 {
                checksum ^= 0x18;
            }
            checksum >>= 1;
            checksum |= feedback << 7;
            data >>= 1;
        }
    }
    checksum
}

/// Encode a five-byte read request.
pub fn encode_read(address: Address) -> Result<Frame, CodecError> {
    let mut frame = Frame::new();
    frame.extend(&[address.device, address.channel, address.register, 0]);
    frame.finish_crc();
    Ok(frame)
}

/// Encode one supported scalar write request with little-endian integer data.
pub fn encode_write(address: Address, value: MetakonValue) -> Result<Frame, CodecError> {
    let mut frame = Frame::new();
    frame.extend(&[
        address.device,
        address.channel,
        address.register,
        1,
        value.value_type().code(),
    ]);
    value.append_data(&mut frame);
    frame.finish_crc();
    Ok(frame)
}

/// Decode a read response only when address, function, flags, type, length and CRC match.
pub fn decode_read(bytes: &[u8], expected: ExpectedRead) -> Result<MetakonValue, CodecError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLong);
    }
    if bytes.len() != expected.frame_len() {
        return Err(CodecError::WrongLength);
    }
    validate_bound_and_crc(bytes)?;
    if bytes[..4]
        != [
            expected.address.device,
            expected.address.channel,
            expected.address.register,
            0,
        ]
    {
        return Err(CodecError::WrongAddress);
    }
    let expected_flags = expected.value_type.code()
        | if expected.readable { 0x40 } else { 0 }
        | if expected.writable { 0x80 } else { 0 };
    if bytes[4] != expected_flags {
        return Err(CodecError::WrongType);
    }
    decode_value(expected.value_type, &bytes[5..bytes.len() - 1])
}

/// Validate the exact five-byte acknowledgement for one write address.
pub fn decode_ack(bytes: &[u8], expected: Address) -> Result<(), CodecError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLong);
    }
    if bytes.len() != 5 {
        return Err(CodecError::WrongLength);
    }
    validate_bound_and_crc(bytes)?;
    if bytes[..4] != [expected.device, expected.channel, expected.register, 1] {
        return Err(CodecError::WrongAddress);
    }
    Ok(())
}

fn validate_bound_and_crc(bytes: &[u8]) -> Result<(), CodecError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLong);
    }
    let Some((&received, body)) = bytes.split_last() else {
        return Err(CodecError::WrongLength);
    };
    if crc(body) != received {
        return Err(CodecError::BadCrc);
    }
    Ok(())
}

fn decode_value(value_type: MetakonType, bytes: &[u8]) -> Result<MetakonValue, CodecError> {
    match (value_type, bytes) {
        (MetakonType::Bool, [0]) => Ok(MetakonValue::Bool(false)),
        (MetakonType::Bool, [1]) => Ok(MetakonValue::Bool(true)),
        (MetakonType::Bool, [_]) => Err(CodecError::InvalidValue),
        (MetakonType::U8, [value]) => Ok(MetakonValue::U8(*value)),
        (MetakonType::I8, [value]) => Ok(MetakonValue::I8(*value as i8)),
        (MetakonType::U16, [low, high]) => Ok(MetakonValue::U16(u16::from_le_bytes([*low, *high]))),
        (MetakonType::I16, [low, high]) => Ok(MetakonValue::I16(i16::from_le_bytes([*low, *high]))),
        _ => Err(CodecError::WrongLength),
    }
}

/// Domain interpretation of the Metakon temperature register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemperatureReading {
    /// Finite scaled temperature.
    Value(f64),
    /// The protocol's `i16::MIN` sensor-fault sentinel, never a numeric sample.
    SensorFault,
}

/// Convert register 1 to engineering temperature with explicit sentinel handling.
pub fn scale_temperature(
    value: MetakonValue,
    scale: f64,
) -> Result<TemperatureReading, CodecError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(CodecError::Unrepresentable);
    }
    let MetakonValue::I16(raw) = value else {
        return Err(CodecError::WrongType);
    };
    if raw == i16::MIN {
        return Ok(TemperatureReading::SensorFault);
    }
    if !(-999..=9999).contains(&raw) {
        return Err(CodecError::InvalidValue);
    }
    let engineering = f64::from(raw) * scale;
    if !engineering.is_finite() {
        return Err(CodecError::Unrepresentable);
    }
    Ok(TemperatureReading::Value(engineering))
}

/// Convert a domain float to register 6 without truncation, clamping or coercion.
pub fn encode_scaled_i8(value: &Value, scale: f64) -> Result<i8, CodecError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(CodecError::Unrepresentable);
    }
    let Value::Float(value) = value else {
        return Err(CodecError::WrongType);
    };
    if !value.is_finite() {
        return Err(CodecError::Unrepresentable);
    }
    let raw = *value / scale;
    if !raw.is_finite() || raw.fract() != 0.0 || !(-100.0..=100.0).contains(&raw) {
        return Err(CodecError::Unrepresentable);
    }
    Ok(raw as i8)
}
