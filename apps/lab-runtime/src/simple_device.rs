//! Strict schema and compiler for persistent simple-device definitions.
//!
//! Untrusted JSON and deployment instance fields terminate here. The compiler
//! emits only bounded concrete requests, fixed comparisons, scalar metadata, and
//! ordinary Core descriptors. It owns no transport and performs no I/O.

use crate::configuration::{validate_display_name, validate_key};
use lab_core::{
    AccessMode, InstrumentId, ParameterDescriptor, ParameterId, ParameterRole, SignalId, Unit,
    ValueSpec, WriteEffect,
    simple_device::{
        MAX_SIMPLE_CHECKSUM_INPUT_BYTES, SimpleAckPlan, SimpleChecksum, SimpleParameterConfig,
        SimpleReadPlan, SimpleResponseChecksum, SimpleResponseMatch, SimpleResponseMatchKind,
        SimpleResponsePlan, SimpleScalarEncoding, SimpleWritePlan, SimpleWriteSegment,
    },
    transport::MAX_TRANSACTION_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, sync::Arc};

pub(crate) const MAX_SIMPLE_DEFINITION_BYTES: usize = 8_192;
pub(crate) const MAX_SIMPLE_CANONICAL_BYTES: usize = 6_144;
pub(crate) const MAX_SIMPLE_DEFINITIONS: usize = 16;
pub(crate) const MAX_SIMPLE_INSTANCES: usize = 32;
const MAX_PARAMETERS: usize = 16;
const MAX_REQUEST_SEGMENTS: usize = 8;
const MAX_REQUEST_LITERAL_BYTES: usize = 32;
const MAX_RESPONSE_MATCHES: usize = 8;
const MAX_MATCH_LITERAL_BYTES: usize = 16;
const MAX_NONLITERAL_INSERTS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SimpleDefinitionError(String);

impl SimpleDefinitionError {
    fn new(message: impl AsRef<str>) -> Self {
        Self(message.as_ref().chars().take(256).collect())
    }
}

impl fmt::Display for SimpleDefinitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SimpleDefinitionError {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompiledSimpleDefinition {
    pub(crate) format_version: u16,
    pub(crate) definition_id: String,
    pub(crate) definition_version: u32,
    pub(crate) canonical: Arc<[u8]>,
    pub(crate) canonical_sha256: [u8; 32],
    parameters: Vec<CompiledParameterTemplate>,
}

#[derive(Clone, Debug, PartialEq)]
struct CompiledParameterTemplate {
    id: ParameterId,
    name: String,
    role: ParameterRole,
    unit: Unit,
    value_spec: ValueSpec,
    encoding: EncodingDto,
    access: AccessMode,
    write_effect: WriteEffect,
    read: Option<ReadDto>,
    write: Option<WriteDto>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompiledSimpleInstance {
    pub(crate) parameters: Vec<SimpleParameterConfig>,
    pub(crate) correlations: Vec<CompiledResponseCorrelation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledResponseCorrelation {
    pub(crate) parameter: ParameterId,
    pub(crate) instance_matches: Vec<(usize, Vec<u8>)>,
}

type CompiledResponse = (SimpleResponsePlan, Vec<(usize, Vec<u8>)>);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct DefinitionDto {
    format_version: u16,
    definition_id: String,
    definition_version: u32,
    parameters: Vec<ParameterDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ParameterDto {
    parameter_id: u64,
    key: String,
    display_name: String,
    role: RoleDto,
    access: AccessDto,
    unit_id: String,
    unit_symbol: String,
    value_type: ValueTypeDto,
    engineering_min: serde_json::Value,
    engineering_max: serde_json::Value,
    write_effect: WriteEffectDto,
    encoding: EncodingDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    read: Option<ReadDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    write: Option<WriteDto>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RoleDto {
    Measurement,
    Diagnostic,
    Actuator,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AccessDto {
    ReadOnly,
    ReadWrite,
    WriteOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ValueTypeDto {
    Integer,
    Float,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WriteEffectDto {
    None,
    OutputAffecting,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct EncodingDto {
    raw: RawEncodingDto,
    scale: f64,
    offset: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RawEncodingDto {
    U8,
    I8,
    U16Le,
    U16Be,
    I16Le,
    I16Be,
    U32Le,
    U32Be,
    I32Le,
    I32Be,
    F32Le,
    F32Be,
}

impl From<RawEncodingDto> for SimpleScalarEncoding {
    fn from(value: RawEncodingDto) -> Self {
        match value {
            RawEncodingDto::U8 => Self::U8,
            RawEncodingDto::I8 => Self::I8,
            RawEncodingDto::U16Le => Self::U16Le,
            RawEncodingDto::U16Be => Self::U16Be,
            RawEncodingDto::I16Le => Self::I16Le,
            RawEncodingDto::I16Be => Self::I16Be,
            RawEncodingDto::U32Le => Self::U32Le,
            RawEncodingDto::U32Be => Self::U32Be,
            RawEncodingDto::I32Le => Self::I32Le,
            RawEncodingDto::I32Be => Self::I32Be,
            RawEncodingDto::F32Le => Self::F32Le,
            RawEncodingDto::F32Be => Self::F32Be,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ReadDto {
    request: RequestDto,
    response: ResponseDto,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct WriteDto {
    request: RequestDto,
    ack: ResponseDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    readback: Option<ReadDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RequestDto {
    segments: Vec<RequestSegmentDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RequestSegmentDto {
    Literal {
        hex: String,
    },
    InstanceField {
        field: InstanceFieldDto,
        encoding: InstanceEncodingDto,
    },
    ValueField,
    Checksum {
        algorithm: ChecksumDto,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InstanceFieldDto {
    Address,
    Channel,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InstanceEncodingDto {
    U8,
    U16Le,
    U16Be,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ChecksumDto {
    Sum8,
    Xor8,
    Crc16Modbus,
}

impl From<ChecksumDto> for SimpleChecksum {
    fn from(value: ChecksumDto) -> Self {
        match value {
            ChecksumDto::Sum8 => Self::Sum8,
            ChecksumDto::Xor8 => Self::Xor8,
            ChecksumDto::Crc16Modbus => Self::Crc16Modbus,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ResponseDto {
    exact_length: usize,
    matches: Vec<ResponseMatchDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extract: Option<ScalarExtractDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checksum: Option<ResponseChecksumDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ResponseMatchDto {
    LiteralMatch {
        offset: usize,
        hex: String,
    },
    InstanceMatch {
        offset: usize,
        field: InstanceFieldDto,
        encoding: InstanceEncodingDto,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ScalarExtractDto {
    ScalarExtract { offset: usize },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ResponseChecksumDto {
    Checksum {
        offset: usize,
        algorithm: ChecksumDto,
    },
}

pub(crate) fn parse_simple_definition(
    bytes: &[u8],
) -> Result<CompiledSimpleDefinition, SimpleDefinitionError> {
    if bytes.len() > MAX_SIMPLE_DEFINITION_BYTES {
        return Err(SimpleDefinitionError::new(
            "simple definition exceeds 8192 bytes",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| SimpleDefinitionError::new("simple definition must be UTF-8"))?;
    let mut dto: DefinitionDto = serde_json::from_str(text)
        .map_err(|error| SimpleDefinitionError::new(format!("invalid simple JSON: {error}")))?;
    validate_definition(&dto)?;
    normalize_validated_ranges(&mut dto)?;
    let canonical = serde_json::to_vec(&dto)
        .map_err(|_| SimpleDefinitionError::new("simple definition normalization failed"))?;
    if canonical.len() > MAX_SIMPLE_CANONICAL_BYTES {
        return Err(SimpleDefinitionError::new(
            "canonical simple definition exceeds 6144 bytes",
        ));
    }
    let parameters = dto
        .parameters
        .iter()
        .map(compile_parameter_template)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledSimpleDefinition {
        format_version: dto.format_version,
        definition_id: dto.definition_id,
        definition_version: dto.definition_version,
        canonical_sha256: Sha256::digest(&canonical).into(),
        canonical: Arc::from(canonical),
        parameters,
    })
}

fn normalize_validated_ranges(dto: &mut DefinitionDto) -> Result<(), SimpleDefinitionError> {
    for parameter in &mut dto.parameters {
        match parameter.value_type {
            ValueTypeDto::Integer => {
                let minimum = parameter
                    .engineering_min
                    .as_i64()
                    .ok_or_else(|| SimpleDefinitionError::new("integer minimum must fit i64"))?;
                let maximum = parameter
                    .engineering_max
                    .as_i64()
                    .ok_or_else(|| SimpleDefinitionError::new("integer maximum must fit i64"))?;
                parameter.engineering_min = serde_json::Value::from(minimum);
                parameter.engineering_max = serde_json::Value::from(maximum);
            }
            ValueTypeDto::Float => {
                let minimum = parameter
                    .engineering_min
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| SimpleDefinitionError::new("float minimum must be finite"))?;
                let maximum = parameter
                    .engineering_max
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| SimpleDefinitionError::new("float maximum must be finite"))?;
                parameter.engineering_min = serde_json::Value::from(minimum);
                parameter.engineering_max = serde_json::Value::from(maximum);
            }
        }
    }
    Ok(())
}

fn validate_definition(dto: &DefinitionDto) -> Result<(), SimpleDefinitionError> {
    if dto.format_version != 1 {
        return Err(SimpleDefinitionError::new(
            "unsupported simple format_version",
        ));
    }
    validate_key(&dto.definition_id)
        .map_err(|error| SimpleDefinitionError::new(error.to_string()))?;
    if dto.definition_version == 0 {
        return Err(SimpleDefinitionError::new(
            "definition_version must be nonzero",
        ));
    }
    if dto.parameters.is_empty() || dto.parameters.len() > MAX_PARAMETERS {
        return Err(SimpleDefinitionError::new("parameter count must be 1..=16"));
    }
    let mut ids = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut actuators = 0usize;
    for parameter in &dto.parameters {
        if parameter.parameter_id == 0
            || !ids.insert(parameter.parameter_id)
            || !keys.insert(parameter.key.as_str())
        {
            return Err(SimpleDefinitionError::new(
                "duplicate or zero parameter identity",
            ));
        }
        validate_key(&parameter.key)
            .map_err(|error| SimpleDefinitionError::new(error.to_string()))?;
        validate_display_name(&parameter.display_name)
            .map_err(|error| SimpleDefinitionError::new(error.to_string()))?;
        Unit::new(&parameter.unit_id, &parameter.unit_symbol)
            .map_err(|error| SimpleDefinitionError::new(error.to_string()))?;
        let observation = matches!(parameter.role, RoleDto::Measurement | RoleDto::Diagnostic)
            && parameter.access == AccessDto::ReadOnly
            && parameter.write_effect == WriteEffectDto::None
            && parameter.read.is_some()
            && parameter.write.is_none();
        let actuator = parameter.role == RoleDto::Actuator
            && matches!(
                parameter.access,
                AccessDto::ReadWrite | AccessDto::WriteOnly
            )
            && parameter.write_effect == WriteEffectDto::OutputAffecting
            && parameter.value_type == ValueTypeDto::Float
            && parameter.write.is_some()
            && (parameter.access == AccessDto::WriteOnly || parameter.read.is_some());
        if !observation && !actuator {
            return Err(SimpleDefinitionError::new(
                "invalid simple parameter semantics",
            ));
        }
        if actuator {
            actuators += 1;
            if actuators > 1 {
                return Err(SimpleDefinitionError::new(
                    "at most one actuator is allowed",
                ));
            }
        }
        validate_range(parameter)?;
        validate_encoding(&parameter.encoding)?;
        if let Some(read) = &parameter.read {
            validate_request_shape(&read.request, false, parameter.encoding.raw.into())?;
            validate_response_shape(&read.response, parameter.encoding.raw.into(), true)?;
        }
        if let Some(write) = &parameter.write {
            validate_request_shape(&write.request, true, parameter.encoding.raw.into())?;
            validate_response_shape(&write.ack, parameter.encoding.raw.into(), false)?;
            if write.ack.matches.is_empty() {
                return Err(SimpleDefinitionError::new("ACK requires semantic match"));
            }
            if let Some(readback) = &write.readback {
                validate_request_shape(&readback.request, false, parameter.encoding.raw.into())?;
                validate_response_shape(&readback.response, parameter.encoding.raw.into(), true)?;
            }
        }
    }
    Ok(())
}

fn validate_range(parameter: &ParameterDto) -> Result<(), SimpleDefinitionError> {
    match parameter.value_type {
        ValueTypeDto::Integer => {
            let minimum = parameter
                .engineering_min
                .as_i64()
                .ok_or_else(|| SimpleDefinitionError::new("integer minimum must fit i64"))?;
            let maximum = parameter
                .engineering_max
                .as_i64()
                .ok_or_else(|| SimpleDefinitionError::new("integer maximum must fit i64"))?;
            if minimum > maximum {
                return Err(SimpleDefinitionError::new(
                    "invalid integer engineering range",
                ));
            }
        }
        ValueTypeDto::Float => {
            let minimum = parameter
                .engineering_min
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| SimpleDefinitionError::new("float minimum must be finite"))?;
            let maximum = parameter
                .engineering_max
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| SimpleDefinitionError::new("float maximum must be finite"))?;
            if minimum >= maximum || is_negative_zero(minimum) || is_negative_zero(maximum) {
                return Err(SimpleDefinitionError::new(
                    "invalid float engineering range",
                ));
            }
        }
    }
    Ok(())
}

fn validate_encoding(encoding: &EncodingDto) -> Result<(), SimpleDefinitionError> {
    if !encoding.scale.is_finite()
        || encoding.scale == 0.0
        || is_negative_zero(encoding.scale)
        || !encoding.offset.is_finite()
        || is_negative_zero(encoding.offset)
    {
        return Err(SimpleDefinitionError::new("invalid scalar transform"));
    }
    Ok(())
}

fn is_negative_zero(value: f64) -> bool {
    value == 0.0 && value.is_sign_negative()
}

fn validate_request_shape(
    request: &RequestDto,
    require_value: bool,
    encoding: SimpleScalarEncoding,
) -> Result<(), SimpleDefinitionError> {
    if request.segments.is_empty() || request.segments.len() > MAX_REQUEST_SEGMENTS {
        return Err(SimpleDefinitionError::new(
            "request segment count must be 1..=8",
        ));
    }
    let mut checksum_seen = false;
    let mut inserts = 0usize;
    let mut encoded_length = 0usize;
    let mut value_fields = 0usize;
    for (index, segment) in request.segments.iter().enumerate() {
        let width = match segment {
            RequestSegmentDto::Literal { hex } => decode_hex(hex, MAX_REQUEST_LITERAL_BYTES)?.len(),
            RequestSegmentDto::InstanceField { encoding, .. } => {
                inserts += 1;
                instance_width(*encoding)
            }
            RequestSegmentDto::ValueField => {
                inserts += 1;
                value_fields += 1;
                encoding.width()
            }
            RequestSegmentDto::Checksum { algorithm } => {
                if checksum_seen || index + 1 != request.segments.len() {
                    return Err(SimpleDefinitionError::new(
                        "request checksum must occur once and last",
                    ));
                }
                if encoded_length > MAX_SIMPLE_CHECKSUM_INPUT_BYTES {
                    return Err(SimpleDefinitionError::new(
                        "request checksum input exceeds 62 bytes",
                    ));
                }
                checksum_seen = true;
                SimpleChecksum::from(*algorithm).width()
            }
        };
        encoded_length = encoded_length
            .checked_add(width)
            .ok_or_else(|| SimpleDefinitionError::new("request length overflow"))?;
    }
    if value_fields != usize::from(require_value) {
        return Err(SimpleDefinitionError::new(if require_value {
            "WRITE request requires exactly one value_field"
        } else {
            "READ request cannot contain value_field"
        }));
    }
    if inserts > MAX_NONLITERAL_INSERTS {
        return Err(SimpleDefinitionError::new("too many request field inserts"));
    }
    if !(1..=MAX_TRANSACTION_BYTES).contains(&encoded_length) {
        return Err(SimpleDefinitionError::new(
            "compiled request length must be 1..=64",
        ));
    }
    Ok(())
}

fn validate_response_shape(
    response: &ResponseDto,
    encoding: SimpleScalarEncoding,
    require_extract: bool,
) -> Result<(), SimpleDefinitionError> {
    if !(1..=MAX_TRANSACTION_BYTES).contains(&response.exact_length)
        || response.matches.len() > MAX_RESPONSE_MATCHES
    {
        return Err(SimpleDefinitionError::new(
            "invalid response length or match count",
        ));
    }
    if response.extract.is_some() != require_extract {
        return Err(SimpleDefinitionError::new(
            "invalid response extract cardinality",
        ));
    }
    let extract_range = response
        .extract
        .map(|extract| {
            let ScalarExtractDto::ScalarExtract { offset } = extract;
            let end = offset
                .checked_add(encoding.width())
                .ok_or_else(|| SimpleDefinitionError::new("response offset overflow"))?;
            if end > response.exact_length {
                return Err(SimpleDefinitionError::new("scalar extract out of bounds"));
            }
            Ok((offset, end))
        })
        .transpose()?;
    let mut match_ranges = Vec::with_capacity(response.matches.len());
    let mut match_evidence: Vec<Option<(u8, bool)>> = vec![None; response.exact_length];
    for comparison in &response.matches {
        let (offset, width, literal) = match comparison {
            ResponseMatchDto::LiteralMatch { offset, hex } => {
                let bytes = decode_hex(hex, MAX_MATCH_LITERAL_BYTES)?;
                if offset.saturating_add(bytes.len()) > response.exact_length {
                    return Err(SimpleDefinitionError::new("literal match out of bounds"));
                }
                (*offset, bytes.len(), Some(bytes))
            }
            ResponseMatchDto::InstanceMatch {
                offset, encoding, ..
            } => {
                if offset.saturating_add(instance_width(*encoding)) > response.exact_length {
                    return Err(SimpleDefinitionError::new("instance match out of bounds"));
                }
                (*offset, instance_width(*encoding), None)
            }
        };
        for index in 0..width {
            let (byte, is_literal) = literal
                .as_ref()
                .map_or((0, false), |bytes| (bytes[index], true));
            if let Some((prior, prior_literal)) = match_evidence[offset + index]
                && (!is_literal || !prior_literal || prior != byte)
            {
                return Err(SimpleDefinitionError::new("invalid response match overlap"));
            }
            match_evidence[offset + index] = Some((byte, is_literal));
        }
        if extract_range
            .is_some_and(|(start, end)| ranges_overlap(offset, offset + width, start, end))
        {
            return Err(SimpleDefinitionError::new(
                "extract overlaps response match",
            ));
        }
        match_ranges.push((offset, offset + width));
    }
    if let Some(ResponseChecksumDto::Checksum { offset, algorithm }) = response.checksum {
        let width = SimpleChecksum::from(algorithm).width();
        if offset == 0
            || offset > MAX_SIMPLE_CHECKSUM_INPUT_BYTES
            || offset.saturating_add(width) > response.exact_length
        {
            return Err(SimpleDefinitionError::new(
                "response checksum out of bounds",
            ));
        }
        let checksum_end = offset + width;
        if extract_range
            .is_some_and(|(start, end)| ranges_overlap(start, end, offset, checksum_end))
            || match_ranges
                .iter()
                .any(|(start, end)| ranges_overlap(*start, *end, offset, checksum_end))
        {
            return Err(SimpleDefinitionError::new(
                "checksum overlaps response field",
            ));
        }
    }
    Ok(())
}

fn compile_parameter_template(
    parameter: &ParameterDto,
) -> Result<CompiledParameterTemplate, SimpleDefinitionError> {
    let value_spec = match parameter.value_type {
        ValueTypeDto::Integer => ValueSpec::Integer {
            min: parameter
                .engineering_min
                .as_i64()
                .expect("validated integer"),
            max: parameter
                .engineering_max
                .as_i64()
                .expect("validated integer"),
        },
        ValueTypeDto::Float => ValueSpec::Float {
            min: parameter.engineering_min.as_f64().expect("validated float"),
            max: parameter.engineering_max.as_f64().expect("validated float"),
        },
    };
    Ok(CompiledParameterTemplate {
        id: ParameterId::new(parameter.parameter_id),
        name: parameter.display_name.clone(),
        role: match parameter.role {
            RoleDto::Measurement => ParameterRole::Measurement,
            RoleDto::Diagnostic => ParameterRole::Diagnostic,
            RoleDto::Actuator => ParameterRole::Actuator,
        },
        access: match parameter.access {
            AccessDto::ReadOnly => AccessMode::ReadOnly,
            AccessDto::ReadWrite => AccessMode::ReadWrite,
            AccessDto::WriteOnly => AccessMode::WriteOnly,
        },
        write_effect: match parameter.write_effect {
            WriteEffectDto::None => WriteEffect::None,
            WriteEffectDto::OutputAffecting => WriteEffect::OutputAffecting,
        },
        unit: Unit::new(&parameter.unit_id, &parameter.unit_symbol)
            .map_err(|error| SimpleDefinitionError::new(error.to_string()))?,
        value_spec,
        encoding: parameter.encoding,
        read: parameter.read.clone(),
        write: parameter.write.clone(),
    })
}

pub(crate) fn compile_simple_instance(
    definition: &CompiledSimpleDefinition,
    instrument: InstrumentId,
    address: u16,
    channel: u16,
) -> Result<CompiledSimpleInstance, SimpleDefinitionError> {
    let mut parameters = Vec::with_capacity(definition.parameters.len());
    let mut correlations = Vec::with_capacity(definition.parameters.len());
    for template in &definition.parameters {
        let mut parameter_correlations = Vec::new();
        let read = template
            .read
            .as_ref()
            .map(|read| {
                let request = compile_request(&read.request, address, channel, None)?;
                let (response, instance_matches) = compile_response(
                    &read.response,
                    template.encoding.raw.into(),
                    template.encoding.scale,
                    template.encoding.offset,
                    address,
                    channel,
                )?;
                parameter_correlations.push(instance_matches);
                Ok(SimpleReadPlan { request, response })
            })
            .transpose()?;
        let write = template
            .write
            .as_ref()
            .map(|write| {
                let request = compile_write_request(&write.request, address, channel)?;
                let (ack, ack_matches) = compile_ack(&write.ack, address, channel)?;
                parameter_correlations.push(ack_matches);
                let readback = write
                    .readback
                    .as_ref()
                    .map(|readback| {
                        let request = compile_request(&readback.request, address, channel, None)?;
                        let (response, instance_matches) = compile_response(
                            &readback.response,
                            template.encoding.raw.into(),
                            template.encoding.scale,
                            template.encoding.offset,
                            address,
                            channel,
                        )?;
                        parameter_correlations.push(instance_matches);
                        Ok(SimpleReadPlan { request, response })
                    })
                    .transpose()?;
                Ok(SimpleWritePlan {
                    request,
                    ack,
                    readback,
                    encoding: template.encoding.raw.into(),
                    scale: template.encoding.scale,
                    engineering_offset: template.encoding.offset,
                })
            })
            .transpose()?;
        let descriptor = ParameterDescriptor {
            id: template.id,
            name: template.name.clone(),
            value_spec: template.value_spec.clone(),
            unit: template.unit,
            access: template.access,
            role: template.role,
            write_effect: template.write_effect,
            signal: (template.access != AccessMode::WriteOnly)
                .then_some(SignalId::new(instrument, template.id)),
        };
        descriptor
            .validate_definition()
            .map_err(|error| SimpleDefinitionError::new(error.to_string()))?;
        parameters.push(SimpleParameterConfig {
            descriptor,
            read,
            write,
        });
        correlations.extend(parameter_correlations.into_iter().map(|instance_matches| {
            CompiledResponseCorrelation {
                parameter: template.id,
                instance_matches,
            }
        }));
    }
    Ok(CompiledSimpleInstance {
        parameters,
        correlations,
    })
}

fn compile_request(
    request: &RequestDto,
    address: u16,
    channel: u16,
    value: Option<&[u8]>,
) -> Result<Vec<u8>, SimpleDefinitionError> {
    let mut bytes = Vec::with_capacity(MAX_TRANSACTION_BYTES);
    for segment in &request.segments {
        match segment {
            RequestSegmentDto::Literal { hex } => {
                bytes.extend(decode_hex(hex, MAX_REQUEST_LITERAL_BYTES)?)
            }
            RequestSegmentDto::InstanceField { field, encoding } => {
                bytes.extend(encode_instance(*field, *encoding, address, channel)?);
            }
            RequestSegmentDto::ValueField => bytes.extend_from_slice(
                value.ok_or_else(|| SimpleDefinitionError::new("unexpected value_field"))?,
            ),
            RequestSegmentDto::Checksum { algorithm } => {
                if bytes.len() > MAX_SIMPLE_CHECKSUM_INPUT_BYTES {
                    return Err(SimpleDefinitionError::new(
                        "request checksum input exceeds 62 bytes",
                    ));
                }
                let prefix = bytes.clone();
                SimpleChecksum::from(*algorithm).append(&prefix, &mut bytes);
            }
        }
        if bytes.len() > MAX_TRANSACTION_BYTES {
            return Err(SimpleDefinitionError::new(
                "compiled request exceeds 64 bytes",
            ));
        }
    }
    if bytes.is_empty() {
        return Err(SimpleDefinitionError::new("compiled request is empty"));
    }
    Ok(bytes)
}

fn compile_write_request(
    request: &RequestDto,
    address: u16,
    channel: u16,
) -> Result<Vec<SimpleWriteSegment>, SimpleDefinitionError> {
    let mut segments = Vec::with_capacity(request.segments.len());
    for segment in &request.segments {
        segments.push(match segment {
            RequestSegmentDto::Literal { hex } => {
                SimpleWriteSegment::Literal(decode_hex(hex, MAX_REQUEST_LITERAL_BYTES)?)
            }
            RequestSegmentDto::InstanceField { field, encoding } => {
                SimpleWriteSegment::Literal(encode_instance(*field, *encoding, address, channel)?)
            }
            RequestSegmentDto::ValueField => SimpleWriteSegment::Value,
            RequestSegmentDto::Checksum { algorithm } => {
                SimpleWriteSegment::Checksum((*algorithm).into())
            }
        });
    }
    Ok(segments)
}

fn compile_ack(
    response: &ResponseDto,
    address: u16,
    channel: u16,
) -> Result<CompiledAck, SimpleDefinitionError> {
    let (matches, instance_matches, checksum) =
        compile_response_evidence(response, address, channel)?;
    Ok((
        SimpleAckPlan {
            exact_length: response.exact_length,
            matches,
            checksum,
        },
        instance_matches,
    ))
}

fn compile_response(
    response: &ResponseDto,
    encoding: SimpleScalarEncoding,
    scale: f64,
    engineering_offset: f64,
    address: u16,
    channel: u16,
) -> Result<CompiledResponse, SimpleDefinitionError> {
    let (matches, instance_matches, checksum) =
        compile_response_evidence(response, address, channel)?;
    let mut occupied: Vec<Option<(u8, SimpleResponseMatchKind)>> =
        vec![None; response.exact_length];
    for comparison in &matches {
        for (index, expected_byte) in comparison.expected.iter().copied().enumerate() {
            occupied[comparison.offset + index] = Some((expected_byte, comparison.kind));
        }
    }
    let Some(ScalarExtractDto::ScalarExtract {
        offset: extract_offset,
    }) = response.extract
    else {
        return Err(SimpleDefinitionError::new("scalar response lacks extract"));
    };
    let extract_end = extract_offset + encoding.width();
    if occupied[extract_offset..extract_end]
        .iter()
        .any(Option::is_some)
    {
        return Err(SimpleDefinitionError::new(
            "extract overlaps response match",
        ));
    }
    Ok((
        SimpleResponsePlan {
            exact_length: response.exact_length,
            matches,
            extract_offset,
            encoding,
            scale,
            engineering_offset,
            checksum,
        },
        instance_matches,
    ))
}

fn compile_response_evidence(
    response: &ResponseDto,
    address: u16,
    channel: u16,
) -> Result<CompiledEvidence, SimpleDefinitionError> {
    let mut matches = Vec::with_capacity(response.matches.len());
    let mut instance_matches = Vec::new();
    let mut occupied: Vec<Option<(u8, SimpleResponseMatchKind)>> =
        vec![None; response.exact_length];
    for comparison in &response.matches {
        let (offset, expected, instance) = match comparison {
            ResponseMatchDto::LiteralMatch { offset, hex } => {
                (*offset, decode_hex(hex, MAX_MATCH_LITERAL_BYTES)?, false)
            }
            ResponseMatchDto::InstanceMatch {
                offset,
                field,
                encoding,
            } => (
                *offset,
                encode_instance(*field, *encoding, address, channel)?,
                true,
            ),
        };
        let kind = if instance {
            SimpleResponseMatchKind::Instance
        } else {
            SimpleResponseMatchKind::Literal
        };
        for (index, expected_byte) in expected.iter().copied().enumerate() {
            let slot = &mut occupied[offset + index];
            if let Some((prior, prior_kind)) = *slot
                && (prior != expected_byte
                    || prior_kind != SimpleResponseMatchKind::Literal
                    || kind != SimpleResponseMatchKind::Literal)
            {
                return Err(SimpleDefinitionError::new("invalid response match overlap"));
            }
            *slot = Some((expected_byte, kind));
        }
        if instance {
            instance_matches.push((offset, expected.clone()));
        }
        matches.push(SimpleResponseMatch {
            kind,
            offset,
            expected,
        });
    }
    let checksum = response.checksum.map(|checksum| match checksum {
        ResponseChecksumDto::Checksum { offset, algorithm } => SimpleResponseChecksum {
            offset,
            algorithm: algorithm.into(),
        },
    });
    if let Some(checksum) = checksum {
        if checksum.offset > MAX_SIMPLE_CHECKSUM_INPUT_BYTES {
            return Err(SimpleDefinitionError::new(
                "response checksum input exceeds 62 bytes",
            ));
        }
        let end = checksum.offset + checksum.algorithm.width();
        if occupied[checksum.offset..end].iter().any(Option::is_some) {
            return Err(SimpleDefinitionError::new(
                "checksum overlaps response field",
            ));
        }
    }
    Ok((matches, instance_matches, checksum))
}

type CompiledEvidence = (
    Vec<SimpleResponseMatch>,
    Vec<(usize, Vec<u8>)>,
    Option<SimpleResponseChecksum>,
);
type CompiledAck = (SimpleAckPlan, Vec<(usize, Vec<u8>)>);

fn encode_instance(
    field: InstanceFieldDto,
    encoding: InstanceEncodingDto,
    address: u16,
    channel: u16,
) -> Result<Vec<u8>, SimpleDefinitionError> {
    let value = match field {
        InstanceFieldDto::Address => address,
        InstanceFieldDto::Channel => channel,
    };
    match encoding {
        InstanceEncodingDto::U8 => u8::try_from(value)
            .map(|value| vec![value])
            .map_err(|_| SimpleDefinitionError::new("instance field does not fit u8")),
        InstanceEncodingDto::U16Le => Ok(value.to_le_bytes().to_vec()),
        InstanceEncodingDto::U16Be => Ok(value.to_be_bytes().to_vec()),
    }
}

fn instance_width(encoding: InstanceEncodingDto) -> usize {
    match encoding {
        InstanceEncodingDto::U8 => 1,
        InstanceEncodingDto::U16Le | InstanceEncodingDto::U16Be => 2,
    }
}

fn decode_hex(value: &str, maximum: usize) -> Result<Vec<u8>, SimpleDefinitionError> {
    if value.is_empty()
        || !value.len().is_multiple_of(2)
        || value.len() / 2 > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SimpleDefinitionError::new("invalid lowercase bounded hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("validated ASCII");
            u8::from_str_radix(text, 16).map_err(|_| SimpleDefinitionError::new("invalid hex"))
        })
        .collect()
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
}

pub(crate) fn correlations_distinguish(
    left: &CompiledResponseCorrelation,
    right: &CompiledResponseCorrelation,
) -> bool {
    left.instance_matches
        .iter()
        .any(|(left_offset, left_bytes)| {
            right
                .instance_matches
                .iter()
                .any(|(right_offset, right_bytes)| {
                    let start = (*left_offset).max(*right_offset);
                    let end =
                        (left_offset + left_bytes.len()).min(right_offset + right_bytes.len());
                    start < end
                        && (start..end).any(|offset| {
                            left_bytes[offset - left_offset] != right_bytes[offset - right_offset]
                        })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        configuration::{
            ArtifactReader, ConfigurationError, MAX_PROPERTY_OVERLAYS, PropertyValue,
            parse_runtime_toml,
        },
        host::{Clock, HostCore},
        recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore},
    };
    use lab_core::{
        Command, Query, QueryResult,
        control::{ControllerId, ControllerState},
        output::{ActuatorId, DispatchOutcome, OutputState},
        simple_device::SimpleChecksum,
        transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
    };
    use serde_json::{Value, json};
    use std::{
        cell::RefCell,
        collections::{BTreeMap, VecDeque},
        path::{Path, PathBuf},
        rc::Rc,
        time::{Duration, Instant},
    };

    fn definition() -> Vec<u8> {
        br#"{"format_version":1,"definition_id":"furnace-v1","definition_version":1,"parameters":[{"parameter_id":1,"key":"temperature","display_name":"Temperature","role":"measurement","access":"read_only","unit_id":"degC","unit_symbol":"C","value_type":"float","engineering_min":-50.0,"engineering_max":500.0,"write_effect":"none","encoding":{"raw":"i16_be","scale":0.1,"offset":0.0},"read":{"request":{"segments":[{"type":"literal","hex":"10"},{"type":"instance_field","field":"address","encoding":"u8"},{"type":"instance_field","field":"channel","encoding":"u8"},{"type":"checksum","algorithm":"crc16_modbus"}]},"response":{"exact_length":7,"matches":[{"type":"literal_match","offset":0,"hex":"10"},{"type":"instance_match","offset":1,"field":"address","encoding":"u8"},{"type":"instance_match","offset":2,"field":"channel","encoding":"u8"}],"extract":{"type":"scalar_extract","offset":3},"checksum":{"type":"checksum","offset":5,"algorithm":"crc16_modbus"}}}}]}"#.to_vec()
    }

    fn writable_definition() -> Vec<u8> {
        br#"{"format_version":1,"definition_id":"furnace-output-v1","definition_version":1,"parameters":[{"parameter_id":2,"key":"heater","display_name":"Heater","role":"actuator","access":"read_write","unit_id":"percent","unit_symbol":"%","value_type":"float","engineering_min":0.0,"engineering_max":100.0,"write_effect":"output_affecting","encoding":{"raw":"u16_be","scale":0.1,"offset":0.0},"read":{"request":{"segments":[{"type":"literal","hex":"22"},{"type":"instance_field","field":"address","encoding":"u8"}]},"response":{"exact_length":4,"matches":[{"type":"literal_match","offset":0,"hex":"22"},{"type":"instance_match","offset":1,"field":"address","encoding":"u8"}],"extract":{"type":"scalar_extract","offset":2}}},"write":{"request":{"segments":[{"type":"literal","hex":"20"},{"type":"instance_field","field":"address","encoding":"u8"},{"type":"instance_field","field":"channel","encoding":"u8"},{"type":"value_field"},{"type":"checksum","algorithm":"crc16_modbus"}]},"ack":{"exact_length":5,"matches":[{"type":"literal_match","offset":0,"hex":"20"},{"type":"instance_match","offset":1,"field":"address","encoding":"u8"},{"type":"literal_match","offset":2,"hex":"00"}],"checksum":{"type":"checksum","offset":3,"algorithm":"crc16_modbus"}},"readback":{"request":{"segments":[{"type":"literal","hex":"21"},{"type":"instance_field","field":"address","encoding":"u8"},{"type":"instance_field","field":"channel","encoding":"u8"}]},"response":{"exact_length":5,"matches":[{"type":"literal_match","offset":0,"hex":"21"},{"type":"instance_match","offset":1,"field":"address","encoding":"u8"}],"extract":{"type":"scalar_extract","offset":3}}}}}]}"#.to_vec()
    }

    fn controlled_writable_definition() -> Vec<u8> {
        let mut combined: Value = serde_json::from_slice(&definition()).unwrap();
        let writable: Value = serde_json::from_slice(&writable_definition()).unwrap();
        combined["definition_id"] = json!("controlled-furnace-v1");
        combined["parameters"]
            .as_array_mut()
            .unwrap()
            .push(writable["parameters"][0].clone());
        serde_json::to_vec(&combined).unwrap()
    }

    #[test]
    fn writable_schema_compiles_exact_write_ack_and_readback() {
        let definition = parse_simple_definition(&writable_definition()).unwrap();
        let instance = compile_simple_instance(&definition, InstrumentId::new(1001), 1, 2).unwrap();
        let parameter = &instance.parameters[0];
        assert_eq!(parameter.descriptor.role, ParameterRole::Actuator);
        assert_eq!(parameter.descriptor.access, AccessMode::ReadWrite);
        let write = parameter.write.as_ref().unwrap();
        let (request, raw) = write.encode(25.0).unwrap();
        assert_eq!(raw, [0x00, 0xfa]);
        assert_eq!(&request[..5], &[0x20, 1, 2, 0, 0xfa]);
        assert_eq!(request.len(), 7);
        assert_eq!(write.ack.exact_length, 5);
        assert!(write.readback.is_some());
    }

    #[test]
    fn writable_schema_rejects_invalid_actuator_and_ack_shapes() {
        let base: Value = serde_json::from_slice(&writable_definition()).unwrap();
        let mut second = base.clone();
        second["parameters"]
            .as_array_mut()
            .unwrap()
            .push(base["parameters"][0].clone());
        second["parameters"][1]["parameter_id"] = json!(3);
        second["parameters"][1]["key"] = json!("heater2");
        assert!(parse_simple_definition(&serde_json::to_vec(&second).unwrap()).is_err());

        for (field, value) in [
            ("value_type", json!("integer")),
            ("access", json!("read_only")),
            ("write_effect", json!("none")),
        ] {
            let mut invalid = base.clone();
            invalid["parameters"][0][field] = value;
            assert!(parse_simple_definition(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut checksum_only = base;
        checksum_only["parameters"][0]["write"]["ack"]["matches"] = json!([]);
        assert!(parse_simple_definition(&serde_json::to_vec(&checksum_only).unwrap()).is_err());
    }

    fn hash_hex(hash: [u8; 32]) -> String {
        hash.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn strict_definition_compiles_exact_instance_bytes_and_canonical_hash() {
        let definition = parse_simple_definition(&definition()).unwrap();
        let compiled = compile_simple_instance(&definition, InstrumentId::new(1001), 1, 2).unwrap();
        let request = &compiled.parameters[0].read.as_ref().unwrap().request;
        assert_eq!(&request[..3], &[0x10, 1, 2]);
        assert_eq!(request.len(), 5);
        assert!(definition.canonical.len() <= MAX_SIMPLE_CANONICAL_BYTES);
        assert_eq!(
            hash_hex(definition.canonical_sha256),
            "d761429af6f2d511150ca71b2cdb96f95d49c5c4c9cc91c527b76754c78b1c7c"
        );
    }

    #[test]
    fn canonical_definition_has_fixed_bytes_and_hash_across_json_spelling() {
        let expected = definition();
        let original = parse_simple_definition(&expected).unwrap();
        assert_eq!(original.canonical.as_ref(), expected.as_slice());

        let value: Value = serde_json::from_slice(&expected).unwrap();
        let reordered = format!(
            "\n{{\n  \"parameters\": {},\n  \"definition_version\": 1,\n  \"definition_id\": \"furnace-v1\",\n  \"format_version\": 1\n}}\n",
            value["parameters"]
        );
        let alternate = parse_simple_definition(reordered.as_bytes()).unwrap();
        assert_eq!(alternate.canonical.as_ref(), expected.as_slice());
        assert_eq!(alternate.canonical_sha256, original.canonical_sha256);
        assert_eq!(
            hash_hex(alternate.canonical_sha256),
            "d761429af6f2d511150ca71b2cdb96f95d49c5c4c9cc91c527b76754c78b1c7c"
        );

        let expected_text = std::str::from_utf8(&expected).unwrap();
        for numeric_spelling in [
            expected_text
                .replace("\"engineering_min\":-50.0", "\"engineering_min\":-50")
                .replace("\"engineering_max\":500.0", "\"engineering_max\":500"),
            expected_text
                .replace("\"engineering_min\":-50.0", "\"engineering_min\":-5e1")
                .replace("\"engineering_max\":500.0", "\"engineering_max\":5e2"),
        ] {
            let equivalent = parse_simple_definition(numeric_spelling.as_bytes()).unwrap();
            assert_eq!(equivalent.canonical.as_ref(), expected.as_slice());
            assert_eq!(equivalent.canonical_sha256, original.canonical_sha256);
            assert_eq!(
                hash_hex(equivalent.canonical_sha256),
                "d761429af6f2d511150ca71b2cdb96f95d49c5c4c9cc91c527b76754c78b1c7c"
            );
        }
    }

    #[test]
    fn equivalent_float_bound_spellings_share_definition_identity() {
        struct TwoDefinitionReader {
            first: Vec<u8>,
            second: Vec<u8>,
        }
        impl ArtifactReader for TwoDefinitionReader {
            fn read(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
                assert_eq!(maximum, MAX_SIMPLE_DEFINITION_BYTES);
                if path.ends_with("a.json") {
                    Ok(self.first.clone())
                } else if path.ends_with("b.json") {
                    Ok(self.second.clone())
                } else {
                    panic!("unexpected definition path: {}", path.display());
                }
            }
        }

        let config = br#"schema_version=1
[runtime]
key="bench"
display_name="Bench"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com_read_only"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
kind="simple_device"
id=1001
key="furnace-1"
display_name="Furnace 1"
definition="definitions/a.json"
resource_id=7
address=1
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
[[instruments]]
kind="simple_device"
id=1002
key="furnace-2"
display_name="Furnace 2"
definition="definitions/b.json"
resource_id=7
address=2
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#;
        let first = definition();
        let first_text = std::str::from_utf8(&first).unwrap();
        let equivalent = first_text
            .replace("\"engineering_min\":-50.0", "\"engineering_min\":-50")
            .replace("\"engineering_max\":500.0", "\"engineering_max\":500")
            .into_bytes();
        parse_runtime_toml(
            config,
            Path::new("C:/bench"),
            &mut TwoDefinitionReader {
                first: first.clone(),
                second: equivalent,
            },
        )
        .expect("equivalent float spellings must share normalized identity");

        let different = first_text
            .replace("\"engineering_max\":500.0", "\"engineering_max\":501.0")
            .into_bytes();
        let error = parse_runtime_toml(
            config,
            Path::new("C:/bench"),
            &mut TwoDefinitionReader {
                first,
                second: different,
            },
        )
        .expect_err("different float bounds must conflict for reused ID/version");
        assert!(error.to_string().contains("conflicting normalized content"));
    }

    #[test]
    fn unknown_field_and_u8_instance_overflow_are_rejected() {
        let mut value: serde_json::Value = serde_json::from_slice(&definition()).unwrap();
        value["extra"] = serde_json::json!(true);
        assert!(parse_simple_definition(&serde_json::to_vec(&value).unwrap()).is_err());
        let definition = parse_simple_definition(&definition()).unwrap();
        assert!(compile_simple_instance(&definition, InstrumentId::new(1001), 256, 0).is_err());
    }

    fn parsed_definition() -> Value {
        serde_json::from_slice(&definition()).unwrap()
    }

    fn encoded(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn raw_and_canonical_definition_bounds_are_independent_and_exact() {
        let compact = definition();
        let mut exact_raw = compact.clone();
        exact_raw.resize(MAX_SIMPLE_DEFINITION_BYTES, b' ');
        assert_eq!(exact_raw.len(), MAX_SIMPLE_DEFINITION_BYTES);
        parse_simple_definition(&exact_raw).unwrap();
        exact_raw.push(b' ');
        assert!(parse_simple_definition(&exact_raw).is_err());

        let mut value = parsed_definition();
        let template = value["parameters"][0].clone();
        value["parameters"].as_array_mut().unwrap().clear();
        for index in 1..=MAX_PARAMETERS {
            let mut parameter = template.clone();
            parameter["parameter_id"] = json!(index);
            parameter["key"] = json!(format!("parameter-{index}"));
            parameter["display_name"] = json!("x".repeat(128));
            parameter["read"]["response"]["matches"] = json!([
                {"type":"literal_match","offset":0,"hex":"00"},
                {"type":"literal_match","offset":1,"hex":"01"},
                {"type":"literal_match","offset":2,"hex":"02"},
                {"type":"literal_match","offset":3,"hex":"03"},
                {"type":"literal_match","offset":4,"hex":"04"},
                {"type":"literal_match","offset":5,"hex":"05"},
                {"type":"literal_match","offset":6,"hex":"06"},
                {"type":"literal_match","offset":7,"hex":"07"}
            ]);
            parameter["read"]["response"]["exact_length"] = json!(12);
            parameter["read"]["response"]["extract"]["offset"] = json!(8);
            parameter["read"]["response"]
                .as_object_mut()
                .unwrap()
                .remove("checksum");
            value["parameters"].as_array_mut().unwrap().push(parameter);
            if encoded(&value).len() > MAX_SIMPLE_CANONICAL_BYTES {
                break;
            }
        }
        let oversized_canonical = encoded(&value);
        assert!(oversized_canonical.len() <= MAX_SIMPLE_DEFINITION_BYTES);
        assert!(oversized_canonical.len() > MAX_SIMPLE_CANONICAL_BYTES);
        assert!(parse_simple_definition(&oversized_canonical).is_err());
    }

    #[test]
    fn closed_schema_rejects_invalid_semantics_and_transaction_shapes() {
        let mut cases = Vec::new();
        let mut push = |name: &'static str, mutate: fn(&mut Value)| {
            let mut value = parsed_definition();
            mutate(&mut value);
            cases.push((name, value));
        };
        push("format", |value| value["format_version"] = json!(2));
        push("empty parameters", |value| value["parameters"] = json!([]));
        push("zero parameter", |value| {
            value["parameters"][0]["parameter_id"] = json!(0)
        });
        push("duplicate parameter identity", |value| {
            let duplicate = value["parameters"][0].clone();
            value["parameters"].as_array_mut().unwrap().push(duplicate);
        });
        push("too many parameters", |value| {
            let template = value["parameters"][0].clone();
            let parameters = value["parameters"].as_array_mut().unwrap();
            parameters.clear();
            for index in 1..=17 {
                let mut parameter = template.clone();
                parameter["parameter_id"] = json!(index);
                parameter["key"] = json!(format!("p-{index}"));
                parameters.push(parameter);
            }
        });
        push("unit id", |value| {
            value["parameters"][0]["unit_id"] = json!("u".repeat(33))
        });
        push("unit symbol", |value| {
            value["parameters"][0]["unit_symbol"] = json!("s".repeat(17));
        });
        push("fractional integer bound", |value| {
            value["parameters"][0]["value_type"] = json!("integer");
            value["parameters"][0]["engineering_min"] = json!(0.5);
        });
        push("integer bound outside i64", |value| {
            value["parameters"][0]["value_type"] = json!("integer");
            value["parameters"][0]["engineering_max"] = json!(9_223_372_036_854_775_808u64);
        });
        push("inverted float range", |value| {
            value["parameters"][0]["engineering_min"] = json!(5.0);
            value["parameters"][0]["engineering_max"] = json!(5.0);
        });
        push("zero scale", |value| {
            value["parameters"][0]["encoding"]["scale"] = json!(0.0)
        });
        push("unsupported raw", |value| {
            value["parameters"][0]["encoding"]["raw"] = json!("f64_be");
        });
        push("empty request", |value| {
            value["parameters"][0]["read"]["request"]["segments"] = json!([]);
        });
        push("uppercase hex", |value| {
            value["parameters"][0]["read"]["request"]["segments"][0]["hex"] = json!("AA");
        });
        push("checksum not last", |value| {
            value["parameters"][0]["read"]["request"]["segments"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"literal","hex":"00"}));
        });
        push("zero response", |value| {
            value["parameters"][0]["read"]["response"]["exact_length"] = json!(0);
        });
        push("match out of bounds", |value| {
            value["parameters"][0]["read"]["response"]["matches"][0]["offset"] = json!(7);
        });
        push("extract out of bounds", |value| {
            value["parameters"][0]["read"]["response"]["extract"]["offset"] = json!(6);
        });
        push("checksum overlap", |value| {
            value["parameters"][0]["read"]["response"]["checksum"]["offset"] = json!(4);
        });
        for (name, value) in cases {
            assert!(
                parse_simple_definition(&encoded(&value)).is_err(),
                "invalid case was accepted: {name}"
            );
        }
    }

    #[test]
    fn all_twelve_raw_encodings_compile_from_the_closed_schema() {
        for raw in [
            "u8", "i8", "u16_le", "u16_be", "i16_le", "i16_be", "u32_le", "u32_be", "i32_le",
            "i32_be", "f32_le", "f32_be",
        ] {
            let mut value = parsed_definition();
            value["parameters"][0]["encoding"]["raw"] = json!(raw);
            value["parameters"][0]["read"]["response"]["extract"]["offset"] = json!(3);
            value["parameters"][0]["read"]["response"]["checksum"]["offset"] =
                json!(if matches!(raw, "u8" | "i8") { 4 } else { 7 });
            value["parameters"][0]["read"]["response"]["exact_length"] =
                json!(if matches!(raw, "u8" | "i8") { 6 } else { 9 });
            let definition = parse_simple_definition(&encoded(&value)).unwrap();
            compile_simple_instance(&definition, InstrumentId::new(1), 1, 0).unwrap();
        }
    }

    #[test]
    fn fixed_request_and_response_bounds_accept_sixty_four_and_reject_sixty_five() {
        let mut value = parsed_definition();
        value["parameters"][0]["read"]["request"]["segments"] = json!([
            {"type":"literal","hex":"00".repeat(32)},
            {"type":"literal","hex":"11".repeat(32)}
        ]);
        value["parameters"][0]["read"]["response"] = json!({
            "exact_length":64,
            "matches":[],
            "extract":{"type":"scalar_extract","offset":0}
        });
        let definition = parse_simple_definition(&encoded(&value)).unwrap();
        let instance = compile_simple_instance(&definition, InstrumentId::new(1), 0, 0).unwrap();
        assert_eq!(
            instance.parameters[0].read.as_ref().unwrap().request.len(),
            64
        );
        assert_eq!(
            instance.parameters[0]
                .read
                .as_ref()
                .unwrap()
                .response
                .exact_length,
            64
        );

        value["parameters"][0]["read"]["request"]["segments"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"checksum","algorithm":"sum8"}));
        assert!(parse_simple_definition(&encoded(&value)).is_err());
        value["parameters"][0]["read"]["response"]["exact_length"] = json!(65);
        assert!(parse_simple_definition(&encoded(&value)).is_err());
    }

    #[test]
    fn response_overlap_rule_allows_only_identical_literal_evidence() {
        let mut value = parsed_definition();
        value["parameters"][0]["read"]["response"]["matches"] = json!([
            {"type":"literal_match","offset":0,"hex":"1011"},
            {"type":"literal_match","offset":1,"hex":"11"}
        ]);
        let definition = parse_simple_definition(&encoded(&value)).unwrap();
        compile_simple_instance(&definition, InstrumentId::new(1), 1, 2).unwrap();

        value["parameters"][0]["read"]["response"]["matches"][1]["hex"] = json!("12");
        assert!(parse_simple_definition(&encoded(&value)).is_err());

        let mut value = parsed_definition();
        value["parameters"][0]["read"]["response"]["matches"] = json!([
            {"type":"literal_match","offset":0,"hex":"1001"},
            {"type":"instance_match","offset":1,"field":"address","encoding":"u8"}
        ]);
        assert!(parse_simple_definition(&encoded(&value)).is_err());

        let mut value = parsed_definition();
        value["parameters"][0]["read"]["response"]["matches"] = json!([
            {"type":"literal_match","offset":0,"hex":"10"},
            {"type":"literal_match","offset":3,"hex":"00"}
        ]);
        assert!(parse_simple_definition(&encoded(&value)).is_err());

        let mut value = parsed_definition();
        value["parameters"][0]["read"]["response"]["matches"] = json!([
            {"type":"literal_match","offset":5,"hex":"00"}
        ]);
        assert!(parse_simple_definition(&encoded(&value)).is_err());

        let mut value = parsed_definition();
        value["parameters"][0]["read"]["response"]["checksum"]["offset"] = json!(3);
        assert!(parse_simple_definition(&encoded(&value)).is_err());
    }

    #[test]
    fn persistent_freeze_retains_exact_and_canonical_provenance() {
        struct Reader(Vec<u8>);
        impl ArtifactReader for Reader {
            fn read(&mut self, _: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
                assert_eq!(maximum, MAX_SIMPLE_DEFINITION_BYTES);
                Ok(self.0.clone())
            }
        }
        let config = br#"schema_version=1
[runtime]
key="bench"
display_name="Bench"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com_read_only"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
kind="simple_device"
id=1001
key="furnace-1"
display_name="Furnace 1"
definition="definitions/simple.json"
resource_id=7
address=1
channel=2
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#;
        let source = definition();
        let compiled = parse_simple_definition(&source).unwrap();
        let deployment =
            parse_runtime_toml(config, Path::new("C:/bench"), &mut Reader(source.clone())).unwrap();
        let entries = deployment.provenance_entries(1);
        assert!(entries.iter().any(|(kind, encoding, content)| {
            kind == "simple_device_definition_raw" && encoding == "utf8" && content == &source
        }));
        assert!(entries.iter().any(|(kind, encoding, content)| {
            kind == "simple_device_definition_canonical"
                && encoding == "json"
                && content == compiled.canonical.as_ref()
        }));
        let instance = entries
            .iter()
            .find(|(kind, _, _)| kind == "simple_device_instance")
            .expect("instance provenance missing");
        let instance: Value = serde_json::from_slice(&instance.2).unwrap();
        assert_eq!(instance["source"], "persistent_startup_deployment");
        assert_eq!(instance["instrument_id"], "1001");
        assert_eq!(instance["definition_id"], "furnace-v1");
        assert_eq!(instance["definition_version"], 1);
        assert_eq!(instance["resource_id"], "7");
        assert_eq!(instance["binding_generation"], "1");
        assert_eq!(instance["mapping_revision"], "1");

        let changed_address = String::from_utf8(config.to_vec())
            .unwrap()
            .replace("address=1", "address=2");
        let changed = parse_runtime_toml(
            changed_address.as_bytes(),
            Path::new("C:/bench"),
            &mut Reader(source.clone()),
        )
        .unwrap();
        assert!(changed.changes_from(&deployment).restart_required);

        let changed_poll = String::from_utf8(config.to_vec())
            .unwrap()
            .replace("poll_period_ms=250", "poll_period_ms=500");
        let changed = parse_runtime_toml(
            changed_poll.as_bytes(),
            Path::new("C:/bench"),
            &mut Reader(source),
        )
        .unwrap();
        let effects = changed.changes_from(&deployment);
        assert!(!effects.restart_required);
        assert!(effects.ordinary_live);
        assert!(!effects.rebind);
        assert!(!effects.controller_rewarm);
        assert!(!effects.safe_barrier);

        let changed_display = String::from_utf8(config.to_vec())
            .unwrap()
            .replace("display_name=\"Furnace 1\"", "display_name=\"Furnace A\"");
        let changed = parse_runtime_toml(
            changed_display.as_bytes(),
            Path::new("C:/bench"),
            &mut Reader(definition()),
        )
        .unwrap();
        let effects = changed.changes_from(&deployment);
        assert!(effects.live_safe);
        assert!(!effects.rebind);
        assert!(!effects.controller_rewarm);
        assert!(!effects.safe_barrier);
        assert!(!effects.restart_required);
    }

    struct WritableReader;

    impl ArtifactReader for WritableReader {
        fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
            Ok(writable_definition())
        }
    }

    fn writable_toml(resource_kind: &str, include_safe: bool) -> Vec<u8> {
        let safe = if include_safe {
            r#"
[[safe_profiles]]
instrument_id=1001
parameter_id=2
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
"#
        } else {
            ""
        };
        format!(
            r#"schema_version=1
[runtime]
key="simple-output"
display_name="Simple output"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="{resource_kind}"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
kind="simple_device"
id=1001
key="furnace-1"
display_name="Furnace 1"
definition="definitions/furnace-output.json"
resource_id=7
address=1
channel=2
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
{safe}"#,
        )
        .into_bytes()
    }

    #[derive(Default)]
    struct WritableWire {
        readable: VecDeque<u8>,
        writes: Vec<Vec<u8>>,
        raw: [u8; 2],
        bad_next_nonzero_ack: bool,
    }

    struct WritableTransport(Rc<RefCell<WritableWire>>);

    impl ByteTransport for WritableTransport {
        fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
            let mut wire = self.0.borrow_mut();
            wire.writes.push(bytes.to_vec());
            let response = match bytes.first().copied() {
                Some(0x10) => {
                    let address = *bytes.get(1).ok_or(TransportIoError::Other)?;
                    let channel = *bytes.get(2).ok_or(TransportIoError::Other)?;
                    let mut response = vec![0x10, address, channel, 0, 250];
                    let prefix = response.clone();
                    SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
                    response
                }
                Some(0x20) => {
                    wire.raw
                        .copy_from_slice(bytes.get(3..5).ok_or(TransportIoError::Other)?);
                    let bad_ack = wire.bad_next_nonzero_ack && wire.raw != [0, 0];
                    wire.bad_next_nonzero_ack &= !bad_ack;
                    let mut response = vec![0x20, 1, u8::from(bad_ack)];
                    let prefix = response.clone();
                    SimpleChecksum::Crc16Modbus.append(&prefix, &mut response);
                    response
                }
                Some(0x21) => vec![0x21, 1, 2, wire.raw[0], wire.raw[1]],
                Some(0x22) => vec![0x22, 1, wire.raw[0], wire.raw[1]],
                _ => return Err(TransportIoError::Other),
            };
            wire.readable.extend(response);
            Ok(bytes.len())
        }

        fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
            let mut wire = self.0.borrow_mut();
            let count = bytes.len().min(wire.readable.len());
            for byte in &mut bytes[..count] {
                *byte = wire.readable.pop_front().expect("bounded readable length");
            }
            Ok(count)
        }

        fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
            Ok(RecoveryStatus::Complete)
        }
    }

    struct TestClock(Duration);

    impl Clock for TestClock {
        fn now(&self) -> Duration {
            self.0
        }
    }

    struct ControlledReader;

    impl ArtifactReader for ControlledReader {
        fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
            Ok(controlled_writable_definition())
        }
    }

    fn controlled_writable_toml() -> Vec<u8> {
        br#"schema_version=1
[runtime]
key="controlled-output"
display_name="Controlled output"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
[[instruments]]
kind="simple_device"
id=1001
key="furnace-1"
display_name="Furnace 1"
definition="definitions/controlled-furnace.json"
resource_id=7
address=1
channel=2
poll_period_ms=100
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
[[references]]
id=10
key="setpoint"
kind="fixed"
value=40.0
unit_id="degC"
unit_symbol="C"
[[safe_profiles]]
instrument_id=1001
parameter_id=2
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
[[controllers]]
id=20
key="pid"
input_instrument_id=1001
input_parameter_id=1
output_instrument_id=1001
output_parameter_id=2
reference_id=10
period_ms=100
ema_time_constant_ms=100
ema_warmup_samples=1
kp=1.0
ki=0.0
kd=0.0
output_min=0.0
output_max=100.0
max_input_age_ms=500
max_tick_gap_ms=500
lease_lifetime_ms=2000
proposal_ttl_ms=200
"#
        .to_vec()
    }

    fn temporary_database() -> PathBuf {
        let mut entropy = [0u8; 8];
        getrandom::fill(&mut entropy).unwrap();
        let suffix = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        std::env::temp_dir().join(format!("m16-simple-output-{suffix}.sqlite"))
    }

    fn advance(host: &mut HostCore, clock: &mut TestClock, milliseconds: u64) {
        for _ in 0..milliseconds / 10 {
            clock.0 += Duration::from_millis(10);
            host.service(clock).unwrap();
            while host.recording_status().is_some_and(|status| {
                status.state == RecordingState::Recording && status.outstanding_groups >= 3
            }) {
                host.service(clock).unwrap();
                std::thread::yield_now();
            }
        }
    }

    #[test]
    fn persistent_writable_host_uses_compiled_safe_write_ack_and_readback() {
        let mut reader = WritableReader;
        let deployment = parse_runtime_toml(
            &writable_toml("windows_com", true),
            Path::new("C:/bench"),
            &mut reader,
        )
        .unwrap();
        let wire = Rc::new(RefCell::new(WritableWire::default()));
        let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
        transports.insert(
            ResourceId::new(7),
            Box::new(WritableTransport(wire.clone())),
        );
        let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
        assert!(host.configured_probes_ready().unwrap());
        host.request_configured_physical_safe(Duration::ZERO)
            .unwrap();
        for millisecond in 0..100 {
            host.service(&TestClock(Duration::from_millis(millisecond)))
                .unwrap();
            if host.configured_physical_outputs_safe().unwrap() {
                break;
            }
        }
        assert!(host.configured_physical_outputs_safe().unwrap());
        let writes = &wire.borrow().writes;
        let write = writes
            .iter()
            .find(|bytes| bytes.first() == Some(&0x20))
            .unwrap();
        assert_eq!(&write[..5], &[0x20, 1, 2, 0, 0]);
        assert!(writes.iter().any(|bytes| bytes == &[0x21, 1, 2]));
    }

    #[test]
    fn persistent_writable_controller_and_recorder_use_generic_authority_and_binding() {
        let mut reader = ControlledReader;
        let deployment = parse_runtime_toml(
            &controlled_writable_toml(),
            Path::new("C:/bench"),
            &mut reader,
        )
        .unwrap();
        let wire = Rc::new(RefCell::new(WritableWire::default()));
        let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
        transports.insert(
            ResourceId::new(7),
            Box::new(WritableTransport(wire.clone())),
        );
        let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
        let database = temporary_database();
        let worker = RecorderWorker::open(&database, RecorderLimits::default()).unwrap();
        host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut clock = TestClock(Duration::ZERO);
        while !host.recording_activation_committed().unwrap() {
            assert!(Instant::now() < deadline, "activation did not commit");
            host.service(&clock).unwrap();
            std::thread::yield_now();
        }
        host.start_recording("simple writable acceptance", clock.0)
            .unwrap();
        while host.recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < deadline, "Recorder did not start");
            host.service(&clock).unwrap();
            std::thread::yield_now();
        }

        assert!(host.configured_probes_ready().unwrap());
        host.request_configured_physical_safe(clock.0).unwrap();
        advance(&mut host, &mut clock, 100);
        assert!(host.configured_physical_outputs_safe().unwrap());
        host.prepare_configured_physical_controllers().unwrap();
        let QueryResult::Controller(ready) = host
            .query(Query::Controller(ControllerId::new(20)))
            .unwrap()
        else {
            panic!("controller missing");
        };
        assert_eq!(ready.state, ControllerState::Ready);

        advance(&mut host, &mut clock, 300);
        host.command(Command::StartController {
            controller: ControllerId::new(20),
            at: clock.0,
        })
        .unwrap();
        let actuator = ActuatorId::new(InstrumentId::new(1001), ParameterId::new(2));
        let expected_nonzero = {
            let mut request = vec![0x20, 1, 2, 0, 150];
            let prefix = request.clone();
            SimpleChecksum::Crc16Modbus.append(&prefix, &mut request);
            request
        };
        while !wire
            .borrow()
            .writes
            .iter()
            .any(|write| write == &expected_nonzero)
        {
            assert!(Instant::now() < deadline, "controller WRITE did not occur");
            advance(&mut host, &mut clock, 10);
        }
        let applied = loop {
            let QueryResult::Output(applied) = host.query(Query::Output(actuator)).unwrap() else {
                panic!("output missing");
            };
            if applied.outcome == Some(DispatchOutcome::ReadbackVerified) {
                break applied;
            }
            assert!(Instant::now() < deadline, "readback did not settle output");
            advance(&mut host, &mut clock, 10);
        };
        assert_eq!(applied.outcome, Some(DispatchOutcome::ReadbackVerified));
        assert!(
            applied.lease.is_some(),
            "physical evidence must retain the lease"
        );
        let QueryResult::Controller(running) = host
            .query(Query::Controller(ControllerId::new(20)))
            .unwrap()
        else {
            panic!("controller missing");
        };
        assert_eq!(running.state, ControllerState::Running);

        wire.borrow_mut().bad_next_nonzero_ack = true;
        let nonzero_before_fault = wire
            .borrow()
            .writes
            .iter()
            .filter(|write| write.first() == Some(&0x20) && write.get(3..5) != Some(&[0, 0]))
            .count();
        loop {
            advance(&mut host, &mut clock, 10);
            let QueryResult::Controller(controller) = host
                .query(Query::Controller(ControllerId::new(20)))
                .unwrap()
            else {
                unreachable!();
            };
            if controller.state == ControllerState::Failed {
                break;
            }
            assert!(Instant::now() < deadline, "bad ACK did not fail controller");
        }
        let nonzero_after_fault = wire
            .borrow()
            .writes
            .iter()
            .filter(|write| write.first() == Some(&0x20) && write.get(3..5) != Some(&[0, 0]))
            .count();
        assert_eq!(nonzero_after_fault, nonzero_before_fault + 1);
        advance(&mut host, &mut clock, 500);
        assert_eq!(
            wire.borrow()
                .writes
                .iter()
                .filter(|write| write.first() == Some(&0x20) && write.get(3..5) != Some(&[0, 0]))
                .count(),
            nonzero_after_fault,
            "ambiguous nonzero WRITE must not be resent"
        );
        let QueryResult::Controller(failed) = host
            .query(Query::Controller(ControllerId::new(20)))
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(failed.state, ControllerState::Failed);
        let QueryResult::Output(faulted) = host.query(Query::Output(actuator)).unwrap() else {
            unreachable!();
        };
        assert!(matches!(
            faulted.state,
            OutputState::SafePending | OutputState::FaultLatched | OutputState::Disarmed
        ));

        while host.recording_status().unwrap().outstanding_groups != 0 {
            assert!(Instant::now() < deadline, "Recorder facts did not drain");
            host.service(&clock).unwrap();
            std::thread::yield_now();
        }
        assert_eq!(
            host.recording_status().unwrap().state,
            RecordingState::Recording,
            "Recorder failed before stop: {:?}",
            host.recording_status().unwrap()
        );
        host.stop_recording_at(clock.0).unwrap();
        while host.recording_status().unwrap().state != RecordingState::Idle {
            assert!(Instant::now() < deadline, "Recorder did not stop");
            host.service(&clock).unwrap();
            std::thread::yield_now();
        }
        host.finish_recorder().unwrap();
        while host.recording_status().unwrap().state != RecordingState::Closed {
            assert!(Instant::now() < deadline, "Recorder did not close");
            host.service(&clock).unwrap();
            std::thread::yield_now();
        }
        drop(host);

        let db = rusqlite::Connection::open(&database).unwrap();
        let instrument_id = 1001u64.to_be_bytes();
        let parameter_id = 2u64.to_be_bytes();
        for stage in [
            "requested",
            "authorized",
            "send_started",
            "acknowledged",
            "readback_verified",
            "safe_requested",
            "safe_send_started",
            "safe_acknowledged",
            "safe_readback_verified",
        ] {
            let count: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM output_events
                     WHERE instrument_id=?1 AND parameter_id=?2 AND stage=?3
                       AND resource_id=?4 AND generation=?5 AND revision=?6",
                    rusqlite::params![
                        instrument_id.as_slice(),
                        parameter_id.as_slice(),
                        stage,
                        7u64.to_be_bytes().as_slice(),
                        1u64.to_be_bytes().as_slice(),
                        1u64.to_be_bytes().as_slice(),
                    ],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(count >= 1, "missing durable physical context for {stage}");
        }

        let canonical: Vec<u8> = db
            .query_row(
                "SELECT content FROM provenance_content
                 WHERE kind='simple_device_definition_canonical'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let canonical_hash: [u8; 32] = Sha256::digest(&canonical).into();
        let raw_hash: [u8; 32] = Sha256::digest(controlled_writable_definition()).into();
        let native_hash: Vec<u8> = db
            .query_row(
                "SELECT content_hash FROM provenance_content WHERE kind='native_composition'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let (instrument_definition, instrument_source, binding): (Vec<u8>, Vec<u8>, String) = db
            .query_row(
                "SELECT definition_hash,source_hash,instance_binding FROM object_snapshots
                 WHERE object_kind='instrument' AND object_id=?1",
                [instrument_id.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(instrument_definition, canonical_hash);
        assert_eq!(instrument_source, raw_hash);
        let binding: Value = serde_json::from_str(&binding).unwrap();
        assert_eq!(binding["r"], "7");
        assert_eq!(binding["b"], "1");
        assert_eq!(binding["m"], "1");

        let mut actuator_object_id = Vec::from(instrument_id);
        actuator_object_id.extend_from_slice(&parameter_id);
        let (actuator_definition, actuator_descriptor): (Vec<u8>, String) = db
            .query_row(
                "SELECT definition_hash,descriptor FROM object_snapshots
                 WHERE object_kind='actuator' AND object_id=?1",
                [actuator_object_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(actuator_definition, native_hash);
        let actuator_descriptor: Value = serde_json::from_str(&actuator_descriptor).unwrap();
        assert_eq!(actuator_descriptor["safe_value"], 0.0);
        assert_eq!(actuator_descriptor["required_evidence"], "readback");
        for (kind, id) in [("controller", 20u64), ("reference", 10u64)] {
            let definition: Vec<u8> = db
                .query_row(
                    "SELECT definition_hash FROM object_snapshots
                     WHERE object_kind=?1 AND object_id=?2",
                    rusqlite::params![kind, id.to_be_bytes().as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(definition, native_hash, "wrong {kind} definition identity");
        }
        drop(db);
        std::fs::remove_file(&database).unwrap();
        let _ = std::fs::remove_file(format!("{}-wal", database.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database.display()));
    }

    #[test]
    fn writable_persistent_com_requires_existing_safe_profile() {
        let mut reader = WritableReader;
        parse_runtime_toml(
            &writable_toml("windows_com", true),
            Path::new("C:/bench"),
            &mut reader,
        )
        .unwrap();
        for invalid in [
            writable_toml("windows_com_read_only", true),
            writable_toml("windows_com", false),
        ] {
            let mut reader = WritableReader;
            assert!(parse_runtime_toml(&invalid, Path::new("C:/bench"), &mut reader).is_err());
        }
    }

    #[test]
    fn maximum_simple_provenance_cardinality_fits_existing_recorder_credits() {
        struct ManyReader;
        impl ArtifactReader for ManyReader {
            fn read(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
                assert_eq!(maximum, MAX_SIMPLE_DEFINITION_BYTES);
                let ordinal = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.strip_prefix("simple-"))
                    .and_then(|ordinal| ordinal.parse::<usize>().ok())
                    .expect("bounded test definition path");
                let mut value = parsed_definition();
                value["definition_id"] = json!(format!("furnace-v{ordinal}"));
                Ok(encoded(&value))
            }
        }

        struct IdleTransport;
        impl ByteTransport for IdleTransport {
            fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
                Ok(bytes.len())
            }

            fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
                Ok(0)
            }

            fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
                Ok(RecoveryStatus::Complete)
            }
        }

        let mut config = String::from(
            r#"schema_version=1
[runtime]
key="bench"
display_name="Bench"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[resources]]
id=7
key="bus"
kind="windows_com_read_only"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=50
write_timeout_ms=50
open_timeout_ms=500
recovery_timeout_ms=500
"#,
        );
        for ordinal in 1..=MAX_SIMPLE_INSTANCES {
            let definition = (ordinal - 1) % MAX_SIMPLE_DEFINITIONS + 1;
            config.push_str(&format!(
                r#"
[[instruments]]
kind="simple_device"
id={}
key="furnace-{ordinal}"
display_name="Furnace {ordinal}"
definition="definitions/simple-{definition}.json"
resource_id=7
address={ordinal}
channel=0
poll_period_ms=250
queue_timeout_ms=250
transaction_timeout_ms=500
history_capacity=1024
"#,
                1000 + ordinal
            ));
        }
        let mut deployment =
            parse_runtime_toml(config.as_bytes(), Path::new("C:/bench"), &mut ManyReader).unwrap();
        assert_eq!(deployment.artifacts().len(), MAX_SIMPLE_DEFINITIONS);
        for ordinal in 1..MAX_SIMPLE_INSTANCES {
            deployment = deployment
                .with_property_override(
                    "instrument",
                    (1000 + ordinal) as u64,
                    "poll_period_ms",
                    PropertyValue::Integer(500),
                )
                .unwrap();
        }
        assert_eq!(
            deployment.property_overlay_count(),
            MAX_PROPERTY_OVERLAYS - 1
        );

        let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
        transports.insert(ResourceId::new(7), Box::new(IdleTransport));
        let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
        let candidate = deployment
            .with_property_override(
                "instrument",
                (1000 + MAX_SIMPLE_INSTANCES) as u64,
                "poll_period_ms",
                PropertyValue::Integer(500),
            )
            .unwrap();
        assert_eq!(candidate.property_overlay_count(), MAX_PROPERTY_OVERLAYS);
        host.apply_live_configuration(&candidate, 2).unwrap();

        let (entries, objects) = host.frozen_activation_entries().unwrap();
        assert_eq!(entries.len(), 100);
        assert_eq!(objects.len(), MAX_SIMPLE_INSTANCES);
        let provenance_charge: usize = entries
            .iter()
            .map(|entry| {
                entry.kind.capacity() + entry.encoding.capacity() + entry.content.capacity() + 128
            })
            .sum();
        let object_charge: usize = objects
            .iter()
            .map(|object| {
                object.id.capacity()
                    + object.logical_key.capacity()
                    + object.label.capacity()
                    + object.descriptor.capacity()
                    + object.unit_key.as_ref().map_or(0, String::capacity)
                    + object.binding.as_ref().map_or(0, String::capacity)
                    + 256
            })
            .sum();
        assert!(provenance_charge < 1024 * 1024);
        assert!(object_charge < 256 * 1024);
        for object in &objects {
            let descriptor: Value = serde_json::from_str(&object.descriptor).unwrap();
            assert_eq!(descriptor["simple_device"]["configuration_revision"], "2");
            let binding: Value = serde_json::from_str(object.binding.as_deref().unwrap()).unwrap();
            assert_eq!(binding["b"], "1");
            assert_eq!(binding["m"], "1");
        }

        let mut entropy = [0u8; 8];
        getrandom::fill(&mut entropy).unwrap();
        let suffix = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let database = std::env::temp_dir().join(format!("m16-simple-capacity-{suffix}.sqlite"));
        let mut store = SqliteStore::open(&database).unwrap();
        store.commit_activation(&entries, &objects).unwrap();
        drop(store);
        std::fs::remove_file(&database).unwrap();
        let _ = std::fs::remove_file(format!("{}-wal", database.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database.display()));
    }
}
