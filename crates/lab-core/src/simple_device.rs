//! Trusted compiled read-only plans for bounded declarative simple devices.
//!
//! This module contains no JSON, filesystem, serial-port, or user-programmable
//! parser. The Runtime host validates and compiles external definitions before
//! constructing these fixed plans. Core remains the sole owner of transport
//! correlation and ordinary signal commits.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AccessMode, Error, InstrumentDescriptor, InstrumentId, ParameterDescriptor, ParameterId,
    ParameterRole, SignalId, Value, ValueType, WriteEffect, model::validate_name,
    signal::SignalBuffer, transport::ResourceId,
};

/// Maximum parameters retained by one compiled simple-device definition.
pub const MAX_SIMPLE_PARAMETERS: usize = 16;
/// Maximum read-only simple-device instances owned by one Runtime.
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
        let raw = self.encoding.decode(bytes.get(self.extract_offset..end)?)?;
        let engineering = raw * self.scale + self.engineering_offset;
        if !engineering.is_finite() {
            return None;
        }
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
        Some(value)
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

/// One ordinary read-only parameter plus its trusted compiled read plan.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleParameterConfig {
    /// Ordinary descriptor projected to every generic consumer.
    pub descriptor: ParameterDescriptor,
    /// Concrete fixed request and strict response plan.
    pub read: SimpleReadPlan,
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
}

/// Atomic registration candidate for one read-only simple-device instance.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleDeviceInstrumentConfig {
    /// Stable instrument identity.
    pub id: InstrumentId,
    /// Bounded display name.
    pub name: String,
    /// One to sixteen distinct read-only parameters.
    pub parameters: Vec<SimpleParameterConfig>,
    /// Existing physical resource and completion fences.
    pub binding: SimpleDeviceBinding,
    /// Per-signal recent attempt capacity.
    pub history_capacity: usize,
}

pub(crate) struct SimpleDeviceInstrument {
    pub(crate) descriptor: InstrumentDescriptor,
    pub(crate) plans: BTreeMap<ParameterId, SimpleReadPlan>,
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
        for parameter in config.parameters {
            let descriptor = parameter.descriptor;
            if descriptor.id.get() == 0
                || !ids.insert(descriptor.id)
                || descriptor.access != AccessMode::ReadOnly
                || !matches!(
                    descriptor.role,
                    ParameterRole::Measurement | ParameterRole::Diagnostic
                )
                || descriptor.write_effect != WriteEffect::None
                || descriptor.signal != Some(SignalId::new(config.id, descriptor.id))
            {
                return Err(Error::InvalidConfiguration(
                    "invalid simple-device parameter",
                ));
            }
            descriptor.validate_definition()?;
            if parameter.read.request.is_empty()
                || parameter.read.request.len() > crate::transport::MAX_TRANSACTION_BYTES
            {
                return Err(Error::InvalidConfiguration("invalid simple request length"));
            }
            parameter.read.response.validate()?;
            let signal = descriptor.signal.expect("validated simple signal");
            signals.insert(signal, SignalBuffer::new(signal, config.history_capacity)?);
            plans.insert(descriptor.id, parameter.read);
            descriptors.push(descriptor);
        }
        Ok(Self {
            descriptor: InstrumentDescriptor {
                id: config.id,
                name: config.name,
                parameters: descriptors,
            },
            plans,
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
        Command, Query, QueryResult, Runtime, SampleQuality, Unit, ValueSpec,
        transport::{ByteTransport, RecoveryStatus, TransportIoError},
    };
    use std::{collections::VecDeque, time::Duration};

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
                read: SimpleReadPlan {
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
                },
            }],
            binding: SimpleDeviceBinding {
                resource: ResourceId::new(7),
                binding_generation: 1,
                mapping_revision: 1,
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
