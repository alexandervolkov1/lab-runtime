//! Serialized Runtime orchestration, independent of client lifetime.
//!
//! [`crate::host::HostCore`] owns exactly one [`lab_core::Runtime`] plus schedules, derived event state,
//! Recorder ingress/lifecycle and configured adapter catalogs. Runtime remains the
//! authoritative experiment owner; the host stores only orchestration, fencing and
//! delivery state around it.
//!
//! One bounded service turn checks work in this order: Recorder receipts, safety and
//! transport progress, stopping/configuration fences, thermal models, Metakon read
//! admission, references, controllers, periodic managed sources and
//! observation-driven transforms. Every lower-priority section yields when safety
//! becomes due. A skipped periodic slot receives one actual-time opportunity, never
//! a replay at an old deadline.
//!
//! Physical acquisition enters Runtime through `Command::QueueMetakonRead`; committed
//! samples then feed current/history projections, events, controllers and semantic
//! Recorder facts. Physical output returns through Runtime's OutputAuthority and
//! resource executor rather than through a host-side write bypass.
//!
//! # Implementation map
//!
//! - `instruments` composes configured instrument kinds and their schedules;
//! - `scheduler` preserves the safety-first owner-turn order described above;
//! - `components` connects the bounded native executor to Runtime;
//! - `recording` admits semantic facts and lifecycle work to Recorder;
//! - `configuration` applies validated live/rebind changes;
//! - `lifecycle` owns commands, shutdown and adapter retirement.

mod components;
mod configuration;
mod instruments;
mod lifecycle;
mod recording;
mod scheduler;

use crate::recorder::{
    AnnotationRecord, BoundarySnapshot, ConfigurationLifecycleRecord, HistoryCursor, HistoryFilter,
    HistoryPage, OperationRecord, ProvenanceEntry, ProvenanceObject, RecorderGap, RecorderWorker,
    RecordingPolicy, RecordingState, RecordingStatus, RunsCursor, RunsPage, StorageError,
};
use crate::{
    build_identity::runtime_binary_sha256,
    configuration::{EvidenceDto, FrozenDeployment, InstrumentDto, ReferenceKindDto},
    definition::parse_definition_json,
    events::{EventError, EventLog},
    managed_executor::{
        MOVING_MEAN_IMPLEMENTATION, NativeComponentDefinition, build_component_definition,
    },
};
use lab_core::{
    AccessMode, Command, CommandResult, Error, InstrumentId, MeasurementFailure,
    ParameterDescriptor, ParameterRole, Query, QueryResult, Runtime, Sample, SampleQuality,
    SignalId, Unit, Value, ValueSpec, WriteEffect,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    instrument::{KnownOperation, MetakonBinding, MetakonInstrumentConfig},
    managed::{
        ComponentError, ComponentExecutor, ComponentId, ComponentState, PlainData, PlainValue,
    },
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId, ReferenceSnapshot},
    transport::{ByteTransport, ExecutorState, ResourceId, TransactionOutcome},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

const PLANT: InstrumentId = InstrumentId::new(1);
const REFERENCE: ReferenceId = ReferenceId::new(1);
const CONTROLLER: ControllerId = ControllerId::new(1);
const MANAGED_FILTER: ComponentId = ComponentId::new(202);
const DEPENDENT_PLANT: InstrumentId = InstrumentId::new(301);
const DEPENDENT_CONTROLLER: ControllerId = ControllerId::new(2);
const DEPENDENT_REFERENCE: ReferenceId = ReferenceId::new(2);

/// Trusted elapsed-time source; production and deterministic tests share the interface.
pub trait Clock {
    /// Return nondecreasing elapsed monotonic Runtime time.
    fn now(&self) -> Duration;
}

fn parameter_activation_json(parameter: &ParameterDescriptor) -> serde_json::Value {
    let value_spec = match &parameter.value_spec {
        ValueSpec::Float { min, max } => serde_json::json!({"kind":"float","min":min,"max":max}),
        ValueSpec::Integer { min, max } => {
            serde_json::json!({"kind":"integer","min":min,"max":max})
        }
        ValueSpec::Boolean => serde_json::json!({"kind":"boolean"}),
        ValueSpec::Text { max_bytes } => serde_json::json!({"kind":"text","max_bytes":max_bytes}),
        ValueSpec::Enum { choices } => serde_json::json!({"kind":"enum","choices":choices}),
    };
    serde_json::json!({"id":parameter.id.get().to_string(),"name":parameter.name,
        "value_spec":value_spec,"unit":{"id":parameter.unit.id(),
            "symbol":parameter.unit.symbol()},
        "access":match parameter.access {AccessMode::ReadOnly=>"read_only",
            AccessMode::ReadWrite=>"read_write",AccessMode::WriteOnly=>"write_only"},
        "role":match parameter.role {ParameterRole::Measurement=>"measurement",
            ParameterRole::Configuration=>"configuration",ParameterRole::Actuator=>"actuator",
            ParameterRole::Action=>"action",ParameterRole::Diagnostic=>"diagnostic"},
        "write_effect":match parameter.write_effect {WriteEffect::None=>"none",
            WriteEffect::ConfigurationOnly=>"configuration_only",
            WriteEffect::OutputAffecting=>"output_affecting"}})
}

fn plain_data_activation_json(data: &PlainData) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    for (key, value) in &data.fields {
        fields.insert(
            key.clone(),
            match value {
                PlainValue::Number(number) => serde_json::json!({"kind":"number","value":number}),
                PlainValue::Boolean(value) => serde_json::json!({"kind":"boolean","value":value}),
                PlainValue::Text(value) => serde_json::json!({"kind":"text","value":value}),
                PlainValue::Numbers(values) => serde_json::json!({"kind":"numbers","value":values}),
            },
        );
    }
    serde_json::Value::Object(fields)
}

fn hex_sha256(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sample_boundary_json(sample: &Sample) -> serde_json::Value {
    let value = match sample.value() {
        Some(lab_core::Value::Float(value)) => serde_json::json!({"kind":"float","value":value}),
        Some(lab_core::Value::Integer(value)) => {
            serde_json::json!({"kind":"integer","value":value})
        }
        Some(lab_core::Value::Boolean(value)) => {
            serde_json::json!({"kind":"boolean","value":value})
        }
        Some(lab_core::Value::Text(value)) => serde_json::json!({"kind":"text","value":value}),
        Some(lab_core::Value::Enum(value)) => serde_json::json!({"kind":"enum","value":value}),
        None => serde_json::Value::Null,
    };
    let failure = sample.failure().map(|reason| match reason {
        MeasurementFailure::Disabled => "disabled",
        MeasurementFailure::SensorFault => "sensor_fault",
        MeasurementFailure::Transport => "transport",
        MeasurementFailure::ProcessingWarmup => "processing_warmup",
        MeasurementFailure::ComponentFailure => "component_failure",
    });
    serde_json::json!({"instrument":sample.signal().instrument().get().to_string(),
        "parameter":sample.signal().parameter().get().to_string(),
        "unit_id":sample.unit().id(),"published_at_ns":sample.at().as_nanos().to_string(),
        "observed_at_ns":sample.freshness_at().as_nanos().to_string(),
        "quality":match sample.quality(){SampleQuality::Good=>"good",
            SampleQuality::Unavailable=>"unavailable"},
        "failure":failure,"value":value})
}

/// Production clock with one Instant origin, never wall time.
#[derive(Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}
impl SystemClock {
    /// Create one process-local monotonic origin before host startup.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}
impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Clone)]
struct Periodic {
    next_due: Duration,
    period: Duration,
}
impl Periodic {
    fn new(period: Duration) -> Self {
        Self {
            next_due: Duration::ZERO,
            period,
        }
    }
    fn due(&self, now: Duration) -> bool {
        now >= self.next_due
    }
    fn take(&mut self, now: Duration) -> Result<Option<u64>, Error> {
        if !self.due(now) {
            return Ok(None);
        }
        let missed = (now - self.next_due).as_nanos() / self.period.as_nanos();
        let missed: u64 = missed
            .try_into()
            .map_err(|_| Error::InvalidConfiguration("scheduler count exhausted"))?;
        let slots = missed
            .checked_add(1)
            .ok_or(Error::InvalidConfiguration("scheduler count exhausted"))?;
        let nanos = self
            .period
            .as_nanos()
            .checked_mul(u128::from(slots))
            .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
        let seconds: u64 = (nanos / 1_000_000_000)
            .try_into()
            .map_err(|_| Error::InvalidConfiguration("scheduler time exhausted"))?;
        let advance = Duration::new(seconds, (nanos % 1_000_000_000) as u32);
        self.next_due = self
            .next_due
            .checked_add(advance)
            .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
        Ok(Some(missed))
    }
}

/// Locally trusted schedule bindings for one bounded virtual deployment.
pub struct SchedulePlan {
    safety: Periodic,
    plants: Vec<(InstrumentId, Periodic)>,
    metakon_reads: Vec<MetakonReadSchedule>,
    references: Vec<(ReferenceId, Periodic)>,
    controllers: Vec<(ControllerId, SignalId, Periodic)>,
    sources: Vec<(ComponentId, Periodic)>,
    transforms: Vec<(ComponentId, SignalId)>,
}

struct MetakonReadSchedule {
    instrument: InstrumentId,
    parameter: lab_core::ParameterId,
    slot: Periodic,
    queue_ttl: Duration,
    timeout: Duration,
}

struct ConfiguredProbe {
    instrument: InstrumentId,
    parameter: lab_core::ParameterId,
    queue_ttl: Duration,
    timeout: Duration,
    queued: bool,
    baseline: Option<Sample>,
}

fn thermal_configuration_changed(old: &InstrumentDto, new: &InstrumentDto) -> bool {
    match (old, new) {
        (
            InstrumentDto::ThermalPlant {
                history_capacity: old_history,
                ambient_temperature: old_ambient,
                initial_temperature: old_initial,
                gain_per_percent: old_gain,
                time_constant_ms: old_time_constant,
                ..
            },
            InstrumentDto::ThermalPlant {
                history_capacity: new_history,
                ambient_temperature: new_ambient,
                initial_temperature: new_initial,
                gain_per_percent: new_gain,
                time_constant_ms: new_time_constant,
                ..
            },
        ) => {
            old_history != new_history
                || old_ambient != new_ambient
                || old_initial != new_initial
                || old_gain != new_gain
                || old_time_constant != new_time_constant
        }
        _ => false,
    }
}

impl SchedulePlan {
    fn virtual_demo() -> Self {
        Self {
            safety: Periodic::new(Duration::from_millis(10)),
            plants: vec![(PLANT, Periodic::new(Duration::from_millis(100)))],
            metakon_reads: Vec::new(),
            references: vec![(REFERENCE, Periodic::new(Duration::from_millis(100)))],
            controllers: vec![(
                CONTROLLER,
                SignalId::new(PLANT, lab_core::TEMPERATURE),
                Periodic::new(Duration::from_millis(100)),
            )],
            sources: Vec::new(),
            transforms: Vec::new(),
        }
    }

    /// Trusted fixture with native service but no automatic measurement refresh.
    /// This can demonstrate that repeated input cannot manufacture PID progress.
    pub fn without_measurements(
        _plant: InstrumentId,
        reference: ReferenceId,
        controller: ControllerId,
        input: SignalId,
    ) -> Self {
        Self {
            safety: Periodic::new(Duration::from_millis(10)),
            plants: Vec::new(),
            metakon_reads: Vec::new(),
            references: vec![(reference, Periodic::new(Duration::from_millis(100)))],
            controllers: vec![(controller, input, Periodic::new(Duration::from_millis(100)))],
            sources: Vec::new(),
            transforms: Vec::new(),
        }
    }
}

/// Bounded service opportunities taken during one owner turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServiceReport {
    /// Combined safety/deadline/transport service calls.
    pub safety: u64,
    /// Actual-time plant refreshes, not missed historical slots.
    pub measurements: u64,
    /// Independently evaluated References.
    pub references: u64,
    /// Distinct-input native controller attempts.
    pub controller_ticks: u64,
    /// Deadlines skipped/coalesced while keeping prior-deadline phase.
    pub skipped_deadlines: u64,
}

/// Honest in-process shutdown evidence; unfinished workers prevent a clean exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShutdownStatus {
    /// Every configured output has no lease and obtained its required safe evidence.
    pub safe_confirmed: bool,
    /// Fixed executor slots still executing; Rust safe work never waits for them.
    pub unfinished_workers: usize,
    /// Physical byte resources whose adapter/worker has not confirmed close.
    pub unfinished_transports: usize,
    /// Every configured byte resource confirmed finite retirement.
    pub transports_closed: bool,
    /// A fatal owner/identity fault occurred even if virtual safe evidence remains.
    pub fatal_error: bool,
    /// Every admitted recording fact and terminal seal was confirmed before close.
    pub recorder_flushed: bool,
    /// A Recorder worker remained open at the finite flush deadline.
    pub recorder_unfinished: bool,
    /// Sticky Recorder failure prevents claiming durable completion.
    pub recorder_error: bool,
    /// Safe evidence and worker cleanup both finished.
    pub exit_success: bool,
}

// Identity and at most one bounded Recorder payload survive in each fixed slot.
// A safe-reducing Pause terminal may wait here for ordinary ingress credit; no
// authority-increasing mutation uses this postcommit deferral.
struct PendingOperation {
    command: &'static str,
    accepted_at: Duration,
    accepted_recorded: bool,
    deferred_start: Option<OperationRecord>,
    deferred_safe_terminal: Option<OperationRecord>,
}

/// Serialized host composition around the sole mutable Core [`Runtime`] owner.
///
/// `HostCore` retains schedules, adapter catalogs, transient event projection and
/// Recorder admission state needed to progress Runtime. Those fields coordinate
/// work; they do not duplicate authoritative instrument, controller,
/// OutputAuthority or measurement state. [`HostCore::service`] is the single
/// monotonic owner turn and never delegates ownership to an API client or worker.
pub struct HostCore {
    runtime: Runtime,
    events: EventLog,
    plan: SchedulePlan,
    consumed: BTreeMap<ControllerId, Option<Sample>>,
    consumed_managed: BTreeMap<ComponentId, Option<Sample>>,
    components: Vec<(ComponentId, &'static str)>,
    outputs: Vec<ActuatorId>,
    active_safety_profiles: Vec<(ActuatorId, SafeProfile)>,
    resources: Vec<ResourceId>,
    closed_resources: BTreeSet<ResourceId>,
    last_now: Duration,
    stopping: bool,
    recorder: Option<RecorderWorker>,
    recording_policy: Option<RecordingPolicy>,
    recording_status: Option<RecordingStatus>,
    last_recording_submission: Option<Duration>,
    recorder_finish_requested: bool,
    pending_operations: BTreeMap<(String, u64), PendingOperation>,
    deployment_provenance: Vec<ProvenanceEntry>,
    virtual_model_generations: BTreeMap<InstrumentId, u64>,
    emulator_targets: BTreeSet<SignalId>,
    physical_instruments: BTreeSet<InstrumentId>,
    configured_probes: Vec<ConfiguredProbe>,
    configuration_quiesced: bool,
    reconnect_quiesced_resources: BTreeSet<ResourceId>,
}

impl HostCore {
    /// Construct a validated configured native observation graph without opening
    /// storage, listeners or physical resources.
    pub fn configured_native(deployment: &FrozenDeployment) -> Result<Self, Error> {
        Self::configured_with_transports(deployment, BTreeMap::new())
    }

    /// Construct a configured graph with exactly one adapter for every declared
    /// resource. Adapters are created outside Core and remain exclusively owned
    /// by its existing bounded M3 resource executor.
    pub fn configured_with_transports(
        deployment: &FrozenDeployment,
        mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>>,
    ) -> Result<Self, Error> {
        let dto = &deployment.effective().dto;
        let mut runtime = Runtime::new();
        let mut resources = Vec::with_capacity(dto.resources.len());
        for resource in &dto.resources {
            let id = ResourceId::new(resource.id);
            let adapter = transports
                .remove(&id)
                .ok_or(Error::InvalidConfiguration("configured transport missing"))?;
            runtime.register_transport_with_recovery_timeout(
                id,
                adapter,
                Duration::from_millis(resource.recovery_timeout_ms),
            )?;
            resources.push(id);
        }
        if !transports.is_empty() {
            return Err(Error::InvalidConfiguration(
                "undeclared configured transport",
            ));
        }
        let instruments = instruments::register_configured_instruments(&mut runtime, deployment)?;
        let measurements = instruments.measurements;
        let metakon_reads = instruments.metakon_reads;
        let virtual_model_generations = instruments.virtual_model_generations;
        let emulator_targets = instruments.emulator_targets;
        let physical_instruments = instruments.physical_instruments;
        let configured_probes = instruments.configured_probes;
        let mut outputs = Vec::with_capacity(dto.safe_profiles.len());
        let mut active_safety_profiles = Vec::with_capacity(dto.safe_profiles.len());
        for safe in &dto.safe_profiles {
            let actuator = ActuatorId::new(
                InstrumentId::new(safe.instrument_id),
                lab_core::ParameterId::new(safe.parameter_id),
            );
            let profile = SafeProfile {
                min: safe.min,
                max: safe.max,
                safe_value: safe.safe_value,
                max_lease: Duration::from_millis(safe.max_lease_ms),
                max_proposal_ttl: Duration::from_millis(safe.max_proposal_ttl_ms),
                required_evidence: match safe.required_evidence {
                    EvidenceDto::Ack => EvidenceLevel::Acknowledgement,
                    EvidenceDto::Readback => EvidenceLevel::Readback,
                },
            };
            runtime.command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BindProfile(profile.clone()),
            })?;
            if runtime.metakon_binding(actuator.instrument()).is_none() {
                runtime.command(Command::Output {
                    actuator,
                    at: Duration::ZERO,
                    command: OutputCommand::RequestSafe,
                })?;
                let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
                    runtime.command(Command::Output {
                        actuator,
                        at: Duration::ZERO,
                        command: OutputCommand::BeginDispatch,
                    })?
                else {
                    return Err(Error::InvalidConfiguration(
                        "configured safe dispatch unavailable",
                    ));
                };
                runtime.command(Command::Output {
                    actuator,
                    at: Duration::ZERO,
                    command: OutputCommand::Complete {
                        dispatch_id: dispatch.id(),
                        outcome: DispatchOutcome::ReadbackVerified,
                    },
                })?;
            }
            outputs.push(actuator);
            active_safety_profiles.push((actuator, profile));
        }
        let mut references = Vec::with_capacity(dto.references.len());
        for reference in &dto.references {
            let id = ReferenceId::new(reference.id);
            let unit = Unit::new(&reference.unit_id, &reference.unit_symbol)?;
            runtime.command(Command::RegisterReference(match reference.kind {
                ReferenceKindDto::Fixed => ReferenceConfig::Fixed {
                    id,
                    value: reference.value,
                    unit,
                },
                ReferenceKindDto::Ramp => ReferenceConfig::Ramp {
                    id,
                    start: reference.value,
                    target: reference
                        .target
                        .ok_or(Error::InvalidConfiguration("ramp target missing"))?,
                    rate: reference
                        .rate
                        .ok_or(Error::InvalidConfiguration("ramp rate missing"))?,
                    unit,
                    at: Duration::ZERO,
                },
            }))?;
            // Reference progression is independent of controller execution. The
            // schema-v1 fixed 100 ms cadence is deterministic and bounded.
            references.push((id, Periodic::new(Duration::from_millis(100))));
        }
        let mut controllers = Vec::with_capacity(dto.controllers.len());
        for controller in &dto.controllers {
            let id = ControllerId::new(controller.id);
            let input = SignalId::new(
                InstrumentId::new(controller.input_instrument_id),
                lab_core::ParameterId::new(controller.input_parameter_id),
            );
            runtime.command(Command::RegisterController(NativeControllerConfig {
                id,
                input,
                output: ActuatorId::new(
                    InstrumentId::new(controller.output_instrument_id),
                    lab_core::ParameterId::new(controller.output_parameter_id),
                ),
                reference: ReferenceId::new(controller.reference_id),
                ema: EmaConfig {
                    time_constant: Duration::from_millis(controller.ema_time_constant_ms),
                    warmup_samples: controller.ema_warmup_samples,
                    unit: match runtime
                        .query(Query::Reference(ReferenceId::new(controller.reference_id)))?
                    {
                        QueryResult::Reference(ReferenceSnapshot::Fixed { unit, .. }) => unit,
                        QueryResult::Reference(ReferenceSnapshot::Ramp { state, .. }) => state.unit,
                        _ => {
                            return Err(Error::InvalidConfiguration(
                                "configured Reference missing",
                            ));
                        }
                    },
                },
                pid: PidConfig {
                    kp: controller.kp,
                    ki: controller.ki,
                    kd: controller.kd,
                    output_min: controller.output_min,
                    output_max: controller.output_max,
                },
                max_input_age: Duration::from_millis(controller.max_input_age_ms),
                max_tick_gap: Duration::from_millis(controller.max_tick_gap_ms),
                lease_lifetime: Duration::from_millis(controller.lease_lifetime_ms),
                proposal_ttl: Duration::from_millis(controller.proposal_ttl_ms),
            }))?;
            if runtime
                .metakon_binding(InstrumentId::new(controller.output_instrument_id))
                .is_none()
            {
                runtime.command(Command::PrepareController(id))?;
            }
            controllers.push((
                id,
                input,
                Periodic::new(Duration::from_millis(controller.period_ms)),
            ));
        }
        let controller_ids: Vec<_> = controllers.iter().map(|(id, _, _)| *id).collect();
        let reference_ids: Vec<_> = references.iter().map(|(id, _)| *id).collect();
        let events = EventLog::new(
            &runtime,
            &controller_ids,
            &reference_ids,
            "00000000000000000000000000000000",
        );
        let deployment_provenance = deployment
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(Self {
            runtime,
            events,
            plan: SchedulePlan {
                safety: Periodic::new(Duration::from_millis(10)),
                plants: measurements,
                metakon_reads,
                references,
                controllers,
                sources: Vec::new(),
                transforms: Vec::new(),
            },
            consumed: BTreeMap::new(),
            consumed_managed: BTreeMap::new(),
            components: Vec::new(),
            outputs,
            active_safety_profiles,
            resources,
            closed_resources: BTreeSet::new(),
            last_now: Duration::ZERO,
            stopping: false,
            recorder: None,
            recording_policy: None,
            recording_status: None,
            last_recording_submission: None,
            recorder_finish_requested: false,
            pending_operations: BTreeMap::new(),
            deployment_provenance,
            virtual_model_generations,
            emulator_targets,
            physical_instruments,
            configured_probes,
            configuration_quiesced: false,
            reconnect_quiesced_resources: BTreeSet::new(),
        })
    }

    /// Construct the bounded trusted native virtual slice in safe Ready state.
    pub fn virtual_demo() -> Result<Self, Error> {
        let mut runtime = Runtime::new();
        runtime.command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: PLANT,
            name: "Virtual thermal plant".into(),
            history_capacity: 64,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))?;
        let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
        let safe_profile = SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(2),
            max_proposal_ttl: Duration::from_millis(200),
            required_evidence: EvidenceLevel::Readback,
        };
        runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(safe_profile.clone()),
        })?;
        runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })?;
        let CommandResult::Output(OutputResult::Dispatched(safe)) =
            runtime.command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BeginDispatch,
            })?
        else {
            return Err(Error::InvalidConfiguration("virtual safe dispatch absent"));
        };
        runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::Complete {
                dispatch_id: safe.id(),
                outcome: DispatchOutcome::ReadbackVerified,
            },
        })?;
        runtime.command(Command::RegisterReference(ReferenceConfig::Ramp {
            id: REFERENCE,
            start: 20.0,
            target: 50.0,
            rate: 2.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        }))?;
        runtime.command(Command::RegisterController(NativeControllerConfig {
            id: CONTROLLER,
            input: SignalId::new(PLANT, lab_core::TEMPERATURE),
            output: actuator,
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_millis(200),
                warmup_samples: 3,
                unit: Unit::CELSIUS,
            },
            pid: PidConfig {
                kp: 3.0,
                ki: 0.4,
                kd: 0.2,
                output_min: 0.0,
                output_max: 100.0,
            },
            max_input_age: Duration::from_millis(500),
            max_tick_gap: Duration::from_millis(500),
            lease_lifetime: Duration::from_secs(2),
            proposal_ttl: Duration::from_millis(200),
        }))?;
        runtime.command(Command::PrepareController(CONTROLLER))?;
        let events = EventLog::new(
            &runtime,
            &[CONTROLLER],
            &[REFERENCE],
            "00000000000000000000000000000000",
        );
        Ok(Self {
            runtime,
            events,
            plan: SchedulePlan::virtual_demo(),
            consumed: BTreeMap::new(),
            consumed_managed: BTreeMap::new(),
            components: Vec::new(),
            outputs: vec![actuator],
            active_safety_profiles: vec![(actuator, safe_profile)],
            resources: Vec::new(),
            closed_resources: BTreeSet::new(),
            last_now: Duration::ZERO,
            stopping: false,
            recorder: None,
            recording_policy: None,
            recording_status: None,
            last_recording_submission: None,
            recorder_finish_requested: false,
            pending_operations: BTreeMap::new(),
            deployment_provenance: Vec::new(),
            virtual_model_generations: BTreeMap::from([(PLANT, 1)]),
            emulator_targets: BTreeSet::new(),
            physical_instruments: BTreeSet::new(),
            configured_probes: Vec::new(),
            configuration_quiesced: false,
            reconnect_quiesced_resources: BTreeSet::new(),
        })
    }

    /// Trusted profile's plant identity; clients should use discovery instead.
    pub const fn plant_id(&self) -> InstrumentId {
        PLANT
    }
    /// Trusted profile's temperature signal identity.
    pub const fn temperature(&self) -> SignalId {
        SignalId::new(PLANT, lab_core::TEMPERATURE)
    }
    /// Trusted profile's independent Ramp identity.
    pub const fn reference_id(&self) -> ReferenceId {
        REFERENCE
    }
    /// Trusted profile's native controller identity.
    pub const fn controller_id(&self) -> ControllerId {
        CONTROLLER
    }
}
fn event_domain_error(error: EventError) -> Error {
    match error {
        EventError::Gap | EventError::Future => Error::InvalidConfiguration("event cursor invalid"),
        EventError::Exhausted => Error::InvalidConfiguration("event sequence exhausted"),
        EventError::Oversized => Error::InvalidConfiguration("event record exceeds M6 bound"),
    }
}
#[cfg(test)]
mod recorder_outbox_host_tests {
    use super::*;
    use crate::recorder::{RecorderLimits, RecordingState, WriterBarrier};
    use std::{path::PathBuf, time::Instant};

    fn temporary_database() -> PathBuf {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::temp_dir().join(format!("lab-runtime-m7-outbox-host-{suffix}.sqlite"))
    }

    #[test]
    fn core_outbox_overflow_marks_best_effort_gap_without_stopping_native_control() {
        let path = temporary_database();
        let barrier = WriterBarrier::held();
        let worker =
            RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
                .unwrap();
        let mut host = HostCore::virtual_demo().unwrap();
        host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
            .unwrap();
        host.start_recording("outbox host overflow", Duration::ZERO)
            .unwrap();
        let by = Instant::now() + Duration::from_secs(3);
        while host.recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::ZERO);
            std::thread::yield_now();
        }
        host.service(&test_clock(Duration::ZERO)).unwrap();
        host.command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
        // A trusted test packs one owner unit without the normal Host drain.
        // This proves the Host's overflow handoff, not a normal client path.
        for position in 1..=257u64 {
            host.runtime
                .command(Command::RefreshMeasurement {
                    instrument: PLANT,
                    parameter: lab_core::TEMPERATURE,
                    at: Duration::from_millis(position),
                })
                .unwrap();
        }
        assert!(host.runtime.recording_facts_overflowed());
        let first_missing = host.runtime.recording_first_lost_fact();
        assert!(first_missing.is_some());
        host.admit_recording_facts(Duration::from_millis(257));
        let failed = host.recording_status().unwrap();
        assert_eq!(failed.state, RecordingState::Failed);
        assert_eq!(failed.coverage, "gap");
        assert_eq!(failed.first_missing_fact, first_missing);
        assert_eq!(
            failed.first_error.as_deref(),
            Some("core fact outbox overflow")
        );
        barrier.release();
        for step in 3..=8 {
            host.service(&test_clock(Duration::from_millis(step * 100)))
                .unwrap();
        }
        let controller = host.query(Query::Controller(CONTROLLER)).unwrap();
        assert!(matches!(controller, QueryResult::Controller(snapshot)
            if snapshot.state == ControllerState::Running));
        host.finish_recorder().unwrap();
        while !host.recording_status().unwrap().terminal_seal_committed {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::from_millis(800));
            std::thread::yield_now();
        }
        drop(host);
        let db = rusqlite::Connection::open(&path).unwrap();
        let gaps: i64 = db
            .query_row("SELECT COUNT(*) FROM gaps", [], |row| row.get(0))
            .unwrap();
        assert_eq!(gaps, 1);
        drop(db);
        let close_by = Instant::now() + Duration::from_secs(3);
        while std::fs::remove_file(&path).is_err() {
            assert!(Instant::now() < close_by);
            std::thread::yield_now();
        }
    }

    #[test]
    fn safe_pause_terminal_waits_in_bounded_owner_slot_when_four_groups_are_busy() {
        let path = temporary_database();
        let barrier = WriterBarrier::held();
        let worker =
            RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
                .unwrap();
        let mut host = HostCore::virtual_demo().unwrap();
        host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
            .unwrap();
        host.start_recording("safe terminal credit", Duration::ZERO)
            .unwrap();
        let by = Instant::now() + Duration::from_secs(3);
        while host.recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::ZERO);
            std::thread::yield_now();
        }
        for seq in 1..=4 {
            host.record_operation(OperationRecord {
                scope: "safe-terminal-credit".into(),
                request_seq: seq,
                command: if seq == 1 {
                    "controller_pause"
                } else {
                    "reference_retune"
                },
                phase: "accepted",
                data: "{}".into(),
                outcome_basis: "application_admission",
                at: Duration::from_millis(seq),
            });
        }
        assert_eq!(host.recording_status().unwrap().outstanding_groups, 4);
        host.record_operation(OperationRecord {
            scope: "safe-terminal-credit".into(),
            request_seq: 1,
            command: "controller_pause",
            phase: "completed",
            data: "{}".into(),
            outcome_basis: "domain_result",
            at: Duration::from_millis(5),
        });
        assert_eq!(
            host.recording_status().unwrap().state,
            RecordingState::Recording
        );
        assert!(
            host.pending_operations
                .get(&("safe-terminal-credit".into(), 1))
                .is_some_and(|pending| pending.deferred_safe_terminal.is_some())
        );

        barrier.release();
        while host
            .pending_operations
            .contains_key(&("safe-terminal-credit".into(), 1))
        {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::from_millis(10));
            std::thread::yield_now();
        }
        assert_eq!(
            host.recording_status().unwrap().state,
            RecordingState::Recording
        );
        host.stop_recording_at(Duration::from_millis(20)).unwrap();
        while host.recording_status().unwrap().state != RecordingState::Idle {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::from_millis(20));
            std::thread::yield_now();
        }
        host.finish_recorder().unwrap();
        while host.recording_status().unwrap().state != RecordingState::Closed {
            assert!(Instant::now() < by);
            host.poll_recorder(Duration::from_millis(20));
            std::thread::yield_now();
        }
        drop(host);
        let archive = rusqlite::Connection::open(&path).unwrap();
        let terminals: i64 = archive
            .query_row(
                "SELECT COUNT(*) FROM operation_events WHERE command='controller_pause' AND phase='completed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminals, 1);
        drop(archive);
        std::fs::remove_file(path).unwrap();
    }

    #[derive(Clone, Copy)]
    struct TestClock(Duration);
    impl Clock for TestClock {
        fn now(&self) -> Duration {
            self.0
        }
    }
    fn test_clock(now: Duration) -> TestClock {
        TestClock(now)
    }
}

#[cfg(test)]
mod reconnect_recorder_ordering_tests;
