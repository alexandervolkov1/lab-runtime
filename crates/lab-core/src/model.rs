//! Domain identity, scalar schemas and validation shared by all adapters.
//! Descriptors are data, not device drivers or output permissions. Validation
//! precedes mutation so a rejected candidate cannot damage an existing configuration.

use std::{cmp::Ordering, fmt, hash::Hash, time::Duration};

/// Maximum UTF-8 bytes in a nonblank display name.
pub const MAX_NAME_BYTES: usize = 128;
/// Maximum declared byte bound for a text value or enum choice.
pub const MAX_TEXT_BYTES: usize = 1024;
/// Maximum choices in a validated enum definition.
pub const MAX_ENUM_CHOICES: usize = 64;

/// Local identity, independent of a display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentId(u64);

impl InstrumentId {
    /// Construct a local identifier; registration, not this wrapper, enforces uniqueness.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Return the numeric local identity for display or adapter encoding, not a display name.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identity scoped to an instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParameterId(u64);

impl ParameterId {
    /// Construct a local identifier; registration, not this wrapper, enforces uniqueness.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Return the numeric local identity for display or adapter encoding, not a display name.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One measurement stream per parameter; no replacement/generation semantics in M1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalId {
    /// Target instrument identity, independent of its display name.
    instrument: InstrumentId,
    /// Parameter identity scoped to the target instrument.
    parameter: ParameterId,
}

impl SignalId {
    /// Construct a local identifier; registration, not this wrapper, enforces uniqueness.
    pub const fn new(instrument: InstrumentId, parameter: ParameterId) -> Self {
        Self {
            instrument,
            parameter,
        }
    }
    /// Return the instrument part of this stream identity.
    pub const fn instrument(self) -> InstrumentId {
        self.instrument
    }
    /// Return the parameter part of this stream identity.
    pub const fn parameter(self) -> ParameterId {
        self.parameter
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The exact scalar kind used by descriptors; clients must not infer it from formatting.
pub enum ValueType {
    /// A floating-point scalar; values must be finite when validated.
    Float,
    /// A signed integer scalar; no implicit conversion to floating point.
    Integer,
    /// A logical value, distinct from integer zero or one.
    Boolean,
    /// UTF-8 text subject to the descriptor's byte bound.
    Text,
    /// A named choice, distinct from free-form text.
    Enum,
}

#[derive(Clone, Debug, PartialEq)]
/// Owned scalar data crossing the domain boundary. Validation is explicit, never an implicit conversion.
pub enum Value {
    /// A floating-point scalar; values must be finite when validated.
    Float(f64),
    /// A signed integer scalar; no implicit conversion to floating point.
    Integer(i64),
    /// A logical value, distinct from integer zero or one.
    Boolean(bool),
    /// UTF-8 text subject to the descriptor's byte bound.
    Text(String),
    /// A named choice, distinct from free-form text.
    Enum(String),
}

impl Value {
    /// Return the exact scalar kind without converting or validating the payload.
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
    /// A floating-point scalar; values must be finite when validated.
    Float {
        /// Inclusive lower bound; floating-point bounds must be finite.
        min: f64,
        /// Inclusive upper bound, no smaller than the lower bound.
        max: f64,
    },
    /// A signed integer scalar; no implicit conversion to floating point.
    Integer {
        /// Inclusive lower bound; floating-point bounds must be finite.
        min: i64,
        /// Inclusive upper bound, no smaller than the lower bound.
        max: i64,
    },
    /// A logical value, distinct from integer zero or one.
    Boolean,
    /// UTF-8 text subject to the descriptor's byte bound.
    Text {
        /// Maximum UTF-8 byte length, bounded by MAX_TEXT_BYTES.
        max_bytes: usize,
    },
    /// A named choice, distinct from free-form text.
    Enum {
        /// Nonempty, unique, nonblank allowed names, bounded in count and byte length.
        choices: Vec<String>,
    },
}

impl ValueSpec {
    /// Return the exact scalar kind without converting or validating the payload.
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Float { .. } => ValueType::Float,
            Self::Integer { .. } => ValueType::Integer,
            Self::Boolean => ValueType::Boolean,
            Self::Text { .. } => ValueType::Text,
            Self::Enum { .. } => ValueType::Enum,
        }
    }

    /// Check declared bounds and constraints without mutating any state. Invalid definitions return InvalidConfiguration.
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

    /// Check definition, exact type, finiteness and constraints without mutation; return a typed rejection on failure.
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

/// Maximum bytes in a stable engineering-unit identity.
pub const MAX_UNIT_ID_BYTES: usize = 32;
/// Maximum UTF-8 bytes in an engineering-unit display symbol.
pub const MAX_UNIT_SYMBOL_BYTES: usize = 16;

#[derive(Clone, Copy)]
struct UnitText {
    bytes: [u8; MAX_UNIT_ID_BYTES],
    len: u8,
}

impl UnitText {
    const fn literal(value: &str) -> Self {
        let source = value.as_bytes();
        assert!(source.len() <= MAX_UNIT_ID_BYTES);
        let mut bytes = [0; MAX_UNIT_ID_BYTES];
        let mut index = 0;
        while index < source.len() {
            bytes[index] = source[index];
            index += 1;
        }
        Self {
            bytes,
            len: source.len() as u8,
        }
    }

    fn checked(value: &str, maximum: usize) -> Result<Self, Error> {
        if value.trim().is_empty() || value.len() > maximum {
            return Err(Error::InvalidConfiguration("invalid engineering unit"));
        }
        let mut bytes = [0; MAX_UNIT_ID_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("UnitText is built only from validated UTF-8")
    }
}

/// Bounded, extensible engineering-unit metadata.
///
/// Equality and ordering use the canonical identity only. Symbols are presentation
/// metadata: `Unit::new("percent", "pct")` identifies the same unit as [`Unit::PERCENT`].
/// The type intentionally performs no dimensional analysis or automatic conversion.
#[derive(Clone, Copy)]
pub struct Unit {
    id: UnitText,
    symbol: UnitText,
}

impl Unit {
    /// Degrees Celsius, represented by the canonical identity `degC`.
    pub const CELSIUS: Self = Self::literal("degC", "°C");
    /// Percent, represented by the canonical identity `percent`.
    pub const PERCENT: Self = Self::literal("percent", "%");
    /// Pascals, represented by the canonical identity `Pa`.
    pub const PASCAL: Self = Self::literal("Pa", "Pa");
    /// Dimensionless metadata, represented by the canonical identity `1`.
    pub const UNITLESS: Self = Self::literal("1", "1");

    const fn literal(id: &str, symbol: &str) -> Self {
        Self {
            id: UnitText::literal(id),
            symbol: UnitText::literal(symbol),
        }
    }

    /// Build a unit supplied by trusted configuration without a new Core enum variant.
    ///
    /// Identities are 1..=32 printable ASCII bytes without whitespace. Symbols are
    /// nonblank UTF-8 up to 16 bytes. These deliberately small rules bound metadata
    /// without claiming a universal engineering-unit grammar.
    pub fn new(id: &str, symbol: &str) -> Result<Self, Error> {
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
        {
            return Err(Error::InvalidConfiguration("invalid engineering unit"));
        }
        Ok(Self {
            id: UnitText::checked(id, MAX_UNIT_ID_BYTES)?,
            symbol: UnitText::checked(symbol, MAX_UNIT_SYMBOL_BYTES)?,
        })
    }

    /// Return the stable identity used for exact compatibility checks.
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// Return the generic presentation symbol, never a conversion rule.
    pub fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}

impl PartialEq for Unit {
    fn eq(&self, other: &Self) -> bool {
        self.id.bytes == other.id.bytes && self.id.len == other.id.len
    }
}
impl Eq for Unit {}
impl PartialOrd for Unit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Unit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id().cmp(other.id())
    }
}
impl Hash for Unit {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}
impl fmt::Debug for Unit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Unit")
            .field("id", &self.id())
            .field("symbol", &self.symbol())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Declared device access capability, not permission to bypass output authority.
pub enum AccessMode {
    /// Observation is supported; configuration writes are rejected.
    ReadOnly,
    /// Reads and writes are declared, but writes still require semantic authorization.
    ReadWrite,
    /// An operation may be requested without claiming a readable state.
    WriteOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Semantic purpose of a parameter; writable numeric configuration is not automatically an actuator.
pub enum ParameterRole {
    /// A measured quantity that can produce a Signal.
    Measurement,
    /// A setting whose declared effects must be checked before mutation.
    Configuration,
    /// A quantity that can affect the process, requiring output authority.
    Actuator,
    /// An explicit operation rather than an ordinary measurement.
    Action,
    /// Information about computation or device health, not output authority.
    Diagnostic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Declared consequences of a write. Output-affecting operations require centralized authority.
pub enum WriteEffect {
    /// No write side effects are declared; appropriate for read-only parameters.
    None,
    /// A configuration change with no actuator effect.
    ConfigurationOnly,
    /// A write can affect the controlled process and must not use the generic configuration path.
    OutputAffecting,
}

#[derive(Clone, Debug, PartialEq)]
/// Owned parameter metadata used for generic introspection and validation, separate from observations.
pub struct ParameterDescriptor {
    /// Stable local identity; display-name changes do not replace it.
    pub id: ParameterId,
    /// Human-readable display name, not an identity or lookup key.
    pub name: String,
    /// Declared scalar type and inclusive range or bounded text/choice constraints.
    pub value_spec: ValueSpec,
    /// Engineering unit attached to the declared value; no implicit conversion.
    pub unit: Unit,
    /// Device access declaration, distinct from runtime authorization.
    pub access: AccessMode,
    /// Explicit semantic classification used to distinguish configuration from actuation.
    pub role: ParameterRole,
    /// Consequences of writing this parameter, checked independently of its type.
    pub write_effect: WriteEffect,
    /// Identity of the associated measurement stream, when one exists.
    pub signal: Option<SignalId>,
}

impl ParameterDescriptor {
    /// Validate type/unit/access/effect and signal consistency before registration; does not mutate state.
    pub fn validate_definition(&self) -> Result<(), Error> {
        validate_name(&self.name)?;
        self.value_spec.validate_definition()?;
        if self.unit != Unit::UNITLESS
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
/// An owned catalog snapshot. Changing this copy never mutates the registered instrument.
pub struct InstrumentDescriptor {
    /// Stable local identity; display-name changes do not replace it.
    pub id: InstrumentId,
    /// Human-readable display name, not an identity or lookup key.
    pub name: String,
    /// Parameter metadata in stable descriptor order.
    pub parameters: Vec<ParameterDescriptor>,
}

impl InstrumentDescriptor {
    /// Look up metadata by stable parameter ID; a missing ID returns None without side effects.
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
/// Why an explicit measurement attempt could not produce a usable value.
pub enum MeasurementFailure {
    /// Measurement was explicitly disabled in the virtual configuration.
    Disabled,
    /// The device reported its explicit sensor-fault sentinel.
    SensorFault,
    /// Transport or protocol validation did not produce a trustworthy value.
    Transport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Typed domain rejection or observation failure; MeasurementUnavailable alone commits a failed sample.
pub enum Error {
    /// Native controller/reference registration or lifecycle was rejected.
    Controller(crate::control::ControllerError),
    /// A central output-authority operation was rejected.
    Output(crate::output::OutputError),
    /// A bounded byte executor operation was rejected.
    Transport(crate::transport::TransportError),
    /// Metakon framing or domain representation was invalid.
    Protocol(crate::metakon::CodecError),
    /// The requested instrument is not registered.
    UnknownInstrument(InstrumentId),
    /// The instrument exists but has no parameter with the requested identity.
    UnknownParameter {
        /// Target instrument identity, independent of its display name.
        instrument: InstrumentId,
        /// Parameter identity scoped to the target instrument.
        parameter: ParameterId,
    },
    /// No stream is bound to this signal identity.
    UnknownSignal(SignalId),
    /// Registration would replace an existing identity and is rejected.
    DuplicateInstrument(InstrumentId),
    /// The supplied scalar kind differs from the descriptor; coercion is not performed.
    WrongType {
        /// Scalar kind required by the descriptor.
        expected: ValueType,
        /// Scalar kind supplied by the caller.
        actual: ValueType,
    },
    /// NaN and infinities cannot enter a numeric signal/configuration path.
    NonFinite,
    /// A scalar violates its inclusive range or text/choice constraint.
    OutOfRange,
    /// The requested write targets read-only metadata.
    ReadOnlyParameter(ParameterId),
    /// The operation is unsupported or would cross the output authority boundary.
    OperationNotAllowed(ParameterId),
    /// A configuration is inconsistent or exceeds a declared resource limit.
    InvalidConfiguration(&'static str),
    /// The attempted observation does not follow the previous one in monotonic time.
    NonMonotonicTime {
        /// Identity of the associated measurement stream, when one exists.
        signal: SignalId,
        /// Timestamp of the last committed observation.
        previous: Duration,
        /// Rejected elapsed timestamp supplied by the caller.
        requested: Duration,
    },
    /// An unavailable observation WAS committed; unlike validation errors.
    MeasurementUnavailable {
        /// Identity of the associated measurement stream, when one exists.
        signal: SignalId,
        /// Explicit cause of the failed attempt, not a substitute numeric value.
        reason: MeasurementFailure,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Controller(error) => write!(f, "controller: {error:?}"),
            Self::Output(error) => write!(f, "output authority: {error:?}"),
            Self::Transport(error) => write!(f, "transport: {error:?}"),
            Self::Protocol(error) => write!(f, "Metakon protocol: {error:?}"),
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
