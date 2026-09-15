//! The synchronous application boundary and sole owner of registered state.
//! Commands mutate owned instances; queries clone bounded snapshots. No GUI,
//! transport, clock polling or external client owns these instruments.

use crate::control::{
    ControllerError, ControllerId, ControllerSnapshot, ControllerState, NativeController,
    NativeControllerConfig, Pid, PidConfig,
};
use crate::instrument::{
    KnownOperation, MetakonBinding, MetakonInstrument, MetakonInstrumentConfig,
};
use crate::managed::{
    CapturedInput, ComponentCompletion, ComponentDefinition, ComponentError, ComponentExecutor,
    ComponentId, ComponentKind, ComponentManifest, ComponentSnapshot, ComponentState,
    ComponentStatus, Correlation, Invocation, InvocationPhase, MAX_COMPONENTS, MAX_SOURCE_BYTES,
    PlainData,
};
use crate::metakon::{
    Address, ExpectedRead, MetakonType, MetakonValue, TemperatureReading, decode_ack, decode_read,
    encode_read, encode_scaled_i8, encode_write, scale_temperature,
};
use crate::output::{
    ActuatorId, DispatchOutcome, OutputAuthority, OutputCommand, OutputError, OutputOwner,
    OutputProposal, OutputResult, OutputSnapshot,
};
use crate::plant::{ThermalPlantConfig, ThermalPlantInstrument};
use crate::processing::EmaStatus;
use crate::reference::{
    ReferenceConfig, ReferenceError, ReferenceId, ReferenceSnapshot, ReferenceValue, RetunedRamp,
    RuntimeReference,
};
use crate::transport::{
    AuthorizationStep, ByteTransport, ExecutorSnapshot, ResourceExecutor, ResourceId,
    TransactionId, TransactionOutcome, TransportError, TransportEvent,
};
use crate::{
    Error, InstrumentDescriptor, InstrumentId, ParameterId, ParameterRole, Sample, SampleQuality,
    SignalId, TEMPERATURE, Unit, Value, ValueSpec, VirtualInstrumentConfig, model::validate_name,
    signal::SignalBuffer, virtual_instrument::VirtualInstrument,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_COMPONENT_RUNTIME: AtomicU64 = AtomicU64::new(1);

/// Maximum retained instruments per Runtime, bounding registry and observation ownership.
pub const MAX_INSTRUMENTS: usize = 64;
/// Maximum independently progressing byte resources owned by one Runtime.
pub const MAX_TRANSPORT_RESOURCES: usize = 8;
/// Maximum native References retained by one Runtime.
pub const MAX_REFERENCES: usize = 64;
/// Maximum native controllers retained by one Runtime.
pub const MAX_CONTROLLERS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
/// Local mutation requests serialized by the Runtime owner; no networking or hidden query effects.
pub enum Command {
    /// Stage one bounded managed source/transform; init runs on a nonblocking worker.
    StageComponent {
        /// Immutable shape/source/config supplied by trusted local deployment.
        definition: ComponentDefinition,
        /// Existing committed component for a staged same-shape replacement, if any.
        replaces: Option<ComponentId>,
        /// Trusted Runtime monotonic admission time.
        at: Duration,
    },
    /// Submit at most one managed Step, returning Busy if pending/worker occupied.
    InvokeComponent {
        /// Committed component to process.
        component: ComponentId,
        /// Trusted scheduled observation/model time.
        at: Duration,
    },
    /// Service safety/deadline and poll at most two bounded component completions.
    PollComponents {
        /// Trusted nondecreasing Runtime service/publication time.
        at: Duration,
    },
    /// Stop all managed admissions, fence pending results and fail their dependents.
    QuiesceManaged {
        /// Trusted nondecreasing Runtime shutdown time.
        at: Duration,
    },
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
    /// Register a deterministic first-order virtual thermal plant.
    RegisterThermalPlant(ThermalPlantConfig),
    /// Register an independent native target source.
    RegisterReference(ReferenceConfig),
    /// Advance one independent Reference at trusted monotonic Runtime time.
    EvaluateReference {
        /// Existing native Reference identity.
        reference: ReferenceId,
        /// Nondecreasing trusted evaluation time.
        at: Duration,
    },
    /// Retune an existing Ramp continuously with an optimistic configuration revision.
    RetuneRampReference {
        /// Existing Ramp identity.
        reference: ReferenceId,
        /// New finite target in the existing unit.
        target: f64,
        /// Positive finite engineering units per second.
        rate: f64,
        /// Revision read from a pure Reference snapshot.
        expected_revision: u64,
        /// Trusted nondecreasing Runtime mutation time.
        at: Duration,
    },
    /// Register controller data without acquiring output authority.
    RegisterController(NativeControllerConfig),
    /// Atomically configure gains/limits in Ready or safely Paused without output.
    ConfigureControllerPid {
        /// Existing controller whose bindings/timing stay unchanged.
        controller: ControllerId,
        /// Full finite five-field replacement value.
        pid: PidConfig,
        /// Revision read from a pure controller snapshot.
        expected_revision: u64,
    },
    /// Check a Created controller against current descriptors and Reference units.
    PrepareController(ControllerId),
    /// Begin distinct-sample warm-up; acquire only when the configured count is reached.
    StartController {
        /// Controller to start from Ready.
        controller: ControllerId,
        /// Explicit monotonic Runtime time.
        at: Duration,
    },
    /// Advance warm-up or consume one distinct sample for native output progress.
    TickController {
        /// Running controller to advance.
        controller: ControllerId,
        /// Explicit monotonic Runtime time.
        at: Duration,
    },
    /// Cancel warm-up or revoke automatic authority and complete safe output.
    PauseController {
        /// Running controller to pause.
        controller: ControllerId,
        /// Explicit monotonic Runtime time.
        at: Duration,
    },
    /// Reset algorithm memory and repeat the same warm-up policy as Start.
    ResumeController {
        /// Paused controller to resume.
        controller: ControllerId,
        /// Explicit monotonic Runtime time.
        at: Duration,
    },
    /// Explicitly recover a failed native loop only after Rust safe evidence and fault acknowledgement.
    ResetFailedController {
        /// Existing failed native controller; no client session owns its lifetime.
        controller: ControllerId,
        /// Nondecreasing trusted Runtime recovery time.
        at: Duration,
    },
    /// Append an explicit unavailable attempt to an M4 thermal plant signal.
    InjectPlantMeasurementFailure {
        /// Registered thermal plant identity.
        instrument: InstrumentId,
        /// Explicit monotonic attempt time.
        at: Duration,
    },
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
    /// Advance watchdog, native dependency checks and safe/transport work independently of producers.
    ServiceSafety {
        /// Nondecreasing trusted monotonic Runtime time.
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
    /// Candidate admitted for worker init, not yet installed as an instrument.
    ComponentStaged(ComponentId),
    /// Managed step admitted, not a Good sample or completed callback.
    ComponentInvoked(Correlation),
    /// Bounded completion/safety service was polled, possibly with no result.
    ComponentsPolled,
    /// Managed admission is permanently fenced for this Runtime instance.
    ManagedQuiesced,
    /// Authority outcome; queue acceptance is not delivery or safe-state confirmation.
    Output(OutputResult),
    /// Registration completed without starting acquisition or output authority.
    Registered(InstrumentId),
    /// A native Reference was registered without evaluating it.
    ReferenceRegistered(ReferenceId),
    /// One independent Reference evaluation, not a query or a controller tick.
    ReferenceEvaluated(ReferenceValue),
    /// Full committed continuous Ramp state and advanced configuration revision.
    ReferenceRetuned(RetunedRamp),
    /// A native controller registration or lifecycle transition completed.
    ControllerUpdated(ControllerSnapshot),
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
    /// Bounded safety/watchdog work was serviced without running a producer callback.
    SafetyServiced,
    /// An explicit attempt produced this good sample.
    MeasurementRefreshed(Sample),
    /// An explicit unavailable plant measurement attempt was retained.
    MeasurementFailed(Sample),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Pure snapshot requests. Reading a query neither refreshes measurements nor advances time.
pub enum Query {
    /// Copy a committed component snapshot without evaluating its script.
    Component(ComponentId),
    /// Copy current output authority/evidence without executing watchdog work.
    Output(ActuatorId),
    /// Copy one byte resource's bounded state without polling it.
    Transport(ResourceId),
    /// Copy one native controller's bounded lifecycle/algorithm diagnostics.
    Controller(ControllerId),
    /// Copy native controller construction/config values without progressing it.
    ControllerConfig(ControllerId),
    /// Copy one native Reference's bounded state without advancing it.
    Reference(ReferenceId),
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
    /// Committed bounded component state without its worker VM.
    Component(ComponentSnapshot),
    /// Bounded output snapshot, distinct from measurement/configuration state.
    Output(OutputSnapshot),
    /// Bounded resource state; no response history or raw adapter handle is exposed.
    Transport(ExecutorSnapshot),
    /// Bounded native controller diagnostics with no output capability.
    Controller(ControllerSnapshot),
    /// Full immutable controller configuration; no output capability is conveyed.
    ControllerConfig(NativeControllerConfig),
    /// Bounded native Reference state with no time advancement.
    Reference(ReferenceSnapshot),
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
    managed: BTreeMap<ComponentId, ManagedInstance>,
    staged: Option<StagedComponent>,
    executor: Option<Box<dyn ComponentExecutor>>,
    managed_quiesced: bool,
    component_runtime: u64,
    instruments: BTreeMap<InstrumentId, VirtualInstrument>,
    metakon_instruments: BTreeMap<InstrumentId, MetakonInstrument>,
    thermal_plants: BTreeMap<InstrumentId, ThermalPlantInstrument>,
    references: BTreeMap<ReferenceId, RuntimeReference>,
    controllers: BTreeMap<ControllerId, NativeController>,
    outputs: BTreeMap<ActuatorId, OutputAuthority>,
    resources: BTreeMap<ResourceId, ResourceExecutor>,
    pending_reads: BTreeMap<(ResourceId, TransactionId), PendingRead>,
    unsettled_outputs: BTreeMap<ResourceId, (ActuatorId, crate::output::DispatchId)>,
    output_time: Duration,
    transport_time: Duration,
    recording_facts: crate::recording::FactOutbox,
    required_recording: crate::recording::RequiredGate,
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

struct ManagedInstance {
    definition: ComponentDefinition,
    descriptor: InstrumentDescriptor,
    signal: SignalBuffer,
    generation: u64,
    revision: u64,
    attempt: u64,
    state: ComponentState,
    committed: PlainData,
    good_steps: usize,
    last_service: Option<Duration>,
    last_observation: Option<Duration>,
    pending: Option<Correlation>,
    pending_input: Option<CapturedInput>,
    pending_at: Option<Duration>,
    diagnostics: Vec<String>,
}

struct StagedComponent {
    definition: ComponentDefinition,
    replaces: Option<ComponentId>,
    correlation: Correlation,
}

impl ManagedInstance {
    fn snapshot(&self) -> ComponentSnapshot {
        ComponentSnapshot {
            id: self.definition.manifest.id,
            instrument: self.definition.manifest.instrument,
            generation: self.generation,
            revision: self.revision,
            state: self.state,
            good_steps: self.good_steps,
            committed_state: self.committed.clone(),
            pending: self.pending,
            diagnostics: self.diagnostics.clone(),
        }
    }
}

impl Runtime {
    /// Create an empty owner with no registered instruments, measurements or active work.
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm bounded source-fact capture for a host-owned recording run.
    /// This does not assert that any fact has reached durable storage.
    pub fn enable_recording_facts(&mut self) {
        self.recording_facts.enable();
    }

    /// Stop fact capture at a durable interval boundary; discarded idle facts
    /// never masquerade as a complete recorded interval.
    pub fn disable_recording_facts(&mut self) {
        self.recording_facts.disable();
    }

    /// Drain captured facts without disk I/O or waiting for a host worker.
    pub fn take_recording_facts(&mut self) -> Vec<crate::recording::RecordingFact> {
        self.recording_facts.take()
    }

    /// Report a sticky capture overflow; the host must apply its selected policy.
    pub fn recording_facts_overflowed(&self) -> bool {
        self.recording_facts.overflowed()
    }

    /// First attempted fact identity whose payload was not retained in the
    /// bounded outbox; None means no known capture loss in this run.
    pub fn recording_first_lost_fact(&self) -> Option<u64> {
        self.recording_facts.first_lost_sequence()
    }

    /// Count of suppressed facts, or None after its checked counter exhausts.
    pub fn recording_suppressed_fact_count(&self) -> Option<u64> {
        self.recording_facts.suppressed_facts()
    }

    /// Install a closed Required prerequisite before exposing tracked control.
    /// This local trusted seam is intentionally absent from all wire DTOs.
    pub fn require_recording(&mut self, _at: Duration) {
        self.required_recording.enabled = true;
        self.required_recording.open = false;
        self.required_recording.failed = false;
        self.required_recording.valid_until = None;
        self.required_recording.confirmed_submission = None;
        self.required_recording.generation = self.required_recording.generation.saturating_add(1);
    }

    /// Open Required control after the start barrier actually committed.
    /// The deadline derives from the original submission time, never receipt arrival.
    pub fn confirm_recording_start(
        &mut self,
        submitted_at: Duration,
        now: Duration,
    ) -> Result<(), Error> {
        if !self.required_recording.enabled || self.required_recording.failed || submitted_at > now
        {
            return Err(Error::RecordingUnavailable);
        }
        let deadline = submitted_at
            .checked_add(crate::recording::REQUIRED_PROGRESS_AGE)
            .ok_or(Error::RecordingUnavailable)?;
        if now >= deadline {
            self.recording_failure(now);
            return Err(Error::RecordingUnavailable);
        }
        self.required_recording.confirmed_submission = Some(submitted_at);
        self.required_recording.valid_until = Some(deadline);
        self.required_recording.open = true;
        Ok(())
    }

    /// Advance the gate only for confirmed, strictly newer worker submissions.
    pub fn confirm_recording_progress(
        &mut self,
        submitted_at: Duration,
        now: Duration,
    ) -> Result<(), Error> {
        self.service_required_recording_deadline(now);
        if !self.required_recording.open
            || submitted_at > now
            || self
                .required_recording
                .confirmed_submission
                .is_some_and(|old| submitted_at <= old)
        {
            return Err(Error::RecordingUnavailable);
        }
        let deadline = submitted_at
            .checked_add(crate::recording::REQUIRED_PROGRESS_AGE)
            .ok_or(Error::RecordingUnavailable)?;
        if now >= deadline {
            self.recording_failure(now);
            return Err(Error::RecordingUnavailable);
        }
        self.required_recording.confirmed_submission = Some(submitted_at);
        self.required_recording.valid_until = Some(deadline);
        Ok(())
    }

    /// Close Required recording and trip all tracked control without disk I/O.
    /// A safe service opportunity remains available after this call.
    pub fn recording_failure(&mut self, at: Duration) {
        if !self.required_recording.enabled || self.required_recording.failed {
            return;
        }
        self.required_recording.open = false;
        self.required_recording.failed = true;
        self.required_recording.valid_until = None;
        // Remove Warming as well as Running eligibility before any output trip.
        for (id, controller) in &mut self.controllers {
            if matches!(
                controller.state,
                ControllerState::Warming | ControllerState::Running
            ) {
                controller.state = ControllerState::Failed;
                controller.lease = None;
                self.recording_facts.controller(
                    *id,
                    ControllerState::Failed,
                    controller.config_revision,
                    None,
                    at,
                );
            }
        }
        // Trip every tracked output even if one authority rejects its transition.
        // Safety dispatch stays on the ordinary Core/transport service path.
        for (actuator, authority) in &mut self.outputs {
            if authority.command(OutputCommand::Trip, at).is_ok() {
                self.recording_facts.output(
                    *actuator,
                    crate::recording::OutputStage::Revoked,
                    None,
                    at,
                );
            }
        }
    }

    /// Close a clean Required interval after all tracked control is paused.
    /// A later start requires a new committed barrier and never rearms output.
    pub fn close_required_recording(&mut self) {
        if self.required_recording.enabled {
            self.required_recording.open = false;
            self.required_recording.valid_until = None;
        }
    }

    fn service_required_recording_deadline(&mut self, at: Duration) {
        if self.required_recording.enabled && self.recording_facts.overflowed() {
            self.recording_failure(at);
        }
        if self.required_recording.enabled
            && self.required_recording.open
            && self
                .required_recording
                .valid_until
                .is_some_and(|deadline| at >= deadline)
        {
            self.recording_failure(at);
        }
    }

    fn require_recording_open(&mut self, at: Duration) -> Result<(), Error> {
        self.service_required_recording_deadline(at);
        if self.required_recording.enabled && !self.required_recording.open {
            return Err(Error::RecordingUnavailable);
        }
        Ok(())
    }

    /// Install exactly one trusted nonblocking executor port before staging work.
    /// Its implementation must never share a Runtime/authority lock with a worker.
    pub fn install_component_executor(
        &mut self,
        executor: Box<dyn ComponentExecutor>,
    ) -> Result<(), Error> {
        if self.executor.is_some() || self.staged.is_some() || !self.managed.is_empty() {
            return Err(ComponentError::Busy.into());
        }
        let identity = NEXT_COMPONENT_RUNTIME
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ComponentError::IdentityExhausted)?;
        self.component_runtime = identity;
        self.executor = Some(executor);
        Ok(())
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

    /// Inspect unfinished worker count without waiting for interpreter execution.
    pub fn unfinished_component_workers(&self) -> usize {
        self.executor
            .as_ref()
            .map_or(0, |executor| executor.unfinished_workers())
    }

    /// Configuration/registration validation failures are atomic.
    /// MeasurementUnavailable commits a failed observation with the attempt time.
    /// Output commands first advance the explicit watchdog: expired authority may
    /// be revoked even when the requested producer action is then rejected.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        match command {
            Command::StageComponent {
                definition,
                replaces,
                at,
            } => self.stage_component(definition, replaces, at),
            Command::InvokeComponent { component, at } => self.invoke_component(component, at),
            Command::PollComponents { at } => {
                self.poll_components(at)?;
                Ok(CommandResult::ComponentsPolled)
            }
            Command::QuiesceManaged { at } => {
                self.check_output_time(at)?;
                if !self.managed_quiesced {
                    self.managed_quiesced = true;
                    if let Some(staged) = self.staged.take()
                        && let Some(executor) = self.executor.as_mut()
                    {
                        executor.try_cancel(staged.correlation);
                    }
                    let identities: Vec<_> = self.managed.keys().copied().collect();
                    for id in identities {
                        self.fail_component(id, at);
                    }
                    if let Some(executor) = self.executor.as_mut() {
                        executor.begin_shutdown();
                    }
                }
                Ok(CommandResult::ManagedQuiesced)
            }
            Command::Output {
                actuator,
                command,
                at,
            } => {
                self.service_required_recording_deadline(at);
                let ordinary = matches!(
                    command,
                    OutputCommand::Acquire { .. }
                        | OutputCommand::Propose(_)
                        | OutputCommand::AcknowledgeFault
                ) || matches!(command, OutputCommand::BeginDispatch)
                    && self.outputs.get(&actuator).is_some_and(|authority| {
                        authority.snapshot().state != crate::output::OutputState::SafePending
                    });
                if ordinary {
                    self.require_recording_open(at)?;
                }
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
                if self.warming_on(actuator) {
                    match command {
                        OutputCommand::RequestSafe => {
                            self.cancel_warming(actuator, ControllerState::Paused);
                        }
                        OutputCommand::Trip => {
                            self.cancel_warming(actuator, ControllerState::Failed);
                        }
                        OutputCommand::Acquire { .. } => return Err(OutputError::Busy.into()),
                        OutputCommand::BindProfile(_) => {
                            return Err(OutputError::InvalidState.into());
                        }
                        _ => {}
                    }
                }
                let issued = command.clone();
                let was_safe_completion = self
                    .outputs
                    .get(&actuator)
                    .and_then(|authority| authority.snapshot().in_flight)
                    .is_some_and(|dispatch| dispatch.is_safe());
                let result = self
                    .outputs
                    .get_mut(&actuator)
                    .ok_or(OutputError::UnknownActuator)?
                    .command(command, at)?;
                match (&issued, &result) {
                    (OutputCommand::RequestSafe, OutputResult::Updated) => {
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::SafeRequested,
                            None,
                            at,
                        );
                    }
                    (OutputCommand::Trip, OutputResult::Updated) => {
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::Revoked,
                            None,
                            at,
                        );
                    }
                    (OutputCommand::Propose(_), OutputResult::Queued) => {
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::Requested,
                            self.outputs
                                .get(&actuator)
                                .and_then(|authority| authority.snapshot().requested),
                            at,
                        );
                    }
                    (OutputCommand::BeginDispatch, OutputResult::Dispatched(dispatch))
                        if !dispatch.is_safe() =>
                    {
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::Authorized,
                            Some(dispatch.value()),
                            at,
                        );
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::SendStarted,
                            Some(dispatch.value()),
                            at,
                        );
                    }
                    (OutputCommand::BeginDispatch, OutputResult::Dispatched(dispatch)) => {
                        self.recording_facts.output(
                            actuator,
                            crate::recording::OutputStage::SafeSendStarted,
                            Some(dispatch.value()),
                            at,
                        );
                    }
                    (OutputCommand::Complete { outcome, .. }, OutputResult::Completed) => {
                        let stage = match (was_safe_completion, outcome) {
                            (true, DispatchOutcome::Acknowledged) => {
                                crate::recording::OutputStage::SafeAcknowledged
                            }
                            (true, DispatchOutcome::ReadbackVerified) => {
                                crate::recording::OutputStage::SafeReadbackVerified
                            }
                            (false, DispatchOutcome::Acknowledged) => {
                                crate::recording::OutputStage::Acknowledged
                            }
                            (false, DispatchOutcome::ReadbackVerified) => {
                                crate::recording::OutputStage::ReadbackVerified
                            }
                            (_, DispatchOutcome::Failed) => crate::recording::OutputStage::Failed,
                            (_, DispatchOutcome::Ambiguous) => {
                                crate::recording::OutputStage::Ambiguous
                            }
                        };
                        self.recording_facts.output_with_source(
                            actuator,
                            stage,
                            self.outputs.get(&actuator).and_then(|authority| {
                                authority.snapshot().sent.map(|sent| sent.value)
                            }),
                            at,
                            crate::recording::OutputEvidenceSource::VirtualSimulation,
                        );
                    }
                    _ => {}
                }
                if let OutputResult::Dispatched(dispatch) = result {
                    self.apply_virtual_dispatch(actuator, dispatch.value())?;
                    Ok(CommandResult::Output(OutputResult::Dispatched(dispatch)))
                } else {
                    Ok(CommandResult::Output(result))
                }
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
            Command::RegisterThermalPlant(config) => {
                let id = config.id;
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = ThermalPlantInstrument::new(config)?;
                let actuator = ActuatorId::new(id, crate::HEATER_POWER);
                let parameter = instrument
                    .descriptor
                    .parameter(crate::HEATER_POWER)
                    .ok_or(OutputError::UnknownActuator)?;
                let authority =
                    OutputAuthority::new(actuator, parameter.value_spec.clone(), parameter.unit)?;
                self.thermal_plants.insert(id, instrument);
                self.outputs.insert(actuator, authority);
                Ok(CommandResult::Registered(id))
            }
            Command::RegisterReference(config) => {
                let id = config.id();
                if self.references.contains_key(&id) {
                    return Err(ControllerError::DuplicateReference.into());
                }
                if self.references.len() >= MAX_REFERENCES {
                    return Err(ControllerError::InvalidConfiguration.into());
                }
                let reference = RuntimeReference::new(config)
                    .map_err(|_| ControllerError::InvalidConfiguration)?;
                self.references.insert(id, reference);
                self.capture_reference_fact(
                    id,
                    match config {
                        ReferenceConfig::Fixed { .. } => Duration::ZERO,
                        ReferenceConfig::Ramp { at, .. } => at,
                    },
                );
                Ok(CommandResult::ReferenceRegistered(id))
            }
            Command::EvaluateReference { reference, at } => {
                let result = self
                    .references
                    .get_mut(&reference)
                    .ok_or(ControllerError::UnknownReference.into())
                    .and_then(|item| {
                        item.value_at(at)
                            .map(CommandResult::ReferenceEvaluated)
                            .map_err(map_reference_error)
                            .map_err(Into::into)
                    });
                if result.is_ok() {
                    self.capture_reference_fact(reference, at);
                }
                result
            }
            Command::RetuneRampReference {
                reference,
                target,
                rate,
                expected_revision,
                at,
            } => {
                let result = self
                    .references
                    .get_mut(&reference)
                    .ok_or(ControllerError::UnknownReference.into())
                    .and_then(|item| {
                        item.retune(target, rate, expected_revision, at)
                            .map(CommandResult::ReferenceRetuned)
                            .map_err(map_reference_error)
                            .map_err(Into::into)
                    });
                if result.is_ok() {
                    self.capture_reference_fact(reference, at);
                }
                result
            }
            Command::RegisterController(config) => {
                let id = config.id;
                if self.controllers.contains_key(&id) {
                    return Err(ControllerError::DuplicateController.into());
                }
                if self.controllers.len() >= MAX_CONTROLLERS {
                    return Err(ControllerError::InvalidConfiguration.into());
                }
                let controller = NativeController::new(config)?;
                let snapshot = controller.snapshot();
                self.controllers.insert(id, controller);
                self.recording_facts.controller(
                    id,
                    snapshot.state,
                    snapshot.config_revision,
                    Some(config.pid),
                    self.output_time,
                );
                Ok(CommandResult::ControllerUpdated(snapshot))
            }
            Command::ConfigureControllerPid {
                controller,
                pid,
                expected_revision,
            } => self.configure_controller_pid(controller, pid, expected_revision),
            Command::PrepareController(id) => {
                let result = self.prepare_controller(id);
                if result.is_ok() {
                    self.capture_controller_fact(id, self.output_time);
                }
                result
            }
            Command::StartController { controller, at } => {
                self.require_recording_open(at)?;
                let result =
                    self.start_or_resume_controller(controller, at, ControllerState::Ready);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::TickController { controller, at } => {
                self.require_recording_open(at)?;
                let result = self.tick_controller(controller, at);
                self.capture_controller_fact(controller, at);
                result
            }
            Command::PauseController { controller, at } => {
                let result = self.pause_controller(controller, at);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::ResumeController { controller, at } => {
                self.require_recording_open(at)?;
                let result =
                    self.start_or_resume_controller(controller, at, ControllerState::Paused);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::ResetFailedController { controller, at } => {
                let result = self.reset_failed_controller(controller, at);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::InjectPlantMeasurementFailure { instrument, at } => {
                let plant = self
                    .thermal_plants
                    .get_mut(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                let result = plant.inject_failure(at);
                if result.is_ok()
                    && let Some(sample) = plant.signal.latest().cloned()
                {
                    self.recording_facts.measurement(sample, 1, 1);
                }
                result.map(CommandResult::MeasurementFailed)
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
            Command::ServiceSafety { at } => {
                self.service_safety(at)?;
                Ok(CommandResult::SafetyServiced)
            }
            Command::RebindMetakon {
                instrument,
                binding,
                at,
            } => {
                if self.controllers.values().any(|controller| {
                    controller.state == ControllerState::Warming
                        && controller.config.output.instrument() == instrument
                }) {
                    return Err(OutputError::Busy.into());
                }
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
                } else if let Some(instance) = self.thermal_plants.get_mut(&instrument) {
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
                } else if self.metakon_instruments.contains_key(&instrument)
                    || self.thermal_plants.contains_key(&instrument)
                {
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
                    let result = instance.refresh(parameter, at);
                    if parameter == TEMPERATURE
                        && let Some(sample) = instance.signal.latest().cloned()
                        && sample.at() == at
                    {
                        self.recording_facts.measurement(sample, 1, 1);
                    }
                    result.map(CommandResult::MeasurementRefreshed)
                } else if let Some(instance) = self.thermal_plants.get_mut(&instrument) {
                    if parameter != TEMPERATURE {
                        return Err(Error::OperationNotAllowed(parameter));
                    }
                    let result = instance.refresh(at);
                    if let Some(sample) = instance.signal.latest().cloned()
                        && sample.at() == at
                    {
                        self.recording_facts.measurement(sample, 1, 1);
                    }
                    result.map(CommandResult::MeasurementRefreshed)
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
            Query::Component(id) => self
                .managed
                .get(&id)
                .map(|component| QueryResult::Component(component.snapshot()))
                .ok_or(ComponentError::Unknown.into()),
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
            Query::Controller(id) => self
                .controllers
                .get(&id)
                .map(|controller| QueryResult::Controller(controller.snapshot()))
                .ok_or(ControllerError::UnknownController.into()),
            Query::ControllerConfig(id) => self
                .controllers
                .get(&id)
                .map(|controller| QueryResult::ControllerConfig(controller.config))
                .ok_or(ControllerError::UnknownController.into()),
            Query::Reference(id) => self
                .references
                .get(&id)
                .map(|reference| QueryResult::Reference(reference.snapshot()))
                .ok_or(ControllerError::UnknownReference.into()),
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
                    .chain(
                        self.thermal_plants
                            .values()
                            .map(|instrument| instrument.descriptor.clone()),
                    )
                    .chain(
                        self.managed
                            .values()
                            .map(|component| component.descriptor.clone()),
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
                } else if let Some(instrument) = self.thermal_plants.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        configured: instrument.configured(),
                        observations: vec![ParameterObservation {
                            parameter: TEMPERATURE,
                            signal: SignalId::new(id, TEMPERATURE),
                            latest: instrument.signal.latest().cloned(),
                        }],
                    }))
                } else if let Some(component) = self
                    .managed
                    .values()
                    .find(|item| item.definition.manifest.instrument == id)
                {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        configured: vec![],
                        observations: vec![ParameterObservation {
                            parameter: component.definition.manifest.parameter,
                            signal: SignalId::new(id, component.definition.manifest.parameter),
                            latest: component.signal.latest().cloned(),
                        }],
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

    /// Validate all authority/descriptor constraints before touching controller memory.
    fn configure_controller_pid(
        &mut self,
        id: ControllerId,
        pid: PidConfig,
        expected_revision: u64,
    ) -> Result<CommandResult, Error> {
        let controller = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?;
        if controller.config_revision != expected_revision {
            return Err(ControllerError::RevisionConflict.into());
        }
        if !matches!(
            controller.state,
            ControllerState::Ready | ControllerState::Paused
        ) || controller.lease.is_some()
            || self.warming_on(controller.config.output)
        {
            return Err(ControllerError::InvalidState.into());
        }
        let next = controller
            .config_revision
            .checked_add(1)
            .ok_or(ControllerError::RevisionExhausted)?;
        let new_pid = Pid::new(pid).map_err(|_| ControllerError::InvalidConfiguration)?;
        let authority = self
            .outputs
            .get(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if !authority.accepts_pid_limits(pid.output_min, pid.output_max) {
            return Err(ControllerError::Output.into());
        }
        let controller = self.controllers.get_mut(&id).expect("checked above");
        controller.pid = new_pid;
        controller.config.pid = pid;
        controller.reset_algorithms();
        controller.last_tick = None;
        controller.config_revision = next;
        self.recording_facts.controller(
            id,
            controller.state,
            controller.config_revision,
            Some(pid),
            self.output_time,
        );
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    fn prepare_controller(&mut self, id: ControllerId) -> Result<CommandResult, Error> {
        let config = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?
            .config;
        if self.controllers.get(&id).expect("looked up above").state != ControllerState::Created {
            return Err(ControllerError::InvalidState.into());
        }

        let input = self
            .descriptor(config.input.instrument())?
            .parameter(config.input.parameter())
            .filter(|parameter| {
                parameter.signal == Some(config.input)
                    && parameter.role == ParameterRole::Measurement
            })
            .ok_or(ControllerError::InvalidConfiguration)?;
        if !matches!(input.value_spec, ValueSpec::Float { .. }) || input.unit != config.ema.unit {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        let reference = self
            .references
            .get(&config.reference)
            .ok_or(ControllerError::UnknownReference)?;
        if reference.unit() != input.unit {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        let output = self
            .descriptor(config.output.instrument())?
            .parameter(config.output.parameter())
            .filter(|parameter| parameter.role == ParameterRole::Actuator)
            .ok_or(ControllerError::InvalidConfiguration)?;
        let ValueSpec::Float { min, max } = output.value_spec else {
            return Err(ControllerError::InvalidConfiguration.into());
        };
        if config.pid.output_min < min
            || config.pid.output_max > max
            || !self.outputs.contains_key(&config.output)
            || !self
                .thermal_plants
                .contains_key(&config.output.instrument())
        {
            return Err(ControllerError::InvalidConfiguration.into());
        }
        if !self
            .outputs
            .get(&config.output)
            .expect("output checked above")
            .valid_native_duration(config.max_tick_gap, config.lease_lifetime)
        {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        let controller = self.controllers.get_mut(&id).expect("validated above");
        controller.state = ControllerState::Ready;
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    /// Validate the manifest and topology before sending disposable init work to a worker.
    fn validate_component_definition(
        &self,
        definition: &ComponentDefinition,
        replaces: Option<ComponentId>,
    ) -> Result<(), Error> {
        let manifest = &definition.manifest;
        validate_name(&manifest.name)?;
        definition.config.validate()?;
        if manifest.schema_version != 1
            || definition.source.is_empty()
            || definition.source.len() > MAX_SOURCE_BYTES
            || !manifest.min.is_finite()
            || !manifest.max.is_finite()
            || manifest.min >= manifest.max
            || !(1..=64).contains(&manifest.warmup_samples)
            || manifest.max_input_age.is_zero()
        {
            return Err(ComponentError::InvalidConfiguration.into());
        }
        SignalBuffer::new(
            SignalId::new(manifest.instrument, manifest.parameter),
            manifest.history_capacity,
        )?;
        match replaces {
            None => {
                if self.managed.contains_key(&manifest.id)
                    || self.contains_instrument(manifest.instrument)
                    || self.managed.len() >= MAX_COMPONENTS
                    || self.instrument_count() >= MAX_INSTRUMENTS
                {
                    return Err(ComponentError::InvalidConfiguration.into());
                }
            }
            Some(id) => {
                let old = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
                let previous = &old.definition.manifest;
                if id != manifest.id
                    || previous.instrument != manifest.instrument
                    || previous.parameter != manifest.parameter
                    || previous.kind != manifest.kind
                    || previous.unit != manifest.unit
                    || previous.min != manifest.min
                    || previous.max != manifest.max
                    || previous.name != manifest.name
                    || previous.warmup_samples != manifest.warmup_samples
                    || previous.max_input_age != manifest.max_input_age
                    || previous.history_capacity != manifest.history_capacity
                {
                    return Err(ComponentError::InvalidConfiguration.into());
                }
                old.generation
                    .checked_add(1)
                    .ok_or(ComponentError::IdentityExhausted)?;
            }
        }
        if let ComponentKind::Transform { input } = manifest.kind {
            if input.instrument() == manifest.instrument {
                return Err(ComponentError::InvalidConfiguration.into());
            }
            if self.managed.values().any(|item| {
                item.definition.manifest.instrument == input.instrument()
                    && matches!(
                        item.definition.manifest.kind,
                        ComponentKind::Transform { .. }
                    )
            }) {
                return Err(ComponentError::InvalidConfiguration.into());
            }
            let parameter = self
                .descriptor(input.instrument())?
                .parameter(input.parameter())
                .ok_or(ComponentError::InvalidConfiguration)?;
            if parameter.signal != Some(input)
                || parameter.role != ParameterRole::Measurement
                || !matches!(parameter.value_spec, ValueSpec::Float { .. })
                || parameter.unit != manifest.unit
            {
                return Err(ComponentError::InvalidConfiguration.into());
            }
        }
        Ok(())
    }

    fn stage_component(
        &mut self,
        definition: ComponentDefinition,
        replaces: Option<ComponentId>,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        if self.managed_quiesced {
            return Err(ComponentError::Executor.into());
        }
        if self.staged.is_some() {
            return Err(ComponentError::Busy.into());
        }
        self.validate_component_definition(&definition, replaces)?;
        self.check_output_time(at)?;
        let generation = replaces.map_or(Ok(1), |id| {
            self.managed
                .get(&id)
                .expect("validated above")
                .generation
                .checked_add(1)
                .ok_or(ComponentError::IdentityExhausted)
        })?;
        let revision = replaces.map_or(0, |id| {
            self.managed.get(&id).expect("validated above").revision
        });
        let correlation = Correlation {
            runtime: self.component_runtime,
            component: definition.manifest.id,
            generation,
            attempt: 0,
            revision,
        };
        let job = Invocation {
            correlation,
            phase: InvocationPhase::Init,
            definition: definition.clone(),
            state: PlainData::default(),
            at,
            dt: Duration::ZERO,
            input: None,
        };
        self.executor
            .as_mut()
            .ok_or(ComponentError::Executor)?
            .try_submit(job)?;
        let id = definition.manifest.id;
        self.staged = Some(StagedComponent {
            definition,
            replaces,
            correlation,
        });
        Ok(CommandResult::ComponentStaged(id))
    }

    fn managed_descriptor(manifest: &ComponentManifest) -> Result<InstrumentDescriptor, Error> {
        let signal = SignalId::new(manifest.instrument, manifest.parameter);
        let parameter = crate::ParameterDescriptor {
            id: manifest.parameter,
            name: "value".into(),
            value_spec: ValueSpec::Float {
                min: manifest.min,
                max: manifest.max,
            },
            unit: manifest.unit,
            access: crate::AccessMode::ReadOnly,
            role: ParameterRole::Measurement,
            write_effect: crate::WriteEffect::None,
            signal: Some(signal),
        };
        parameter.validate_definition()?;
        Ok(InstrumentDescriptor {
            id: manifest.instrument,
            name: manifest.name.clone(),
            parameters: vec![parameter],
        })
    }

    fn validate_managed_result(
        result: &crate::managed::ComponentResult,
        manifest: &ComponentManifest,
        phase: InvocationPhase,
    ) -> Result<(), ComponentError> {
        if result.unit != manifest.unit
            || result.state.validate().is_err()
            || result.diagnostics.len() > 4
            || result.diagnostics.iter().any(|line| line.len() > 256)
            || result.diagnostics.iter().map(String::len).sum::<usize>() > 1024
        {
            return Err(ComponentError::InvalidResult);
        }
        match (phase, result.status, result.value) {
            (InvocationPhase::Init, ComponentStatus::Init, None) => Ok(()),
            (InvocationPhase::Step, ComponentStatus::Warming, None)
            | (InvocationPhase::Step, ComponentStatus::Unavailable, None) => Ok(()),
            (InvocationPhase::Step, ComponentStatus::Ready, Some(value))
                if value.is_finite() && (manifest.min..=manifest.max).contains(&value) =>
            {
                Ok(())
            }
            _ => Err(ComponentError::InvalidResult),
        }
    }

    /// Init is validated before quiescing dependents or replacing the committed definition.
    fn commit_staged(
        &mut self,
        staged: StagedComponent,
        result: crate::managed::ComponentResult,
        at: Duration,
    ) -> Result<(), Error> {
        Self::validate_managed_result(&result, &staged.definition.manifest, InvocationPhase::Init)?;
        if let Some(id) = staged.replaces {
            let old = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
            let pending_to_cancel = old.pending;
            if old.generation.checked_add(1) != Some(staged.correlation.generation)
                || old.revision != staged.correlation.revision
                || old.signal.latest().is_some_and(|last| last.at() > at)
            {
                return Err(ComponentError::InvalidResult.into());
            }
            let signal = SignalId::new(
                old.definition.manifest.instrument,
                old.definition.manifest.parameter,
            );
            let dependent_ids: Vec<_> = self
                .managed
                .iter()
                .filter(|(other, component)| {
                    **other != id
                        && component.definition.manifest.kind
                            == ComponentKind::Transform { input: signal }
                })
                .map(|(other, _)| *other)
                .collect();
            let mut affected_signals = vec![signal];
            for dependent in &dependent_ids {
                if let Some(component) = self.managed.get(dependent) {
                    if component.signal.latest().is_some_and(|last| last.at() > at) {
                        return Err(ComponentError::InvalidResult.into());
                    }
                    affected_signals.push(SignalId::new(
                        component.definition.manifest.instrument,
                        component.definition.manifest.parameter,
                    ));
                }
            }
            let affected: Vec<_> = self
                .controllers
                .iter()
                .filter(|(_, controller)| {
                    affected_signals.contains(&controller.config.input)
                        && matches!(
                            controller.state,
                            ControllerState::Running | ControllerState::Warming
                        )
                })
                .map(|(controller, _)| *controller)
                .collect();
            for controller in affected {
                self.pause_controller(controller, at)?;
            }
            if let Some(correlation) = pending_to_cancel
                && let Some(executor) = self.executor.as_mut()
            {
                executor.try_cancel(correlation);
            }
            for dependent in dependent_ids {
                if let Some(other) = self.managed.get_mut(&dependent) {
                    if let Some(correlation) = other.pending
                        && let Some(executor) = self.executor.as_mut()
                    {
                        executor.try_cancel(correlation);
                    }
                    other.pending = None;
                    other.pending_input = None;
                    other.pending_at = None;
                    other.state = ComponentState::Failed;
                    if other.signal.latest().is_some() {
                        other
                            .signal
                            .invalidate(at, crate::MeasurementFailure::ComponentFailure)?;
                    }
                }
            }
        }
        let manifest = &staged.definition.manifest;
        let descriptor = Self::managed_descriptor(manifest)?;
        let signal_id = SignalId::new(manifest.instrument, manifest.parameter);
        let signal = if let Some(old) = staged.replaces {
            let mut retained = self
                .managed
                .remove(&old)
                .ok_or(ComponentError::Unknown)?
                .signal;
            if retained.latest().is_some() {
                retained.invalidate(at, crate::MeasurementFailure::ComponentFailure)?;
            }
            retained
        } else {
            SignalBuffer::new(signal_id, manifest.history_capacity)?
        };
        self.managed.insert(
            manifest.id,
            ManagedInstance {
                definition: staged.definition,
                descriptor,
                signal,
                generation: staged.correlation.generation,
                revision: 0,
                attempt: 0,
                state: ComponentState::Warming,
                committed: result.state,
                good_steps: 0,
                last_service: None,
                last_observation: None,
                pending: None,
                pending_input: None,
                pending_at: None,
                diagnostics: result.diagnostics,
            },
        );
        Ok(())
    }

    /// Validate a fresh typed input before any executor work is admitted.
    fn capture_component_input(
        &self,
        manifest: &ComponentManifest,
        at: Duration,
    ) -> Result<Option<CapturedInput>, ComponentError> {
        let ComponentKind::Transform { input } = manifest.kind else {
            return Ok(None);
        };
        let sample = self
            .signal(input)
            .ok()
            .and_then(SignalBuffer::latest)
            .ok_or(ComponentError::InputUnavailable)?;
        if sample.quality() != SampleQuality::Good
            || sample.unit() != manifest.unit
            || at < sample.freshness_at()
            || at - sample.freshness_at() >= manifest.max_input_age
        {
            return Err(ComponentError::InputUnavailable);
        }
        let Some(Value::Float(value)) = sample.value() else {
            return Err(ComponentError::InputUnavailable);
        };
        if !value.is_finite() {
            return Err(ComponentError::InputUnavailable);
        }
        Ok(Some(CapturedInput {
            signal: input,
            value: *value,
            unit: sample.unit(),
            at: sample.at(),
            freshness_at: sample.freshness_at(),
        }))
    }

    fn invoke_component(&mut self, id: ComponentId, at: Duration) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        if self.managed_quiesced {
            return Err(ComponentError::Executor.into());
        }
        let component = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
        if component.state == ComponentState::Failed {
            return Err(ComponentError::InvalidResult.into());
        }
        if component.pending.is_some() {
            return Err(ComponentError::Busy.into());
        }
        let captured = self.capture_component_input(&component.definition.manifest, at);
        let input = match captured {
            Ok(input) => input,
            Err(error) => {
                self.fail_component(id, at);
                return Err(error.into());
            }
        };
        let observation = input.map_or(at, |reading| reading.freshness_at);
        if component
            .last_observation
            .is_some_and(|last| observation <= last)
            || component.last_service.is_some_and(|last| at <= last)
        {
            return Err(ComponentError::InputUnavailable.into());
        }
        let dt = component
            .last_observation
            .map_or(Duration::ZERO, |last| observation - last);
        let attempt = component
            .attempt
            .checked_add(1)
            .ok_or(ComponentError::IdentityExhausted)?;
        let correlation = Correlation {
            runtime: self.component_runtime,
            component: id,
            generation: component.generation,
            attempt,
            revision: component.revision,
        };
        let job = Invocation {
            correlation,
            phase: InvocationPhase::Step,
            definition: component.definition.clone(),
            state: component.committed.clone(),
            at,
            dt,
            input,
        };
        self.executor
            .as_mut()
            .ok_or(ComponentError::Executor)?
            .try_submit(job)?;
        let component = self.managed.get_mut(&id).expect("checked above");
        component.attempt = attempt;
        component.pending = Some(correlation);
        component.pending_input = input;
        component.pending_at = Some(at);
        Ok(CommandResult::ComponentInvoked(correlation))
    }

    /// Invalidate a failed branch and its fixed M5 dependents before further producer work.
    fn fail_component(&mut self, id: ComponentId, at: Duration) {
        let Some(component) = self.managed.get_mut(&id) else {
            return;
        };
        if let Some(pending) = component.pending
            && let Some(executor) = self.executor.as_mut()
        {
            executor.try_cancel(pending);
        }
        let signal = SignalId::new(
            component.definition.manifest.instrument,
            component.definition.manifest.parameter,
        );
        component.state = ComponentState::Failed;
        component.pending = None;
        component.pending_input = None;
        component.pending_at = None;
        if component.signal.latest().is_some() {
            let _ = component
                .signal
                .invalidate(at, crate::MeasurementFailure::ComponentFailure);
        } else {
            let _ = component.signal.push(Sample::unavailable(
                signal,
                component.definition.manifest.unit,
                at,
                crate::MeasurementFailure::ComponentFailure,
            ));
        }
        if let Some(sample) = component.signal.latest().cloned()
            && sample.at() == at
        {
            self.recording_facts
                .measurement(sample, component.generation, component.revision);
        }
        let dependents: Vec<_> = self
            .managed
            .iter()
            .filter(|(other, instance)| {
                **other != id
                    && instance.definition.manifest.kind
                        == ComponentKind::Transform { input: signal }
            })
            .map(|(other, _)| *other)
            .collect();
        for dependent in &dependents {
            if let Some(other) = self.managed.get_mut(dependent) {
                if let Some(pending) = other.pending
                    && let Some(executor) = self.executor.as_mut()
                {
                    executor.try_cancel(pending);
                }
                other.state = ComponentState::Failed;
                other.pending = None;
                other.pending_input = None;
                other.pending_at = None;
                let downstream = SignalId::new(
                    other.definition.manifest.instrument,
                    other.definition.manifest.parameter,
                );
                if other.signal.latest().is_some() {
                    let _ = other
                        .signal
                        .invalidate(at, crate::MeasurementFailure::ComponentFailure);
                } else {
                    let _ = other.signal.push(Sample::unavailable(
                        downstream,
                        other.definition.manifest.unit,
                        at,
                        crate::MeasurementFailure::ComponentFailure,
                    ));
                }
                if let Some(sample) = other.signal.latest().cloned()
                    && sample.at() == at
                {
                    self.recording_facts
                        .measurement(sample, other.generation, other.revision);
                }
            }
        }
        let affected_signals: Vec<_> = std::iter::once(signal)
            .chain(dependents.iter().filter_map(|dependent| {
                self.managed.get(dependent).map(|instance| {
                    SignalId::new(
                        instance.definition.manifest.instrument,
                        instance.definition.manifest.parameter,
                    )
                })
            }))
            .collect();
        let affected: Vec<_> = self
            .controllers
            .iter()
            .filter(|(_, native)| {
                affected_signals.contains(&native.config.input)
                    && matches!(
                        native.state,
                        ControllerState::Running | ControllerState::Warming
                    )
            })
            .map(|(native, _)| *native)
            .collect();
        for native in affected {
            let mut controller = self.controllers.remove(&native).expect("listed above");
            if controller.state == ControllerState::Warming {
                controller.state = ControllerState::Failed;
            } else {
                let _ = self.fail_controller(&mut controller, at);
            }
            self.controllers.insert(native, controller);
        }
    }

    fn commit_step(&mut self, completion: ComponentCompletion, at: Duration) {
        let id = completion.correlation.component;
        let Some(component) = self.managed.get(&id) else {
            return;
        };
        if component.pending != Some(completion.correlation)
            || component.generation != completion.correlation.generation
            || component.revision != completion.correlation.revision
            || completion.correlation.runtime != self.component_runtime
            || component.state == ComponentState::Failed
        {
            return;
        }
        let captured = component.pending_input;
        let scheduled = component.pending_at;
        let manifest = component.definition.manifest.clone();
        if !completion.timely
            || at < scheduled.unwrap_or(Duration::ZERO)
            || captured.is_some_and(|input| {
                at < input.freshness_at
                    || at - input.freshness_at >= manifest.max_input_age
                    || self
                        .signal(input.signal)
                        .ok()
                        .and_then(SignalBuffer::latest)
                        .is_none_or(|latest| {
                            latest.quality() != SampleQuality::Good
                                || latest.freshness_at() < input.freshness_at
                        })
            })
        {
            self.fail_component(id, at);
            return;
        }
        let Ok(result) = completion.outcome else {
            self.fail_component(id, at);
            return;
        };
        if Self::validate_managed_result(&result, &manifest, InvocationPhase::Step).is_err() {
            self.fail_component(id, at);
            return;
        }
        let component = self.managed.get_mut(&id).expect("checked above");
        let Some(next_revision) = component.revision.checked_add(1) else {
            self.fail_component(id, at);
            return;
        };
        let count = component
            .good_steps
            .saturating_add(1)
            .min(manifest.warmup_samples);
        if result.status == ComponentStatus::Unavailable
            || (result.status == ComponentStatus::Ready && count < manifest.warmup_samples)
            || (component.state == ComponentState::Ready && result.status != ComponentStatus::Ready)
        {
            self.fail_component(id, at);
            return;
        }
        let signal = SignalId::new(manifest.instrument, manifest.parameter);
        let source_at = captured.map_or(
            scheduled.expect("pending job has scheduled time"),
            |input| input.freshness_at,
        );
        let published = if let Some(value) = result.value {
            Sample::derived_good(signal, manifest.unit, at, source_at, Value::Float(value))
        } else {
            Ok(Sample::unavailable(
                signal,
                manifest.unit,
                at,
                crate::MeasurementFailure::ProcessingWarmup,
            ))
        };
        let Ok(sample) = published else {
            self.fail_component(id, at);
            return;
        };
        if component.signal.push(sample.clone()).is_err() {
            self.fail_component(id, at);
            return;
        }
        component.committed = result.state;
        component.revision = next_revision;
        component.good_steps = count;
        component.state = if result.status == ComponentStatus::Ready {
            ComponentState::Ready
        } else {
            ComponentState::Warming
        };
        component.pending = None;
        component.pending_input = None;
        component.pending_at = None;
        component.diagnostics = result.diagnostics;
        component.last_service = Some(at);
        component.last_observation = Some(source_at);
        self.recording_facts
            .measurement(sample, component.generation, component.revision);
    }

    fn poll_components(&mut self, at: Duration) -> Result<(), Error> {
        self.service_safety(at)?;
        if let Some(staged) = &self.staged
            && self
                .executor
                .as_mut()
                .is_some_and(|port| port.try_expire(staged.correlation))
        {
            let staged = self.staged.take().expect("checked above");
            self.executor
                .as_mut()
                .expect("checked above")
                .try_cancel(staged.correlation);
        }
        let expired: Vec<_> = self
            .managed
            .iter()
            .filter_map(|(id, instance)| {
                instance
                    .pending
                    .filter(|correlation| {
                        self.executor
                            .as_mut()
                            .is_some_and(|port| port.try_expire(*correlation))
                    })
                    .map(|_| *id)
            })
            .collect();
        for id in expired {
            self.fail_component(id, at);
        }
        for _ in 0..2 {
            let Some(completion) = self.executor.as_mut().and_then(|port| port.try_poll()) else {
                break;
            };
            if self
                .staged
                .as_ref()
                .is_some_and(|candidate| candidate.correlation == completion.correlation)
            {
                let staged = self.staged.take().expect("checked above");
                if completion.timely
                    && let Ok(result) = completion.outcome
                {
                    let _ = self.commit_staged(staged, result, at);
                }
            } else {
                self.commit_step(completion, at);
            }
        }
        Ok(())
    }

    fn start_or_resume_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
        expected: ControllerState,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        let result = (|| {
            if controller.state != expected {
                return Err(ControllerError::InvalidState.into());
            }
            if self.warming_on(controller.config.output) {
                return Err(OutputError::Busy.into());
            }
            if !self
                .outputs
                .get(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?
                .can_prepare()
            {
                return Err(ControllerError::Output.into());
            }
            if !self
                .outputs
                .get(&controller.config.output)
                .expect("checked above")
                .valid_native_duration(
                    controller.config.max_tick_gap,
                    controller.config.lease_lifetime,
                )
            {
                return Err(ControllerError::InvalidConfiguration.into());
            }
            let (measurement, unit, sample_at) =
                self.control_input(controller.config.input, at, controller.config.max_input_age)?;
            if unit != controller.config.ema.unit {
                return Err(ControllerError::InvalidConfiguration.into());
            }
            let reference = self
                .references
                .get_mut(&controller.config.reference)
                .ok_or(ControllerError::UnknownReference)?
                .value_at(at)
                .map_err(map_reference_error)?;
            if reference.unit != unit {
                return Err(ControllerError::InvalidConfiguration.into());
            }

            controller.reset_algorithms();
            let update = controller
                .ema
                .update(Some(measurement), SampleQuality::Good, unit, sample_at)
                .map_err(|_| ControllerError::Algorithm)?;
            controller.last_tick = Some(at);
            controller.state = ControllerState::Warming;
            if update.status == EmaStatus::Ready {
                self.activate_controller(&mut controller, at)?;
            }
            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
        })();
        self.controllers.insert(id, controller);
        result
    }

    fn tick_controller(&mut self, id: ControllerId, at: Duration) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        if !matches!(
            controller.state,
            ControllerState::Running | ControllerState::Warming
        ) {
            self.controllers.insert(id, controller);
            return Err(ControllerError::InvalidState.into());
        }
        if controller.last_tick.is_some_and(|previous| at <= previous) {
            self.controllers.insert(id, controller);
            return Err(ControllerError::InvalidTickTime.into());
        }
        if controller.state == ControllerState::Warming {
            let result = self.advance_warming(&mut controller, at);
            self.controllers.insert(id, controller);
            return result;
        }

        let update = self.calculate_controller_update(&mut controller, at);
        let result = match update {
            Ok((pid, unit, ttl)) => {
                let lease = controller
                    .lease
                    .expect("Running controller owns its last token");
                let output = controller.config.output;
                let delivery = self.deliver_simulated(
                    output,
                    OutputProposal {
                        lease,
                        value: Value::Float(pid.output),
                        unit,
                        ttl,
                    },
                    at,
                );
                match delivery {
                    Ok(()) => {
                        let renewal = self
                            .outputs
                            .get_mut(&output)
                            .ok_or(Error::from(OutputError::UnknownActuator))
                            .and_then(|authority| {
                                authority.renew_native(lease, controller.config.lease_lifetime, at)
                            });
                        if let Ok(replacement) = renewal {
                            controller.lease = Some(replacement);
                            controller.last_tick = Some(at);
                            controller.latest_output = Some(pid);
                            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
                        } else {
                            let _ = self.fail_controller(&mut controller, at);
                            Err(ControllerError::Output.into())
                        }
                    }
                    Err(_) => {
                        let _ = self.fail_controller(&mut controller, at);
                        Err(ControllerError::Output.into())
                    }
                }
            }
            Err(error) => {
                let _ = self.fail_controller(&mut controller, at);
                Err(error.into())
            }
        };
        self.controllers.insert(id, controller);
        result
    }

    /// Serially acquire the actuator only after all warm-up observations are committed.
    fn activate_controller(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(), Error> {
        let authority = self
            .outputs
            .get_mut(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if !authority.can_prepare() {
            controller.state = ControllerState::Failed;
            return Err(ControllerError::Output.into());
        }
        let acquisition = authority.command(
            OutputCommand::Acquire {
                owner: OutputOwner::Automatic(controller.config.id.get()),
                lifetime: controller.config.lease_lifetime,
            },
            at,
        );
        match acquisition {
            Ok(OutputResult::Lease(lease)) => {
                controller.lease = Some(lease);
                controller.state = ControllerState::Running;
                Ok(())
            }
            Ok(_) => unreachable!("Acquire has one successful result kind"),
            Err(error) => {
                controller.state = ControllerState::Failed;
                Err(error)
            }
        }
    }

    /// A Warming service may observe the same Good attempt once without recounting it.
    fn advance_warming(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        let attempt = (|| {
            let previous = controller.last_tick.ok_or(ControllerError::InvalidState)?;
            if at - previous > controller.config.max_tick_gap {
                return Err(ControllerError::InvalidTickTime);
            }
            let (value, unit, sample_at) =
                self.control_input(controller.config.input, at, controller.config.max_input_age)?;
            if unit != controller.config.ema.unit {
                return Err(ControllerError::InvalidConfiguration);
            }
            let reference = self
                .references
                .get_mut(&controller.config.reference)
                .ok_or(ControllerError::UnknownReference)?
                .value_at(at)
                .map_err(map_reference_error)?;
            if reference.unit != unit {
                return Err(ControllerError::InvalidConfiguration);
            }
            if controller
                .ema
                .snapshot()
                .last_at
                .is_some_and(|last| sample_at <= last)
            {
                controller.last_tick = Some(at);
                return Ok(());
            }
            let updated = controller
                .ema
                .update(Some(value), SampleQuality::Good, unit, sample_at)
                .map_err(|_| ControllerError::Algorithm)?;
            controller.last_tick = Some(at);
            if updated.status == EmaStatus::Ready {
                self.activate_controller(controller, at)
                    .map_err(|_| ControllerError::Output)?;
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => Ok(CommandResult::ControllerUpdated(controller.snapshot())),
            Err(error) => {
                // Preparation never owned the actuator, so its algorithm fault needs no Trip.
                controller.state = ControllerState::Failed;
                controller.lease = None;
                Err(error.into())
            }
        }
    }

    fn calculate_controller_update(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(crate::control::PidUpdate, Unit, Duration), ControllerError> {
        let previous = controller.last_tick.ok_or(ControllerError::InvalidState)?;
        if at - previous > controller.config.max_tick_gap {
            return Err(ControllerError::InvalidTickTime);
        }
        let lease = controller.lease.ok_or(ControllerError::Output)?;
        if at >= lease.expires()
            || self
                .outputs
                .get(&controller.config.output)
                .ok_or(ControllerError::Output)?
                .snapshot()
                .lease
                != Some(lease)
        {
            return Err(ControllerError::Output);
        }
        let (measurement, unit, sample_at) =
            self.control_input(controller.config.input, at, controller.config.max_input_age)?;
        let reference = self
            .references
            .get_mut(&controller.config.reference)
            .ok_or(ControllerError::UnknownReference)?
            .value_at(at)
            .map_err(map_reference_error)?;
        if reference.unit != unit {
            return Err(ControllerError::InvalidConfiguration);
        }
        let filtered = controller
            .ema
            .update(Some(measurement), SampleQuality::Good, unit, sample_at)
            .map_err(|_| ControllerError::Algorithm)?;
        if filtered.status != EmaStatus::Ready {
            return Err(ControllerError::Algorithm);
        }
        let pid = controller
            .pid
            .update(
                filtered.value.ok_or(ControllerError::Algorithm)?,
                reference.value,
                at,
            )
            .map_err(|_| ControllerError::Algorithm)?;
        let remaining_freshness = controller.config.max_input_age - (at - sample_at);
        let ttl = controller
            .config
            .proposal_ttl
            .min(remaining_freshness)
            .min(lease.expires() - at);
        if ttl.is_zero() {
            return Err(ControllerError::StaleInput);
        }
        Ok((pid, self.output_unit(controller.config.output)?, ttl))
    }

    fn pause_controller(&mut self, id: ControllerId, at: Duration) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        let result = (|| {
            if controller.state == ControllerState::Warming {
                controller.state = ControllerState::Paused;
                return Ok(CommandResult::ControllerUpdated(controller.snapshot()));
            }
            if controller.state != ControllerState::Running {
                return Err(ControllerError::InvalidState.into());
            }
            let lease = controller
                .lease
                .take()
                .ok_or(ControllerError::InvalidState)?;
            let authority = self
                .outputs
                .get_mut(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?;
            controller.state = ControllerState::Paused;
            if authority.snapshot().lease == Some(lease) {
                authority.command(OutputCommand::Release(lease), at)?;
            } else if authority.snapshot().lease.is_none()
                && authority.snapshot().state == crate::output::OutputState::SafePending
            {
                // The Runtime watchdog may already have revoked an expired lease.
                // Pausing must still settle the reserved safe action and must not
                // leave the controller claiming authority it no longer owns.
                authority.command(OutputCommand::RequestSafe, at)?;
            } else {
                // A previously revoked controller cannot mutate a subsequent owner.
                return Ok(CommandResult::ControllerUpdated(controller.snapshot()));
            }
            self.complete_simulated_safe(controller.config.output, at)?;
            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
        })();
        self.controllers.insert(id, controller);
        result
    }

    /// A validated new component generation never restarts Failed control implicitly.
    /// Recovery requires prior Rust virtual safe readback and deliberate acknowledgement.
    fn reset_failed_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let controller = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?;
        if controller.state != ControllerState::Failed || controller.lease.is_some() {
            return Err(ControllerError::InvalidState.into());
        }
        if self.warming_on(controller.config.output)
            || !self
                .outputs
                .get(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?
                .can_prepare()
        {
            return Err(ControllerError::Output.into());
        }
        let controller = self.controllers.get_mut(&id).expect("validated above");
        controller.reset_algorithms();
        controller.last_tick = None;
        controller.state = ControllerState::Paused;
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    fn fail_controller(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(), Error> {
        controller.state = ControllerState::Failed;
        let lease = controller.lease.take();
        let authority = self
            .outputs
            .get_mut(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if authority.snapshot().lease == lease && lease.is_some() {
            authority.command(OutputCommand::Trip, at)?;
        } else if authority.snapshot().lease.is_none()
            && authority.snapshot().state == crate::output::OutputState::SafePending
        {
            // A failed loop latches its own fault even if watchdog already requested safe.
            authority.command(OutputCommand::Trip, at)?;
        } else {
            return Ok(());
        }
        self.complete_simulated_safe(controller.config.output, at)
    }

    fn control_input(
        &self,
        signal: SignalId,
        at: Duration,
        max_age: Duration,
    ) -> Result<(f64, Unit, Duration), ControllerError> {
        let sample = self
            .signal(signal)
            .ok()
            .and_then(SignalBuffer::latest)
            .ok_or(ControllerError::InputUnavailable)?;
        if sample.quality() != SampleQuality::Good {
            return Err(ControllerError::InputUnavailable);
        }
        if at < sample.freshness_at() {
            return Err(ControllerError::InvalidTickTime);
        }
        if at - sample.freshness_at() >= max_age {
            return Err(ControllerError::StaleInput);
        }
        let Some(Value::Float(value)) = sample.value() else {
            return Err(ControllerError::InputUnavailable);
        };
        if !value.is_finite() {
            return Err(ControllerError::InputUnavailable);
        }
        Ok((*value, sample.unit(), sample.freshness_at()))
    }

    fn output_unit(&self, actuator: ActuatorId) -> Result<Unit, ControllerError> {
        self.descriptor(actuator.instrument())
            .ok()
            .and_then(|descriptor| descriptor.parameter(actuator.parameter()))
            .map(|parameter| parameter.unit)
            .ok_or(ControllerError::InvalidConfiguration)
    }

    fn capture_reference_fact(&mut self, id: ReferenceId, at: Duration) {
        let Some(snapshot) = self.references.get(&id).map(RuntimeReference::snapshot) else {
            return;
        };
        match snapshot {
            crate::reference::ReferenceSnapshot::Fixed {
                value,
                unit,
                revision,
                ..
            } => self
                .recording_facts
                .reference(crate::recording::ReferenceDetails {
                    reference: id,
                    revision,
                    value,
                    target: None,
                    rate: None,
                    unit,
                    at,
                }),
            crate::reference::ReferenceSnapshot::Ramp {
                state, revision, ..
            } => self
                .recording_facts
                .reference(crate::recording::ReferenceDetails {
                    reference: id,
                    revision,
                    value: state.current,
                    target: Some(state.target),
                    rate: Some(state.rate),
                    unit: state.unit,
                    at,
                }),
        }
    }

    fn capture_controller_fact(&mut self, id: ControllerId, at: Duration) {
        if let Some(controller) = self.controllers.get(&id) {
            self.recording_facts.controller(
                id,
                controller.state,
                controller.config_revision,
                None,
                at,
            );
        }
    }

    fn deliver_simulated(
        &mut self,
        actuator: ActuatorId,
        proposal: OutputProposal,
        at: Duration,
    ) -> Result<(), Error> {
        self.outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .command(OutputCommand::Propose(proposal), at)?;
        self.recording_facts.output(
            actuator,
            crate::recording::OutputStage::Requested,
            self.outputs
                .get(&actuator)
                .and_then(|output| output.snapshot().requested),
            at,
        );
        // A one-call PID can propose and send in the same owner unit. Recheck
        // capture capacity before that unit reaches its ordinary send boundary.
        self.require_recording_open(at)?;
        let dispatch = match self
            .outputs
            .get_mut(&actuator)
            .expect("authority exists after proposal")
            .command(OutputCommand::BeginDispatch, at)?
        {
            OutputResult::Dispatched(dispatch) => dispatch,
            _ => unreachable!("BeginDispatch has one successful result kind"),
        };
        self.recording_facts.output(
            actuator,
            crate::recording::OutputStage::Authorized,
            Some(dispatch.value()),
            at,
        );
        self.recording_facts.output(
            actuator,
            crate::recording::OutputStage::SendStarted,
            Some(dispatch.value()),
            at,
        );
        if let Err(error) = self.apply_virtual_dispatch(actuator, dispatch.value()) {
            self.outputs
                .get_mut(&actuator)
                .expect("authority exists after dispatch")
                .command(
                    OutputCommand::Complete {
                        dispatch_id: dispatch.id(),
                        outcome: DispatchOutcome::Failed,
                    },
                    at,
                )?;
            self.recording_facts.output(
                actuator,
                crate::recording::OutputStage::Failed,
                Some(dispatch.value()),
                at,
            );
            return Err(error);
        }
        self.outputs
            .get_mut(&actuator)
            .expect("authority exists after dispatch")
            .command(
                OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
                at,
            )?;
        self.recording_facts.output_with_source(
            actuator,
            crate::recording::OutputStage::ReadbackVerified,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
        );
        Ok(())
    }

    fn complete_simulated_safe(&mut self, actuator: ActuatorId, at: Duration) -> Result<(), Error> {
        let dispatch = match self
            .outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .command(OutputCommand::BeginDispatch, at)?
        {
            OutputResult::Dispatched(dispatch) => dispatch,
            _ => unreachable!("BeginDispatch has one successful result kind"),
        };
        self.recording_facts.output(
            actuator,
            crate::recording::OutputStage::SafeSendStarted,
            Some(dispatch.value()),
            at,
        );
        self.apply_virtual_dispatch(actuator, dispatch.value())?;
        self.outputs
            .get_mut(&actuator)
            .expect("authority exists after safe dispatch")
            .command(
                OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
                at,
            )?;
        self.recording_facts.output_with_source(
            actuator,
            crate::recording::OutputStage::SafeReadbackVerified,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
        );
        Ok(())
    }

    fn apply_virtual_dispatch(&mut self, actuator: ActuatorId, value: f64) -> Result<(), Error> {
        if let Some(plant) = self.thermal_plants.get_mut(&actuator.instrument())
            && actuator.parameter() == crate::HEATER_POWER
        {
            plant.apply_heater(value)?;
        }
        Ok(())
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
            .or_else(|| {
                self.thermal_plants
                    .get(&id)
                    .map(|instrument| &instrument.descriptor)
            })
            .or_else(|| {
                self.managed
                    .values()
                    .find(|item| item.definition.manifest.instrument == id)
                    .map(|item| &item.descriptor)
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

    /// This explicit service needs host scheduling; a finite lease alone cannot send safe bytes.
    /// It does not run EMA/PID or renew a native owner and never waits on a script VM.
    fn service_safety(&mut self, at: Duration) -> Result<(), Error> {
        self.service_required_recording_deadline(at);
        self.check_output_time(at)?;
        let identities: Vec<_> = self.controllers.keys().copied().collect();
        for id in identities {
            let mut controller = self.controllers.remove(&id).expect("listed above");
            let before_state = controller.state;
            let live = matches!(
                controller.state,
                ControllerState::Warming | ControllerState::Running
            );
            if live {
                let timed_out = controller
                    .last_tick
                    .is_some_and(|last| at > last && at - last > controller.config.max_tick_gap);
                let input_failed = self
                    .control_input(controller.config.input, at, controller.config.max_input_age)
                    .is_err();
                let ownership_lost = controller.state == ControllerState::Running
                    && controller.lease.is_some_and(|lease| {
                        self.outputs
                            .get(&controller.config.output)
                            .is_none_or(|authority| authority.snapshot().lease != Some(lease))
                    });
                if timed_out || input_failed || ownership_lost {
                    if controller.state == ControllerState::Warming {
                        controller.state = ControllerState::Failed;
                    } else {
                        let _ = self.fail_controller(&mut controller, at);
                    }
                }
            }
            if controller.state != before_state {
                self.recording_facts.controller(
                    id,
                    controller.state,
                    controller.config_revision,
                    None,
                    at,
                );
            }
            self.controllers.insert(id, controller);
        }
        // Only the known Rust virtual plant may report simulated readback here.
        // A physical Metakon safe request still needs its trusted transport/evidence path.
        let virtual_safe: Vec<_> = self
            .outputs
            .iter()
            .filter(|(actuator, authority)| {
                self.thermal_plants.contains_key(&actuator.instrument())
                    && authority.snapshot().state == crate::output::OutputState::SafePending
                    && authority.snapshot().in_flight.is_none()
            })
            .map(|(actuator, _)| *actuator)
            .collect();
        for actuator in virtual_safe {
            let _ = self.complete_simulated_safe(actuator, at);
        }
        // Every M3 resource gets one bounded recovery/dispatch opportunity.
        self.poll_transports(at)
    }

    /// Warming is a bounded preparation guard, not an output owner or lease.
    fn warming_on(&self, actuator: ActuatorId) -> bool {
        self.controllers.values().any(|controller| {
            controller.state == ControllerState::Warming && controller.config.output == actuator
        })
    }

    fn cancel_warming(&mut self, actuator: ActuatorId, state: ControllerState) {
        for controller in self.controllers.values_mut() {
            if controller.state == ControllerState::Warming && controller.config.output == actuator
            {
                controller.state = state;
            }
        }
    }

    fn check_transport_time(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.transport_time {
            return Err(TransportError::InvalidTime.into());
        }
        self.transport_time = at;
        Ok(())
    }

    fn poll_transports(&mut self, at: Duration) -> Result<(), Error> {
        self.service_required_recording_deadline(at);
        self.check_transport_time(at)?;
        self.check_output_time(at)?;
        let resources: Vec<_> = self.resources.keys().copied().collect();
        for resource in resources {
            self.service_required_recording_deadline(at);
            let required_open = !self.required_recording.enabled || self.required_recording.open;
            // Temporarily taking the executor lets the coordinator borrow the
            // authority registry during the serialized final-check/write call.
            let mut executor = self
                .resources
                .remove(&resource)
                .ok_or(TransportError::UnknownResource)?;
            let instruments = &self.metakon_instruments;
            let outputs = &mut self.outputs;
            let recording_facts = &mut self.recording_facts;
            let event = executor.poll_authorized(at, &mut |intent, step| {
                if !intent.safe && !required_open {
                    return Err(());
                }
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
                    AuthorizationStep::Started => {
                        let dispatch = authority.begin_transport(intent, at).map_err(|_| ())?;
                        if intent.safe {
                            recording_facts.output(
                                intent.actuator,
                                crate::recording::OutputStage::SafeSendStarted,
                                Some(intent.value),
                                at,
                            );
                        } else {
                            recording_facts.output(
                                intent.actuator,
                                crate::recording::OutputStage::Authorized,
                                Some(intent.value),
                                at,
                            );
                            recording_facts.output(
                                intent.actuator,
                                crate::recording::OutputStage::SendStarted,
                                Some(intent.value),
                                at,
                            );
                        }
                        Ok(Some(dispatch))
                    }
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
                if let Some(instrument) = self.metakon_instruments.get(&pending.instrument)
                    && let Some(descriptor) = instrument.descriptor.parameter(pending.parameter)
                    && let Some(signal) = descriptor.signal
                    && let Some(sample) = instrument
                        .signals
                        .get(&signal)
                        .and_then(|buffer| buffer.latest())
                        .cloned()
                    && sample.at() == at
                {
                    self.recording_facts.measurement(
                        sample,
                        instrument.binding.binding_generation,
                        instrument.binding.mapping_revision,
                    );
                }
            }
            TransportEvent::OutputUncertain { intent, dispatch } => {
                if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                    authority.transport_uncertain(dispatch)?;
                    self.recording_facts.output_with_source(
                        intent.actuator,
                        crate::recording::OutputStage::TransportUncertain,
                        Some(intent.value),
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                    );
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
                            self.recording_facts.output_with_source(
                                intent.actuator,
                                if intent.safe {
                                    crate::recording::OutputStage::SafeAcknowledged
                                } else {
                                    crate::recording::OutputStage::Acknowledged
                                },
                                Some(intent.value),
                                at,
                                crate::recording::OutputEvidenceSource::TransportProtocol,
                            );
                        }
                    } else if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                        authority.transport_uncertain(dispatch)?;
                        self.recording_facts.output_with_source(
                            intent.actuator,
                            crate::recording::OutputStage::TransportUncertain,
                            Some(intent.value),
                            at,
                            crate::recording::OutputEvidenceSource::TransportProtocol,
                        );
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
                    self.recording_facts.output_with_source(
                        intent.actuator,
                        crate::recording::OutputStage::Ambiguous,
                        Some(intent.value),
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                    );
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
                    self.recording_facts.output_with_source(
                        actuator,
                        crate::recording::OutputStage::Ambiguous,
                        authority.snapshot().sent.map(|sent| sent.value),
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                    );
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
        self.instruments.contains_key(&id)
            || self.metakon_instruments.contains_key(&id)
            || self.thermal_plants.contains_key(&id)
            || self
                .managed
                .values()
                .any(|item| item.definition.manifest.instrument == id)
    }

    fn instrument_count(&self) -> usize {
        self.instruments.len()
            + self.metakon_instruments.len()
            + self.thermal_plants.len()
            + self.managed.len()
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
            .or_else(|| {
                self.thermal_plants
                    .get(&id.instrument())
                    .filter(|_| id.parameter() == TEMPERATURE)
                    .map(|instrument| &instrument.signal)
            })
            .or_else(|| {
                self.managed
                    .values()
                    .find(|item| {
                        item.definition.manifest.instrument == id.instrument()
                            && item.definition.manifest.parameter == id.parameter()
                    })
                    .map(|item| &item.signal)
            })
            .ok_or(Error::UnknownSignal(id))
    }
}

fn map_reference_error(error: ReferenceError) -> ControllerError {
    match error {
        ReferenceError::InvalidConfiguration => ControllerError::InvalidConfiguration,
        ReferenceError::InvalidTime => ControllerError::InvalidTickTime,
        ReferenceError::RevisionConflict => ControllerError::RevisionConflict,
        ReferenceError::NotRamp => ControllerError::InvalidConfiguration,
        ReferenceError::RevisionExhausted => ControllerError::RevisionExhausted,
    }
}

#[cfg(test)]
mod managed_identity_tests {
    use super::*;
    use crate::managed::{
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, ComponentResult, ComponentStatus, Correlation, Invocation, PlainData,
    };

    #[derive(Default)]
    struct InitExecutor(Option<ComponentCompletion>);
    impl ComponentExecutor for InitExecutor {
        fn try_submit(&mut self, job: Invocation) -> Result<(), ComponentError> {
            self.0 = Some(ComponentCompletion {
                correlation: job.correlation,
                timely: true,
                outcome: Ok(ComponentResult {
                    status: ComponentStatus::Init,
                    value: None,
                    unit: Unit::CELSIUS,
                    state: PlainData::default(),
                    diagnostics: vec![],
                }),
            });
            Ok(())
        }
        fn try_poll(&mut self) -> Option<ComponentCompletion> {
            self.0.take()
        }
        fn try_cancel(&mut self, _: Correlation) {}
    }

    #[test]
    fn generation_exhaustion_rejects_replacement_without_staging_or_reusing_identity() {
        let id = ComponentId::new(201);
        let candidate = ComponentDefinition {
            manifest: ComponentManifest {
                schema_version: 1,
                id,
                instrument: InstrumentId::new(201),
                name: "model".into(),
                parameter: TEMPERATURE,
                kind: ComponentKind::Source,
                unit: Unit::CELSIUS,
                min: 0.0,
                max: 100.0,
                warmup_samples: 1,
                max_input_age: Duration::from_secs(2),
                history_capacity: 8,
            },
            source: "return function(ctx) return ctx end".into(),
            config: PlainData::default(),
        };
        let mut runtime = Runtime::new();
        runtime
            .install_component_executor(Box::new(InitExecutor::default()))
            .unwrap();
        runtime
            .command(Command::StageComponent {
                definition: candidate.clone(),
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        runtime.managed.get_mut(&id).unwrap().generation = u64::MAX;
        assert_eq!(
            runtime.command(Command::StageComponent {
                definition: candidate,
                replaces: Some(id),
                at: Duration::ZERO,
            }),
            Err(ComponentError::IdentityExhausted.into())
        );
        assert!(runtime.staged.is_none());
        assert_eq!(runtime.managed.get(&id).unwrap().generation, u64::MAX);
    }
}
