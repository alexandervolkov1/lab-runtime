//! Deterministic first-order thermal plant used by the M4 virtual control proof.
//!
//! The model is intentionally small and is not a physical furnace certification.
//! Heater application is crate-private so only Runtime's post-arbiter dispatcher
//! can change it.

use crate::{
    AccessMode, Error, HEATER_POWER, InstrumentDescriptor, InstrumentId, MeasurementFailure,
    ParameterDescriptor, ParameterId, ParameterRole, Sample, SignalId, TEMPERATURE, Unit, Value,
    ValueSpec, WriteEffect, model::validate_name, signal::SignalBuffer,
};
use std::time::Duration;

/// Validated creation data for one native virtual thermal plant.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalPlantConfig {
    /// Stable logical instrument identity.
    pub id: InstrumentId,
    /// Human-readable display name.
    pub name: String,
    /// Bounded recent temperature-attempt capacity.
    pub history_capacity: usize,
    /// Finite no-heating equilibrium in degrees Celsius.
    pub ambient_temperature: f64,
    /// Finite initial temperature in descriptor range.
    pub initial_temperature: f64,
    /// Positive equilibrium-temperature rise per percentage point of heater power.
    pub gain_per_percent: f64,
    /// Positive first-order response time constant.
    pub time_constant: Duration,
}

pub(crate) struct ThermalPlantInstrument {
    pub(crate) descriptor: InstrumentDescriptor,
    pub(crate) signal: SignalBuffer,
    pub(crate) generation: u64,
    ambient_temperature: f64,
    gain_per_percent: f64,
    time_constant: Duration,
    temperature: f64,
    heater_percent: f64,
    last_at: Option<Duration>,
}

impl ThermalPlantInstrument {
    pub(crate) fn new(config: ThermalPlantConfig) -> Result<Self, Error> {
        validate_name(&config.name)?;
        if !config.ambient_temperature.is_finite()
            || !config.initial_temperature.is_finite()
            || !config.gain_per_percent.is_finite()
            || config.gain_per_percent <= 0.0
            || config.time_constant.is_zero()
            || !(-100.0..=500.0).contains(&config.initial_temperature)
            || !(-100.0..=500.0).contains(&config.ambient_temperature)
        {
            return Err(Error::InvalidConfiguration("invalid thermal plant"));
        }
        let signal = SignalId::new(config.id, TEMPERATURE);
        let descriptor = InstrumentDescriptor {
            id: config.id,
            name: config.name,
            parameters: vec![
                ParameterDescriptor {
                    id: TEMPERATURE,
                    name: "temperature".into(),
                    value_spec: ValueSpec::Float {
                        min: -100.0,
                        max: 500.0,
                    },
                    unit: Unit::CELSIUS,
                    access: AccessMode::ReadOnly,
                    role: ParameterRole::Measurement,
                    write_effect: WriteEffect::None,
                    signal: Some(signal),
                },
                ParameterDescriptor {
                    id: HEATER_POWER,
                    name: "heater_power".into(),
                    value_spec: ValueSpec::Float {
                        min: 0.0,
                        max: 100.0,
                    },
                    unit: Unit::PERCENT,
                    access: AccessMode::ReadWrite,
                    role: ParameterRole::Actuator,
                    write_effect: WriteEffect::OutputAffecting,
                    signal: None,
                },
            ],
        };
        for parameter in &descriptor.parameters {
            parameter.validate_definition()?;
        }
        Ok(Self {
            descriptor,
            signal: SignalBuffer::new(signal, config.history_capacity)?,
            generation: 1,
            ambient_temperature: config.ambient_temperature,
            gain_per_percent: config.gain_per_percent,
            time_constant: config.time_constant,
            temperature: config.initial_temperature,
            heater_percent: 0.0,
            last_at: None,
        })
    }

    pub(crate) fn refresh(&mut self, at: Duration) -> Result<Sample, Error> {
        self.signal.check_time(at)?;
        let candidate = if let Some(previous_at) = self.last_at {
            if at <= previous_at {
                return Err(Error::NonMonotonicTime {
                    signal: SignalId::new(self.descriptor.id, TEMPERATURE),
                    previous: previous_at,
                    requested: at,
                });
            }
            let elapsed = (at - previous_at).as_secs_f64();
            let tau = self.time_constant.as_secs_f64();
            let alpha = -(-elapsed / tau).exp_m1();
            let equilibrium =
                self.ambient_temperature + self.gain_per_percent * self.heater_percent;
            self.temperature + alpha * (equilibrium - self.temperature)
        } else {
            self.temperature
        };
        if !candidate.is_finite() || !(-100.0..=500.0).contains(&candidate) {
            return Err(Error::InvalidConfiguration(
                "thermal plant produced invalid temperature",
            ));
        }
        let sample = Sample::good(
            SignalId::new(self.descriptor.id, TEMPERATURE),
            Unit::CELSIUS,
            at,
            Value::Float(candidate),
        );
        self.signal.push(sample.clone())?;
        self.temperature = candidate;
        self.last_at = Some(at);
        Ok(sample)
    }

    pub(crate) fn inject_failure(&mut self, at: Duration) -> Result<Sample, Error> {
        let signal = SignalId::new(self.descriptor.id, TEMPERATURE);
        if self.signal.latest().is_some() {
            // A trusted failure at the same publication time invalidates the
            // current Core tail, while Recorder keeps both captured facts.
            self.signal.invalidate(at, MeasurementFailure::Transport)?;
            Ok(self
                .signal
                .latest()
                .expect("invalidation installed a tail")
                .clone())
        } else {
            let sample =
                Sample::unavailable(signal, Unit::CELSIUS, at, MeasurementFailure::Transport);
            self.signal.push(sample.clone())?;
            Ok(sample)
        }
    }

    pub(crate) fn apply_heater(&mut self, value: f64) -> Result<(), Error> {
        ValueSpec::Float {
            min: 0.0,
            max: 100.0,
        }
        .validate(&Value::Float(value))?;
        self.heater_percent = value;
        Ok(())
    }

    pub(crate) fn configured(&self) -> Vec<(ParameterId, Value)> {
        Vec::new()
    }
}
