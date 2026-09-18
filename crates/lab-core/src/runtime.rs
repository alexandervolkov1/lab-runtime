//! The synchronous command/query boundary and sole owner of experiment state.
//!
//! [`Runtime`] owns all registered instrument/component state, committed signal
//! history, references, controllers, `OutputAuthority` instances, bounded
//! [`ResourceExecutor`]s and the semantic Recorder fact outbox. Commands mutate that
//! state; queries clone bounded committed projections. The host schedules progress,
//! but no host, client, worker, Recorder or adapter becomes a second experiment
//! owner.
//!
//! # Physical acquisition index
//!
//! [`Command::QueueMetakonRead`] encodes a READ with [`crate::metakon`], queues it on
//! a [`ResourceExecutor`], then later handles its `TransportEvent`. A strictly decoded
//! value is validated and committed to the instrument signal buffer before current
//! queries, bounded history, controllers and storage-independent [`crate::recording`]
//! facts can observe it.
//!
//! # Physical output index
//!
//! A controller produces an [`OutputProposal`]. Runtime submits it to its private
//! `OutputAuthority`, freezes a transport intent, and lets [`ResourceExecutor`] ask
//! for the final authority/binding-generation check at the first possible output
//! byte. [`crate::metakon`] encodes the WRITE and validates the strict ACK; Runtime
//! then queues a distinct register readback before settling the dispatch.
//!
//! `requested != authorized != send_started != ACK != readback != physical_effect`.
//! Once a write has started, an ambiguous result is never treated as proof that no
//! write occurred and the original command is not blindly retried.
//!
//! # Implementation map
//!
//! - `dispatch` is the command/query entry point;
//! - `physical_io` owns transaction admission, completion correlation and commits;
//! - `controllers` advances native controllers into output proposals;
//! - `managed_components` validates executor completions before signal commit.
//!
//! These private modules split the implementation, not the owner. Their mutable
//! state remains fields of the one [`Runtime`] below.

mod controllers;
mod dispatch;
mod managed_components;
mod physical_io;

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
    ComponentStatus, Correlation, Invocation, InvocationPhase, MAX_COMPONENTS, PlainData,
};
use crate::metakon::{
    Address, ExpectedRead, MetakonType, MetakonValue, TemperatureReading, decode_ack, decode_read,
    encode_read, encode_scaled_i8, encode_write, scale_temperature,
};
use crate::output::{
    ActuatorId, DispatchOutcome, OutputAuthority, OutputCommand, OutputError, OutputIntent,
    OutputOwner, OutputProposal, OutputResult, OutputSnapshot,
};
use crate::plant::{ThermalPlantConfig, ThermalPlantInstrument};
use crate::processing::EmaStatus;
use crate::reference::{
    ReferenceConfig, ReferenceError, ReferenceId, ReferenceSnapshot, ReferenceValue, RetunedRamp,
    RuntimeReference,
};
use crate::transport::{
    AuthorizationStep, ByteTransport, ExecutorSnapshot, MAX_QUEUED_TRANSACTIONS, ResourceExecutor,
    ResourceId, TransactionId, TransactionOutcome, TransportError, TransportEvent,
    TransportShutdown,
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
    /// Publish one Good or explicitly Unavailable observation into a virtual source.
    PublishVirtualMeasurement {
        /// Stable virtual instrument identity.
        instrument: InstrumentId,
        /// Stable measurement parameter identity.
        parameter: ParameterId,
        /// Good scalar, or `None` for virtual Unavailable/disabled.
        value: Option<Value>,
        /// Runtime generation observed by the caller; clients cannot assign it.
        expected_generation: u64,
        /// Authoritative Runtime receipt/commit time.
        at: Duration,
    },
    /// Register a deterministic first-order virtual thermal plant.
    RegisterThermalPlant(ThermalPlantConfig),
    /// Replace one stopped native model and fence every observation from its old generation.
    RestartThermalPlant {
        /// Existing stable logical instrument identity.
        instrument: InstrumentId,
        /// Complete validated replacement model configuration with the same identity.
        config: ThermalPlantConfig,
        /// Generation observed before staging the restart.
        expected_generation: u64,
        /// Monotonic commit time used for recording and dependency fencing.
        at: Duration,
    },
    /// Prepare one replacement init without mutating its committed generation.
    PrepareComponentReplacement {
        /// Complete immutable candidate definition.
        definition: ComponentDefinition,
        /// Existing component identity retained by the replacement.
        replaces: ComponentId,
        /// Trusted monotonic preparation time.
        at: Duration,
    },
    /// Atomically commit a complete bounded set of prepared replacements.
    CommitPreparedComponents {
        /// Distinct prepared identities, at most the accepted component bound.
        components: Vec<ComponentId>,
        /// Monotonic safe-barrier/commit time.
        at: Duration,
    },
    /// Discard every prepared replacement without touching committed components.
    DiscardPreparedComponents,
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
    /// Replace one complete Reference policy under a checked revision fence.
    ReconfigureReference {
        /// Existing Reference identity retained by the replacement.
        reference: ReferenceId,
        /// Complete finite Fixed or Ramp candidate.
        config: ReferenceConfig,
        /// Revision observed before staging the candidate.
        expected_revision: u64,
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
    /// Replace the complete data-only policy of a stopped controller atomically.
    ReconfigureController {
        /// Existing controller identity retained by the replacement.
        controller: ControllerId,
        /// Complete validated configuration with unchanged graph bindings.
        config: NativeControllerConfig,
        /// Revision read before staging the replacement.
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
    /// Atomically replace one stopped read-only Metakon definition and binding.
    ReconfigureMetakon {
        /// Complete validated candidate with the same logical instrument ID.
        config: MetakonInstrumentConfig,
        /// Binding generation observed before the safe/rebind barrier.
        expected_binding_generation: u64,
        /// Mapping revision observed before the safe/rebind barrier.
        expected_mapping_revision: u64,
        /// Nondecreasing trusted monotonic commit time.
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
    /// Candidate init completed and remains outside the active graph.
    ComponentPrepared(ComponentId),
    /// All named prepared generations became active in one owner command.
    ComponentsCommitted(usize),
    /// Prepared candidates were discarded without active mutation.
    PreparedComponentsDiscarded,
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
    /// One externally supplied virtual observation committed through the normal signal path.
    VirtualMeasurementPublished(Sample),
    /// A native virtual model was reset under a checked generation fence.
    ModelRestarted {
        /// Stable logical instrument identity.
        instrument: InstrumentId,
        /// New nonzero model generation.
        generation: u64,
    },
    /// A native Reference was registered without evaluating it.
    ReferenceRegistered(ReferenceId),
    /// One independent Reference evaluation, not a query or a controller tick.
    ReferenceEvaluated(ReferenceValue),
    /// Full committed continuous Ramp state and advanced configuration revision.
    ReferenceRetuned(RetunedRamp),
    /// Complete Reference policy replacement committed without evaluating it.
    ReferenceConfigured(ReferenceSnapshot),
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
    /// Copy a committed component snapshot without evaluating its implementation.
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
    /// Current replacement or binding generation for all observations of this instance.
    pub generation: u64,
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

/// Sole synchronous owner of mutable experiment/domain state.
///
/// A `Runtime` owns instruments, committed observations, References, controllers,
/// output authorities, resource executors, managed-component lifecycle and the
/// semantic Recorder outbox. It advances only through [`Command`] and exposes only
/// owned committed snapshots through [`Query`]. Host scheduling, OS adapters,
/// SQLite, NDJSON sessions and wall-clock projection live outside this type.
///
/// The struct has no interior mutability or background thread: callers must
/// serialize access, which keeps every authority transition in one explicit owner.
#[derive(Default)]
pub struct Runtime {
    managed: BTreeMap<ComponentId, ManagedInstance>,
    staged: Option<StagedComponent>,
    prepared: BTreeMap<ComponentId, PreparedComponent>,
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
    pending_output_writes: BTreeMap<(ResourceId, TransactionId), Duration>,
    pending_output_readbacks: BTreeMap<(ResourceId, TransactionId), PendingOutputReadback>,
    unsettled_outputs: BTreeMap<ResourceId, (OutputIntent, crate::output::DispatchId)>,
    output_time: Duration,
    transport_time: Duration,
    recording_facts: crate::recording::FactOutbox,
    next_output_attempt: u64,
    output_attempts: BTreeMap<ActuatorId, u64>,
    // One current virtual dispatch per binding. M3 intents carry immutable
    // attempt correlation through their own bounded transport lifecycle.
    output_dispatch_attempts: BTreeMap<ActuatorId, (crate::output::DispatchId, u64)>,
    required_recording: crate::recording::RequiredGate,
}

#[derive(Clone)]
struct PendingRead {
    instrument: InstrumentId,
    parameter: ParameterId,
    expected: ExpectedRead,
    operation: KnownOperation,
    scale: f64,
    binding_generation: u64,
    mapping_revision: u64,
    failure_published: bool,
}

#[derive(Clone, Copy)]
struct PendingOutputReadback {
    intent: OutputIntent,
    dispatch: crate::output::DispatchId,
    expected: ExpectedRead,
    scale: f64,
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

#[derive(Clone)]
struct StagedComponent {
    definition: ComponentDefinition,
    replaces: Option<ComponentId>,
    correlation: Correlation,
    prepare_only: bool,
}

#[derive(Clone)]
struct PreparedComponent {
    staged: StagedComponent,
    result: crate::managed::ComponentResult,
}

impl ManagedInstance {
    fn snapshot(&self) -> ComponentSnapshot {
        ComponentSnapshot {
            id: self.definition.manifest.id,
            instrument: self.definition.manifest.instrument,
            implementation: self.definition.implementation.id().clone(),
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
        let actuators: Vec<_> = self.outputs.keys().copied().collect();
        for actuator in actuators {
            self.sync_recording_output_context(actuator);
        }
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

    /// Whether Required ordinary admission currently has durable progress credit.
    pub fn required_recording_open(&self) -> bool {
        self.required_recording.enabled && self.required_recording.open
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
        self.register_transport_with_recovery_timeout(
            id,
            adapter,
            crate::transport::MAX_TRANSACTION_DURATION,
        )
    }

    /// Install one bounded byte adapter with an explicit monotonic recovery bound.
    pub fn register_transport_with_recovery_timeout(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
        recovery_timeout: Duration,
    ) -> Result<(), Error> {
        if self.resources.contains_key(&id) {
            return Err(TransportError::DuplicateResource.into());
        }
        if self.resources.len() >= MAX_TRANSPORT_RESOURCES {
            return Err(TransportError::ResourceLimit.into());
        }
        let executor = ResourceExecutor::with_recovery_timeout(id, adapter, recovery_timeout)?;
        self.resources.insert(id, executor);
        Ok(())
    }

    /// Replace a closed adapter while retaining the stable logical resource ID.
    ///
    /// The old executor first clears queued work and must report finite shutdown
    /// completion. Pending/in-flight ownership returns `ResourceBusy`; the new
    /// adapter is not installed and no second owner is created.
    pub fn replace_transport(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
    ) -> Result<(), Error> {
        let recovery_timeout = self
            .resources
            .get(&id)
            .ok_or(TransportError::UnknownResource)?
            .recovery_timeout();
        self.replace_transport_with_recovery_timeout(id, adapter, recovery_timeout)
    }

    /// Replace a closed adapter and atomically install its new recovery policy.
    pub fn replace_transport_with_recovery_timeout(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
        recovery_timeout: Duration,
    ) -> Result<(), Error> {
        if self.shutdown_transport(id, self.transport_time)? != TransportShutdown::Complete {
            return Err(TransportError::ResourceBusy.into());
        }
        let replacement = ResourceExecutor::with_recovery_timeout(id, adapter, recovery_timeout)?;
        self.resources.insert(id, replacement);
        Ok(())
    }

    /// Copy the committed physical binding used by one Metakon instrument.
    ///
    /// This is provenance-only metadata: it carries no transport handle, does
    /// not poll hardware, and cannot authorize an output. The host freezes the
    /// returned generation and mapping revision before storage work begins.
    pub fn metakon_binding(&self, instrument: InstrumentId) -> Option<MetakonBinding> {
        self.metakon_instruments
            .get(&instrument)
            .map(|instance| instance.binding)
    }

    /// Inspect unfinished worker count without waiting for interpreter execution.
    pub fn unfinished_component_workers(&self) -> usize {
        self.executor
            .as_ref()
            .map_or(0, |executor| executor.unfinished_workers())
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
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, ComponentResult,
        ComponentStatus, Correlation, Invocation, PlainData,
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
            implementation: ComponentImplementation::built_in("test.fake.v1").unwrap(),
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

#[cfg(test)]
mod safe_recording_tests {
    use super::*;
    use crate::output::{EvidenceLevel, OutputState, SafeProfile};

    #[test]
    fn exhausted_output_attempt_counter_marks_a_gap_but_cannot_block_safe_dispatch() {
        let mut runtime = Runtime::new();
        let instrument = InstrumentId::new(901);
        runtime
            .command(Command::RegisterVirtual(VirtualInstrumentConfig {
                id: instrument,
                name: "safe counter exhaustion".into(),
                history_capacity: 1,
                base_temperature: 20.0,
                measurement_enabled: true,
            }))
            .unwrap();
        let actuator = ActuatorId::new(instrument, crate::HEATER_POWER);
        runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BindProfile(SafeProfile {
                    min: 0.0,
                    max: 100.0,
                    safe_value: 0.0,
                    max_lease: Duration::from_secs(1),
                    max_proposal_ttl: Duration::from_millis(100),
                    required_evidence: EvidenceLevel::Readback,
                }),
            })
            .unwrap();
        runtime.enable_recording_facts();
        runtime.next_output_attempt = u64::MAX;
        assert!(
            runtime
                .command(Command::Output {
                    actuator,
                    at: Duration::ZERO,
                    command: OutputCommand::RequestSafe,
                })
                .is_ok()
        );
        assert!(runtime.recording_facts_overflowed());
        assert_eq!(runtime.recording_facts.first_lost_sequence(), Some(1));
        assert_eq!(
            runtime.outputs.get(&actuator).unwrap().snapshot().state,
            OutputState::SafePending
        );
        let CommandResult::Output(OutputResult::Dispatched(safe)) = runtime
            .command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BeginDispatch,
            })
            .unwrap()
        else {
            panic!("safe dispatch was blocked")
        };
        assert!(safe.is_safe());
        assert!(
            runtime.take_recording_facts().is_empty(),
            "failed audit cannot fabricate safe evidence"
        );
    }
}
