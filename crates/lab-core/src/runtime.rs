//! The synchronous application boundary and sole owner of registered state.
//! Commands mutate owned instances; queries clone bounded snapshots. No GUI,
//! transport, clock polling or external client owns these instruments.

use crate::output::{
    ActuatorId, OutputAuthority, OutputCommand, OutputError, OutputResult, OutputSnapshot,
};
use crate::{
    Error, InstrumentDescriptor, InstrumentId, ParameterId, Sample, SignalId, TEMPERATURE, Value,
    VirtualInstrumentConfig, model::validate_name, signal::SignalBuffer,
    virtual_instrument::VirtualInstrument,
};
use std::{collections::BTreeMap, time::Duration};

/// Maximum retained instruments per Runtime, bounding registry and observation ownership.
pub const MAX_INSTRUMENTS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
/// Local mutation requests serialized by the Runtime owner; no networking or hidden query effects.
pub enum Command {
    /// Mutate output authority or step the trusted deterministic simulated executor.
    Output {
        /// Canonical actuator binding, validated against its explicit descriptor.
        actuator: ActuatorId,
        /// Lifecycle, proposal or simulated-dispatch operation.
        command: OutputCommand,
        /// Nondecreasing elapsed time shared by all output authorities in this Runtime.
        at: Duration,
    },
    /// Validate and register a native virtual instrument without producing a measurement.
    RegisterVirtual(VirtualInstrumentConfig),
    /// Change only the display name; preserve all identities and observations.
    RenameInstrument {
        /// Target instrument identity, independent of its display name.
        instrument: InstrumentId,
        /// Human-readable display name, not an identity or lookup key.
        name: String,
    },
    /// Validate a configuration-only value before committing it; preserve observations.
    ConfigureParameter {
        /// Target instrument identity, independent of its display name.
        instrument: InstrumentId,
        /// Parameter identity scoped to the target instrument.
        parameter: ParameterId,
        /// Candidate configuration value, validated before replacing the previous setting.
        value: Value,
    },
    /// Create one explicit observation at caller-supplied monotonic elapsed time.
    RefreshMeasurement {
        /// Target instrument identity, independent of its display name.
        instrument: InstrumentId,
        /// Parameter identity scoped to the target instrument.
        parameter: ParameterId,
        /// Elapsed monotonic runtime time of this explicit measurement attempt.
        at: Duration,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// Completed local command outcomes, not a claim of physical hardware or durable storage success.
pub enum CommandResult {
    /// Authority outcome; queue acceptance is not delivery or safe-state confirmation.
    Output(OutputResult),
    /// Registration completed without starting acquisition or output authority.
    Registered(InstrumentId),
    /// The display name changed without replacing the instance.
    Renamed(InstrumentId),
    /// One configuration value was committed; no new observation was fabricated.
    Configured {
        /// Target instrument identity, independent of its display name.
        instrument: InstrumentId,
        /// Parameter identity scoped to the target instrument.
        parameter: ParameterId,
    },
    /// An explicit attempt produced this good sample.
    MeasurementRefreshed(Sample),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Pure snapshot requests. Reading a query neither refreshes measurements nor advances time.
pub enum Query {
    /// Copy current output authority/evidence without executing watchdog work.
    Output(ActuatorId),
    /// Return all instrument descriptors in stable ID order.
    Discover,
    /// Return metadata for one existing instrument without accessing hardware.
    DescribeInstrument(InstrumentId),
    /// Return configured values separately from the latest observation attempts.
    GetInstrumentState(InstrumentId),
    /// Return the latest attempt or None if no attempt has occurred.
    GetLatestSignal(SignalId),
    /// Copy the bounded recent window in oldest-to-newest order.
    GetSignalWindow(SignalId),
}

#[derive(Clone, Debug, PartialEq)]
/// Owned query responses; callers may retain or edit their copies without affecting Runtime.
pub enum QueryResult {
    /// Bounded output snapshot, distinct from measurement/configuration state.
    Output(OutputSnapshot),
    /// Catalog descriptors in stable instrument-ID order.
    Instruments(Vec<InstrumentDescriptor>),
    /// Metadata for one instrument; this copy has no authority to mutate Runtime.
    Descriptor(InstrumentDescriptor),
    /// Configured state and observations captured by the synchronous owner.
    State(InstrumentState),
    /// The last attempted observation, including failure; None means never refreshed.
    Latest(Option<Sample>),
    /// A bounded copy of recent attempts, ordered by monotonic timestamp.
    Window(Vec<Sample>),
}

/// Configured values are separate from observations. Metadata-only actuators have neither.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentState {
    /// Target instrument identity, independent of its display name.
    pub instrument: InstrumentId,
    /// Current configuration values; these are not sensor observations or commanded outputs.
    pub configured: Vec<(ParameterId, Value)>,
    /// Latest measurement attempts; metadata-only actuators have no fabricated entry.
    pub observations: Vec<ParameterObservation>,
}

#[derive(Clone, Debug, PartialEq)]
/// Latest known attempt for a measurement parameter, including explicit failure or no attempt yet.
pub struct ParameterObservation {
    /// Parameter identity scoped to the target instrument.
    pub parameter: ParameterId,
    /// Identity of the associated measurement stream, when one exists.
    pub signal: SignalId,
    /// Latest attempt, including unavailable quality; None means no refresh has occurred.
    pub latest: Option<Sample>,
}

/// Single synchronous state owner. No I/O, interior mutability or background work.
#[derive(Default)]
pub struct Runtime {
    instruments: BTreeMap<InstrumentId, VirtualInstrument>,
    outputs: BTreeMap<ActuatorId, OutputAuthority>,
    output_time: Duration,
}

impl Runtime {
    /// Create an empty owner with no registered instruments, measurements or active work.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configuration/registration validation failures are atomic.
    /// MeasurementUnavailable commits a failed observation with the attempt time.
    /// Output commands first advance the explicit watchdog: expired authority may
    /// be revoked even when the requested producer action is then rejected.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        match command {
            Command::Output {
                actuator,
                command,
                at,
            } => {
                if !self.outputs.contains_key(&actuator) {
                    return Err(OutputError::UnknownActuator.into());
                }
                if at < self.output_time {
                    return Err(OutputError::InvalidTime.into());
                }
                // Output commands carry trusted Runtime time. Watchdog transitions
                // occur even if the requested producer action subsequently fails.
                // A client therefore cannot keep another expired owner alive by
                // submitting invalid proposals instead of explicit Tick commands.
                self.output_time = at;
                for authority in self.outputs.values_mut() {
                    authority.tick(at)?;
                }
                self.outputs
                    .get_mut(&actuator)
                    .ok_or(OutputError::UnknownActuator)?
                    .command(command, at)
                    .map(CommandResult::Output)
            }
            Command::RegisterVirtual(config) => {
                let id = config.id;
                if self.instruments.contains_key(&id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instruments.len() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = VirtualInstrument::new(config)?;
                let actuator = ActuatorId::new(id, crate::HEATER_POWER);
                let parameter = instrument.parameter(crate::HEATER_POWER)?;
                if parameter.role != crate::ParameterRole::Actuator
                    || parameter.write_effect != crate::WriteEffect::OutputAffecting
                {
                    return Err(OutputError::UnknownActuator.into());
                }
                let authority =
                    OutputAuthority::new(actuator, parameter.value_spec.clone(), parameter.unit)?;
                self.instruments.insert(id, instrument);
                self.outputs.insert(actuator, authority);
                Ok(CommandResult::Registered(id))
            }
            Command::RenameInstrument { instrument, name } => {
                let instance = self.instrument_mut(instrument)?;
                validate_name(&name)?;
                instance.descriptor.name = name;
                Ok(CommandResult::Renamed(instrument))
            }
            Command::ConfigureParameter {
                instrument,
                parameter,
                value,
            } => {
                self.instrument_mut(instrument)?
                    .configure(parameter, value)?;
                Ok(CommandResult::Configured {
                    instrument,
                    parameter,
                })
            }
            Command::RefreshMeasurement {
                instrument,
                parameter,
                at,
            } => self
                .instrument_mut(instrument)?
                .refresh(parameter, at)
                .map(CommandResult::MeasurementRefreshed),
        }
    }

    /// Reads owned snapshots only. Never refreshes, reads a clock or advances simulation.
    pub fn query(&self, query: Query) -> Result<QueryResult, Error> {
        match query {
            Query::Output(id) => self
                .outputs
                .get(&id)
                .map(|authority| QueryResult::Output(authority.snapshot()))
                .ok_or(OutputError::UnknownActuator.into()),
            Query::Discover => Ok(QueryResult::Instruments(
                self.instruments
                    .values()
                    .map(|instrument| instrument.descriptor.clone())
                    .collect(),
            )),
            Query::DescribeInstrument(id) => Ok(QueryResult::Descriptor(
                self.instrument(id)?.descriptor.clone(),
            )),
            Query::GetInstrumentState(id) => {
                let instrument = self.instrument(id)?;
                Ok(QueryResult::State(InstrumentState {
                    instrument: id,
                    configured: instrument.configured(),
                    observations: vec![ParameterObservation {
                        parameter: TEMPERATURE,
                        signal: SignalId::new(id, TEMPERATURE),
                        latest: instrument.signal.latest().cloned(),
                    }],
                }))
            }
            Query::GetLatestSignal(id) => {
                Ok(QueryResult::Latest(self.signal(id)?.latest().cloned()))
            }
            Query::GetSignalWindow(id) => Ok(QueryResult::Window(self.signal(id)?.window())),
        }
    }

    fn instrument(&self, id: InstrumentId) -> Result<&VirtualInstrument, Error> {
        self.instruments
            .get(&id)
            .ok_or(Error::UnknownInstrument(id))
    }

    fn instrument_mut(&mut self, id: InstrumentId) -> Result<&mut VirtualInstrument, Error> {
        self.instruments
            .get_mut(&id)
            .ok_or(Error::UnknownInstrument(id))
    }

    fn signal(&self, id: SignalId) -> Result<&SignalBuffer, Error> {
        self.instruments
            .get(&id.instrument())
            .filter(|_| id.parameter() == TEMPERATURE)
            .map(|instrument| &instrument.signal)
            .ok_or(Error::UnknownSignal(id))
    }
}
