//! Validated instrument definitions for the narrow native Metakon profile.
//!
//! Definitions describe domain metadata and select known operations. They cannot
//! inject raw frames, executable expressions or new protocol behavior. Runtime
//! combines a validated definition with a separate physical binding.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AccessMode, Error, InstrumentDescriptor, InstrumentId, ParameterDescriptor, ParameterId,
    ParameterRole, SignalId, Unit, ValueSpec, WriteEffect, model::validate_name,
    signal::SignalBuffer, transport::ResourceId,
};

/// Maximum parameters accepted from one data-driven definition.
pub const MAX_DEFINITION_PARAMETERS: usize = 16;

/// Trusted operation primitives supported by the `metakon-5x3-v1` profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KnownOperation {
    /// Read register 0 and require channel type 3.
    ChannelType,
    /// Read signed register 1 and apply positive scaling.
    Temperature,
    /// Read/write signed register 6 through central output authority.
    Output,
}

impl KnownOperation {
    /// Return the fixed register selected by the trusted protocol profile.
    pub const fn register(self) -> u8 {
        match self {
            Self::ChannelType => 0,
            Self::Temperature => 1,
            Self::Output => 6,
        }
    }
}

/// One declarative parameter mapped to a known Metakon primitive.
#[derive(Clone, Debug, PartialEq)]
pub struct DataParameterDefinition {
    /// Stable parameter identity within the logical instrument.
    pub id: ParameterId,
    /// Human-readable display name.
    pub name: String,
    /// Exact scalar kind and inclusive domain limits.
    pub value_spec: ValueSpec,
    /// Bounded engineering-unit metadata, including custom units.
    pub unit: Unit,
    /// Declared device access; not output permission.
    pub access: AccessMode,
    /// Domain role used by generic discovery and safety validation.
    pub role: ParameterRole,
    /// Explicit write consequence classification.
    pub write_effect: WriteEffect,
    /// Known protocol primitive; arbitrary register mappings are not accepted.
    pub operation: KnownOperation,
    /// Positive finite raw-to-engineering multiplier.
    pub scale: f64,
}

/// Versioned, bounded logical instrument definition.
#[derive(Clone, Debug, PartialEq)]
pub struct DataInstrumentDefinition {
    /// Only schema version 1 is accepted in M3.
    pub schema_version: u16,
    /// Stable logical instrument identity.
    pub id: InstrumentId,
    /// Human-readable display name.
    pub name: String,
    /// At most 16 parameters using distinct IDs and operations.
    pub parameters: Vec<DataParameterDefinition>,
}

impl DataInstrumentDefinition {
    /// Validate schema, bounds, unique mappings and trusted operation metadata.
    ///
    /// This performs no registration or I/O and leaves the candidate unchanged.
    pub fn validate(&self) -> Result<(), Error> {
        self.validate_and_describe().map(|_| ())
    }

    pub(crate) fn validate_and_describe(&self) -> Result<InstrumentDescriptor, Error> {
        if self.schema_version != 1 {
            return Err(Error::InvalidConfiguration(
                "unsupported instrument definition version",
            ));
        }
        validate_name(&self.name)?;
        if self.parameters.is_empty() || self.parameters.len() > MAX_DEFINITION_PARAMETERS {
            return Err(Error::InvalidConfiguration(
                "definition parameter count must be in 1..=16",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut operations = BTreeSet::new();
        let mut parameters = Vec::with_capacity(self.parameters.len());
        for parameter in &self.parameters {
            if !ids.insert(parameter.id) || !operations.insert(parameter.operation) {
                return Err(Error::InvalidConfiguration(
                    "duplicate parameter identity or operation",
                ));
            }
            if !parameter.scale.is_finite() || parameter.scale <= 0.0 {
                return Err(Error::InvalidConfiguration(
                    "operation scale must be finite and positive",
                ));
            }
            validate_operation(parameter)?;
            let signal = matches!(
                parameter.role,
                ParameterRole::Measurement | ParameterRole::Diagnostic
            )
            .then(|| SignalId::new(self.id, parameter.id));
            let descriptor = ParameterDescriptor {
                id: parameter.id,
                name: parameter.name.clone(),
                value_spec: parameter.value_spec.clone(),
                unit: parameter.unit,
                access: parameter.access,
                role: parameter.role,
                write_effect: parameter.write_effect,
                signal,
            };
            descriptor.validate_definition()?;
            parameters.push(descriptor);
        }
        Ok(InstrumentDescriptor {
            id: self.id,
            name: self.name.clone(),
            parameters,
        })
    }

    pub(crate) fn parameter_definition(&self, id: ParameterId) -> Option<&DataParameterDefinition> {
        self.parameters.iter().find(|parameter| parameter.id == id)
    }
}

fn validate_operation(parameter: &DataParameterDefinition) -> Result<(), Error> {
    let valid = match parameter.operation {
        KnownOperation::ChannelType => {
            parameter.value_spec.value_type() == crate::ValueType::Integer
                && parameter.unit == Unit::UNITLESS
                && parameter.access == AccessMode::ReadOnly
                && parameter.role == ParameterRole::Diagnostic
                && parameter.write_effect == WriteEffect::None
                && parameter.scale == 1.0
        }
        KnownOperation::Temperature => {
            parameter.value_spec.value_type() == crate::ValueType::Float
                && parameter.access == AccessMode::ReadOnly
                && parameter.role == ParameterRole::Measurement
                && parameter.write_effect == WriteEffect::None
        }
        KnownOperation::Output => {
            parameter.value_spec.value_type() == crate::ValueType::Float
                && matches!(
                    parameter.access,
                    AccessMode::ReadWrite | AccessMode::WriteOnly
                )
                && parameter.role == ParameterRole::Actuator
                && parameter.write_effect == WriteEffect::OutputAffecting
                && parameter.scale == 1.0
        }
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidConfiguration(
            "parameter metadata conflicts with known operation",
        ))
    }
}

/// Deployment binding kept separate from reusable logical metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetakonBinding {
    /// Resource whose single executor serializes this instrument's transactions.
    pub resource: ResourceId,
    /// Metakon device byte.
    pub device: u8,
    /// Metakon channel byte.
    pub channel: u8,
    /// Nonzero generation changed on reconnect/rebind.
    pub binding_generation: u64,
    /// Nonzero revision changed when operation mapping/scaling changes.
    pub mapping_revision: u64,
    /// Trusted expected actuator unit; must match the descriptor exactly.
    pub expected_output_unit: Option<Unit>,
}

/// Atomic registration candidate for one logical Metakon instrument.
#[derive(Clone, Debug, PartialEq)]
pub struct MetakonInstrumentConfig {
    /// Validated logical metadata and known operation selections.
    pub definition: DataInstrumentDefinition,
    /// Physical resource/address and generation fences.
    pub binding: MetakonBinding,
    /// Per-signal recent attempt capacity, bounded by the common signal policy.
    pub history_capacity: usize,
}

pub(crate) struct MetakonInstrument {
    pub(crate) descriptor: InstrumentDescriptor,
    pub(crate) definition: DataInstrumentDefinition,
    pub(crate) binding: MetakonBinding,
    pub(crate) signals: BTreeMap<SignalId, SignalBuffer>,
}

impl MetakonInstrument {
    pub(crate) fn new(config: MetakonInstrumentConfig) -> Result<Self, Error> {
        if config.binding.binding_generation == 0 || config.binding.mapping_revision == 0 {
            return Err(Error::InvalidConfiguration(
                "binding generation and mapping revision must be nonzero",
            ));
        }
        let descriptor = config.definition.validate_and_describe()?;
        let outputs: Vec<_> = descriptor
            .parameters
            .iter()
            .filter(|parameter| parameter.role == ParameterRole::Actuator)
            .collect();
        match (outputs.as_slice(), config.binding.expected_output_unit) {
            ([], None) => {}
            ([output], Some(expected)) if output.unit == expected => {}
            _ => {
                return Err(Error::InvalidConfiguration(
                    "output binding unit does not match descriptor",
                ));
            }
        }
        let mut signals = BTreeMap::new();
        for parameter in &descriptor.parameters {
            if let Some(signal) = parameter.signal {
                signals.insert(signal, SignalBuffer::new(signal, config.history_capacity)?);
            }
        }
        Ok(Self {
            descriptor,
            definition: config.definition,
            binding: config.binding,
            signals,
        })
    }

    pub(crate) fn configured(&self) -> Vec<(ParameterId, crate::Value)> {
        Vec::new()
    }
}
