//! Native M1 virtual instrument behind the common Runtime boundary.
//! Configuration changes preserve observations. Only explicit refresh evaluates
//! the time function; heater_power is metadata, never a generic write escape hatch.

use crate::{
    AccessMode, Error, InstrumentDescriptor, InstrumentId, MeasurementFailure, ParameterDescriptor,
    ParameterId, ParameterRole, Sample, SignalId, Unit, Value, ValueSpec, WriteEffect,
    model::validate_name, signal::SignalBuffer,
};
use std::time::Duration;

/// Stable measurement parameter ID in the native virtual instrument.
pub const TEMPERATURE: ParameterId = ParameterId::new(1);
/// Stable actuator parameter ID; generic configuration writes must not actuate it.
pub const HEATER_POWER: ParameterId = ParameterId::new(2);
/// Stable ID of the configuration-only generator baseline.
pub const BASE_TEMPERATURE: ParameterId = ParameterId::new(3);
/// Stable ID of the measurement fault-injection switch.
pub const MEASUREMENT_ENABLED: ParameterId = ParameterId::new(4);

/// Explicit creation command data, not a persisted or wire-format configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualInstrumentConfig {
    /// Stable local identity; display-name changes do not replace it.
    pub id: InstrumentId,
    /// Human-readable display name, not an identity or lookup key.
    pub name: String,
    /// Maximum retained attempts, validated in 1..=MAX_HISTORY_CAPACITY.
    pub history_capacity: usize,
    /// Generator baseline in Celsius, validated in the inclusive range -100..=100.
    pub base_temperature: f64,
    /// False makes explicit refresh produce Unavailable, for deterministic fault injection.
    pub measurement_enabled: bool,
}

pub(crate) struct VirtualInstrument {
    pub(crate) descriptor: InstrumentDescriptor,
    pub(crate) signal: SignalBuffer,
    base_temperature: f64,
    measurement_enabled: bool,
}

impl VirtualInstrument {
    pub(crate) fn new(config: VirtualInstrumentConfig) -> Result<Self, Error> {
        validate_name(&config.name)?;
        base_spec().validate(&Value::Float(config.base_temperature))?;
        let signal_id = SignalId::new(config.id, TEMPERATURE);
        let descriptor = InstrumentDescriptor {
            id: config.id,
            name: config.name,
            parameters: vec![
                ParameterDescriptor {
                    id: TEMPERATURE,
                    name: "temperature".into(),
                    value_spec: ValueSpec::Float {
                        min: -100.0,
                        max: 110.0,
                    },
                    unit: Unit::Celsius,
                    access: AccessMode::ReadOnly,
                    role: ParameterRole::Measurement,
                    write_effect: WriteEffect::None,
                    signal: Some(signal_id),
                },
                ParameterDescriptor {
                    id: HEATER_POWER,
                    name: "heater_power".into(),
                    value_spec: ValueSpec::Float {
                        min: 0.0,
                        max: 100.0,
                    },
                    unit: Unit::Percent,
                    access: AccessMode::ReadWrite,
                    role: ParameterRole::Actuator,
                    write_effect: WriteEffect::OutputAffecting,
                    signal: None,
                },
                ParameterDescriptor {
                    id: BASE_TEMPERATURE,
                    name: "base_temperature".into(),
                    value_spec: base_spec(),
                    unit: Unit::Celsius,
                    access: AccessMode::ReadWrite,
                    role: ParameterRole::Configuration,
                    write_effect: WriteEffect::ConfigurationOnly,
                    signal: None,
                },
                ParameterDescriptor {
                    id: MEASUREMENT_ENABLED,
                    name: "measurement_enabled".into(),
                    value_spec: ValueSpec::Boolean,
                    unit: Unit::Unitless,
                    access: AccessMode::ReadWrite,
                    role: ParameterRole::Configuration,
                    write_effect: WriteEffect::ConfigurationOnly,
                    signal: None,
                },
            ],
        };
        for parameter in &descriptor.parameters {
            parameter.validate_definition()?;
        }
        Ok(Self {
            descriptor,
            signal: SignalBuffer::new(signal_id, config.history_capacity)?,
            base_temperature: config.base_temperature,
            measurement_enabled: config.measurement_enabled,
        })
    }

    pub(crate) fn parameter(&self, id: ParameterId) -> Result<&ParameterDescriptor, Error> {
        self.descriptor
            .parameter(id)
            .ok_or(Error::UnknownParameter {
                instrument: self.descriptor.id,
                parameter: id,
            })
    }

    pub(crate) fn configured(&self) -> Vec<(ParameterId, Value)> {
        vec![
            (BASE_TEMPERATURE, Value::Float(self.base_temperature)),
            (
                MEASUREMENT_ENABLED,
                Value::Boolean(self.measurement_enabled),
            ),
        ]
    }

    pub(crate) fn configure(&mut self, parameter: ParameterId, value: Value) -> Result<(), Error> {
        let descriptor = self.parameter(parameter)?;
        if descriptor.access == AccessMode::ReadOnly {
            return Err(Error::ReadOnlyParameter(parameter));
        }
        if descriptor.role != ParameterRole::Configuration
            || descriptor.write_effect != WriteEffect::ConfigurationOnly
        {
            return Err(Error::OperationNotAllowed(parameter));
        }
        descriptor.value_spec.validate(&value)?;
        // No fallible work after assignment. Observations are intentionally preserved.
        match (parameter, value) {
            (BASE_TEMPERATURE, Value::Float(value)) => self.base_temperature = value,
            (MEASUREMENT_ENABLED, Value::Boolean(value)) => self.measurement_enabled = value,
            _ => return Err(Error::OperationNotAllowed(parameter)),
        }
        Ok(())
    }

    pub(crate) fn refresh(
        &mut self,
        parameter: ParameterId,
        at: Duration,
    ) -> Result<Sample, Error> {
        let descriptor = self.parameter(parameter)?;
        if descriptor.role != ParameterRole::Measurement {
            return Err(Error::OperationNotAllowed(parameter));
        }
        let signal = descriptor
            .signal
            .ok_or(Error::OperationNotAllowed(parameter))?;
        let unit = descriptor.unit;
        self.signal.check_time(at)?;
        if !self.measurement_enabled {
            let reason = MeasurementFailure::Disabled;
            self.signal
                .push(Sample::unavailable(signal, unit, at, reason))?;
            return Err(Error::MeasurementUnavailable { signal, reason });
        }
        // New deterministic demonstration function; not a donor thermal model.
        // Modulo before conversion bounds the integer and avoids large-time precision loss.
        let value = Value::Float(self.base_temperature + (at.as_millis() % 10_000) as f64 / 1000.0);
        let sample = Sample::good(signal, unit, at, value);
        self.signal.push(sample.clone())?;
        Ok(sample)
    }
}

fn base_spec() -> ValueSpec {
    ValueSpec::Float {
        min: -100.0,
        max: 100.0,
    }
}
