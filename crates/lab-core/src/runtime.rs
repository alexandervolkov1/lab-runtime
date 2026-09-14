//! The synchronous application boundary and sole owner of registered state.
//! Commands mutate owned instances; queries clone bounded snapshots. No GUI,
//! transport, clock polling or external client owns these instruments.

use crate::instrument::{
    KnownOperation, MetakonBinding, MetakonInstrument, MetakonInstrumentConfig,
};
use crate::metakon::{
    Address, ExpectedRead, MetakonType, MetakonValue, TemperatureReading, decode_ack, decode_read,
    encode_read, encode_scaled_i8, encode_write, scale_temperature,
};
use crate::output::{
    ActuatorId, OutputAuthority, OutputCommand, OutputError, OutputResult, OutputSnapshot,
};
use crate::transport::{
    AuthorizationStep, ByteTransport, ExecutorSnapshot, ResourceExecutor, ResourceId,
    TransactionId, TransactionOutcome, TransportError, TransportEvent,
};
use crate::{
    Error, InstrumentDescriptor, InstrumentId, ParameterId, Sample, SignalId, TEMPERATURE, Value,
    VirtualInstrumentConfig, model::validate_name, signal::SignalBuffer,
    virtual_instrument::VirtualInstrument,
};
use std::{collections::BTreeMap, time::Duration};

/// Maximum retained instruments per Runtime, bounding registry and observation ownership.
pub const MAX_INSTRUMENTS: usize = 64;
/// Maximum independently progressing byte resources owned by one Runtime.
pub const MAX_TRANSPORT_RESOURCES: usize = 8;

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
    /// Validate and atomically register a known-profile Metakon definition and binding.
    RegisterMetakon(MetakonInstrumentConfig),
    /// Queue one trusted known-profile read; this is a mutation because it schedules I/O.
    QueueMetakonRead {
        /// Registered logical instrument.
        instrument: InstrumentId,
        /// Readable parameter mapped to a known operation.
        parameter: ParameterId,
        /// Current monotonic Runtime time.
        at: Duration,
        /// Maximum queue wait, capped at 60 seconds.
        queue_ttl: Duration,
        /// Execution timeout after resource ownership begins.
        timeout: Duration,
    },
    /// Queue the currently reserved typed proposal/safe action for Metakon output.
    QueueMetakonOutput {
        /// Actuator whose authority already holds a proposal or safe request.
        actuator: ActuatorId,
        /// Current monotonic Runtime time.
        at: Duration,
        /// Maximum queue wait, additionally clipped by proposal/lease deadlines.
        queue_ttl: Duration,
        /// Execution timeout; an ambiguous write is never blindly retried.
        timeout: Duration,
    },
    /// Advance every resource by one bounded write/read/recovery attempt.
    PollTransports {
        /// Nondecreasing monotonic Runtime time.
        at: Duration,
    },
    /// Replace physical binding/mapping generations and invalidate all old authority.
    RebindMetakon {
        /// Existing logical instrument; its display identity is retained.
        instrument: InstrumentId,
        /// Fully validated replacement binding.
        binding: MetakonBinding,
        /// Nondecreasing monotonic Runtime time of replacement.
        at: Duration,
    },
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
    /// A bounded transport transaction was admitted, not yet completed.
    TransportQueued(TransactionId),
    /// Every registered resource received one bounded progress opportunity.
    TransportsPolled,
    /// An explicit attempt produced this good sample.
    MeasurementRefreshed(Sample),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Pure snapshot requests. Reading a query neither refreshes measurements nor advances time.
pub enum Query {
    /// Copy current output authority/evidence without executing watchdog work.
    Output(ActuatorId),
    /// Copy one byte resource's bounded state without polling it.
    Transport(ResourceId),
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
    /// Bounded resource state; no response history or raw adapter handle is exposed.
    Transport(ExecutorSnapshot),
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
    metakon_instruments: BTreeMap<InstrumentId, MetakonInstrument>,
    outputs: BTreeMap<ActuatorId, OutputAuthority>,
    resources: BTreeMap<ResourceId, ResourceExecutor>,
    pending_reads: BTreeMap<(ResourceId, TransactionId), PendingRead>,
    unsettled_outputs: BTreeMap<ResourceId, (ActuatorId, crate::output::DispatchId)>,
    output_time: Duration,
    transport_time: Duration,
}

struct PendingRead {
    instrument: InstrumentId,
    parameter: ParameterId,
    expected: ExpectedRead,
    operation: KnownOperation,
    scale: f64,
    binding_generation: u64,
    mapping_revision: u64,
}

impl Runtime {
    /// Create an empty owner with no registered instruments, measurements or active work.
    pub fn new() -> Self {
        Self::default()
    }

    /// Install one bounded byte adapter under exclusive Runtime ownership.
    ///
    /// The adapter is never exposed to instruments, controllers or clients.
    pub fn register_transport(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
    ) -> Result<(), Error> {
        if self.resources.contains_key(&id) {
            return Err(TransportError::DuplicateResource.into());
        }
        if self.resources.len() >= MAX_TRANSPORT_RESOURCES {
            return Err(TransportError::ResourceLimit.into());
        }
        self.resources
            .insert(id, ResourceExecutor::new(id, adapter));
        Ok(())
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
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
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
            Command::RegisterMetakon(config) => {
                let id = config.definition.id;
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = MetakonInstrument::new(config)?;
                let mut authorities = Vec::new();
                for parameter in &instrument.descriptor.parameters {
                    if parameter.role == crate::ParameterRole::Actuator {
                        let actuator = ActuatorId::new(id, parameter.id);
                        authorities.push((
                            actuator,
                            OutputAuthority::new(
                                actuator,
                                parameter.value_spec.clone(),
                                parameter.unit,
                            )?,
                        ));
                    }
                }
                self.metakon_instruments.insert(id, instrument);
                self.outputs.extend(authorities);
                Ok(CommandResult::Registered(id))
            }
            Command::QueueMetakonRead {
                instrument,
                parameter,
                at,
                queue_ttl,
                timeout,
            } => {
                self.check_transport_time(at)?;
                let instance = self
                    .metakon_instruments
                    .get(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                let definition = instance.definition.parameter_definition(parameter).ok_or(
                    Error::UnknownParameter {
                        instrument,
                        parameter,
                    },
                )?;
                if definition.access == crate::AccessMode::WriteOnly {
                    return Err(Error::OperationNotAllowed(parameter));
                }
                let binding = instance.binding;
                let address = Address::new(
                    binding.device,
                    binding.channel,
                    definition.operation.register(),
                );
                let (value_type, writable) = match definition.operation {
                    KnownOperation::ChannelType => (MetakonType::U8, false),
                    KnownOperation::Temperature => (MetakonType::I16, false),
                    KnownOperation::Output => (MetakonType::I8, true),
                };
                let expected = ExpectedRead::new(address, value_type, true, writable);
                let deadline = at
                    .checked_add(queue_ttl)
                    .ok_or(TransportError::InvalidTransaction)?;
                let request = encode_read(address)?;
                let executor = self
                    .resources
                    .get_mut(&binding.resource)
                    .ok_or(TransportError::UnknownResource)?;
                let transaction = executor.enqueue_read(
                    request.as_bytes(),
                    expected.frame_len(),
                    at,
                    deadline,
                    timeout,
                    true,
                    binding.binding_generation,
                    binding.mapping_revision,
                )?;
                self.pending_reads.insert(
                    (binding.resource, transaction),
                    PendingRead {
                        instrument,
                        parameter,
                        expected,
                        operation: definition.operation,
                        scale: definition.scale,
                        binding_generation: binding.binding_generation,
                        mapping_revision: binding.mapping_revision,
                    },
                );
                Ok(CommandResult::TransportQueued(transaction))
            }
            Command::QueueMetakonOutput {
                actuator,
                at,
                queue_ttl,
                timeout,
            } => {
                self.check_transport_time(at)?;
                let deadline = at
                    .checked_add(queue_ttl)
                    .ok_or(TransportError::InvalidTransaction)?;
                let instrument = self
                    .metakon_instruments
                    .get(&actuator.instrument())
                    .ok_or(OutputError::UnknownActuator)?;
                let definition = instrument
                    .definition
                    .parameter_definition(actuator.parameter())
                    .filter(|definition| definition.operation == KnownOperation::Output)
                    .ok_or(OutputError::UnknownActuator)?;
                let binding = instrument.binding;
                let authority = self
                    .outputs
                    .get_mut(&actuator)
                    .ok_or(OutputError::UnknownActuator)?;
                let intent = authority.reserve_transport(
                    at,
                    deadline,
                    binding.binding_generation,
                    binding.mapping_revision,
                )?;
                let raw = encode_scaled_i8(&Value::Float(intent.value), definition.scale)?;
                let address = Address::new(binding.device, binding.channel, 6);
                let request = encode_write(address, MetakonValue::I8(raw))?;
                let enqueue = self
                    .resources
                    .get_mut(&binding.resource)
                    .ok_or(TransportError::UnknownResource)
                    .and_then(|executor| {
                        executor.enqueue_output(
                            request.as_bytes(),
                            5,
                            at,
                            deadline,
                            timeout,
                            intent,
                        )
                    });
                match enqueue {
                    Ok(transaction) => Ok(CommandResult::TransportQueued(transaction)),
                    Err(error) => {
                        authority.abort_transport(intent);
                        Err(error.into())
                    }
                }
            }
            Command::PollTransports { at } => {
                self.poll_transports(at)?;
                Ok(CommandResult::TransportsPolled)
            }
            Command::RebindMetakon {
                instrument,
                binding,
                at,
            } => {
                self.check_output_time(at)?;
                let instance = self
                    .metakon_instruments
                    .get(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                if binding.binding_generation == 0
                    || binding.mapping_revision == 0
                    || !self.resources.contains_key(&binding.resource)
                {
                    return Err(Error::InvalidConfiguration("invalid replacement binding"));
                }
                let outputs: Vec<_> = instance
                    .descriptor
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.role == crate::ParameterRole::Actuator)
                    .collect();
                match (outputs.as_slice(), binding.expected_output_unit) {
                    ([], None) => {}
                    ([output], Some(unit)) if output.unit == unit => {}
                    _ => {
                        return Err(Error::InvalidConfiguration(
                            "output binding unit does not match descriptor",
                        ));
                    }
                }
                let mut replacements = Vec::new();
                for parameter in outputs {
                    let actuator = ActuatorId::new(instrument, parameter.id);
                    replacements.push((
                        actuator,
                        OutputAuthority::new(
                            actuator,
                            parameter.value_spec.clone(),
                            parameter.unit,
                        )?,
                    ));
                }
                self.metakon_instruments
                    .get_mut(&instrument)
                    .expect("validated above")
                    .binding = binding;
                self.outputs.extend(replacements);
                Ok(CommandResult::Registered(instrument))
            }
            Command::RenameInstrument { instrument, name } => {
                validate_name(&name)?;
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    instance.descriptor.name = name;
                } else if let Some(instance) = self.metakon_instruments.get_mut(&instrument) {
                    instance.descriptor.name = name;
                } else {
                    return Err(Error::UnknownInstrument(instrument));
                }
                Ok(CommandResult::Renamed(instrument))
            }
            Command::ConfigureParameter {
                instrument,
                parameter,
                value,
            } => {
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    instance.configure(parameter, value)?;
                } else if self.metakon_instruments.contains_key(&instrument) {
                    return Err(Error::OperationNotAllowed(parameter));
                } else {
                    return Err(Error::UnknownInstrument(instrument));
                }
                Ok(CommandResult::Configured {
                    instrument,
                    parameter,
                })
            }
            Command::RefreshMeasurement {
                instrument,
                parameter,
                at,
            } => {
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    instance
                        .refresh(parameter, at)
                        .map(CommandResult::MeasurementRefreshed)
                } else if self.metakon_instruments.contains_key(&instrument) {
                    Err(Error::OperationNotAllowed(parameter))
                } else {
                    Err(Error::UnknownInstrument(instrument))
                }
            }
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
            Query::Transport(id) => self
                .resources
                .get(&id)
                .map(|executor| QueryResult::Transport(executor.snapshot()))
                .ok_or(TransportError::UnknownResource.into()),
            Query::Discover => {
                let mut descriptors: Vec<_> = self
                    .instruments
                    .values()
                    .map(|instrument| instrument.descriptor.clone())
                    .chain(
                        self.metakon_instruments
                            .values()
                            .map(|instrument| instrument.descriptor.clone()),
                    )
                    .collect();
                descriptors.sort_by_key(|descriptor| descriptor.id);
                Ok(QueryResult::Instruments(descriptors))
            }
            Query::DescribeInstrument(id) => {
                Ok(QueryResult::Descriptor(self.descriptor(id)?.clone()))
            }
            Query::GetInstrumentState(id) => {
                if let Some(instrument) = self.instruments.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        configured: instrument.configured(),
                        observations: vec![ParameterObservation {
                            parameter: TEMPERATURE,
                            signal: SignalId::new(id, TEMPERATURE),
                            latest: instrument.signal.latest().cloned(),
                        }],
                    }))
                } else if let Some(instrument) = self.metakon_instruments.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        configured: instrument.configured(),
                        observations: instrument
                            .descriptor
                            .parameters
                            .iter()
                            .filter_map(|parameter| {
                                parameter.signal.map(|signal| ParameterObservation {
                                    parameter: parameter.id,
                                    signal,
                                    latest: instrument
                                        .signals
                                        .get(&signal)
                                        .and_then(SignalBuffer::latest)
                                        .cloned(),
                                })
                            })
                            .collect(),
                    }))
                } else {
                    Err(Error::UnknownInstrument(id))
                }
            }
            Query::GetLatestSignal(id) => {
                Ok(QueryResult::Latest(self.signal(id)?.latest().cloned()))
            }
            Query::GetSignalWindow(id) => Ok(QueryResult::Window(self.signal(id)?.window())),
        }
    }

    fn descriptor(&self, id: InstrumentId) -> Result<&InstrumentDescriptor, Error> {
        self.instruments
            .get(&id)
            .map(|instrument| &instrument.descriptor)
            .or_else(|| {
                self.metakon_instruments
                    .get(&id)
                    .map(|instrument| &instrument.descriptor)
            })
            .ok_or(Error::UnknownInstrument(id))
    }

    fn check_output_time(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.output_time {
            return Err(OutputError::InvalidTime.into());
        }
        self.output_time = at;
        for authority in self.outputs.values_mut() {
            authority.tick(at)?;
        }
        Ok(())
    }

    fn check_transport_time(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.transport_time {
            return Err(TransportError::InvalidTime.into());
        }
        self.transport_time = at;
        Ok(())
    }

    fn poll_transports(&mut self, at: Duration) -> Result<(), Error> {
        self.check_transport_time(at)?;
        self.check_output_time(at)?;
        let resources: Vec<_> = self.resources.keys().copied().collect();
        for resource in resources {
            // Temporarily taking the executor lets the coordinator borrow the
            // authority registry during the serialized final-check/write call.
            let mut executor = self
                .resources
                .remove(&resource)
                .ok_or(TransportError::UnknownResource)?;
            let instruments = &self.metakon_instruments;
            let outputs = &mut self.outputs;
            let event = executor.poll_authorized(at, &mut |intent, step| {
                let instrument = instruments.get(&intent.actuator.instrument()).ok_or(())?;
                let binding = instrument.binding;
                if binding.resource != resource
                    || binding.binding_generation != intent.binding_generation
                    || binding.mapping_revision != intent.mapping_revision
                {
                    return Err(());
                }
                let authority = outputs.get_mut(&intent.actuator).ok_or(())?;
                match step {
                    AuthorizationStep::Validate => {
                        authority.validate_transport(intent, at).map_err(|_| ())?;
                        Ok(None)
                    }
                    AuthorizationStep::Started => authority
                        .begin_transport(intent, at)
                        .map(Some)
                        .map_err(|_| ()),
                }
            })?;
            self.resources.insert(resource, executor);
            if let Some(event) = event {
                self.handle_transport_event(resource, event, at)?;
            }
        }
        Ok(())
    }

    fn handle_transport_event(
        &mut self,
        resource: ResourceId,
        event: TransportEvent,
        at: Duration,
    ) -> Result<(), Error> {
        match event {
            TransportEvent::ReadTerminal { record, response } => {
                let Some(pending) = self.pending_reads.remove(&(resource, record.id)) else {
                    return Ok(());
                };
                let current = self.metakon_instruments.get(&pending.instrument);
                let generation_matches = current.is_some_and(|instrument| {
                    instrument.binding.resource == resource
                        && instrument.binding.binding_generation == pending.binding_generation
                        && instrument.binding.mapping_revision == pending.mapping_revision
                });
                if !generation_matches {
                    return Ok(());
                }
                if record.outcome == TransactionOutcome::Completed {
                    let decoded = response
                        .as_deref()
                        .ok_or(crate::metakon::CodecError::WrongLength)
                        .and_then(|bytes| decode_read(bytes, pending.expected));
                    match decoded {
                        Ok(value) => self.apply_metakon_value(&pending, value, at)?,
                        Err(_) => {
                            self.push_transport_failure(&pending, at)?;
                            self.resources
                                .get_mut(&resource)
                                .expect("executor reinserted before event handling")
                                .protocol_failure();
                        }
                    }
                } else {
                    self.push_transport_failure(&pending, at)?;
                }
            }
            TransportEvent::OutputUncertain { intent, dispatch } => {
                if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                    authority.transport_uncertain(dispatch)?;
                }
            }
            TransportEvent::OutputTerminal {
                intent,
                dispatch,
                record,
                response,
            } => {
                let Some(dispatch) = dispatch else {
                    if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                        authority.abort_transport(intent);
                    }
                    return Ok(());
                };
                let current_binding = self
                    .metakon_instruments
                    .get(&intent.actuator.instrument())
                    .map(|instrument| instrument.binding);
                if record.outcome == TransactionOutcome::Completed
                    && current_binding.is_some_and(|binding| {
                        binding.resource == resource
                            && binding.binding_generation == intent.binding_generation
                            && binding.mapping_revision == intent.mapping_revision
                    })
                {
                    let binding = current_binding.expect("checked above");
                    let address = Address::new(binding.device, binding.channel, 6);
                    if response
                        .as_deref()
                        .is_some_and(|bytes| decode_ack(bytes, address).is_ok())
                    {
                        if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                            authority.complete_transport(
                                dispatch,
                                crate::output::DispatchOutcome::Acknowledged,
                                at,
                            )?;
                        }
                    } else if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                        authority.transport_uncertain(dispatch)?;
                        self.unsettled_outputs
                            .insert(resource, (intent.actuator, dispatch));
                        self.resources
                            .get_mut(&resource)
                            .expect("executor reinserted before event handling")
                            .protocol_failure();
                    }
                } else if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                    authority.complete_transport(
                        dispatch,
                        crate::output::DispatchOutcome::Ambiguous,
                        at,
                    )?;
                }
            }
            TransportEvent::BoundaryRecovered => {
                if let Some((actuator, dispatch)) = self.unsettled_outputs.remove(&resource)
                    && let Some(authority) = self.outputs.get_mut(&actuator)
                {
                    authority.complete_transport(
                        dispatch,
                        crate::output::DispatchOutcome::Ambiguous,
                        at,
                    )?;
                }
            }
            TransportEvent::BoundaryFailed => {}
        }
        Ok(())
    }

    fn apply_metakon_value(
        &mut self,
        pending: &PendingRead,
        value: MetakonValue,
        at: Duration,
    ) -> Result<(), Error> {
        let instrument = self
            .metakon_instruments
            .get_mut(&pending.instrument)
            .ok_or(Error::UnknownInstrument(pending.instrument))?;
        let descriptor =
            instrument
                .descriptor
                .parameter(pending.parameter)
                .ok_or(Error::UnknownParameter {
                    instrument: pending.instrument,
                    parameter: pending.parameter,
                })?;
        let Some(signal) = descriptor.signal else {
            return Ok(());
        };
        let domain_value = match pending.operation {
            KnownOperation::ChannelType => match value {
                MetakonValue::U8(3) => Value::Integer(3),
                _ => {
                    instrument
                        .signals
                        .get_mut(&signal)
                        .expect("validated signal")
                        .push(Sample::unavailable(
                            signal,
                            descriptor.unit,
                            at,
                            crate::MeasurementFailure::Transport,
                        ))?;
                    return Ok(());
                }
            },
            KnownOperation::Temperature => match scale_temperature(value, pending.scale)? {
                TemperatureReading::Value(value) => Value::Float(value),
                TemperatureReading::SensorFault => {
                    instrument
                        .signals
                        .get_mut(&signal)
                        .expect("validated signal")
                        .push(Sample::unavailable(
                            signal,
                            descriptor.unit,
                            at,
                            crate::MeasurementFailure::SensorFault,
                        ))?;
                    return Ok(());
                }
            },
            KnownOperation::Output => return Ok(()),
        };
        descriptor.value_spec.validate(&domain_value)?;
        instrument
            .signals
            .get_mut(&signal)
            .expect("validated signal")
            .push(Sample::good(signal, descriptor.unit, at, domain_value))
    }

    fn push_transport_failure(&mut self, pending: &PendingRead, at: Duration) -> Result<(), Error> {
        let instrument = self
            .metakon_instruments
            .get_mut(&pending.instrument)
            .ok_or(Error::UnknownInstrument(pending.instrument))?;
        let descriptor =
            instrument
                .descriptor
                .parameter(pending.parameter)
                .ok_or(Error::UnknownParameter {
                    instrument: pending.instrument,
                    parameter: pending.parameter,
                })?;
        if let Some(signal) = descriptor.signal {
            instrument
                .signals
                .get_mut(&signal)
                .expect("validated signal")
                .push(Sample::unavailable(
                    signal,
                    descriptor.unit,
                    at,
                    crate::MeasurementFailure::Transport,
                ))?;
        }
        Ok(())
    }

    fn contains_instrument(&self, id: InstrumentId) -> bool {
        self.instruments.contains_key(&id) || self.metakon_instruments.contains_key(&id)
    }

    fn instrument_count(&self) -> usize {
        self.instruments.len() + self.metakon_instruments.len()
    }

    fn signal(&self, id: SignalId) -> Result<&SignalBuffer, Error> {
        self.instruments
            .get(&id.instrument())
            .filter(|_| id.parameter() == TEMPERATURE)
            .map(|instrument| &instrument.signal)
            .or_else(|| {
                self.metakon_instruments
                    .get(&id.instrument())
                    .and_then(|instrument| instrument.signals.get(&id))
            })
            .ok_or(Error::UnknownSignal(id))
    }
}
