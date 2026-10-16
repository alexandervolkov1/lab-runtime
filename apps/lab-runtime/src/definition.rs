//! Strict JSON loader for the single trusted `metakon-5x3-v1` profile.
//!
//! The format selects known Rust operations and metadata. It cannot supply raw
//! addresses, frames, functions or executable expressions.

use lab_core::{
    AccessMode, Error, InstrumentId, ParameterId, ParameterRole, Unit, ValueSpec, WriteEffect,
    instrument::{DataInstrumentDefinition, DataParameterDefinition, KnownOperation},
};
use serde::Deserialize;
use std::fmt;

/// Maximum UTF-8 bytes accepted before JSON parsing.
pub const MAX_DEFINITION_BYTES: usize = 16 * 1024;
/// Maximum JSON object/array nesting accepted by this small format.
pub const MAX_DEFINITION_DEPTH: usize = 8;

/// Failure while loading or validating a declarative instrument.
#[derive(Debug)]
pub enum DefinitionError {
    /// Input exceeded the fixed 16 KiB parser admission bound.
    TooLarge,
    /// Object/array nesting exceeded the fixed depth bound.
    TooDeep,
    /// JSON syntax, duplicate fields or unknown fields were rejected.
    InvalidJson(String),
    /// Parsed metadata conflicts with the trusted Core profile.
    InvalidDomain(Error),
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("instrument definition exceeds 16 KiB"),
            Self::TooDeep => formatter.write_str("instrument definition nesting exceeds 8"),
            Self::InvalidJson(error) => write!(formatter, "invalid definition JSON: {error}"),
            Self::InvalidDomain(error) => {
                write!(formatter, "invalid instrument definition: {error}")
            }
        }
    }
}

impl std::error::Error for DefinitionError {}

impl From<Error> for DefinitionError {
    fn from(error: Error) -> Self {
        Self::InvalidDomain(error)
    }
}

/// Parse and fully validate one bounded schema-version-1 definition.
///
/// Serde's struct visitor rejects duplicate known fields and `deny_unknown_fields`
/// rejects unrecognized ones. A lightweight pre-scan bounds nesting before allocation;
/// strings and escapes are excluded from that scan.
pub fn parse_definition_json(input: &str) -> Result<DataInstrumentDefinition, DefinitionError> {
    if input.len() > MAX_DEFINITION_BYTES {
        return Err(DefinitionError::TooLarge);
    }
    check_depth(input)?;
    let dto: DefinitionDto = serde_json::from_str(input)
        .map_err(|error| DefinitionError::InvalidJson(error.to_string()))?;
    if dto.profile != "metakon-5x3-v1" {
        return Err(Error::InvalidConfiguration("unsupported instrument profile").into());
    }
    let parameters = dto
        .parameters
        .into_iter()
        .map(ParameterDto::into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    let definition = DataInstrumentDefinition {
        schema_version: dto.schema_version,
        id: InstrumentId::new(dto.id),
        name: dto.name,
        parameters,
    };
    definition.validate()?;
    Ok(definition)
}

fn check_depth(input: &str) -> Result<(), DefinitionError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in input.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEFINITION_DEPTH {
                    return Err(DefinitionError::TooDeep);
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionDto {
    schema_version: u16,
    profile: String,
    id: u64,
    name: String,
    parameters: Vec<ParameterDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterDto {
    id: u64,
    name: String,
    value_type: ValueTypeDto,
    unit: UnitDto,
    min: f64,
    max: f64,
    role: RoleDto,
    access: AccessDto,
    operation: OperationDto,
    scale: f64,
    write_effect: EffectDto,
}

impl ParameterDto {
    fn into_domain(self) -> Result<DataParameterDefinition, DefinitionError> {
        let value_spec = match self.value_type {
            ValueTypeDto::Float => ValueSpec::Float {
                min: self.min,
                max: self.max,
            },
            ValueTypeDto::Integer => {
                if self.min.fract() != 0.0
                    || self.max.fract() != 0.0
                    || self.min < i64::MIN as f64
                    || self.max > i64::MAX as f64
                {
                    return Err(Error::InvalidConfiguration("invalid integer bounds").into());
                }
                ValueSpec::Integer {
                    min: self.min as i64,
                    max: self.max as i64,
                }
            }
        };
        Ok(DataParameterDefinition {
            id: ParameterId::new(self.id),
            name: self.name,
            value_spec,
            unit: Unit::new(&self.unit.id, &self.unit.symbol)?,
            access: self.access.into(),
            role: self.role.into(),
            write_effect: self.write_effect.into(),
            operation: self.operation.into(),
            scale: self.scale,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitDto {
    id: String,
    symbol: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ValueTypeDto {
    Float,
    Integer,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoleDto {
    Measurement,
    Actuator,
    Diagnostic,
}
impl From<RoleDto> for ParameterRole {
    fn from(value: RoleDto) -> Self {
        match value {
            RoleDto::Measurement => Self::Measurement,
            RoleDto::Actuator => Self::Actuator,
            RoleDto::Diagnostic => Self::Diagnostic,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AccessDto {
    ReadOnly,
    ReadWrite,
    WriteOnly,
}
impl From<AccessDto> for AccessMode {
    fn from(value: AccessDto) -> Self {
        match value {
            AccessDto::ReadOnly => Self::ReadOnly,
            AccessDto::ReadWrite => Self::ReadWrite,
            AccessDto::WriteOnly => Self::WriteOnly,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationDto {
    ChannelType,
    Temperature,
    Output,
}
impl From<OperationDto> for KnownOperation {
    fn from(value: OperationDto) -> Self {
        match value {
            OperationDto::ChannelType => Self::ChannelType,
            OperationDto::Temperature => Self::Temperature,
            OperationDto::Output => Self::Output,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EffectDto {
    None,
    OutputAffecting,
}
impl From<EffectDto> for WriteEffect {
    fn from(value: EffectDto) -> Self {
        match value {
            EffectDto::None => Self::None,
            EffectDto::OutputAffecting => Self::OutputAffecting,
        }
    }
}
