//! Trusted compiled read and output plans for bounded declarative simple devices.
//!
//! This module contains no JSON, filesystem, serial-port, or user-programmable
//! parser. The Runtime host validates and compiles external definitions before
//! constructing these fixed plans. Core remains the sole owner of transport
//! correlation and ordinary signal commits.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::{
    AccessMode, Error, InstrumentDescriptor, InstrumentId, ParameterDescriptor, ParameterId,
    ParameterRole, SignalId, Value, ValueType, WriteEffect, model::validate_name,
    signal::SignalBuffer, transport::ResourceId,
};

/// Maximum parameters retained by one compiled simple-device definition.
pub const MAX_SIMPLE_PARAMETERS: usize = 16;
/// Maximum simple-device instances owned by one Runtime.
pub const MAX_SIMPLE_INSTRUMENTS: usize = 32;
/// Maximum compiled response comparisons retained by one read plan.
pub const MAX_SIMPLE_MATCHES: usize = 8;
/// Maximum fixed response prefix covered by a simple-device checksum.
pub const MAX_SIMPLE_CHECKSUM_INPUT_BYTES: usize = 62;

/// Fixed raw scalar representations accepted by the simple-device v1 profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleScalarEncoding {
    /// Unsigned 8-bit integer.
    U8,
    /// Signed 8-bit integer.
    I8,
    /// Little-endian unsigned 16-bit integer.
    U16Le,
    /// Big-endian unsigned 16-bit integer.
    U16Be,
    /// Little-endian signed 16-bit integer.
    I16Le,
    /// Big-endian signed 16-bit integer.
    I16Be,
    /// Little-endian unsigned 32-bit integer.
    U32Le,
    /// Big-endian unsigned 32-bit integer.
    U32Be,
    /// Little-endian signed 32-bit integer.
    I32Le,
    /// Big-endian signed 32-bit integer.
    I32Be,
    /// Little-endian IEEE-754 binary32.
    F32Le,
    /// Big-endian IEEE-754 binary32.
    F32Be,
}

impl SimpleScalarEncoding {
    /// Exact encoded byte width.
    pub const fn width(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16Le | Self::U16Be | Self::I16Le | Self::I16Be => 2,
            Self::U32Le | Self::U32Be | Self::I32Le | Self::I32Be | Self::F32Le | Self::F32Be => 4,
        }
    }

    fn decode(self, bytes: &[u8]) -> Option<f64> {
        let value = match self {
            Self::U8 => f64::from(*bytes.first()?),
            Self::I8 => f64::from(*bytes.first()? as i8),
            Self::U16Le => f64::from(u16::from_le_bytes(bytes.try_into().ok()?)),
            Self::U16Be => f64::from(u16::from_be_bytes(bytes.try_into().ok()?)),
            Self::I16Le => f64::from(i16::from_le_bytes(bytes.try_into().ok()?)),
            Self::I16Be => f64::from(i16::from_be_bytes(bytes.try_into().ok()?)),
            Self::U32Le => f64::from(u32::from_le_bytes(bytes.try_into().ok()?)),
            Self::U32Be => f64::from(u32::from_be_bytes(bytes.try_into().ok()?)),
            Self::I32Le => f64::from(i32::from_le_bytes(bytes.try_into().ok()?)),
            Self::I32Be => f64::from(i32::from_be_bytes(bytes.try_into().ok()?)),
            Self::F32Le => f64::from(f32::from_le_bytes(bytes.try_into().ok()?)),
            Self::F32Be => f64::from(f32::from_be_bytes(bytes.try_into().ok()?)),
        };
        value.is_finite().then_some(value)
    }

    /// Encode one engineering-domain float into this parameter's exact raw form.
    pub fn encode(self, engineering: f64, scale: f64, offset: f64) -> Option<Vec<u8>> {
        if !engineering.is_finite() || !scale.is_finite() || scale == 0.0 || !offset.is_finite() {
            return None;
        }
        let raw = (engineering - offset) / scale;
        if !raw.is_finite() {
            return None;
        }
        let bytes = match self {
            Self::U8 => u8::try_from(exact_integer(raw)?)
                .ok()?
                .to_ne_bytes()
                .to_vec(),
            Self::I8 => i8::try_from(exact_integer(raw)?)
                .ok()?
                .to_ne_bytes()
                .to_vec(),
            Self::U16Le => u16::try_from(exact_integer(raw)?)
                .ok()?
                .to_le_bytes()
                .to_vec(),
            Self::U16Be => u16::try_from(exact_integer(raw)?)
                .ok()?
                .to_be_bytes()
                .to_vec(),
            Self::I16Le => i16::try_from(exact_integer(raw)?)
                .ok()?
                .to_le_bytes()
                .to_vec(),
            Self::I16Be => i16::try_from(exact_integer(raw)?)
                .ok()?
                .to_be_bytes()
                .to_vec(),
            Self::U32Le => u32::try_from(exact_integer(raw)?)
                .ok()?
                .to_le_bytes()
                .to_vec(),
            Self::U32Be => u32::try_from(exact_integer(raw)?)
                .ok()?
                .to_be_bytes()
                .to_vec(),
            Self::I32Le => i32::try_from(exact_integer(raw)?)
                .ok()?
                .to_le_bytes()
                .to_vec(),
            Self::I32Be => i32::try_from(exact_integer(raw)?)
                .ok()?
                .to_be_bytes()
                .to_vec(),
            Self::F32Le | Self::F32Be => {
                let encoded = raw as f32;
                if !encoded.is_finite() || f64::from(encoded) != raw {
                    return None;
                }
                let encoded = if encoded == 0.0 { 0.0 } else { encoded };
                match self {
                    Self::F32Le => encoded.to_le_bytes().to_vec(),
                    Self::F32Be => encoded.to_be_bytes().to_vec(),
                    _ => unreachable!(),
                }
            }
        };
        Some(bytes)
    }

    /// Normalize and validate raw bytes for exact readback comparison.
    pub fn normalize_raw(self, bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.len() != self.width() {
            return None;
        }
        match self {
            Self::F32Le => {
                let value = f32::from_le_bytes(bytes.try_into().ok()?);
                value.is_finite().then(|| {
                    if value == 0.0 { 0.0f32 } else { value }
                        .to_le_bytes()
                        .to_vec()
                })
            }
            Self::F32Be => {
                let value = f32::from_be_bytes(bytes.try_into().ok()?);
                value.is_finite().then(|| {
                    if value == 0.0 { 0.0f32 } else { value }
                        .to_be_bytes()
                        .to_vec()
                })
            }
            _ => Some(bytes.to_vec()),
        }
    }
}

fn exact_integer(value: f64) -> Option<i128> {
    if value.fract() != 0.0 || value < i128::MIN as f64 || value >= 2f64.powi(127) {
        None
    } else {
        Some(value as i128)
    }
}

/// Closed checksum set used by compiled request and response plans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleChecksum {
    /// Sum of prefix bytes modulo 256.
    Sum8,
    /// XOR of every prefix byte.
    Xor8,
    /// CRC-16/Modbus, polynomial 0xA001, initial 0xFFFF, little-endian field.
    Crc16Modbus,
}

impl SimpleChecksum {
    /// Exact checksum field width.
    pub const fn width(self) -> usize {
        match self {
            Self::Sum8 | Self::Xor8 => 1,
            Self::Crc16Modbus => 2,
        }
    }

    /// Append the checksum of `prefix` to a bounded request.
    pub fn append(self, prefix: &[u8], output: &mut Vec<u8>) {
        match self {
            Self::Sum8 => output.push(prefix.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))),
            Self::Xor8 => output.push(prefix.iter().fold(0u8, |sum, byte| sum ^ byte)),
            Self::Crc16Modbus => output.extend_from_slice(&crc16_modbus(prefix).to_le_bytes()),
        }
    }

    fn matches(self, prefix: &[u8], field: &[u8]) -> bool {
        let mut expected = Vec::with_capacity(self.width());
        self.append(prefix, &mut expected);
        expected == field
    }
}

fn crc16_modbus(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Origin of one compiled response comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleResponseMatchKind {
    /// Definition-owned literal evidence.
    Literal,
    /// Concrete instance address/channel evidence.
    Instance,
}

/// One fixed byte comparison in a compiled response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleResponseMatch {
    /// Evidence origin retained for the literal-only overlap rule.
    pub kind: SimpleResponseMatchKind,
    /// First compared response byte.
    pub offset: usize,
    /// Exact expected bytes, already specialized for one instance.
    pub expected: Vec<u8>,
}

/// One optional checksum field in a compiled response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimpleResponseChecksum {
    /// First checksum byte. Coverage is the fixed prefix before this offset.
    pub offset: usize,
    /// Fixed checksum algorithm.
    pub algorithm: SimpleChecksum,
}

/// Immutable strict decoder for one scalar response.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleResponsePlan {
    /// Exact accepted response length.
    pub exact_length: usize,
    /// Fixed literal and instance-specialized comparisons.
    pub matches: Vec<SimpleResponseMatch>,
    /// First byte of the one required scalar.
    pub extract_offset: usize,
    /// Raw representation shared by this parameter's transactions.
    pub encoding: SimpleScalarEncoding,
    /// Finite nonzero raw-to-engineering multiplier.
    pub scale: f64,
    /// Finite raw-to-engineering offset.
    pub engineering_offset: f64,
    /// Optional strict checksum field.
    pub checksum: Option<SimpleResponseChecksum>,
}

impl SimpleResponsePlan {
    fn validate(&self) -> Result<(), Error> {
        use crate::transport::MAX_TRANSACTION_BYTES;

        if !(1..=MAX_TRANSACTION_BYTES).contains(&self.exact_length)
            || self.matches.len() > MAX_SIMPLE_MATCHES
            || !self.scale.is_finite()
            || self.scale == 0.0
            || !self.engineering_offset.is_finite()
        {
            return Err(Error::InvalidConfiguration("invalid simple response plan"));
        }
        let extract_end = self
            .extract_offset
            .checked_add(self.encoding.width())
            .ok_or(Error::InvalidConfiguration(
                "simple response offset overflow",
            ))?;
        if extract_end > self.exact_length {
            return Err(Error::InvalidConfiguration("simple extract out of bounds"));
        }
        let mut occupied: Vec<Option<(u8, SimpleResponseMatchKind)>> =
            vec![None; self.exact_length];
        for comparison in &self.matches {
            if comparison.expected.is_empty() || comparison.expected.len() > 16 {
                return Err(Error::InvalidConfiguration("invalid simple response match"));
            }
            let end = comparison
                .offset
                .checked_add(comparison.expected.len())
                .ok_or(Error::InvalidConfiguration(
                    "simple response offset overflow",
                ))?;
            if end > self.exact_length {
                return Err(Error::InvalidConfiguration("simple match out of bounds"));
            }
            for (index, expected) in comparison.expected.iter().copied().enumerate() {
                let slot = &mut occupied[comparison.offset + index];
                if let Some((prior, prior_kind)) = *slot
                    && (prior != expected
                        || prior_kind != SimpleResponseMatchKind::Literal
                        || comparison.kind != SimpleResponseMatchKind::Literal)
                {
                    return Err(Error::InvalidConfiguration(
                        "invalid simple response match overlap",
                    ));
                }
                *slot = Some((expected, comparison.kind));
            }
        }
        if occupied[self.extract_offset..extract_end]
            .iter()
            .any(Option::is_some)
        {
            return Err(Error::InvalidConfiguration("simple extract overlaps match"));
        }
        if let Some(checksum) = self.checksum {
            let end = checksum
                .offset
                .checked_add(checksum.algorithm.width())
                .ok_or(Error::InvalidConfiguration(
                    "simple checksum offset overflow",
                ))?;
            if checksum.offset == 0
                || checksum.offset > MAX_SIMPLE_CHECKSUM_INPUT_BYTES
                || end > self.exact_length
            {
                return Err(Error::InvalidConfiguration("simple checksum out of bounds"));
            }
            if occupied[checksum.offset..end].iter().any(Option::is_some)
                || ranges_overlap(self.extract_offset, extract_end, checksum.offset, end)
            {
                return Err(Error::InvalidConfiguration("simple checksum overlap"));
            }
        }
        Ok(())
    }

    pub(crate) fn decode(&self, bytes: &[u8], descriptor: &ParameterDescriptor) -> Option<Value> {
        self.decode_with_raw(bytes, descriptor)
            .map(|(value, _)| value)
    }

    pub(crate) fn decode_with_raw(
        &self,
        bytes: &[u8],
        descriptor: &ParameterDescriptor,
    ) -> Option<(Value, Vec<u8>)> {
        let (engineering, normalized_raw) = self.decode_scalar(bytes)?;
        let value = match descriptor.value_spec.value_type() {
            ValueType::Integer
                if engineering.fract() == 0.0
                    && engineering >= i64::MIN as f64
                    && engineering < 9_223_372_036_854_775_808.0 =>
            {
                Value::Integer(engineering as i64)
            }
            ValueType::Float => Value::Float(engineering),
            _ => return None,
        };
        descriptor.value_spec.validate(&value).ok()?;
        Some((value, normalized_raw))
    }

    fn decode_scalar(&self, bytes: &[u8]) -> Option<(f64, Vec<u8>)> {
        if bytes.len() != self.exact_length
            || self.matches.iter().any(|comparison| {
                bytes.get(comparison.offset..comparison.offset + comparison.expected.len())
                    != Some(comparison.expected.as_slice())
            })
        {
            return None;
        }
        if let Some(checksum) = self.checksum {
            let end = checksum.offset + checksum.algorithm.width();
            if !checksum
                .algorithm
                .matches(&bytes[..checksum.offset], &bytes[checksum.offset..end])
            {
                return None;
            }
        }
        let end = self.extract_offset + self.encoding.width();
        let raw_bytes = bytes.get(self.extract_offset..end)?;
        let normalized_raw = self.encoding.normalize_raw(raw_bytes)?;
        let raw = self.encoding.decode(raw_bytes)?;
        let engineering = raw * self.scale + self.engineering_offset;
        if !engineering.is_finite() {
            return None;
        }
        Some((engineering, normalized_raw))
    }

    pub(crate) fn verify_readback(
        &self,
        bytes: &[u8],
        descriptor: &ParameterDescriptor,
        expected_raw: &[u8],
    ) -> Option<(f64, bool)> {
        let (engineering, normalized_raw) = self.decode_scalar(bytes)?;
        let matched = normalized_raw == expected_raw;
        if matched {
            descriptor
                .value_spec
                .validate(&Value::Float(engineering))
                .ok()?;
        }
        Some((engineering, matched))
    }
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
}

/// One immutable concrete read transaction for one parameter and instance.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleReadPlan {
    /// Fully specialized request bytes, including any checksum.
    pub request: Vec<u8>,
    /// Strict response decoder.
    pub response: SimpleResponsePlan,
}

/// One fixed part of a compiled WRITE request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimpleWriteSegment {
    /// Definition or instance bytes fixed at composition.
    Literal(Vec<u8>),
    /// The one encoded actuator scalar.
    Value,
    /// Checksum of all preceding request bytes.
    Checksum(SimpleChecksum),
}

/// Strict fixed-length ACK validator with no scalar extraction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleAckPlan {
    /// Exact accepted response length.
    pub exact_length: usize,
    /// Required semantic literal/instance comparisons.
    pub matches: Vec<SimpleResponseMatch>,
    /// Optional integrity checksum.
    pub checksum: Option<SimpleResponseChecksum>,
}

impl SimpleAckPlan {
    fn validate(&self) -> Result<(), Error> {
        validate_response_evidence(self.exact_length, &self.matches, self.checksum)?;
        if self.matches.is_empty() {
            return Err(Error::InvalidConfiguration(
                "simple ACK lacks semantic match",
            ));
        }
        Ok(())
    }

    pub(crate) fn accepts(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.exact_length
            && self.matches.iter().all(|comparison| {
                bytes.get(comparison.offset..comparison.offset + comparison.expected.len())
                    == Some(comparison.expected.as_slice())
            })
            && self.checksum.is_none_or(|checksum| {
                let end = checksum.offset + checksum.algorithm.width();
                checksum
                    .algorithm
                    .matches(&bytes[..checksum.offset], &bytes[checksum.offset..end])
            })
    }
}

fn validate_response_evidence(
    exact_length: usize,
    matches: &[SimpleResponseMatch],
    checksum: Option<SimpleResponseChecksum>,
) -> Result<Vec<Option<(u8, SimpleResponseMatchKind)>>, Error> {
    use crate::transport::MAX_TRANSACTION_BYTES;
    if !(1..=MAX_TRANSACTION_BYTES).contains(&exact_length) || matches.len() > MAX_SIMPLE_MATCHES {
        return Err(Error::InvalidConfiguration("invalid simple response plan"));
    }
    let mut occupied = vec![None; exact_length];
    for comparison in matches {
        if comparison.expected.is_empty() || comparison.expected.len() > 16 {
            return Err(Error::InvalidConfiguration("invalid simple response match"));
        }
        let end = comparison
            .offset
            .checked_add(comparison.expected.len())
            .ok_or(Error::InvalidConfiguration(
                "simple response offset overflow",
            ))?;
        if end > exact_length {
            return Err(Error::InvalidConfiguration("simple match out of bounds"));
        }
        for (index, expected) in comparison.expected.iter().copied().enumerate() {
            let slot = &mut occupied[comparison.offset + index];
            if let Some((prior, prior_kind)) = *slot
                && (prior != expected
                    || prior_kind != SimpleResponseMatchKind::Literal
                    || comparison.kind != SimpleResponseMatchKind::Literal)
            {
                return Err(Error::InvalidConfiguration(
                    "invalid simple response match overlap",
                ));
            }
            *slot = Some((expected, comparison.kind));
        }
    }
    if let Some(checksum) = checksum {
        let end = checksum
            .offset
            .checked_add(checksum.algorithm.width())
            .ok_or(Error::InvalidConfiguration(
                "simple checksum offset overflow",
            ))?;
        if checksum.offset == 0
            || checksum.offset > MAX_SIMPLE_CHECKSUM_INPUT_BYTES
            || end > exact_length
            || occupied[checksum.offset..end].iter().any(Option::is_some)
        {
            return Err(Error::InvalidConfiguration("simple checksum overlap"));
        }
    }
    Ok(occupied)
}

/// Compiled bounded WRITE, strict ACK, and optional independent readback.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleWritePlan {
    /// Request segments specialized except for the one actuator scalar.
    pub request: Vec<SimpleWriteSegment>,
    /// Strict acknowledgement response.
    pub ack: SimpleAckPlan,
    /// Optional independent readback transaction.
    pub readback: Option<SimpleReadPlan>,
    /// Parameter raw representation.
    pub encoding: SimpleScalarEncoding,
    /// Raw-to-engineering scale.
    pub scale: f64,
    /// Raw-to-engineering offset.
    pub engineering_offset: f64,
}

impl SimpleWritePlan {
    fn validate(&self) -> Result<(), Error> {
        if self.request.is_empty()
            || self.request.len() > 8
            || !self.scale.is_finite()
            || self.scale == 0.0
            || !self.engineering_offset.is_finite()
            || self
                .request
                .iter()
                .filter(|part| matches!(part, SimpleWriteSegment::Value))
                .count()
                != 1
        {
            return Err(Error::InvalidConfiguration("invalid simple write request"));
        }
        let mut encoded_length = 0usize;
        let mut checksum_seen = false;
        for (index, segment) in self.request.iter().enumerate() {
            let width = match segment {
                SimpleWriteSegment::Literal(bytes) if bytes.is_empty() => {
                    return Err(Error::InvalidConfiguration("invalid simple write literal"));
                }
                SimpleWriteSegment::Literal(bytes) => bytes.len(),
                SimpleWriteSegment::Value => self.encoding.width(),
                SimpleWriteSegment::Checksum(algorithm) => {
                    if checksum_seen || index + 1 != self.request.len() {
                        return Err(Error::InvalidConfiguration("invalid simple write checksum"));
                    }
                    checksum_seen = true;
                    algorithm.width()
                }
            };
            encoded_length =
                encoded_length
                    .checked_add(width)
                    .ok_or(Error::InvalidConfiguration(
                        "simple write request length overflow",
                    ))?;
        }
        if !(1..=crate::transport::MAX_TRANSACTION_BYTES).contains(&encoded_length) {
            return Err(Error::InvalidConfiguration(
                "invalid simple write request length",
            ));
        }
        self.ack.validate()?;
        if let Some(readback) = &self.readback {
            if readback.request.is_empty()
                || readback.request.len() > crate::transport::MAX_TRANSACTION_BYTES
            {
                return Err(Error::InvalidConfiguration(
                    "invalid simple readback request",
                ));
            }
            readback.response.validate()?;
            if readback.response.encoding != self.encoding
                || readback.response.scale != self.scale
                || readback.response.engineering_offset != self.engineering_offset
            {
                return Err(Error::InvalidConfiguration(
                    "simple readback encoding differs from write encoding",
                ));
            }
        }
        Ok(())
    }

    /// Encode exact immutable WRITE bytes and retain the normalized raw scalar.
    pub fn encode(&self, engineering: f64) -> Option<(Vec<u8>, Vec<u8>)> {
        let raw = self
            .encoding
            .encode(engineering, self.scale, self.engineering_offset)?;
        let mut request = Vec::with_capacity(crate::transport::MAX_TRANSACTION_BYTES);
        for segment in &self.request {
            match segment {
                SimpleWriteSegment::Literal(bytes) => request.extend_from_slice(bytes),
                SimpleWriteSegment::Value => request.extend_from_slice(&raw),
                SimpleWriteSegment::Checksum(algorithm) => {
                    let prefix = request.clone();
                    algorithm.append(&prefix, &mut request);
                }
            }
            if request.len() > crate::transport::MAX_TRANSACTION_BYTES {
                return None;
            }
        }
        (!request.is_empty()).then_some((request, raw))
    }
}

/// One ordinary parameter plus its trusted compiled read/output plans.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleParameterConfig {
    /// Ordinary descriptor projected to every generic consumer.
    pub descriptor: ParameterDescriptor,
    /// Concrete fixed request and strict response plan.
    pub read: Option<SimpleReadPlan>,
    /// Compiled physical output plan for the sole optional actuator.
    pub write: Option<SimpleWritePlan>,
}

/// Physical binding and completion fences for one simple-device instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimpleDeviceBinding {
    /// Existing serialized byte resource.
    pub resource: ResourceId,
    /// Nonzero resource binding generation.
    pub binding_generation: u64,
    /// Nonzero live mapping/correlation revision.
    pub mapping_revision: u64,
    /// Queue lifetime for the optional physical output.
    pub output_queue_ttl: Option<Duration>,
    /// WRITE/ACK and readback transaction timeout.
    pub output_timeout: Option<Duration>,
}

/// Atomic registration candidate for one simple-device instance.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleDeviceInstrumentConfig {
    /// Stable instrument identity.
    pub id: InstrumentId,
    /// Bounded display name.
    pub name: String,
    /// One to sixteen distinct parameters, with at most one actuator.
    pub parameters: Vec<SimpleParameterConfig>,
    /// Existing physical resource and completion fences.
    pub binding: SimpleDeviceBinding,
    /// Per-signal recent attempt capacity.
    pub history_capacity: usize,
}

pub(crate) struct SimpleDeviceInstrument {
    pub(crate) descriptor: InstrumentDescriptor,
    pub(crate) plans: BTreeMap<ParameterId, SimpleReadPlan>,
    pub(crate) writes: BTreeMap<ParameterId, SimpleWritePlan>,
    pub(crate) binding: SimpleDeviceBinding,
    pub(crate) signals: BTreeMap<SignalId, SignalBuffer>,
}

impl SimpleDeviceInstrument {
    pub(crate) fn new(config: SimpleDeviceInstrumentConfig) -> Result<Self, Error> {
        if config.id.get() == 0
            || config.binding.resource.get() == 0
            || config.binding.binding_generation == 0
            || config.binding.mapping_revision == 0
            || config.parameters.is_empty()
            || config.parameters.len() > MAX_SIMPLE_PARAMETERS
        {
            return Err(Error::InvalidConfiguration(
                "invalid simple-device registration",
            ));
        }
        validate_name(&config.name)?;
        let mut ids = BTreeSet::new();
        let mut descriptors = Vec::with_capacity(config.parameters.len());
        let mut plans = BTreeMap::new();
        let mut signals = BTreeMap::new();
        let mut writes = BTreeMap::new();
        let mut actuator_count = 0usize;
        for parameter in config.parameters {
            let descriptor = parameter.descriptor;
            if descriptor.id.get() == 0 || !ids.insert(descriptor.id) {
                return Err(Error::InvalidConfiguration(
                    "invalid simple-device parameter",
                ));
            }
            descriptor.validate_definition()?;
            let observation = matches!(
                descriptor.role,
                ParameterRole::Measurement | ParameterRole::Diagnostic
            ) && descriptor.access == AccessMode::ReadOnly
                && descriptor.write_effect == WriteEffect::None
                && descriptor.signal == Some(SignalId::new(config.id, descriptor.id))
                && parameter.write.is_none();
            let actuator = descriptor.role == ParameterRole::Actuator
                && matches!(
                    descriptor.access,
                    AccessMode::ReadWrite | AccessMode::WriteOnly
                )
                && descriptor.write_effect == WriteEffect::OutputAffecting
                && descriptor.value_spec.value_type() == ValueType::Float
                && descriptor.signal
                    == (descriptor.access == AccessMode::ReadWrite)
                        .then_some(SignalId::new(config.id, descriptor.id));
            if !observation && !actuator {
                return Err(Error::InvalidConfiguration(
                    "invalid simple-device parameter",
                ));
            }
            if actuator && descriptor.access == AccessMode::ReadWrite {
                let read = parameter.read.as_ref().ok_or(Error::InvalidConfiguration(
                    "simple readable actuator lacks plan",
                ))?;
                let write = parameter.write.as_ref().ok_or(Error::InvalidConfiguration(
                    "simple actuator lacks write plan",
                ))?;
                if read.response.encoding != write.encoding
                    || read.response.scale != write.scale
                    || read.response.engineering_offset != write.engineering_offset
                {
                    return Err(Error::InvalidConfiguration(
                        "simple actuator read encoding differs from write encoding",
                    ));
                }
            }
            if let Some(read) = parameter.read {
                if read.request.is_empty()
                    || read.request.len() > crate::transport::MAX_TRANSACTION_BYTES
                {
                    return Err(Error::InvalidConfiguration("invalid simple request length"));
                }
                read.response.validate()?;
                let signal = descriptor.signal.ok_or(Error::InvalidConfiguration(
                    "simple readable parameter lacks signal",
                ))?;
                signals.insert(signal, SignalBuffer::new(signal, config.history_capacity)?);
                plans.insert(descriptor.id, read);
            } else if observation || descriptor.access == AccessMode::ReadWrite {
                return Err(Error::InvalidConfiguration(
                    "simple readable parameter lacks plan",
                ));
            }
            if let Some(write) = parameter.write {
                if !actuator || actuator_count == 1 {
                    return Err(Error::InvalidConfiguration(
                        "invalid simple-device actuator",
                    ));
                }
                write.validate()?;
                actuator_count += 1;
                writes.insert(descriptor.id, write);
            } else if actuator {
                return Err(Error::InvalidConfiguration(
                    "simple actuator lacks write plan",
                ));
            }
            descriptors.push(descriptor);
        }
        if actuator_count == 1
            && (config.binding.output_queue_ttl.is_none()
                || config.binding.output_timeout.is_none()
                || config.binding.output_queue_ttl == Some(Duration::ZERO)
                || config.binding.output_timeout == Some(Duration::ZERO))
        {
            return Err(Error::InvalidConfiguration(
                "simple actuator lacks physical output timing",
            ));
        }
        Ok(Self {
            descriptor: InstrumentDescriptor {
                id: config.id,
                name: config.name,
                parameters: descriptors,
            },
            plans,
            writes,
            binding: config.binding,
            signals,
        })
    }

    pub(crate) fn configured(&self) -> Vec<(ParameterId, Value)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Command, CommandResult, Query, QueryResult, Runtime, SampleQuality, Unit, ValueSpec,
        output::{
            EvidenceLevel, OutputCommand, OutputOwner, OutputProposal, OutputResult, OutputState,
            SafeProfile,
        },
        transport::{ByteTransport, RecoveryStatus, TransportIoError},
    };
    use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

    fn descriptor(value_spec: ValueSpec) -> ParameterDescriptor {
        descriptor_for(InstrumentId::new(1), value_spec)
    }

    fn descriptor_for(instrument: InstrumentId, value_spec: ValueSpec) -> ParameterDescriptor {
        let parameter = ParameterId::new(2);
        ParameterDescriptor {
            id: parameter,
            name: "temperature".into(),
            value_spec,
            unit: Unit::CELSIUS,
            access: AccessMode::ReadOnly,
            role: ParameterRole::Measurement,
            write_effect: WriteEffect::None,
            signal: Some(SignalId::new(instrument, parameter)),
        }
    }

    #[test]
    fn strict_decoder_checks_matches_checksum_transform_and_range() {
        let plan = SimpleResponsePlan {
            exact_length: 6,
            matches: vec![SimpleResponseMatch {
                kind: SimpleResponseMatchKind::Literal,
                offset: 0,
                expected: vec![0x10, 0x01],
            }],
            extract_offset: 2,
            encoding: SimpleScalarEncoding::I16Be,
            scale: 0.1,
            engineering_offset: 0.0,
            checksum: Some(SimpleResponseChecksum {
                offset: 4,
                algorithm: SimpleChecksum::Crc16Modbus,
            }),
        };
        plan.validate().unwrap();
        let mut response = vec![0x10, 0x01, 0x00, 0xfa];
        let prefix = response.clone();
        SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
        assert_eq!(
            plan.decode(
                &response,
                &descriptor(ValueSpec::Float {
                    min: -50.0,
                    max: 500.0
                })
            ),
            Some(Value::Float(25.0))
        );
        response[1] = 2;
        assert_eq!(
            plan.decode(
                &response,
                &descriptor(ValueSpec::Float {
                    min: -50.0,
                    max: 500.0
                })
            ),
            None
        );
    }

    #[test]
    fn integer_engineering_requires_integral_result() {
        let plan = SimpleResponsePlan {
            exact_length: 1,
            matches: vec![],
            extract_offset: 0,
            encoding: SimpleScalarEncoding::U8,
            scale: 0.5,
            engineering_offset: 0.0,
            checksum: None,
        };
        assert_eq!(
            plan.decode(&[3], &descriptor(ValueSpec::Integer { min: 0, max: 10 })),
            None
        );
    }

    #[test]
    fn output_encoding_is_exact_and_never_rounds_or_clips() {
        assert_eq!(
            SimpleScalarEncoding::U8.encode(255.0, 1.0, 0.0),
            Some(vec![255])
        );
        assert_eq!(
            SimpleScalarEncoding::I8.encode(-128.0, 1.0, 0.0),
            Some(vec![128])
        );
        assert_eq!(
            SimpleScalarEncoding::U16Be.encode(25.0, 0.1, 0.0),
            Some(vec![0, 250])
        );
        assert!(SimpleScalarEncoding::U8.encode(255.5, 1.0, 0.0).is_none());
        assert!(SimpleScalarEncoding::U8.encode(256.0, 1.0, 0.0).is_none());
        assert!(SimpleScalarEncoding::I8.encode(-129.0, 1.0, 0.0).is_none());
        assert!(
            SimpleScalarEncoding::F32Be
                .encode(f64::from(f32::MAX) * 2.0, 1.0, 0.0)
                .is_none()
        );
        assert_eq!(
            SimpleScalarEncoding::F32Be.normalize_raw(&(-0.0f32).to_be_bytes()),
            Some(0.0f32.to_be_bytes().to_vec())
        );
    }

    #[test]
    fn strict_ack_and_exact_raw_readback_do_not_use_engineering_tolerance() {
        let ack = SimpleAckPlan {
            exact_length: 3,
            matches: vec![SimpleResponseMatch {
                kind: SimpleResponseMatchKind::Instance,
                offset: 0,
                expected: vec![0x20, 1],
            }],
            checksum: None,
        };
        ack.validate().unwrap();
        assert!(ack.accepts(&[0x20, 1, 0]));
        assert!(!ack.accepts(&[0x20, 2, 0]));

        let plan = SimpleResponsePlan {
            exact_length: 6,
            matches: vec![SimpleResponseMatch {
                kind: SimpleResponseMatchKind::Literal,
                offset: 0,
                expected: vec![0x21, 1],
            }],
            extract_offset: 2,
            encoding: SimpleScalarEncoding::F32Be,
            scale: 1.0,
            engineering_offset: 0.0,
            checksum: None,
        };
        let descriptor = descriptor(ValueSpec::Float {
            min: -10.0,
            max: 10.0,
        });
        let mut negative_zero = vec![0x21, 1];
        negative_zero.extend_from_slice(&(-0.0f32).to_be_bytes());
        assert_eq!(
            plan.verify_readback(&negative_zero, &descriptor, &0.0f32.to_be_bytes()),
            Some((0.0, true))
        );
        let mut close = vec![0x21, 1];
        close.extend_from_slice(&1.0000001f32.to_be_bytes());
        assert_eq!(
            plan.verify_readback(&close, &descriptor, &1.0f32.to_be_bytes()),
            Some((f64::from(1.0000001f32), false))
        );
        let mut nonfinite = vec![0x21, 1];
        nonfinite.extend_from_slice(&f32::NAN.to_be_bytes());
        assert_eq!(
            plan.verify_readback(&nonfinite, &descriptor, &1.0f32.to_be_bytes()),
            None
        );
    }

    #[test]
    fn nonfinite_float_and_out_of_range_values_are_never_decoded() {
        let float_plan = SimpleResponsePlan {
            exact_length: 4,
            matches: vec![],
            extract_offset: 0,
            encoding: SimpleScalarEncoding::F32Be,
            scale: 1.0,
            engineering_offset: 0.0,
            checksum: None,
        };
        assert_eq!(
            float_plan.decode(
                &f32::NAN.to_be_bytes(),
                &descriptor(ValueSpec::Float {
                    min: -10.0,
                    max: 10.0,
                }),
            ),
            None
        );
        let integer_plan = SimpleResponsePlan {
            exact_length: 1,
            matches: vec![],
            extract_offset: 0,
            encoding: SimpleScalarEncoding::U8,
            scale: 1.0,
            engineering_offset: 0.0,
            checksum: None,
        };
        assert_eq!(
            integer_plan.decode(&[11], &descriptor(ValueSpec::Integer { min: 0, max: 10 })),
            None
        );
    }

    struct ScriptedTransport {
        response: Vec<u8>,
        readable: VecDeque<u8>,
    }

    struct WritableTransport {
        readable: VecDeque<u8>,
        writes: Vec<Vec<u8>>,
        last_raw: [u8; 2],
    }

    fn writable_instrument_config() -> SimpleDeviceInstrumentConfig {
        let instrument = InstrumentId::new(9);
        let parameter = ParameterId::new(2);
        SimpleDeviceInstrumentConfig {
            id: instrument,
            name: "Writable furnace".into(),
            parameters: vec![SimpleParameterConfig {
                descriptor: ParameterDescriptor {
                    id: parameter,
                    name: "Heater".into(),
                    value_spec: ValueSpec::Float {
                        min: 0.0,
                        max: 100.0,
                    },
                    unit: Unit::PERCENT,
                    access: AccessMode::WriteOnly,
                    role: ParameterRole::Actuator,
                    write_effect: WriteEffect::OutputAffecting,
                    signal: None,
                },
                read: None,
                write: Some(SimpleWritePlan {
                    request: vec![
                        SimpleWriteSegment::Literal(vec![0x20, 1]),
                        SimpleWriteSegment::Value,
                    ],
                    ack: SimpleAckPlan {
                        exact_length: 3,
                        matches: vec![SimpleResponseMatch {
                            kind: SimpleResponseMatchKind::Literal,
                            offset: 0,
                            expected: vec![0x20, 1, 0],
                        }],
                        checksum: None,
                    },
                    readback: Some(SimpleReadPlan {
                        request: vec![0x21, 1],
                        response: SimpleResponsePlan {
                            exact_length: 4,
                            matches: vec![SimpleResponseMatch {
                                kind: SimpleResponseMatchKind::Literal,
                                offset: 0,
                                expected: vec![0x21, 1],
                            }],
                            extract_offset: 2,
                            encoding: SimpleScalarEncoding::U16Be,
                            scale: 0.1,
                            engineering_offset: 0.0,
                            checksum: None,
                        },
                    }),
                    encoding: SimpleScalarEncoding::U16Be,
                    scale: 0.1,
                    engineering_offset: 0.0,
                }),
            }],
            binding: SimpleDeviceBinding {
                resource: ResourceId::new(7),
                binding_generation: 1,
                mapping_revision: 1,
                output_queue_ttl: Some(Duration::from_secs(1)),
                output_timeout: Some(Duration::from_secs(1)),
            },
            history_capacity: 8,
        }
    }

    impl ByteTransport for WritableTransport {
        fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
            self.writes.push(bytes.to_vec());
            match bytes.first().copied() {
                Some(0x20) => {
                    self.last_raw
                        .copy_from_slice(bytes.get(2..4).ok_or(TransportIoError::Other)?);
                    self.readable.extend([0x20, 1, 0]);
                }
                Some(0x21) => self
                    .readable
                    .extend([0x21, 1, self.last_raw[0], self.last_raw[1]]),
                _ => return Err(TransportIoError::Other),
            }
            Ok(bytes.len())
        }

        fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
            let count = bytes.len().min(self.readable.len());
            for byte in &mut bytes[..count] {
                *byte = self.readable.pop_front().expect("bounded readable length");
            }
            Ok(count)
        }

        fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
            Ok(RecoveryStatus::Complete)
        }
    }

    #[test]
    fn writable_simple_device_uses_existing_safe_authority_ack_and_exact_readback() {
        let instrument = InstrumentId::new(9);
        let parameter = ParameterId::new(2);
        let actuator = crate::output::ActuatorId::new(instrument, parameter);
        let mut runtime = Runtime::new();
        runtime.enable_recording_facts();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(WritableTransport {
                    readable: VecDeque::new(),
                    writes: Vec::new(),
                    last_raw: [0; 2],
                }),
            )
            .unwrap();
        runtime
            .command(Command::RegisterSimpleDevice(writable_instrument_config()))
            .unwrap();
        runtime.enable_recording_facts();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BindProfile(SafeProfile {
                    min: 0.0,
                    max: 100.0,
                    safe_value: 0.0,
                    max_lease: Duration::from_secs(1),
                    max_proposal_ttl: Duration::from_millis(100),
                    required_evidence: EvidenceLevel::Readback,
                }),
            })
            .unwrap();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::RequestSafe,
            })
            .unwrap();
        runtime
            .command(Command::QueueMetakonOutput {
                actuator,
                at: Duration::ZERO,
                queue_ttl: Duration::from_secs(1),
                timeout: Duration::from_secs(1),
            })
            .unwrap();
        for millisecond in 0..20 {
            runtime
                .command(Command::PollTransports {
                    at: Duration::from_millis(millisecond),
                })
                .unwrap();
        }
        let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
            panic!("output snapshot missing");
        };
        assert_eq!(snapshot.state, OutputState::Disarmed);
        assert!(snapshot.safe_confirmed);
        assert_eq!(snapshot.readback.unwrap().value, 0.0);

        let CommandResult::Output(OutputResult::Lease(lease)) = runtime
            .command(Command::Output {
                actuator,
                at: Duration::from_millis(20),
                command: OutputCommand::Acquire {
                    owner: OutputOwner::Manual(7),
                    lifetime: Duration::from_millis(500),
                },
            })
            .unwrap()
        else {
            panic!("lease missing");
        };
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::from_millis(20),
                command: OutputCommand::Propose(OutputProposal {
                    lease,
                    value: Value::Float(25.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(100),
                }),
            })
            .unwrap();
        runtime
            .command(Command::QueueMetakonOutput {
                actuator,
                at: Duration::from_millis(20),
                queue_ttl: Duration::from_millis(100),
                timeout: Duration::from_millis(100),
            })
            .unwrap();
        for millisecond in 20..40 {
            runtime
                .command(Command::PollTransports {
                    at: Duration::from_millis(millisecond),
                })
                .unwrap();
        }
        let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
            panic!("output snapshot missing");
        };
        assert_eq!(
            snapshot.outcome,
            Some(crate::output::DispatchOutcome::ReadbackVerified)
        );
        assert_eq!(snapshot.readback.unwrap().value, 25.0);
        let facts = runtime.take_recording_facts();
        for expected_stage in [
            crate::recording::OutputStage::SafeRequested,
            crate::recording::OutputStage::SafeSendStarted,
            crate::recording::OutputStage::SafeAcknowledged,
            crate::recording::OutputStage::SafeReadbackVerified,
            crate::recording::OutputStage::Requested,
            crate::recording::OutputStage::Authorized,
            crate::recording::OutputStage::SendStarted,
            crate::recording::OutputStage::Acknowledged,
            crate::recording::OutputStage::ReadbackVerified,
        ] {
            assert!(
                facts.iter().any(|fact| {
                    matches!(
                        fact,
                        crate::recording::RecordingFact::Output {
                            resource: Some(resource),
                            binding_generation: Some(1),
                            mapping_revision: Some(1),
                            stage,
                            ..
                        } if *resource == ResourceId::new(7) && *stage == expected_stage
                    )
                }),
                "missing physical binding context for {expected_stage:?}"
            );
        }
    }

    #[test]
    fn prepared_output_discard_cancels_only_pre_send_work() {
        let instrument = InstrumentId::new(9);
        let actuator = crate::output::ActuatorId::new(instrument, ParameterId::new(2));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(CountingTransport(writes.clone())),
            )
            .unwrap();
        runtime
            .command(Command::RegisterPreparedSimpleDevice(
                writable_instrument_config(),
            ))
            .unwrap();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BindProfile(SafeProfile {
                    min: 0.0,
                    max: 100.0,
                    safe_value: 0.0,
                    max_lease: Duration::from_secs(1),
                    max_proposal_ttl: Duration::from_millis(100),
                    required_evidence: EvidenceLevel::Readback,
                }),
            })
            .unwrap();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::RequestSafe,
            })
            .unwrap();
        runtime
            .command(Command::QueueMetakonOutput {
                actuator,
                at: Duration::ZERO,
                queue_ttl: Duration::from_secs(1),
                timeout: Duration::from_secs(1),
            })
            .unwrap();
        runtime
            .command(Command::DiscardPreparedSimpleDevices {
                instruments: vec![instrument],
            })
            .unwrap();
        assert!(
            runtime
                .query(Query::DescribeInstrument(instrument))
                .is_err()
        );
        let QueryResult::Transport(snapshot) =
            runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
        else {
            panic!("transport snapshot missing");
        };
        assert_eq!(snapshot.queue_len, 0);
        assert!(writes.borrow().is_empty());
    }

    struct CountingTransport(Rc<RefCell<Vec<Vec<u8>>>>);

    impl ByteTransport for CountingTransport {
        fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
            self.0.borrow_mut().push(bytes.to_vec());
            Ok(bytes.len())
        }

        fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
            Ok(0)
        }

        fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
            Ok(RecoveryStatus::Complete)
        }
    }

    #[test]
    fn writable_simple_device_rebind_retires_old_authority_without_transport_bytes() {
        let instrument = InstrumentId::new(9);
        let actuator = crate::output::ActuatorId::new(instrument, ParameterId::new(2));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(CountingTransport(writes.clone())),
            )
            .unwrap();
        runtime
            .command(Command::RegisterSimpleDevice(writable_instrument_config()))
            .unwrap();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BindProfile(SafeProfile {
                    min: 0.0,
                    max: 100.0,
                    safe_value: 0.0,
                    max_lease: Duration::from_secs(1),
                    max_proposal_ttl: Duration::from_millis(100),
                    required_evidence: EvidenceLevel::Readback,
                }),
            })
            .unwrap();
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::RequestSafe,
            })
            .unwrap();
        let CommandResult::Output(OutputResult::Dispatched(dispatch)) = runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BeginDispatch,
            })
            .unwrap()
        else {
            panic!("safe dispatch missing");
        };
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: crate::output::DispatchOutcome::ReadbackVerified,
                },
            })
            .unwrap();
        let binding_before = runtime.simple_device_binding(instrument).unwrap();
        let CommandResult::Output(OutputResult::Lease(lease)) = runtime
            .command(Command::Output {
                actuator,
                at: Duration::from_millis(1),
                command: OutputCommand::Acquire {
                    owner: OutputOwner::Manual(1),
                    lifetime: Duration::from_millis(500),
                },
            })
            .unwrap()
        else {
            panic!("lease missing");
        };
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::from_millis(1),
                command: OutputCommand::Propose(OutputProposal {
                    lease,
                    value: Value::Float(25.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(100),
                }),
            })
            .unwrap();
        let replacement = SimpleDeviceBinding {
            binding_generation: 2,
            mapping_revision: 2,
            ..binding_before
        };
        runtime.take_recording_facts();
        runtime
            .command(Command::RebindSimpleDevice {
                instrument,
                binding: replacement,
                at: Duration::from_millis(2),
            })
            .unwrap();
        assert_eq!(runtime.simple_device_binding(instrument), Some(replacement));
        let QueryResult::Output(rebound) = runtime.query(Query::Output(actuator)).unwrap() else {
            panic!("rebound output missing");
        };
        assert_eq!(rebound.state, crate::output::OutputState::Unverified);
        assert!(rebound.lease.is_none());
        assert!(!rebound.safe_confirmed);
        assert!(
            runtime
                .command(Command::Output {
                    actuator,
                    at: Duration::from_millis(3),
                    command: OutputCommand::Propose(OutputProposal {
                        lease,
                        value: Value::Float(25.0),
                        unit: Unit::PERCENT,
                        ttl: Duration::from_millis(100),
                    }),
                })
                .is_err()
        );
        assert!(writes.borrow().is_empty());
    }

    #[test]
    fn core_rejects_inconsistent_compiled_write_plans_before_registration() {
        fn rejected(config: SimpleDeviceInstrumentConfig) {
            let mut runtime = Runtime::new();
            runtime
                .register_transport(
                    ResourceId::new(7),
                    Box::new(CountingTransport(Rc::new(RefCell::new(Vec::new())))),
                )
                .unwrap();
            assert!(matches!(
                runtime.command(Command::RegisterSimpleDevice(config)),
                Err(Error::InvalidConfiguration(_))
            ));
        }

        let mut cases = Vec::new();
        for scale in [0.0, f64::INFINITY] {
            let mut config = writable_instrument_config();
            config.parameters[0].write.as_mut().unwrap().scale = scale;
            cases.push(config);
        }
        let mut config = writable_instrument_config();
        config.parameters[0]
            .write
            .as_mut()
            .unwrap()
            .engineering_offset = f64::NAN;
        cases.push(config);
        for request in [
            Vec::new(),
            vec![
                SimpleWriteSegment::Literal(Vec::new()),
                SimpleWriteSegment::Value,
            ],
            vec![SimpleWriteSegment::Value, SimpleWriteSegment::Value],
            vec![
                SimpleWriteSegment::Checksum(SimpleChecksum::Sum8),
                SimpleWriteSegment::Value,
            ],
            vec![
                SimpleWriteSegment::Value,
                SimpleWriteSegment::Checksum(SimpleChecksum::Sum8),
                SimpleWriteSegment::Checksum(SimpleChecksum::Xor8),
            ],
            vec![
                SimpleWriteSegment::Literal(vec![0; 63]),
                SimpleWriteSegment::Value,
            ],
        ] {
            let mut config = writable_instrument_config();
            config.parameters[0].write.as_mut().unwrap().request = request;
            cases.push(config);
        }
        for mutate in 0..3 {
            let mut config = writable_instrument_config();
            let write = config.parameters[0].write.as_mut().unwrap();
            let response = &mut write.readback.as_mut().unwrap().response;
            match mutate {
                0 => response.encoding = SimpleScalarEncoding::I16Be,
                1 => response.scale = 0.2,
                _ => response.engineering_offset = 1.0,
            }
            cases.push(config);
        }
        let mut config = writable_instrument_config();
        let parameter = &mut config.parameters[0];
        parameter.descriptor.access = AccessMode::ReadWrite;
        parameter.descriptor.signal = Some(SignalId::new(config.id, parameter.descriptor.id));
        parameter.read = Some(SimpleReadPlan {
            request: vec![0x22, 1],
            response: SimpleResponsePlan {
                exact_length: 4,
                matches: vec![SimpleResponseMatch {
                    kind: SimpleResponseMatchKind::Literal,
                    offset: 0,
                    expected: vec![0x22, 1],
                }],
                extract_offset: 2,
                encoding: SimpleScalarEncoding::U16Be,
                scale: 0.2,
                engineering_offset: 0.0,
                checksum: None,
            },
        });
        cases.push(config);

        for config in cases {
            rejected(config);
        }
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(CountingTransport(Rc::new(RefCell::new(Vec::new())))),
            )
            .unwrap();
        runtime
            .command(Command::RegisterSimpleDevice(writable_instrument_config()))
            .unwrap();
    }

    impl ByteTransport for ScriptedTransport {
        fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
            if self.readable.is_empty() {
                self.readable.extend(self.response.clone());
            }
            Ok(bytes.len())
        }

        fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
            let count = bytes.len().min(self.readable.len());
            for byte in &mut bytes[..count] {
                *byte = self.readable.pop_front().expect("count bounded by queue");
            }
            Ok(count)
        }

        fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
            Ok(RecoveryStatus::Complete)
        }
    }

    fn instrument_config(response: &[u8]) -> SimpleDeviceInstrumentConfig {
        instrument_config_for(InstrumentId::new(1), response)
    }

    fn instrument_config_for(
        instrument: InstrumentId,
        response: &[u8],
    ) -> SimpleDeviceInstrumentConfig {
        SimpleDeviceInstrumentConfig {
            id: instrument,
            name: "Furnace".into(),
            parameters: vec![SimpleParameterConfig {
                descriptor: descriptor_for(
                    instrument,
                    ValueSpec::Float {
                        min: -50.0,
                        max: 500.0,
                    },
                ),
                read: Some(SimpleReadPlan {
                    request: vec![0x10, 1],
                    response: SimpleResponsePlan {
                        exact_length: response.len(),
                        matches: vec![SimpleResponseMatch {
                            kind: SimpleResponseMatchKind::Literal,
                            offset: 0,
                            expected: vec![0x10, 1],
                        }],
                        extract_offset: 2,
                        encoding: SimpleScalarEncoding::I16Be,
                        scale: 0.1,
                        engineering_offset: 0.0,
                        checksum: Some(SimpleResponseChecksum {
                            offset: 4,
                            algorithm: SimpleChecksum::Crc16Modbus,
                        }),
                    },
                }),
                write: None,
            }],
            binding: SimpleDeviceBinding {
                resource: ResourceId::new(7),
                binding_generation: 1,
                mapping_revision: 1,
                output_queue_ttl: None,
                output_timeout: None,
            },
            history_capacity: 8,
        }
    }

    #[test]
    fn simple_device_capacity_is_checked_only_by_simple_registration() {
        let response = [0x10, 1, 0, 250, 0, 0];
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(ScriptedTransport {
                    response: response.to_vec(),
                    readable: VecDeque::new(),
                }),
            )
            .unwrap();
        for id in 1..=MAX_SIMPLE_INSTRUMENTS as u64 {
            runtime
                .command(Command::RegisterSimpleDevice(instrument_config_for(
                    InstrumentId::new(id),
                    &response,
                )))
                .unwrap();
        }
        runtime
            .command(Command::RegisterVirtual(crate::VirtualInstrumentConfig {
                id: InstrumentId::new(1_000),
                name: "ordinary virtual".into(),
                history_capacity: 8,
                base_temperature: 20.0,
                measurement_enabled: true,
            }))
            .unwrap();
        assert!(matches!(
            runtime.command(Command::RegisterSimpleDevice(instrument_config_for(
                InstrumentId::new(33),
                &response,
            ))),
            Err(Error::InvalidConfiguration(
                "simple-device instrument limit reached (32)"
            ))
        ));
    }

    #[test]
    fn runtime_commits_an_ordinary_signal_and_recording_fact() {
        let mut response = vec![0x10, 1, 0, 250];
        let prefix = response.clone();
        SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(ScriptedTransport {
                    response: response.clone(),
                    readable: VecDeque::new(),
                }),
            )
            .unwrap();
        runtime.enable_recording_facts();
        runtime
            .command(Command::RegisterSimpleDevice(instrument_config(&response)))
            .unwrap();
        runtime
            .command(Command::QueueSimpleDeviceRead {
                instrument: InstrumentId::new(1),
                parameter: ParameterId::new(2),
                at: Duration::ZERO,
                queue_ttl: Duration::from_secs(1),
                timeout: Duration::from_secs(1),
            })
            .unwrap();
        runtime
            .command(Command::PollTransports { at: Duration::ZERO })
            .unwrap();
        let QueryResult::Latest(Some(sample)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(1),
                ParameterId::new(2),
            )))
            .unwrap()
        else {
            panic!("ordinary latest sample missing");
        };
        assert_eq!(sample.quality(), SampleQuality::Good);
        assert_eq!(sample.value(), Some(&Value::Float(25.0)));
        assert!(runtime.take_recording_facts().iter().any(|fact| {
            matches!(fact, crate::recording::RecordingFact::Measurement { sample: recorded, .. }
                if recorded == &sample)
        }));
    }

    #[test]
    fn stale_completion_after_rebind_cannot_replace_unavailable_fence() {
        let mut response = vec![0x10, 1, 0, 250];
        let prefix = response.clone();
        SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(ScriptedTransport {
                    response: response.clone(),
                    readable: VecDeque::new(),
                }),
            )
            .unwrap();
        runtime
            .command(Command::RegisterSimpleDevice(instrument_config(&response)))
            .unwrap();
        runtime
            .command(Command::QueueSimpleDeviceRead {
                instrument: InstrumentId::new(1),
                parameter: ParameterId::new(2),
                at: Duration::ZERO,
                queue_ttl: Duration::from_secs(1),
                timeout: Duration::from_secs(1),
            })
            .unwrap();
        runtime
            .command(Command::RebindSimpleDevice {
                instrument: InstrumentId::new(1),
                binding: SimpleDeviceBinding {
                    resource: ResourceId::new(7),
                    binding_generation: 2,
                    mapping_revision: 2,
                    output_queue_ttl: None,
                    output_timeout: None,
                },
                at: Duration::from_millis(1),
            })
            .unwrap();
        runtime
            .command(Command::PollTransports {
                at: Duration::from_millis(1),
            })
            .unwrap();
        let QueryResult::Latest(Some(sample)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(1),
                ParameterId::new(2),
            )))
            .unwrap()
        else {
            panic!("rebind fence sample missing");
        };
        assert_eq!(sample.quality(), SampleQuality::Unavailable);
    }

    #[test]
    fn malformed_protocol_reply_commits_only_unavailable_evidence() {
        let mut response = vec![0x11, 1, 0, 250];
        let prefix = response.clone();
        SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
        let mut runtime = Runtime::new();
        runtime
            .register_transport(
                ResourceId::new(7),
                Box::new(ScriptedTransport {
                    response: response.clone(),
                    readable: VecDeque::new(),
                }),
            )
            .unwrap();
        runtime
            .command(Command::RegisterSimpleDevice(instrument_config(&response)))
            .unwrap();
        runtime
            .command(Command::QueueSimpleDeviceRead {
                instrument: InstrumentId::new(1),
                parameter: ParameterId::new(2),
                at: Duration::ZERO,
                queue_ttl: Duration::from_secs(1),
                timeout: Duration::from_secs(1),
            })
            .unwrap();
        runtime
            .command(Command::PollTransports { at: Duration::ZERO })
            .unwrap();
        let QueryResult::Latest(Some(sample)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(1),
                ParameterId::new(2),
            )))
            .unwrap()
        else {
            panic!("protocol failure sample missing");
        };
        assert_eq!(sample.quality(), SampleQuality::Unavailable);
        assert_eq!(sample.value(), None);
    }
}
