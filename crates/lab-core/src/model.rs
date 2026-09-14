use std::{fmt, time::Duration};

pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 1024;
pub const MAX_ENUM_CHOICES: usize = 64;

/// Local identity, independent of a display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentId(u64);

impl InstrumentId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identity scoped to an instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParameterId(u64);

impl ParameterId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One measurement stream per parameter; no replacement/generation semantics in M1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalId {
    instrument: InstrumentId,
    parameter: ParameterId,
}

impl SignalId {
    pub const fn new(instrument: InstrumentId, parameter: ParameterId) -> Self {
        Self {
            instrument,
            parameter,
        }
    }
    pub const fn instrument(self) -> InstrumentId {
        self.instrument
    }
    pub const fn parameter(self) -> ParameterId {
        self.parameter
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Float,
    Integer,
    Boolean,
    Text,
    Enum,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Float(f64),
    Integer(i64),
    Boolean(bool),
    Text(String),
    Enum(String),
}

impl Value {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Float(_) => ValueType::Float,
            Self::Integer(_) => ValueType::Integer,
            Self::Boolean(_) => ValueType::Boolean,
            Self::Text(_) => ValueType::Text,
            Self::Enum(_) => ValueType::Enum,
        }
    }
}

/// Declarative type and inclusive bounds. No implicit coercion or unit conversion.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueSpec {
    Float { min: f64, max: f64 },
    Integer { min: i64, max: i64 },
    Boolean,
    Text { max_bytes: usize },
    Enum { choices: Vec<String> },
}

impl ValueSpec {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Float { .. } => ValueType::Float,
            Self::Integer { .. } => ValueType::Integer,
            Self::Boolean => ValueType::Boolean,
            Self::Text { .. } => ValueType::Text,
            Self::Enum { .. } => ValueType::Enum,
        }
    }

    pub fn validate_definition(&self) -> Result<(), Error> {
        let valid = match self {
            Self::Float { min, max } => min.is_finite() && max.is_finite() && min <= max,
            Self::Integer { min, max } => min <= max,
            Self::Boolean => true,
            Self::Text { max_bytes } => *max_bytes <= MAX_TEXT_BYTES,
            Self::Enum { choices } => {
                !choices.is_empty()
                    && choices.len() <= MAX_ENUM_CHOICES
                    && choices.iter().enumerate().all(|(index, choice)| {
                        !choice.trim().is_empty()
                            && choice.len() <= MAX_TEXT_BYTES
                            && !choices[..index].contains(choice)
                    })
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidConfiguration("invalid value constraint"))
        }
    }

    pub fn validate(&self, value: &Value) -> Result<(), Error> {
        self.validate_definition()?;
        if self.value_type() != value.value_type() {
            return Err(Error::WrongType {
                expected: self.value_type(),
                actual: value.value_type(),
            });
        }
        let valid = match (self, value) {
            (Self::Float { min, max }, Value::Float(value)) => {
                if !value.is_finite() {
                    return Err(Error::NonFinite);
                }
                (min..=max).contains(&value)
            }
            (Self::Integer { min, max }, Value::Integer(value)) => (min..=max).contains(&value),
            (Self::Boolean, Value::Boolean(_)) => true,
            (Self::Text { max_bytes }, Value::Text(value)) => value.len() <= *max_bytes,
            (Self::Enum { choices }, Value::Enum(value)) => choices.contains(value),
            _ => unreachable!("value type checked above"),
        };
        if valid {
            Ok(())
        } else {
            Err(Error::OutOfRange)
        }
    }
}

/// Small vocabulary, not dimensional analysis or a physical safety profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Celsius,
    Percent,
    Pascal,
    Unitless,
}

impl Unit {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Percent => "%",
            Self::Pascal => "Pa",
            Self::Unitless => "1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
    WriteOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterRole {
    Measurement,
    Configuration,
    Actuator,
    Action,
    Diagnostic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteEffect {
    None,
    ConfigurationOnly,
    OutputAffecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterDescriptor {
    pub id: ParameterId,
    pub name: String,
    pub value_spec: ValueSpec,
    pub unit: Unit,
    pub access: AccessMode,
    pub role: ParameterRole,
    pub write_effect: WriteEffect,
    pub signal: Option<SignalId>,
}

impl ParameterDescriptor {
    pub fn validate_definition(&self) -> Result<(), Error> {
        validate_name(&self.name)?;
        self.value_spec.validate_definition()?;
        if self.unit != Unit::Unitless
            && !matches!(
                self.value_spec.value_type(),
                ValueType::Float | ValueType::Integer
            )
        {
            return Err(Error::InvalidConfiguration(
                "non-numeric parameter must be unitless",
            ));
        }
        let effect_valid = match self.access {
            AccessMode::ReadOnly => self.write_effect == WriteEffect::None,
            _ => self.write_effect != WriteEffect::None,
        };
        if !effect_valid
            || (self.role == ParameterRole::Actuator
                && self.access != AccessMode::ReadOnly
                && self.write_effect != WriteEffect::OutputAffecting)
            || (self.write_effect == WriteEffect::ConfigurationOnly
                && self.role != ParameterRole::Configuration)
        {
            return Err(Error::InvalidConfiguration(
                "inconsistent role/access/write effect",
            ));
        }
        if let Some(signal) = self.signal
            && (signal.parameter() != self.id
                || !matches!(
                    self.role,
                    ParameterRole::Measurement | ParameterRole::Diagnostic
                )
                || self.access == AccessMode::WriteOnly)
        {
            return Err(Error::InvalidConfiguration(
                "invalid measurement signal binding",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentDescriptor {
    pub id: InstrumentId,
    pub name: String,
    pub parameters: Vec<ParameterDescriptor>,
}

impl InstrumentDescriptor {
    pub fn parameter(&self, id: ParameterId) -> Option<&ParameterDescriptor> {
        self.parameters.iter().find(|parameter| parameter.id == id)
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() || name.len() > MAX_NAME_BYTES {
        Err(Error::InvalidConfiguration(
            "name must be nonblank and at most 128 UTF-8 bytes",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasurementFailure {
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownInstrument(InstrumentId),
    UnknownParameter {
        instrument: InstrumentId,
        parameter: ParameterId,
    },
    UnknownSignal(SignalId),
    DuplicateInstrument(InstrumentId),
    WrongType {
        expected: ValueType,
        actual: ValueType,
    },
    NonFinite,
    OutOfRange,
    ReadOnlyParameter(ParameterId),
    OperationNotAllowed(ParameterId),
    InvalidConfiguration(&'static str),
    NonMonotonicTime {
        signal: SignalId,
        previous: Duration,
        requested: Duration,
    },
    /// An unavailable observation WAS committed; unlike validation errors.
    MeasurementUnavailable {
        signal: SignalId,
        reason: MeasurementFailure,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownInstrument(id) => write!(f, "unknown instrument {}", id.get()),
            Self::UnknownParameter {
                instrument,
                parameter,
            } => write!(
                f,
                "unknown parameter {} on instrument {}",
                parameter.get(),
                instrument.get()
            ),
            Self::UnknownSignal(id) => write!(f, "unknown signal {id:?}"),
            Self::DuplicateInstrument(id) => write!(f, "duplicate instrument {}", id.get()),
            Self::WrongType { expected, actual } => {
                write!(f, "expected {expected:?}, got {actual:?}")
            }
            Self::NonFinite => f.write_str("non-finite value"),
            Self::OutOfRange => f.write_str("value outside declared constraint"),
            Self::ReadOnlyParameter(id) => write!(f, "read-only parameter {}", id.get()),
            Self::OperationNotAllowed(id) => {
                write!(f, "operation not allowed for parameter {}", id.get())
            }
            Self::InvalidConfiguration(reason) => write!(f, "invalid configuration: {reason}"),
            Self::NonMonotonicTime {
                signal,
                previous,
                requested,
            } => write!(
                f,
                "non-increasing time for {signal:?}: {requested:?} after {previous:?}"
            ),
            Self::MeasurementUnavailable { signal, reason } => {
                write!(f, "measurement unavailable for {signal:?}: {reason:?}")
            }
        }
    }
}

impl std::error::Error for Error {}
