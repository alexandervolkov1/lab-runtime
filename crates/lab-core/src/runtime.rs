use crate::{
    Error, InstrumentDescriptor, InstrumentId, ParameterId, Sample, SignalId, TEMPERATURE, Value,
    VirtualInstrumentConfig, model::validate_name, signal::SignalBuffer,
    virtual_instrument::VirtualInstrument,
};
use std::{collections::BTreeMap, time::Duration};

pub const MAX_INSTRUMENTS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    RegisterVirtual(VirtualInstrumentConfig),
    RenameInstrument {
        instrument: InstrumentId,
        name: String,
    },
    ConfigureParameter {
        instrument: InstrumentId,
        parameter: ParameterId,
        value: Value,
    },
    RefreshMeasurement {
        instrument: InstrumentId,
        parameter: ParameterId,
        at: Duration,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandResult {
    Registered(InstrumentId),
    Renamed(InstrumentId),
    Configured {
        instrument: InstrumentId,
        parameter: ParameterId,
    },
    MeasurementRefreshed(Sample),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Discover,
    DescribeInstrument(InstrumentId),
    GetInstrumentState(InstrumentId),
    GetLatestSignal(SignalId),
    GetSignalWindow(SignalId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueryResult {
    Instruments(Vec<InstrumentDescriptor>),
    Descriptor(InstrumentDescriptor),
    State(InstrumentState),
    Latest(Option<Sample>),
    Window(Vec<Sample>),
}

/// Configured values are separate from observations. Metadata-only actuators have neither.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentState {
    pub instrument: InstrumentId,
    pub configured: Vec<(ParameterId, Value)>,
    pub observations: Vec<ParameterObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterObservation {
    pub parameter: ParameterId,
    pub signal: SignalId,
    pub latest: Option<Sample>,
}

/// Single synchronous state owner. No I/O, interior mutability or background work.
#[derive(Default)]
pub struct Runtime {
    instruments: BTreeMap<InstrumentId, VirtualInstrument>,
}

impl Runtime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validation failures are atomic. MeasurementUnavailable is different:
    /// it commits a failed observation with the time of the explicit attempt.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        match command {
            Command::RegisterVirtual(config) => {
                let id = config.id;
                if self.instruments.contains_key(&id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instruments.len() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = VirtualInstrument::new(config)?;
                self.instruments.insert(id, instrument);
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
