//! The long-running owner will call this bounded scheduler independently of clients.
//!
//! This module keeps one mutable Runtime owner and explicit periods. It does not
//! infer a cadence from a native controller's failure threshold. A skipped slot
//! receives one actual-time opportunity, never a replay at an old deadline.

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
    managed_executor::MOVING_MEAN_IMPLEMENTATION,
};
use lab_core::{
    AccessMode, Command, CommandResult, Error, InstrumentId, MeasurementFailure,
    ParameterDescriptor, ParameterRole, Query, QueryResult, Runtime, Sample, SampleQuality,
    SignalId, Unit, Value, ValueSpec, WriteEffect,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    instrument::{KnownOperation, MetakonBinding, MetakonInstrumentConfig},
    managed::{
        ComponentDefinition, ComponentError, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, ComponentState, PlainData,
        PlainValue,
    },
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId, ReferenceSnapshot},
    transport::{ByteTransport, ExecutorState, ResourceId, TransactionOutcome},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

const PLANT: InstrumentId = InstrumentId::new(1);
const REFERENCE: ReferenceId = ReferenceId::new(1);
const CONTROLLER: ControllerId = ControllerId::new(1);
const MANAGED_SOURCE: ComponentId = ComponentId::new(201);
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

/// Sole mutable Core owner plus one explicit schedule; callers serialize commands.
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
    model_generations: BTreeMap<InstrumentId, u64>,
    emulator_targets: BTreeSet<SignalId>,
    configured_probes: Vec<ConfiguredProbe>,
    configuration_quiesced: bool,
    reconnect_quiesced_resources: BTreeSet<ResourceId>,
}

/// A fully checked scheduling delta for already-prepared managed replacements.
pub(crate) struct ManagedComponentActivation {
    affected: BTreeSet<ComponentId>,
    sources: Vec<(ComponentId, Periodic)>,
    transforms: Vec<(ComponentId, SignalId)>,
    deployment_provenance: Vec<ProvenanceEntry>,
}

impl HostCore {
    /// Construct a validated configured native observation graph without opening
    /// storage, listeners, Lua or physical resources.
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
        let mut measurements = Vec::with_capacity(dto.instruments.len());
        let mut model_generations = BTreeMap::new();
        let mut emulator_targets = BTreeSet::new();
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
        let mut metakon_reads = Vec::new();
        let mut configured_probes = Vec::new();
        for instrument in &dto.instruments {
            match instrument {
                InstrumentDto::VirtualMeasurement {
                    id,
                    display_name,
                    history_capacity,
                    base_temperature,
                    measurement_enabled,
                    external_publication,
                    poll_period_ms,
                    ..
                } => {
                    runtime.command(Command::RegisterVirtual(
                        lab_core::VirtualInstrumentConfig {
                            id: InstrumentId::new(*id),
                            name: display_name.clone(),
                            history_capacity: *history_capacity,
                            base_temperature: *base_temperature,
                            measurement_enabled: *measurement_enabled,
                        },
                    ))?;
                    let signal = SignalId::new(InstrumentId::new(*id), lab_core::TEMPERATURE);
                    if *external_publication {
                        emulator_targets.insert(signal);
                    } else {
                        measurements.push((
                            InstrumentId::new(*id),
                            Periodic::new(Duration::from_millis(*poll_period_ms)),
                        ));
                    }
                }
                InstrumentDto::ThermalPlant {
                    id,
                    display_name,
                    history_capacity,
                    ambient_temperature,
                    initial_temperature,
                    gain_per_percent,
                    time_constant_ms,
                    poll_period_ms,
                    ..
                } => {
                    runtime.command(Command::RegisterThermalPlant(ThermalPlantConfig {
                        id: InstrumentId::new(*id),
                        name: display_name.clone(),
                        history_capacity: *history_capacity,
                        ambient_temperature: *ambient_temperature,
                        initial_temperature: *initial_temperature,
                        gain_per_percent: *gain_per_percent,
                        time_constant: Duration::from_millis(*time_constant_ms),
                    }))?;
                    model_generations.insert(InstrumentId::new(*id), 1);
                    measurements.push((
                        InstrumentId::new(*id),
                        Periodic::new(Duration::from_millis(*poll_period_ms)),
                    ));
                }
                InstrumentDto::Metakon {
                    id,
                    definition,
                    resource_id,
                    address,
                    poll_period_ms,
                    queue_timeout_ms,
                    transaction_timeout_ms,
                    ..
                } => {
                    let bytes = deployment
                        .artifact_bytes(definition)
                        .ok_or(Error::InvalidConfiguration("frozen definition missing"))?;
                    let text = std::str::from_utf8(bytes)
                        .map_err(|_| Error::InvalidConfiguration("definition is not UTF-8"))?;
                    let definition = parse_definition_json(text)
                        .map_err(|_| Error::InvalidConfiguration("invalid frozen definition"))?;
                    if definition.id != InstrumentId::new(*id) {
                        return Err(Error::InvalidConfiguration(
                            "definition and deployment instrument IDs differ",
                        ));
                    }
                    let temperature = definition
                        .parameters
                        .iter()
                        .find(|parameter| parameter.operation == KnownOperation::Temperature)
                        .ok_or(Error::InvalidConfiguration(
                            "read-only Metakon definition lacks temperature",
                        ))?
                        .id;
                    let channel_type = definition
                        .parameters
                        .iter()
                        .find(|parameter| parameter.operation == KnownOperation::ChannelType)
                        .ok_or(Error::InvalidConfiguration(
                            "read-only Metakon definition lacks compatibility probe",
                        ))?
                        .id;
                    runtime.command(Command::RegisterMetakon(MetakonInstrumentConfig {
                        definition,
                        binding: MetakonBinding {
                            resource: ResourceId::new(*resource_id),
                            device: *address,
                            channel: 0,
                            binding_generation: 1,
                            mapping_revision: 1,
                            expected_output_unit: None,
                        },
                        history_capacity: 64,
                    }))?;
                    metakon_reads.push(MetakonReadSchedule {
                        instrument: InstrumentId::new(*id),
                        parameter: temperature,
                        slot: Periodic::new(Duration::from_millis(*poll_period_ms)),
                        queue_ttl: Duration::from_millis(*queue_timeout_ms),
                        timeout: Duration::from_millis(*transaction_timeout_ms),
                    });
                    configured_probes.push(ConfiguredProbe {
                        instrument: InstrumentId::new(*id),
                        parameter: channel_type,
                        queue_ttl: Duration::from_millis(*queue_timeout_ms),
                        timeout: Duration::from_millis(*transaction_timeout_ms),
                        queued: false,
                        baseline: None,
                    });
                }
            }
        }
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
            runtime.command(Command::PrepareController(id))?;
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
            model_generations,
            emulator_targets,
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
            model_generations: BTreeMap::from([(PLANT, 1)]),
            emulator_targets: BTreeSet::new(),
            configured_probes: Vec::new(),
            configuration_quiesced: false,
            reconnect_quiesced_resources: BTreeSet::new(),
        })
    }

    /// Queue each configured read-only compatibility probe exactly once.
    pub fn begin_configured_probes(&mut self, at: Duration) -> Result<(), Error> {
        for probe in &mut self.configured_probes {
            if probe.queued {
                continue;
            }
            probe.baseline = match self.runtime.query(Query::GetLatestSignal(SignalId::new(
                probe.instrument,
                probe.parameter,
            )))? {
                QueryResult::Latest(sample) => sample,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "configured probe query mismatch",
                    ));
                }
            };
            self.runtime.command(Command::QueueMetakonRead {
                instrument: probe.instrument,
                parameter: probe.parameter,
                at,
                queue_ttl: probe.queue_ttl,
                timeout: probe.timeout,
            })?;
            probe.queued = true;
        }
        Ok(())
    }

    /// Quiesce ordinary acquisition for one explicit reconnect without pausing
    /// unrelated resources, Recorder progress, recovery or native safety work.
    pub fn begin_configured_resource_reconnect(
        &mut self,
        resource: ResourceId,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        self.reconnect_quiesced_resources.insert(resource);
        Ok(())
    }

    /// Report the resource-scoped reconnect gate for diagnostics and acceptance.
    pub fn configured_resource_reconnect_quiesced(&self, resource: ResourceId) -> bool {
        self.reconnect_quiesced_resources.contains(&resource)
    }

    /// Queue only the trusted compatibility probes bound to one replacement.
    pub fn begin_configured_probes_for_resource(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<(), Error> {
        let mut found = false;
        for probe in &mut self.configured_probes {
            let bound = self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource);
            if !bound {
                continue;
            }
            found = true;
            if probe.queued {
                continue;
            }
            probe.baseline = match self.runtime.query(Query::GetLatestSignal(SignalId::new(
                probe.instrument,
                probe.parameter,
            )))? {
                QueryResult::Latest(sample) => sample,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "configured probe query mismatch",
                    ));
                }
            };
            self.runtime.command(Command::QueueMetakonRead {
                instrument: probe.instrument,
                parameter: probe.parameter,
                at,
                queue_ttl: probe.queue_ttl,
                timeout: probe.timeout,
            })?;
            probe.queued = true;
        }
        if !found {
            return Err(Error::InvalidConfiguration(
                "configured resource has no compatibility probe",
            ));
        }
        Ok(())
    }

    /// Replace one read-only adapter at an explicit closed boundary and advance
    /// every dependent physical binding generation. No port enumeration or
    /// automatic selection occurs here.
    pub fn rebind_configured_transport(
        &mut self,
        resource: ResourceId,
        adapter: Box<dyn ByteTransport>,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        let instruments: Vec<_> = self
            .plan
            .metakon_reads
            .iter()
            .filter_map(|read| {
                self.runtime
                    .metakon_binding(read.instrument)
                    .filter(|binding| binding.resource == resource)
                    .map(|binding| (read.instrument, binding))
            })
            .collect();
        if instruments.is_empty() {
            return Err(Error::InvalidConfiguration(
                "configured resource has no instrument",
            ));
        }
        let replacements: Vec<_> = instruments
            .iter()
            .map(|(instrument, binding)| {
                Ok((
                    *instrument,
                    MetakonBinding {
                        binding_generation: binding
                            .binding_generation
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("binding generation exhausted"))?,
                        mapping_revision: binding
                            .mapping_revision
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("mapping revision exhausted"))?,
                        ..*binding
                    },
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        self.runtime.replace_transport(resource, adapter)?;
        for (instrument, binding) in replacements {
            self.runtime.command(Command::RebindMetakon {
                instrument,
                binding,
                at,
            })?;
        }
        for probe in &mut self.configured_probes {
            if self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource)
            {
                probe.queued = false;
                probe.baseline = None;
            }
        }
        self.closed_resources.remove(&resource);
        self.observe(at, None)
    }

    /// Replace a closed resource and every candidate definition/address bound
    /// to it in one serialized owner turn. The logical IDs remain stable while
    /// both physical binding generation and mapping revision advance once.
    pub fn rebind_configured_transport_from_configuration(
        &mut self,
        deployment: &FrozenDeployment,
        resource: ResourceId,
        adapter: Box<dyn ByteTransport>,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        let mut replacements = Vec::new();
        for instrument in &deployment.effective().dto.instruments {
            let InstrumentDto::Metakon {
                id,
                definition,
                resource_id,
                address,
                ..
            } = instrument
            else {
                continue;
            };
            if *resource_id != resource.get() {
                continue;
            }
            let old = self
                .runtime
                .metakon_binding(InstrumentId::new(*id))
                .ok_or(Error::InvalidConfiguration("configured binding missing"))?;
            let bytes = deployment
                .artifact_bytes(definition)
                .ok_or(Error::InvalidConfiguration("frozen definition missing"))?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::InvalidConfiguration("definition is not UTF-8"))?;
            let definition = parse_definition_json(text)
                .map_err(|_| Error::InvalidConfiguration("invalid frozen definition"))?;
            if definition.id != InstrumentId::new(*id) {
                return Err(Error::InvalidConfiguration(
                    "definition and deployment instrument IDs differ",
                ));
            }
            let temperature = definition
                .parameters
                .iter()
                .find(|parameter| parameter.operation == KnownOperation::Temperature)
                .ok_or(Error::InvalidConfiguration(
                    "read-only Metakon definition lacks temperature",
                ))?
                .id;
            let channel_type = definition
                .parameters
                .iter()
                .find(|parameter| parameter.operation == KnownOperation::ChannelType)
                .ok_or(Error::InvalidConfiguration(
                    "read-only Metakon definition lacks compatibility probe",
                ))?
                .id;
            replacements.push((
                InstrumentId::new(*id),
                MetakonInstrumentConfig {
                    definition,
                    binding: MetakonBinding {
                        resource,
                        device: *address,
                        channel: 0,
                        binding_generation: old
                            .binding_generation
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("binding generation exhausted"))?,
                        mapping_revision: old
                            .mapping_revision
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("mapping revision exhausted"))?,
                        expected_output_unit: None,
                    },
                    history_capacity: 64,
                },
                old,
                temperature,
                channel_type,
            ));
        }
        if replacements.is_empty() {
            return Err(Error::InvalidConfiguration(
                "candidate resource has no instrument",
            ));
        }
        let recovery_timeout = deployment
            .effective()
            .dto
            .resources
            .iter()
            .find(|candidate| candidate.id == resource.get())
            .ok_or(Error::InvalidConfiguration("candidate resource missing"))?
            .recovery_timeout_ms;
        self.runtime.replace_transport_with_recovery_timeout(
            resource,
            adapter,
            Duration::from_millis(recovery_timeout),
        )?;
        for (instrument, config, old, temperature, channel_type) in replacements {
            self.runtime.command(Command::ReconfigureMetakon {
                config,
                expected_binding_generation: old.binding_generation,
                expected_mapping_revision: old.mapping_revision,
                at,
            })?;
            let read = self
                .plan
                .metakon_reads
                .iter_mut()
                .find(|read| read.instrument == instrument)
                .ok_or(Error::InvalidConfiguration(
                    "configured read schedule missing",
                ))?;
            read.parameter = temperature;
            let probe = self
                .configured_probes
                .iter_mut()
                .find(|probe| probe.instrument == instrument)
                .ok_or(Error::InvalidConfiguration("configured probe missing"))?;
            probe.parameter = channel_type;
            probe.queued = false;
            probe.baseline = None;
        }
        self.closed_resources.remove(&resource);
        self.observe(at, None)
    }

    /// Retire the current adapter without waiting; true means a replacement can
    /// be installed without creating a second handle owner.
    pub fn prepare_configured_transport_replacement(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<bool, Error> {
        Ok(self.runtime.shutdown_transport(resource, at)?
            == lab_core::transport::TransportShutdown::Complete)
    }

    /// Copy the current physical binding generation for diagnostics/tests.
    pub fn configured_binding_generation(&self, instrument: u64) -> Option<u64> {
        self.runtime
            .metakon_binding(InstrumentId::new(instrument))
            .map(|binding| binding.binding_generation)
    }

    /// Highest active binding generation using one stable logical resource.
    pub(crate) fn configured_resource_generation(&self, resource: ResourceId) -> Option<u64> {
        self.plan
            .metakon_reads
            .iter()
            .filter_map(|read| self.runtime.metakon_binding(read.instrument))
            .filter(|binding| binding.resource == resource)
            .map(|binding| binding.binding_generation)
            .max()
    }

    /// Copy the authoritative executor state for bounded reconnect diagnostics.
    pub(crate) fn configured_resource_executor_state(
        &self,
        resource: ResourceId,
    ) -> Option<ExecutorState> {
        match self.runtime.query(Query::Transport(resource)).ok()? {
            QueryResult::Transport(snapshot) => Some(snapshot.state),
            _ => None,
        }
    }

    /// Check frozen channel-type observations without polling or hidden I/O.
    pub fn configured_probes_ready(&self) -> Result<bool, Error> {
        for probe in &self.configured_probes {
            let QueryResult::Latest(Some(sample)) = self.runtime.query(Query::GetLatestSignal(
                SignalId::new(probe.instrument, probe.parameter),
            ))?
            else {
                return Ok(false);
            };
            if probe.baseline.as_ref() == Some(&sample) {
                return Ok(false);
            }
            if sample.quality() != SampleQuality::Good
                || sample.value() != Some(&lab_core::Value::Integer(3))
            {
                return Err(Error::InvalidConfiguration(
                    "Metakon channel compatibility probe failed",
                ));
            }
        }
        Ok(true)
    }

    /// Inspect only the compatibility probes bound to one reconnecting resource.
    /// The rebind-generated Unavailable baseline is pending, not probe failure.
    pub fn configured_probes_ready_for_resource(
        &self,
        resource: ResourceId,
    ) -> Result<bool, Error> {
        let mut found = false;
        for probe in &self.configured_probes {
            let bound = self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource);
            if !bound {
                continue;
            }
            found = true;
            if !probe.queued {
                return Ok(false);
            }
            let QueryResult::Latest(Some(sample)) = self.runtime.query(Query::GetLatestSignal(
                SignalId::new(probe.instrument, probe.parameter),
            ))?
            else {
                return Ok(false);
            };
            if probe.baseline.as_ref() == Some(&sample) {
                return Ok(false);
            }
            if sample.quality() != SampleQuality::Good
                || sample.value() != Some(&lab_core::Value::Integer(3))
            {
                return Err(Error::InvalidConfiguration(
                    "Metakon channel compatibility probe failed",
                ));
            }
        }
        if !found {
            return Err(Error::InvalidConfiguration(
                "configured resource has no compatibility probe",
            ));
        }
        Ok(true)
    }

    /// Release one replacement only after its trusted probe and durable lifecycle
    /// have succeeded. The fresh deadline prevents a reconnect catch-up burst.
    pub fn activate_configured_resource_after_reconnect(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.reconnect_quiesced_resources.contains(&resource)
            || !self.configured_probes_ready_for_resource(resource)?
        {
            return Err(Error::InvalidConfiguration(
                "configured resource reconnect is not ready",
            ));
        }
        for read in &mut self.plan.metakon_reads {
            if self
                .runtime
                .metakon_binding(read.instrument)
                .is_some_and(|binding| binding.resource == resource)
            {
                read.slot.next_due = at
                    .checked_add(read.slot.period)
                    .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
            }
        }
        self.reconnect_quiesced_resources.remove(&resource);
        Ok(())
    }

    /// Retire a replacement that crossed the generation fence but failed its
    /// compatibility gate. The new generation remains authoritative and gated.
    pub fn retire_failed_configured_reconnect(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<bool, Error> {
        if !self.reconnect_quiesced_resources.contains(&resource) {
            return Err(Error::InvalidConfiguration(
                "configured resource reconnect is not quiesced",
            ));
        }
        let complete = self.runtime.shutdown_transport(resource, at)?
            == lab_core::transport::TransportShutdown::Complete;
        self.observe(at, None)?;
        if complete {
            self.closed_resources.insert(resource);
        }
        Ok(complete)
    }

    /// Apply display/cadence-only fields at one owner commit without replacing
    /// instruments, observations, authority or generations.
    pub(crate) fn apply_live_configuration(
        &mut self,
        candidate: &FrozenDeployment,
    ) -> Result<(), Error> {
        for instrument in &candidate.effective().dto.instruments {
            match instrument {
                InstrumentDto::VirtualMeasurement {
                    id,
                    display_name,
                    poll_period_ms,
                    ..
                }
                | InstrumentDto::ThermalPlant {
                    id,
                    display_name,
                    poll_period_ms,
                    ..
                } => {
                    self.runtime.command(Command::RenameInstrument {
                        instrument: InstrumentId::new(*id),
                        name: display_name.clone(),
                    })?;
                    let (_, slot) = self
                        .plan
                        .plants
                        .iter_mut()
                        .find(|(instrument, _)| instrument.get() == *id)
                        .ok_or(Error::InvalidConfiguration("configured schedule missing"))?;
                    slot.period = Duration::from_millis(*poll_period_ms);
                    slot.next_due = self
                        .last_now
                        .checked_add(slot.period)
                        .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
                }
                InstrumentDto::Metakon {
                    id,
                    poll_period_ms,
                    queue_timeout_ms,
                    transaction_timeout_ms,
                    ..
                } => {
                    let read = self
                        .plan
                        .metakon_reads
                        .iter_mut()
                        .find(|read| read.instrument.get() == *id)
                        .ok_or(Error::InvalidConfiguration(
                            "configured read schedule missing",
                        ))?;
                    read.slot.period = Duration::from_millis(*poll_period_ms);
                    read.slot.next_due = self
                        .last_now
                        .checked_add(read.slot.period)
                        .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
                    read.queue_ttl = Duration::from_millis(*queue_timeout_ms);
                    read.timeout = Duration::from_millis(*transaction_timeout_ms);
                }
            }
        }
        self.deployment_provenance = candidate
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(())
    }

    /// Revoke every configured authority and prove the existing virtual safe
    /// policy before configuration replacement. This method exposes no rearm.
    pub(crate) fn enter_configuration_safe_barrier(&mut self, at: Duration) -> Result<bool, Error> {
        let controllers: Vec<_> = self.plan.controllers.iter().map(|(id, _, _)| *id).collect();
        for controller in controllers {
            let QueryResult::Controller(snapshot) =
                self.runtime.query(Query::Controller(controller))?
            else {
                return Err(Error::InvalidConfiguration("configured controller missing"));
            };
            if matches!(
                snapshot.state,
                ControllerState::Warming | ControllerState::Running
            ) {
                self.runtime
                    .command(Command::PauseController { controller, at })?;
            }
        }
        for actuator in self.outputs.clone() {
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::RequestSafe,
            })?;
            let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::BeginDispatch,
                })?
            else {
                return Ok(false);
            };
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
            })?;
        }
        self.observe(at, None)?;
        Ok(self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed && snapshot.lease.is_none())
        }))
    }

    /// Commit a fixed-topology candidate after its safe barrier. All mutations
    /// occur in this serialized owner turn; controllers remain Ready/Paused and
    /// no output authority is acquired.
    pub(crate) fn apply_configuration(
        &mut self,
        active: &FrozenDeployment,
        candidate: &FrozenDeployment,
        at: Duration,
    ) -> Result<(), Error> {
        let old = &active.effective().dto;
        let new = &candidate.effective().dto;

        if old
            .instruments
            .iter()
            .zip(&new.instruments)
            .any(|(old, new)| thermal_configuration_changed(old, new))
        {
            self.restart_configured_models(candidate, at)?;
        }

        for (old_reference, reference) in old.references.iter().zip(&new.references) {
            if old_reference == reference {
                continue;
            }
            let id = ReferenceId::new(reference.id);
            let revision = match self.runtime.query(Query::Reference(id))? {
                QueryResult::Reference(ReferenceSnapshot::Fixed { revision, .. })
                | QueryResult::Reference(ReferenceSnapshot::Ramp { revision, .. }) => revision,
                _ => return Err(Error::InvalidConfiguration("configured Reference missing")),
            };
            let unit = Unit::new(&reference.unit_id, &reference.unit_symbol)?;
            let config = match reference.kind {
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
                    at,
                },
            };
            self.runtime.command(Command::ReconfigureReference {
                reference: id,
                config,
                expected_revision: revision,
            })?;
        }

        for (old_safe, safe) in old.safe_profiles.iter().zip(&new.safe_profiles) {
            if old_safe == safe {
                continue;
            }
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
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::BindProfile(profile.clone()),
            })?;
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::RequestSafe,
            })?;
            let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::BeginDispatch,
                })?
            else {
                return Err(Error::InvalidConfiguration("new safe dispatch absent"));
            };
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
            })?;
            if let Some((_, current)) = self
                .active_safety_profiles
                .iter_mut()
                .find(|(current, _)| *current == actuator)
            {
                *current = profile;
            }
        }

        for (old_controller, controller) in old.controllers.iter().zip(&new.controllers) {
            let id = ControllerId::new(controller.id);
            if old_controller != controller {
                let QueryResult::Controller(snapshot) =
                    self.runtime.query(Query::Controller(id))?
                else {
                    return Err(Error::InvalidConfiguration("configured controller missing"));
                };
                let reference = ReferenceId::new(controller.reference_id);
                let unit = match self.runtime.query(Query::Reference(reference))? {
                    QueryResult::Reference(ReferenceSnapshot::Fixed { unit, .. }) => unit,
                    QueryResult::Reference(ReferenceSnapshot::Ramp { state, .. }) => state.unit,
                    _ => {
                        return Err(Error::InvalidConfiguration("configured Reference missing"));
                    }
                };
                self.runtime.command(Command::ReconfigureController {
                    controller: id,
                    config: NativeControllerConfig {
                        id,
                        input: SignalId::new(
                            InstrumentId::new(controller.input_instrument_id),
                            lab_core::ParameterId::new(controller.input_parameter_id),
                        ),
                        output: ActuatorId::new(
                            InstrumentId::new(controller.output_instrument_id),
                            lab_core::ParameterId::new(controller.output_parameter_id),
                        ),
                        reference,
                        ema: EmaConfig {
                            time_constant: Duration::from_millis(controller.ema_time_constant_ms),
                            warmup_samples: controller.ema_warmup_samples,
                            unit,
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
                    },
                    expected_revision: snapshot.config_revision,
                })?;
            }
            let (_, _, slot) = self
                .plan
                .controllers
                .iter_mut()
                .find(|(current, _, _)| *current == id)
                .ok_or(Error::InvalidConfiguration("controller schedule missing"))?;
            slot.period = Duration::from_millis(controller.period_ms);
            slot.next_due = at
                .checked_add(slot.period)
                .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
        }
        self.apply_live_configuration(candidate)?;
        self.observe(at, None)
    }

    /// Restart every configured native thermal model under its generation fence.
    pub(crate) fn restart_configured_models(
        &mut self,
        deployment: &FrozenDeployment,
        at: Duration,
    ) -> Result<(usize, u64), Error> {
        let mut count = 0usize;
        let mut latest_generation = 0u64;
        for instrument in &deployment.effective().dto.instruments {
            if let InstrumentDto::ThermalPlant {
                id,
                display_name,
                history_capacity,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                ..
            } = instrument
            {
                let id = InstrumentId::new(*id);
                let expected = *self
                    .model_generations
                    .get(&id)
                    .ok_or(Error::InvalidConfiguration("model generation missing"))?;
                let CommandResult::ModelRestarted { generation, .. } =
                    self.runtime.command(Command::RestartThermalPlant {
                        instrument: id,
                        config: ThermalPlantConfig {
                            id,
                            name: display_name.clone(),
                            history_capacity: *history_capacity,
                            ambient_temperature: *ambient_temperature,
                            initial_temperature: *initial_temperature,
                            gain_per_percent: *gain_per_percent,
                            time_constant: Duration::from_millis(*time_constant_ms),
                        },
                        expected_generation: expected,
                        at,
                    })?
                else {
                    return Err(Error::InvalidConfiguration("unexpected restart result"));
                };
                self.model_generations.insert(id, generation);
                latest_generation = generation;
                count += 1;
            }
        }
        if count == 0 {
            return Err(Error::InvalidConfiguration("no configured native models"));
        }
        self.observe(at, None)?;
        Ok((count, latest_generation))
    }

    /// Stage one configured managed component. Startup and source reload call
    /// this serially so Core's accepted one-stage/two-worker bound is unchanged.
    pub(crate) fn stage_configured_component(
        &mut self,
        deployment: &FrozenDeployment,
        index: usize,
        replaces: bool,
        at: Duration,
    ) -> Result<ComponentId, Error> {
        let component = deployment
            .effective()
            .dto
            .managed_components
            .get(index)
            .ok_or(Error::InvalidConfiguration("managed component index"))?;
        let implementation = if let Some(source_path) = &component.source {
            let source = deployment
                .artifact_bytes(source_path)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .ok_or(Error::InvalidConfiguration("frozen managed source missing"))?;
            ComponentImplementation::text(component.implementation.clone(), source)?
        } else {
            ComponentImplementation::built_in(component.implementation.clone())?
        };
        let id = ComponentId::new(component.id);
        let kind = component
            .input_instrument_id
            .map_or(ComponentKind::Source, |input| ComponentKind::Transform {
                input: SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE),
            });
        let definition = ComponentDefinition {
            manifest: ComponentManifest {
                schema_version: 1,
                id,
                instrument: InstrumentId::new(component.instrument_id),
                name: component.display_name.clone(),
                parameter: lab_core::TEMPERATURE,
                kind,
                unit: Unit::CELSIUS,
                min: -100.0,
                max: 500.0,
                warmup_samples: component.configured_window().unwrap_or_else(|| {
                    if component.input_instrument_id.is_some() {
                        3
                    } else {
                        1
                    }
                }),
                max_input_age: Duration::from_secs(2),
                history_capacity: 32,
            },
            implementation,
            config: component
                .plain_config()
                .map_err(|_| Error::InvalidConfiguration("managed PlainData config"))?,
        };
        self.runtime.command(if replaces {
            Command::PrepareComponentReplacement {
                definition,
                replaces: id,
                at,
            }
        } else {
            Command::StageComponent {
                definition,
                replaces: None,
                at,
            }
        })?;
        if !replaces {
            self.events.track_component(id);
            self.components.push((
                id,
                if component.input_instrument_id.is_some() {
                    "transform"
                } else {
                    "source"
                },
            ));
        }
        Ok(id)
    }

    /// Whether one configured init result committed and no callback remains.
    pub(crate) fn component_initialized(&self, id: ComponentId) -> bool {
        matches!(self.runtime.query(Query::Component(id)),Ok(QueryResult::Component(snapshot))
            if matches!(snapshot.state,ComponentState::Warming|ComponentState::Ready)
                && snapshot.pending.is_none())
    }

    pub(crate) fn component_prepared(&self, id: ComponentId) -> bool {
        self.runtime.component_prepared(id)
    }

    pub(crate) fn component_prepare_pending(&self) -> bool {
        self.runtime.component_prepare_pending()
    }

    pub(crate) fn discard_prepared_components(&mut self) {
        let _ = self.runtime.command(Command::DiscardPreparedComponents);
    }

    pub(crate) fn commit_prepared_components(
        &mut self,
        components: Vec<ComponentId>,
        at: Duration,
    ) -> Result<(), Error> {
        let CommandResult::ComponentsCommitted(_) = self
            .runtime
            .command(Command::CommitPreparedComponents { components, at })?
        else {
            return Err(Error::InvalidConfiguration(
                "unexpected component batch result",
            ));
        };
        self.observe(at, None)
    }

    /// Start configured component cadences only after every init committed.
    pub(crate) fn activate_configured_components(
        &mut self,
        deployment: &FrozenDeployment,
        at: Duration,
    ) -> Result<(), Error> {
        self.plan.sources.clear();
        self.plan.transforms.clear();
        for component in &deployment.effective().dto.managed_components {
            let id = ComponentId::new(component.id);
            if !self.component_initialized(id) {
                return Err(Error::InvalidConfiguration("managed init incomplete"));
            }
            if let Some(input) = component.input_instrument_id {
                self.plan.transforms.push((
                    id,
                    SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE),
                ));
            } else {
                let mut slot = Periodic::new(Duration::from_millis(component.period_ms));
                slot.next_due = at;
                self.plan.sources.push((id, slot));
            }
        }
        self.deployment_provenance = deployment
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(())
    }

    /// Validate only the selected prepared branch and build its infallible schedule delta.
    pub(crate) fn prepare_configured_component_activation(
        &self,
        deployment: &FrozenDeployment,
        components: &[ComponentId],
        at: Duration,
    ) -> Result<ManagedComponentActivation, Error> {
        if components.is_empty() || components.len() > lab_core::managed::MAX_COMPONENTS {
            return Err(Error::InvalidConfiguration(
                "managed activation component count",
            ));
        }
        let affected: BTreeSet<_> = components.iter().copied().collect();
        if affected.len() != components.len() {
            return Err(Error::InvalidConfiguration(
                "duplicate managed activation component",
            ));
        }
        let mut sources = Vec::with_capacity(components.len());
        let mut transforms = Vec::with_capacity(components.len());
        for id in components {
            if !self.runtime.component_prepared(*id) {
                return Err(Error::InvalidConfiguration(
                    "managed activation candidate missing",
                ));
            }
            let component = deployment
                .effective()
                .dto
                .managed_components
                .iter()
                .find(|component| ComponentId::new(component.id) == *id)
                .ok_or(Error::InvalidConfiguration(
                    "managed activation declaration missing",
                ))?;
            if let Some(input) = component.input_instrument_id {
                transforms.push((
                    *id,
                    SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE),
                ));
            } else {
                let mut slot = Periodic::new(Duration::from_millis(component.period_ms));
                slot.next_due = at;
                sources.push((*id, slot));
            }
        }
        let deployment_provenance = deployment
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(ManagedComponentActivation {
            affected,
            sources,
            transforms,
            deployment_provenance,
        })
    }

    /// Apply a previously checked selected-branch schedule delta without new failure points.
    pub(crate) fn commit_configured_component_activation(
        &mut self,
        activation: ManagedComponentActivation,
    ) {
        self.plan
            .sources
            .retain(|(id, _)| !activation.affected.contains(id));
        self.plan
            .transforms
            .retain(|(id, _)| !activation.affected.contains(id));
        self.plan.sources.extend(activation.sources);
        self.plan.transforms.extend(activation.transforms);
        self.deployment_provenance = activation.deployment_provenance;
    }

    /// Attach one already-open worker under trusted host composition.
    /// Required control starts closed and opens only on a committed start receipt.
    pub fn attach_recorder(
        &mut self,
        mut worker: RecorderWorker,
        policy: RecordingPolicy,
        at: Duration,
    ) -> Result<(), Error> {
        if self.recorder.is_some() || self.stopping || at < self.last_now {
            return Err(Error::InvalidConfiguration("recorder attach state"));
        }
        let (entries, objects) = self.frozen_activation_entries()?;
        worker
            .configure_policy(policy)
            .map_err(|_| Error::InvalidConfiguration("recorder policy configuration"))?;
        worker
            .request_activation(entries, objects)
            .map_err(|_| Error::InvalidConfiguration("recorder activation admission"))?;
        if policy == RecordingPolicy::Required {
            self.runtime.require_recording(at);
        }
        self.recording_status = Some(worker.poll());
        self.recording_policy = Some(policy);
        self.recorder = Some(worker);
        Ok(())
    }

    /// Read the latest owned receipt/status without SQL or hidden worker polling.
    pub fn recording_status(&self) -> Option<&RecordingStatus> {
        self.recording_status.as_ref()
    }

    /// Observe the startup activation receipt without SQL or owner blocking.
    pub fn recording_activation_committed(&mut self) -> Result<bool, Error> {
        let Some(worker) = self.recorder.as_mut() else {
            return Ok(true);
        };
        let status = worker.poll();
        let committed = status.activation_root.is_some();
        let failed = status.state == RecordingState::Failed;
        self.recording_status = Some(status);
        if failed {
            return Err(Error::InvalidConfiguration("recorder activation failed"));
        }
        Ok(committed)
    }

    /// Close Required ordinary admission before a configuration commit whose
    /// durable activation receipt has not yet arrived.
    pub(crate) fn begin_configuration_recording_fence(&mut self, at: Duration) -> bool {
        let was_open = self.recording_policy == Some(RecordingPolicy::Required)
            && self.runtime.required_recording_open();
        if was_open {
            self.runtime.require_recording(at);
        }
        was_open
    }

    /// Suspend new acquisition/control production while safety and already
    /// admitted transport/Recorder work continue to make bounded progress.
    pub(crate) fn begin_configuration_quiesce(&mut self) {
        self.configuration_quiesced = true;
    }

    /// Resume only acquisition scheduling after a confirmed activation outcome.
    /// Controllers and output authority remain in their post-barrier state.
    pub(crate) fn end_configuration_quiesce(&mut self) {
        self.configuration_quiesced = false;
    }

    /// Drain Core facts and reserve one of the existing Recorder ingress groups.
    /// Outer `None` means backpressure; inner `None` means recording is disabled.
    pub(crate) fn try_reserve_configuration_activation(
        &mut self,
        lifecycle: &ConfigurationLifecycleRecord,
        at: Duration,
    ) -> Result<Option<Option<u64>>, Error> {
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        let Some(worker) = self.recorder.as_mut() else {
            return Ok(Some(None));
        };
        worker
            .try_reserve_live_activation(lifecycle)
            .map(|reservation| reservation.map(Some))
            .map_err(|_| Error::InvalidConfiguration("live activation reservation rejected"))
    }

    /// Fill the reserved activation only after the owner commit has succeeded.
    pub(crate) fn commit_reserved_configuration_activation(
        &mut self,
        generation: Option<u64>,
        lifecycle: ConfigurationLifecycleRecord,
    ) -> Result<(), Error> {
        let Some(generation) = generation else {
            return Ok(());
        };
        let (entries, objects) = self.frozen_activation_entries()?;
        self.recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder reservation lost"))?
            .commit_reserved_live_activation(generation, entries, objects, lifecycle)
            .map_err(|_| Error::InvalidConfiguration("live activation rejected"))
    }

    pub(crate) fn cancel_configuration_activation(
        &mut self,
        generation: Option<u64>,
    ) -> Result<(), Error> {
        let Some(generation) = generation else {
            return Ok(());
        };
        self.recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder reservation lost"))?
            .cancel_live_activation_reservation(generation)
            .map_err(|_| Error::InvalidConfiguration("live activation cancellation failed"))
    }

    pub(crate) fn configuration_recording_failed(&mut self, at: Duration) {
        if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(at);
        }
    }

    /// Poll a specific live activation receipt and reopen Required admission only
    /// from that durable generation.
    pub(crate) fn live_activation_committed(
        &mut self,
        generation: Option<u64>,
        reopen_required: bool,
        submitted_at: Duration,
        now: Duration,
    ) -> Result<bool, Error> {
        let Some(generation) = generation else {
            return Ok(true);
        };
        self.poll_recorder(now);
        let status = self
            .recording_status
            .as_ref()
            .ok_or(Error::InvalidConfiguration("recorder status missing"))?;
        if status.state == RecordingState::Failed {
            self.runtime.recording_failure(now);
            return Err(Error::RecordingUnavailable);
        }
        if status.activation_generation < generation {
            return Ok(false);
        }
        if reopen_required {
            self.runtime.confirm_recording_start(submitted_at, now)?;
        }
        Ok(true)
    }

    // Capture the actual currently committed scalar composition before any
    // worker hashing. All source bytes come from the fixed loaded component
    // definitions, never from a pathname reread at recording time.
    fn frozen_activation_entries(
        &self,
    ) -> Result<(Vec<ProvenanceEntry>, Vec<ProvenanceObject>), Error> {
        let mut entries = self.deployment_provenance.clone();
        entries.reserve(8);
        let push =
            |entries: &mut Vec<ProvenanceEntry>, kind: &str, encoding: &str, content: Vec<u8>| {
                entries.push(ProvenanceEntry {
                    kind: kind.into(),
                    encoding: encoding.into(),
                    content,
                })
            };
        push(
            &mut entries,
            "rust_build",
            "utf8",
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
        );
        let mut controllers = Vec::with_capacity(self.plan.controllers.len());
        for (id, _, _) in &self.plan.controllers {
            let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(*id))?
            else {
                return Err(Error::InvalidConfiguration("controller config unavailable"));
            };
            let QueryResult::Controller(state) = self.runtime.query(Query::Controller(*id))? else {
                return Err(Error::InvalidConfiguration("controller state unavailable"));
            };
            controllers.push(serde_json::json!({
                "id":config.id.get().to_string(),"revision":state.config_revision.to_string(),
                "input":{"instrument":config.input.instrument().get().to_string(),
                    "parameter":config.input.parameter().get().to_string()},
                "output":{"instrument":config.output.instrument().get().to_string(),
                    "parameter":config.output.parameter().get().to_string()},
                "reference":config.reference.get().to_string(),
                "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
                    "output_min":config.pid.output_min,"output_max":config.pid.output_max},
                "ema":{"time_constant_ns":config.ema.time_constant.as_nanos().to_string(),
                    "warmup_samples":config.ema.warmup_samples,"unit":config.ema.unit.id()},
                "max_input_age_ns":config.max_input_age.as_nanos().to_string(),
                "max_tick_gap_ns":config.max_tick_gap.as_nanos().to_string(),
                "lease_lifetime_ns":config.lease_lifetime.as_nanos().to_string(),
                "proposal_ttl_ns":config.proposal_ttl.as_nanos().to_string(),
            }));
        }
        let mut references = Vec::with_capacity(self.plan.references.len());
        for (id, _) in &self.plan.references {
            let QueryResult::Reference(reference) = self.runtime.query(Query::Reference(*id))?
            else {
                return Err(Error::InvalidConfiguration("reference unavailable"));
            };
            references.push(match reference {
                ReferenceSnapshot::Ramp {
                    id,
                    state,
                    revision,
                } => serde_json::json!({
                    "kind":"ramp","id":id.get().to_string(),"revision":revision.to_string(),
                    "current":state.current,"target":state.target,"rate":state.rate,
                    "unit":state.unit.id(),"progress_at_ns":state.last_at.as_nanos().to_string()}),
                ReferenceSnapshot::Fixed {
                    id,
                    value,
                    unit,
                    revision,
                    ..
                } => serde_json::json!({
                    "kind":"fixed","id":id.get().to_string(),"revision":revision.to_string(),
                    "value":value,"unit":unit.id()}),
            });
        }
        let mut outputs = Vec::with_capacity(self.active_safety_profiles.len());
        for (actuator, profile) in &self.active_safety_profiles {
            let unit = match self
                .runtime
                .query(Query::DescribeInstrument(actuator.instrument()))?
            {
                QueryResult::Descriptor(descriptor) => descriptor
                    .parameter(actuator.parameter())
                    .map(|parameter| parameter.unit.id().to_owned())
                    .ok_or(Error::InvalidConfiguration("safety output unit missing"))?,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "safety output descriptor missing",
                    ));
                }
            };
            outputs.push(serde_json::json!({
                "instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"unit":unit,
                "min":profile.min,"max":profile.max,"safe_value":profile.safe_value,
                "max_lease_ns":profile.max_lease.as_nanos().to_string(),
                "max_proposal_ttl_ns":profile.max_proposal_ttl.as_nanos().to_string(),
                "required_evidence":match profile.required_evidence {
                    EvidenceLevel::Acknowledgement=>"ack",EvidenceLevel::Readback=>"readback"},
            }));
        }
        push(
            &mut entries,
            "native_composition",
            "json_v1",
            serde_json::json!({"profile":"virtual-demo","controllers":&controllers,
                "references":&references,"safety_outputs":&outputs})
            .to_string()
            .into_bytes(),
        );
        let mut managed_definition_indices = BTreeMap::new();
        let mut managed_source_hashes = BTreeMap::new();
        let mut cached_binary_hash = None;
        for (id, _) in &self.components {
            let definition = self
                .runtime
                .component_definition(*id)
                .ok_or(Error::InvalidConfiguration("managed definition missing"))?;
            managed_definition_indices.insert(*id, entries.len());
            let mut implementation = serde_json::json!({
                "component":id.get().to_string(),
                "implementation":definition.implementation.id().as_str(),
                "artifact":if definition.implementation.artifact().is_built_in(){
                    "built_in"
                }else{
                    "text"
                },
                "package_version":env!("CARGO_PKG_VERSION"),
                "config":plain_data_activation_json(&definition.config),
            });
            if let Some(source) = definition.implementation.artifact().text() {
                let source_hash: [u8; 32] = Sha256::digest(source.as_bytes()).into();
                implementation["source_content_sha256"] = hex_sha256(source_hash).into();
                managed_source_hashes.insert(*id, source_hash);
            } else {
                let binary_hash = match cached_binary_hash {
                    Some(hash) => hash,
                    None => {
                        let hash = runtime_binary_sha256().map_err(|_| {
                            Error::InvalidConfiguration("runtime binary identity unavailable")
                        })?;
                        cached_binary_hash = Some(hash);
                        hash
                    }
                };
                implementation["runtime_binary_sha256"] = hex_sha256(binary_hash).into();
            }
            push(
                &mut entries,
                "managed_component_implementation",
                "json_v1",
                implementation.to_string().into_bytes(),
            );
            if let Some(source) = definition.implementation.artifact().text() {
                push(
                    &mut entries,
                    "managed_component_source",
                    "utf8",
                    source.as_bytes().to_vec(),
                );
            }
        }
        push(
            &mut entries,
            "deployment_config",
            "utf8",
            b"not_present".to_vec(),
        );
        let mut objects = Vec::new();
        let QueryResult::Instruments(instruments) = self.runtime.query(Query::Discover)? else {
            return Err(Error::InvalidConfiguration(
                "activation discovery unavailable",
            ));
        };
        for instrument in instruments {
            let component = self
                .components
                .iter()
                .find(|(id, _)| id.get() == instrument.id.get())
                .map(|(id, _)| *id);
            let (generation, source_index) = if let Some(id) = component {
                let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(id))?
                else {
                    return Err(Error::InvalidConfiguration(
                        "component baseline unavailable",
                    ));
                };
                (
                    snapshot.generation,
                    *managed_definition_indices
                        .get(&id)
                        .ok_or(Error::InvalidConfiguration("component baseline missing"))?,
                )
            } else {
                (1, 1)
            };
            let parameters: Vec<_> = instrument
                .parameters
                .iter()
                .map(parameter_activation_json)
                .collect();
            let binding = self.runtime.metakon_binding(instrument.id).map(|binding| {
                serde_json::json!({
                    "resource":binding.resource.get().to_string(),
                    "device":binding.device,
                    "channel":binding.channel,
                    "binding_generation":binding.binding_generation.to_string(),
                    "mapping_revision":binding.mapping_revision.to_string(),
                    "expected_output_unit":binding.expected_output_unit.map(|unit|unit.id().to_owned()),
                })
                .to_string()
            });
            objects.push(ProvenanceObject {
                kind: "instrument",
                id: instrument.id.get().to_be_bytes().to_vec(),
                logical_key: format!("instrument:{}", instrument.id.get()),
                label: instrument.name,
                descriptor: serde_json::json!({"parameters":parameters}).to_string(),
                unit_key: None,
                generation: Some(generation),
                binding,
                definition_entry_index: source_index,
                source_content_sha256: None,
            });
        }
        for controller in controllers {
            let id = controller["id"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation controller id"))?;
            objects.push(ProvenanceObject {
                kind: "controller",
                id: id.to_be_bytes().to_vec(),
                logical_key: format!("controller:{id}"),
                label: format!("Native PID {id}"),
                descriptor: controller.to_string(),
                unit_key: None,
                generation: Some(1),
                binding: Some(controller["output"].to_string()),
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for reference in references {
            let id = reference["id"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation reference id"))?;
            objects.push(ProvenanceObject {
                kind: "reference",
                id: id.to_be_bytes().to_vec(),
                logical_key: format!("reference:{id}"),
                label: format!("Reference {id}"),
                unit_key: reference["unit"].as_str().map(str::to_owned),
                descriptor: reference.to_string(),
                generation: Some(1),
                binding: None,
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for output in outputs {
            let instrument = output["instrument"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation output instrument"))?;
            let parameter = output["parameter"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation output parameter"))?;
            let mut id = Vec::with_capacity(16);
            id.extend_from_slice(&instrument.to_be_bytes());
            id.extend_from_slice(&parameter.to_be_bytes());
            objects.push(ProvenanceObject {
                kind: "actuator",
                id,
                logical_key: format!("actuator:{instrument}:{parameter}"),
                label: format!("Actuator {instrument}:{parameter}"),
                unit_key: output["unit"].as_str().map(str::to_owned),
                descriptor: output.to_string(),
                generation: Some(1),
                binding: None,
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for (id, _) in &self.components {
            let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "component snapshot unavailable",
                ));
            };
            objects.push(ProvenanceObject {kind:"managed_component",
                id:id.get().to_be_bytes().to_vec(),logical_key:format!("component:{}",id.get()),
                label:format!("Managed component {}",id.get()),
                descriptor:serde_json::json!({"instrument":snapshot.instrument.get().to_string(),
                    "implementation":snapshot.implementation.as_str(),
                    "generation":snapshot.generation.to_string(),"state_revision":snapshot.revision.to_string(),
                    "committed_state":plain_data_activation_json(&snapshot.committed_state)})
                    .to_string(),unit_key:None,generation:Some(snapshot.generation),binding:None,
                definition_entry_index:*managed_definition_indices.get(id)
                    .ok_or(Error::InvalidConfiguration("component definition index missing"))?,
                source_content_sha256:managed_source_hashes.get(id).copied()});
        }
        Ok((entries, objects))
    }

    /// Stable database identity of this host's durable history, if enabled.
    pub fn recording_database_id(&self) -> Option<&str> {
        self.recorder.as_ref().map(RecorderWorker::database_id)
    }

    /// Boot identity fixed when an already-open worker is attached by trusted
    /// composition, so its archive keys and public serving boot cannot diverge.
    pub fn recording_boot_id(&self) -> Option<&str> {
        self.recorder.as_ref().map(RecorderWorker::boot_id)
    }

    /// Recording policy fixed by trusted startup composition.
    pub fn recording_policy(&self) -> Option<RecordingPolicy> {
        self.recording_policy
    }

    /// Emit one typed application fact through Recorder credit without changing
    /// the command's truthful domain result or waiting for SQLite.
    pub fn record_operation(&mut self, operation: OperationRecord) {
        let at = operation.at;
        let key = (operation.scope.clone(), operation.request_seq);
        let accepted = operation.phase == "accepted";
        let state = self.recording_status.as_ref().map(|status| status.state);
        if accepted {
            if self.pending_operations.len() >= 64 && !self.pending_operations.contains_key(&key) {
                if let Some(worker) = self.recorder.as_mut() {
                    worker.fail_with_gap(RecorderGap {
                        reason: "pending operation identity credit exhausted".into(),
                        at,
                        first_missing_fact: None,
                        known_missing_count: Some(1),
                        last_accepted_fact: None,
                    });
                }
                if self.recording_policy == Some(RecordingPolicy::Required) {
                    self.runtime.recording_failure(at);
                }
                self.poll_recorder(at);
                return;
            }
            let defer =
                state == Some(RecordingState::Idle) && operation.command == "recording_start";
            self.pending_operations.insert(
                key.clone(),
                PendingOperation {
                    command: operation.command,
                    accepted_at: at,
                    accepted_recorded: false,
                    deferred_start: defer.then(|| operation.clone()),
                    deferred_safe_terminal: None,
                },
            );
            if defer {
                self.poll_recorder(at);
                return;
            }
        }
        let in_interval = matches!(
            state,
            Some(RecordingState::Starting | RecordingState::Recording)
        );
        let lifecycle_terminal = !accepted
            && state == Some(RecordingState::Idle)
            && operation.command == "recording_stop";
        if in_interval || lifecycle_terminal {
            let defer_safe_terminal = !accepted
                && operation.command == "controller_pause"
                && self.recorder.as_mut().is_some_and(|worker| {
                    matches!(worker.operation_credit_available(&operation), Ok(false))
                });
            if defer_safe_terminal {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.deferred_safe_terminal = Some(operation);
                }
                self.poll_recorder(at);
                return;
            }
            let admitted = self
                .recorder
                .as_mut()
                .is_some_and(|worker| worker.try_admit_operation(operation).is_ok());
            if admitted && accepted {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.accepted_recorded = true;
                }
            } else if !admitted && self.recording_policy == Some(RecordingPolicy::Required) {
                self.runtime.recording_failure(at);
            }
        }
        if !accepted {
            self.pending_operations.remove(&key);
        }
        self.poll_recorder(at);
    }

    /// Admit one informational annotation while the current interval is active.
    /// The returned identity proves FIFO admission only; SQL remains pending.
    pub fn annotate(&mut self, annotation: AnnotationRecord) -> Result<u64, Error> {
        let at = annotation.at;
        if self.recording_status.as_ref().map(|status| status.state)
            != Some(RecordingState::Recording)
        {
            return Err(Error::RecordingUnavailable);
        }
        let result = self
            .recorder
            .as_mut()
            .ok_or(Error::RecordingUnavailable)?
            .try_admit_annotation(annotation)
            .map_err(|_| Error::RecordingUnavailable);
        if result.is_err() && self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(at);
        }
        self.poll_recorder(at);
        result
    }

    /// Schedule indexed archive work on the storage worker, never on this owner.
    pub fn request_history(
        &mut self,
        filter: HistoryFilter,
        cursor: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.recorder
            .as_mut()
            .ok_or_else(StorageError::disabled)?
            .request_history(filter, cursor, limit)
    }

    /// Take an already-completed bounded worker page without disk I/O.
    pub fn try_take_history(&mut self, job: u64) -> Option<Result<HistoryPage, StorageError>> {
        self.recorder.as_mut()?.try_take_history(job)
    }

    /// Schedule indexed archive-run discovery off the Runtime owner lane.
    pub fn request_runs(
        &mut self,
        cursor: Option<RunsCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.recorder
            .as_mut()
            .ok_or_else(StorageError::disabled)?
            .request_runs(cursor, limit)
    }

    /// Take an already-completed bounded run listing without SQL.
    pub fn try_take_runs(&mut self, job: u64) -> Option<Result<RunsPage, StorageError>> {
        self.recorder.as_mut()?.try_take_runs(job)
    }

    /// Fence a timed-out or disconnected history job before its late result.
    pub fn cancel_history(&mut self, job: u64) {
        if let Some(worker) = self.recorder.as_mut() {
            worker.cancel_history(job);
        }
    }

    /// Submit the start barrier; this does not start a controller or claim durability.
    pub fn start_recording(&mut self, label: &str, at: Duration) -> Result<(), Error> {
        if self.stopping || at < self.last_now {
            return Err(Error::InvalidConfiguration("recording start time/state"));
        }
        let boundary = self.frozen_boundary_snapshot(at)?;
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_start_with_boundary(label, at, boundary)
            .map_err(|_| Error::InvalidConfiguration("recording start rejected"))?;
        // The owner boundary is fixed before returning. Facts produced while
        // SQLite is still committing Start enter FIFO after that barrier, with
        // no Required control authority until its confirmed receipt.
        self.runtime.enable_recording_facts();
        // Inspect the committed successor against the previous owner state.
        // A fast worker may receipt Start before this method returns; assigning
        // its new status first would skip the Required gate and fact-capture
        // transition entirely.
        self.poll_recorder(at);
        Ok(())
    }

    // Copy authoritative committed snapshots while the owner is serialized.
    // Their original timestamps remain inside the boundary, so the archive
    // cannot mistake an old attempt for a new post-start measurement.
    fn frozen_boundary_snapshot(&self, at: Duration) -> Result<BoundarySnapshot, Error> {
        let QueryResult::Instruments(instruments) = self.runtime.query(Query::Discover)? else {
            return Err(Error::InvalidConfiguration(
                "boundary discovery unavailable",
            ));
        };
        let mut latest_samples = Vec::new();
        for instrument in instruments {
            for parameter in instrument.parameters {
                let Some(signal) = parameter.signal else {
                    continue;
                };
                let QueryResult::Latest(latest) =
                    self.runtime.query(Query::GetLatestSignal(signal))?
                else {
                    return Err(Error::InvalidConfiguration("boundary latest unavailable"));
                };
                if let Some(sample) = latest {
                    latest_samples.push(sample_boundary_json(&sample));
                }
            }
        }
        let mut controller_revisions = Vec::new();
        let mut controller_configurations = Vec::new();
        for (id, _, _) in &self.plan.controllers {
            let QueryResult::Controller(snapshot) = self.runtime.query(Query::Controller(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary controller unavailable",
                ));
            };
            controller_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "revision":snapshot.config_revision.to_string()}));
            let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary controller config unavailable",
                ));
            };
            controller_configurations.push(serde_json::json!({
                "id":id.get().to_string(),"revision":snapshot.config_revision.to_string(),
                "input":{"instrument":config.input.instrument().get().to_string(),
                    "parameter":config.input.parameter().get().to_string()},
                "output":{"instrument":config.output.instrument().get().to_string(),
                    "parameter":config.output.parameter().get().to_string()},
                "reference":config.reference.get().to_string(),
                "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
                    "output_min":config.pid.output_min,"output_max":config.pid.output_max},
                "ema":{"time_constant_ns":config.ema.time_constant.as_nanos().to_string(),
                    "warmup_samples":config.ema.warmup_samples,
                    "unit":config.ema.unit.id()},
                "max_input_age_ns":config.max_input_age.as_nanos().to_string(),
                "max_tick_gap_ns":config.max_tick_gap.as_nanos().to_string(),
                "lease_lifetime_ns":config.lease_lifetime.as_nanos().to_string(),
                "proposal_ttl_ns":config.proposal_ttl.as_nanos().to_string(),
            }));
        }
        let mut reference_revisions = Vec::new();
        let mut reference_configurations = Vec::new();
        for (id, _) in &self.plan.references {
            let QueryResult::Reference(snapshot) = self.runtime.query(Query::Reference(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary reference unavailable",
                ));
            };
            let revision = match snapshot {
                ReferenceSnapshot::Fixed { revision, .. }
                | ReferenceSnapshot::Ramp { revision, .. } => revision,
            };
            reference_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "revision":revision.to_string()}));
            reference_configurations.push(match snapshot {
                ReferenceSnapshot::Fixed {
                    value,
                    unit,
                    last_at,
                    ..
                } => serde_json::json!({
                    "kind":"fixed","id":id.get().to_string(),
                    "revision":revision.to_string(),"value":value,"unit":unit.id(),
                    "last_at_ns":last_at.map(|at|at.as_nanos().to_string())}),
                ReferenceSnapshot::Ramp { state, .. } => serde_json::json!({
                    "kind":"ramp","id":id.get().to_string(),
                    "revision":revision.to_string(),"current":state.current,
                    "target":state.target,"rate":state.rate,"unit":state.unit.id(),
                    "progress_at_ns":state.last_at.as_nanos().to_string()}),
            });
        }
        let mut managed_revisions = Vec::new();
        for (id, _) in &self.components {
            let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary component unavailable",
                ));
            };
            managed_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "generation":snapshot.generation.to_string(),
                "revision":snapshot.revision.to_string()}));
        }
        let data = serde_json::json!({"captured_at_ns":at.as_nanos().to_string(),
            "latest_samples":latest_samples,"controller_revisions":controller_revisions,
            "controller_configurations":controller_configurations,
            "reference_revisions":reference_revisions,
            "reference_configurations":reference_configurations,
            "managed_revisions":managed_revisions,
            "pending_operations":self.pending_operations.iter().map(|((scope,seq),pending)|
                serde_json::json!({"scope":scope,"request_seq":seq.to_string(),
                    "command":pending.command,"accepted_at_ns":pending.accepted_at.as_nanos().to_string(),
                    "accepted_recorded":pending.accepted_recorded})).collect::<Vec<_>>()})
        .to_string();
        if data.len() > 64 * 1024 {
            return Err(Error::InvalidConfiguration(
                "boundary snapshot credit exhausted",
            ));
        }
        Ok(BoundarySnapshot { at, data })
    }

    /// Submit the stop barrier only after the tracked Required controllers pause.
    pub fn stop_recording(&mut self) -> Result<(), Error> {
        self.stop_recording_at(self.last_now)
    }

    /// Seal at the actual owner admission time supplied by the public operation.
    pub fn stop_recording_at(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.last_now {
            return Err(Error::InvalidConfiguration("recording stop time regressed"));
        }
        // Move every already-committed Core fact into the Recorder FIFO before
        // reserving the seal. Otherwise a fact can remain behind the public stop
        // boundary merely because owner service has not reached its next drain.
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        if self.recording_policy == Some(RecordingPolicy::Required)
            && (self.controllers_active() || !self.outputs_safe_for_required_stop())
        {
            return Err(Error::InvalidConfiguration(
                "required recording safety unresolved",
            ));
        }
        let summary = self.frozen_stop_summary(at);
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_stop_with_summary(summary, at)
            .map_err(|_| Error::InvalidConfiguration("recording stop rejected"))?;
        if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.close_required_recording();
        }
        self.runtime.disable_recording_facts();
        self.recording_status = Some(worker.poll());
        Ok(())
    }

    /// Request a finite worker close after its already-accepted work.
    pub fn finish_recorder(&mut self) -> Result<(), Error> {
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_finish()
            .map_err(|_| Error::InvalidConfiguration("recording finish rejected"))?;
        self.recorder_finish_requested = true;
        Ok(())
    }

    /// Progress shutdown stop/close barriers without a worker join or SQL call.
    pub fn shutdown_recorder_step(&mut self, now: Duration) {
        self.poll_recorder(now);
        let Some(status) = self.recording_status.as_ref() else {
            return;
        };
        match status.state {
            RecordingState::Recording => {
                let summary = self.frozen_stop_summary(now);
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_stop_with_summary(summary, now).is_ok()
                {
                    self.runtime.close_required_recording();
                    self.runtime.disable_recording_facts();
                }
            }
            RecordingState::Idle if !self.recorder_finish_requested => {
                let summary = self.frozen_shutdown_evidence();
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_finish_with_summary(summary, now).is_ok()
                {
                    self.recorder_finish_requested = true;
                }
            }
            RecordingState::Idle => {}
            RecordingState::Failed
                if status.failure_persisted && !self.recorder_finish_requested =>
            {
                let summary = self.frozen_shutdown_evidence();
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_finish_with_summary(summary, now).is_ok()
                {
                    self.recorder_finish_requested = true;
                }
            }
            RecordingState::Starting
            | RecordingState::Stopping
            | RecordingState::Failed
            | RecordingState::Closed => {}
        }
        self.poll_recorder(now);
    }

    fn controllers_active(&self) -> bool {
        self.plan.controllers.iter().any(|(id, _, _)| {
            matches!(self.runtime.query(Query::Controller(*id)),
                Ok(QueryResult::Controller(snapshot)) if matches!(
                    snapshot.state, ControllerState::Warming | ControllerState::Running))
        })
    }

    // Required coverage can end only after every tracked authority has no
    // lease/pending send and Rust has verified the selected safe evidence.
    fn outputs_safe_for_required_stop(&self) -> bool {
        self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed
                    && snapshot.lease.is_none() && !snapshot.pending
                    && snapshot.in_flight.is_none())
        }) && self.resources.iter().all(|resource| {
            matches!(self.runtime.query(Query::Transport(*resource)),
                Ok(QueryResult::Transport(snapshot)) if snapshot.queue_len == 0
                    && snapshot.active.is_none())
        })
    }

    fn outputs_safe_for_transport_close(&self) -> bool {
        self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed
                    && snapshot.lease.is_none() && !snapshot.pending
                    && snapshot.in_flight.is_none())
        })
    }

    fn poll_recorder(&mut self, now: Duration) {
        let Some(worker) = self.recorder.as_mut() else {
            return;
        };
        let previous = self.recording_status.clone();
        let prior = previous.as_ref().map(|s| s.state);
        let status = worker.poll();
        let activated =
            prior != Some(RecordingState::Recording) && status.state == RecordingState::Recording;
        if activated {
            if self.recording_policy == Some(RecordingPolicy::Required)
                && let Some(submitted) = status.confirmed_submission
            {
                let _ = self.runtime.confirm_recording_start(submitted, now);
            }
        } else if status.state == RecordingState::Recording
            && self.recording_policy == Some(RecordingPolicy::Required)
            && let Some(submitted) = status.confirmed_submission
            && self
                .last_recording_submission
                .is_some_and(|old| submitted > old)
        {
            let _ = self.runtime.confirm_recording_progress(submitted, now);
        }
        if let Some(submitted) = status.confirmed_submission {
            self.last_recording_submission = Some(submitted);
        }
        if status.state == RecordingState::Failed
            && self.recording_policy == Some(RecordingPolicy::Required)
        {
            self.runtime.recording_failure(now);
        }
        if status.state == RecordingState::Failed {
            self.runtime.disable_recording_facts();
        }
        let recording = status.state == RecordingState::Recording;
        let publish_lifecycle = crate::recorder_api::lifecycle_changed(previous.as_ref(), &status);
        self.recording_status = Some(status);
        if publish_lifecycle {
            let event_boot_id = self.event_log().boot_id().to_owned();
            let data = crate::recorder_api::status_json(
                self.recording_status.as_ref(),
                self.recording_database_id(),
                self.recording_policy,
                &event_boot_id,
            );
            let archive_id = self.recording_database_id().map(str::to_owned);
            let _ = self
                .event_log_mut()
                .recorder_state(now, archive_id.as_deref(), data);
        }
        if recording {
            self.flush_deferred_safe_terminals(now);
            if let Some(worker) = self.recorder.as_mut() {
                let _ = worker.request_probe_at(now);
            }
        }
        if activated {
            self.flush_deferred_start_acceptance(now);
        }
    }

    fn flush_deferred_safe_terminals(&mut self, now: Duration) {
        let deferred = self.pending_operations.iter().find_map(|(key, pending)| {
            pending
                .deferred_safe_terminal
                .as_ref()
                .map(|terminal| (key.clone(), terminal.clone()))
        });
        let Some((key, terminal)) = deferred else {
            return;
        };
        let credit = self
            .recorder
            .as_mut()
            .and_then(|worker| worker.operation_credit_available(&terminal).ok())
            .unwrap_or(false);
        if !credit {
            return;
        }
        let admitted = self
            .recorder
            .as_mut()
            .is_some_and(|worker| worker.try_admit_operation(terminal).is_ok());
        if admitted {
            self.pending_operations.remove(&key);
        } else if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(now);
        }
    }

    // Start acceptance predates the new interval. The boundary records it as
    // pending; its small original intent enters the FIFO after committed Start.
    fn flush_deferred_start_acceptance(&mut self, now: Duration) {
        let deferred: Vec<_> = self
            .pending_operations
            .iter()
            .filter_map(|(key, pending)| {
                pending
                    .deferred_start
                    .as_ref()
                    .map(|operation| (key.clone(), operation.clone()))
            })
            .collect();
        for (key, operation) in deferred {
            let admitted = self
                .recorder
                .as_mut()
                .is_some_and(|worker| worker.try_admit_operation(operation).is_ok());
            if admitted {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.deferred_start = None;
                    pending.accepted_recorded = true;
                }
            } else if self.recording_policy == Some(RecordingPolicy::Required) {
                self.runtime.recording_failure(now);
            }
        }
    }

    fn admit_recording_facts(&mut self, now: Duration) {
        if self.recorder.is_none() {
            return;
        }
        let facts = self.runtime.take_recording_facts();
        let overflow = self.runtime.recording_facts_overflowed();
        let first_missing = self.runtime.recording_first_lost_fact();
        let suppressed = self.runtime.recording_suppressed_fact_count();
        let last_accepted = facts
            .last()
            .map(lab_core::recording::RecordingFact::sequence);
        if let Some(worker) = self.recorder.as_mut() {
            if !facts.is_empty()
                && worker.try_admit_at(facts, now).is_err()
                && self.recording_policy == Some(RecordingPolicy::Required)
            {
                self.runtime.recording_failure(now);
            }
            if overflow {
                worker.fail_with_gap(RecorderGap {
                    reason: "core fact outbox overflow".into(),
                    at: now,
                    first_missing_fact: first_missing,
                    known_missing_count: suppressed,
                    last_accepted_fact: last_accepted,
                });
                self.runtime.disable_recording_facts();
                if self.recording_policy == Some(RecordingPolicy::Required) {
                    self.runtime.recording_failure(now);
                }
            }
        }
        // Publish the just-latched gap before the next client Query or snapshot.
        // BestEffort may keep control running, but coverage must be truthful now.
        self.poll_recorder(now);
    }

    /// Install one trusted nonblocking component port before managed startup.
    pub fn install_component_executor(
        &mut self,
        executor: Box<dyn ComponentExecutor>,
    ) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        self.runtime.install_component_executor(executor)
    }

    /// Stage the fixed trusted source adapter; Core permits one staged init at a time.
    pub fn stage_standard_components(&mut self, at: Duration) -> Result<(), Error> {
        if !self.components.is_empty() {
            return Err(Error::InvalidConfiguration(
                "managed profile already staged",
            ));
        }
        let mut config = PlainData::default();
        config
            .fields
            .insert("baseline".into(), PlainValue::Number(20.0));
        config.fields.insert("rate".into(), PlainValue::Number(1.0));
        let manifest =
            |id: ComponentId, kind: ComponentKind, warmup_samples: usize| ComponentManifest {
                schema_version: 1,
                id,
                instrument: InstrumentId::new(id.get()),
                name: format!("Managed observation {}", id.get()),
                parameter: lab_core::TEMPERATURE,
                kind,
                unit: Unit::CELSIUS,
                min: -100.0,
                max: 500.0,
                warmup_samples,
                max_input_age: Duration::from_secs(2),
                history_capacity: 32,
            };
        self.runtime.command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest: manifest(MANAGED_SOURCE, ComponentKind::Source, 1),
                implementation: ComponentImplementation::text(
                    lab_lua::IMPLEMENTATION_ID,
                    lab_lua::fixtures::VIRTUAL_MODEL_SOURCE,
                )?,
                config,
            },
            replaces: None,
            at,
        })?;
        self.events.track_component(MANAGED_SOURCE);
        self.components.push((MANAGED_SOURCE, "source"));
        Ok(())
    }

    /// Stage the fixed one-input Transform only after Source init has committed.
    pub fn stage_standard_filter(&mut self, at: Duration) -> Result<(), Error> {
        if !self.source_component_initialized() {
            return Err(Error::InvalidConfiguration(
                "managed Source init incomplete",
            ));
        }
        let manifest = ComponentManifest {
            schema_version: 1,
            id: MANAGED_FILTER,
            instrument: InstrumentId::new(MANAGED_FILTER.get()),
            name: "Native moving mean".into(),
            parameter: lab_core::TEMPERATURE,
            kind: ComponentKind::Transform {
                input: SignalId::new(
                    InstrumentId::new(MANAGED_SOURCE.get()),
                    lab_core::TEMPERATURE,
                ),
            },
            unit: Unit::CELSIUS,
            min: -100.0,
            max: 500.0,
            warmup_samples: 3,
            max_input_age: Duration::from_secs(2),
            history_capacity: 32,
        };
        self.runtime.command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest,
                implementation: ComponentImplementation::built_in(MOVING_MEAN_IMPLEMENTATION)?,
                config: PlainData {
                    fields: BTreeMap::from([("window".into(), PlainValue::Number(3.0))]),
                },
            },
            replaces: None,
            at,
        })?;
        self.events.track_component(MANAGED_FILTER);
        self.components.push((MANAGED_FILTER, "transform"));
        Ok(())
    }
    /// The first real init result committed without any pending callback.
    pub fn source_component_initialized(&self) -> bool {
        matches!(self.runtime.query(Query::Component(MANAGED_SOURCE)),Ok(QueryResult::Component(s))
            if matches!(s.state,ComponentState::Warming|ComponentState::Ready) && s.pending.is_none())
    }

    /// True only after both real init callbacks have committed and no job waits.
    pub fn standard_components_initialized(&self) -> bool {
        [MANAGED_SOURCE,MANAGED_FILTER].iter().all(|id|matches!(self.runtime.query(Query::Component(*id)),
            Ok(QueryResult::Component(snapshot)) if matches!(snapshot.state,ComponentState::Warming|ComponentState::Ready) && snapshot.pending.is_none()))
    }

    /// Start trusted managed cadence after bounded startup init completes.
    pub fn activate_standard_components(&mut self, at: Duration) -> Result<(), Error> {
        if !self.standard_components_initialized() {
            return Err(Error::InvalidConfiguration("managed init incomplete"));
        }
        let mut source = Periodic::new(Duration::from_millis(200));
        source.next_due = at;
        self.plan.sources.push((MANAGED_SOURCE, source));
        self.plan.transforms.push((
            MANAGED_FILTER,
            SignalId::new(
                InstrumentId::new(MANAGED_SOURCE.get()),
                lab_core::TEMPERATURE,
            ),
        ));
        Ok(())
    }

    /// Fixed trusted component identities/kinds for discovery and typed status.
    pub fn component_catalog(&self) -> &[(ComponentId, &'static str)] {
        &self.components
    }

    /// Public semantic class for discovery; never exposes a Rust implementation type.
    pub fn instrument_kind(&self, id: InstrumentId) -> &'static str {
        if self.runtime.metakon_binding(id).is_some() {
            "physical"
        } else if self
            .components
            .iter()
            .any(|(component, _)| component.get() == id.get())
        {
            "managed"
        } else if self.model_generations.contains_key(&id) {
            "emulated"
        } else {
            "virtual"
        }
    }

    /// Current replacement fence used consistently by discovery and measurement DTOs.
    pub fn signal_generation(&self, signal: SignalId) -> u64 {
        if let Some((component, _)) = self
            .components
            .iter()
            .find(|(component, _)| component.get() == signal.instrument().get())
            && let Ok(QueryResult::Component(snapshot)) =
                self.runtime.query(Query::Component(*component))
        {
            snapshot.generation
        } else if let Some(binding) = self.runtime.metakon_binding(signal.instrument()) {
            binding.binding_generation
        } else {
            self.model_generations
                .get(&signal.instrument())
                .copied()
                .unwrap_or(1)
        }
    }

    /// Whether this exact virtual signal is deployment-authorized for API publication.
    pub fn emulator_writable(&self, signal: SignalId) -> bool {
        self.emulator_targets.contains(&signal)
    }

    /// Number of explicitly configured external-emulator targets.
    pub fn emulator_target_count(&self) -> usize {
        self.emulator_targets.len()
    }

    /// Number of Runtime-owned native virtual models with generation-fenced restart.
    pub fn virtual_model_count(&self) -> usize {
        self.model_generations.len()
    }

    /// Commit one external virtual observation through the ordinary owner path.
    pub fn publish_emulator_measurement(
        &mut self,
        signal: SignalId,
        value: Option<Value>,
        expected_generation: u64,
        at: Duration,
        cause: Option<(String, u64)>,
    ) -> Result<Sample, Error> {
        if !self.emulator_targets.contains(&signal) {
            return Err(Error::OperationNotAllowed(signal.parameter()));
        }
        self.command_with_cause(
            Command::PublishVirtualMeasurement {
                instrument: signal.instrument(),
                parameter: signal.parameter(),
                value,
                expected_generation,
                at,
            },
            cause,
        )?;
        match self.runtime.query(Query::GetLatestSignal(signal))? {
            QueryResult::Latest(Some(sample)) if sample.at() == at => Ok(sample),
            _ => Err(Error::InvalidConfiguration(
                "virtual publication did not commit",
            )),
        }
    }

    /// Add one trusted managed-input controller fixture after Transform init.
    /// It has a separate safe output; the independent native plant remains bound.
    pub fn add_managed_dependent_fixture(&mut self) -> Result<(), Error> {
        if !self.standard_components_initialized() {
            return Err(Error::InvalidConfiguration(
                "managed fixture requires committed Transform",
            ));
        }
        if self.outputs.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 output limit"));
        }
        self.runtime
            .command(Command::RegisterThermalPlant(ThermalPlantConfig {
                id: DEPENDENT_PLANT,
                name: "Managed-input thermal plant".into(),
                history_capacity: 64,
                ambient_temperature: 20.0,
                initial_temperature: 20.0,
                gain_per_percent: 0.8,
                time_constant: Duration::from_secs(8),
            }))?;
        let actuator = ActuatorId::new(DEPENDENT_PLANT, lab_core::HEATER_POWER);
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(2),
                max_proposal_ttl: Duration::from_millis(200),
                required_evidence: EvidenceLevel::Readback,
            }),
        })?;
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })?;
        let CommandResult::Output(OutputResult::Dispatched(safe)) =
            self.runtime.command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BeginDispatch,
            })?
        else {
            return Err(Error::InvalidConfiguration(
                "managed fixture safe dispatch absent",
            ));
        };
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::Complete {
                dispatch_id: safe.id(),
                outcome: DispatchOutcome::ReadbackVerified,
            },
        })?;
        self.outputs.push(actuator);
        self.runtime
            .command(Command::RegisterReference(ReferenceConfig::Fixed {
                id: DEPENDENT_REFERENCE,
                value: 60.0,
                unit: Unit::CELSIUS,
            }))?;
        self.events.track_reference(DEPENDENT_REFERENCE);
        self.runtime
            .command(Command::RegisterController(NativeControllerConfig {
                id: DEPENDENT_CONTROLLER,
                input: SignalId::new(
                    InstrumentId::new(MANAGED_FILTER.get()),
                    lab_core::TEMPERATURE,
                ),
                output: actuator,
                reference: DEPENDENT_REFERENCE,
                ema: EmaConfig {
                    time_constant: Duration::from_millis(200),
                    warmup_samples: 1,
                    unit: Unit::CELSIUS,
                },
                pid: PidConfig {
                    kp: 3.0,
                    ki: 0.4,
                    kd: 0.2,
                    output_min: 0.0,
                    output_max: 100.0,
                },
                max_input_age: Duration::from_millis(750),
                max_tick_gap: Duration::from_millis(750),
                lease_lifetime: Duration::from_secs(2),
                proposal_ttl: Duration::from_millis(200),
            }))?;
        self.runtime
            .command(Command::PrepareController(DEPENDENT_CONTROLLER))?;
        self.events.track_controller(DEPENDENT_CONTROLLER);
        self.plan
            .plants
            .push((DEPENDENT_PLANT, Periodic::new(Duration::from_millis(100))));
        self.plan.references.push((
            DEPENDENT_REFERENCE,
            Periodic::new(Duration::from_millis(100)),
        ));
        self.plan.controllers.push((
            DEPENDENT_CONTROLLER,
            SignalId::new(
                InstrumentId::new(MANAGED_FILTER.get()),
                lab_core::TEMPERATURE,
            ),
            Periodic::new(Duration::from_millis(200)),
        ));
        self.observe(self.last_now, None)?;
        Ok(())
    }
    /// Number of configured safe outputs in this trusted bounded host.
    pub fn configured_output_count(&self) -> usize {
        self.outputs.len()
    }
    /// Include a trusted registered output in the owner's bounded shutdown
    /// evidence set. This local composition seam grants no wire authority;
    /// registration and the safe profile must precede active service use.
    pub fn track_trusted_output(&mut self, actuator: ActuatorId) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        if self.outputs.contains(&actuator) {
            return Ok(());
        }
        if self.outputs.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 output limit"));
        }
        let QueryResult::Output(_) = self.runtime.query(Query::Output(actuator))? else {
            return Err(Error::InvalidConfiguration(
                "trusted output is not registered",
            ));
        };
        self.outputs.push(actuator);
        Ok(())
    }
    /// Install one trusted bounded M3 byte resource without raw TCP I/O.
    pub fn register_transport(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
    ) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        if self.resources.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 resource limit"));
        }
        self.runtime.register_transport(id, adapter)?;
        self.resources.push(id);
        Ok(())
    }
    /// Freeze bounded M3 resource status alongside facts without polling it.
    pub fn resource_records(&self) -> Vec<serde_json::Value> {
        self.resources.iter().filter_map(|id|match self.runtime.query(Query::Transport(*id)){
            Ok(QueryResult::Transport(s))=>Some(serde_json::json!({"kind":"resource","target":{"id":id.get().to_string()},
                "data":{"state":match s.state{ExecutorState::Idle=>"idle",ExecutorState::InFlight=>"in_flight",
                    ExecutorState::Recovering=>"recovering",ExecutorState::Offline=>"offline"},
                    "queue_len":s.queue_len,"generation":s.generation.to_string(),"active":s.active.map(|a|a.get().to_string()),
                    "latest":s.latest.map(|r|serde_json::json!({"id":r.id.get().to_string(),"outcome":match r.outcome{
                        TransactionOutcome::Completed=>"completed",TransactionOutcome::QueueExpired=>"queue_expired",TransactionOutcome::Failed=>"failed"},
                        "started":r.started,"generation":r.generation.to_string()}))}})),_=>None}).collect()
    }
    /// Retain per-output Rust authority/evidence facts through safe shutdown.
    pub fn output_safe_records(&self) -> Vec<serde_json::Value> {
        self.outputs.iter().map(|actuator|match self.runtime.query(Query::Output(*actuator)){
            Ok(QueryResult::Output(s))=>serde_json::json!({"instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"safe_confirmed":s.safe_confirmed,
                "lease_present":s.lease.is_some(),"fault_latched":s.fault_latched,
                "readback":s.readback.map(|r|serde_json::json!({"value":r.value,"at":r.at.as_nanos().to_string()})),
                "state":match s.state{lab_core::output::OutputState::Unverified=>"unverified",
                    lab_core::output::OutputState::SafePending=>"safe_pending",lab_core::output::OutputState::Disarmed=>"disarmed",
                    lab_core::output::OutputState::ArmedManual=>"armed_manual",lab_core::output::OutputState::ArmedAuto=>"armed_auto",
                    lab_core::output::OutputState::FaultLatched=>"fault_latched"}}),
            _=>serde_json::json!({"instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"safe_confirmed":false,"state":"unknown"})}).collect()
    }

    // This snapshot is taken only after the safety grace has resolved; it
    // reports unknown evidence as unknown and cannot confirm physical safety.
    fn frozen_shutdown_evidence(&self) -> serde_json::Value {
        let controllers: Vec<_> = self
            .plan
            .controllers
            .iter()
            .map(
                |(id, _, _)| match self.runtime.query(Query::Controller(*id)) {
                    Ok(QueryResult::Controller(snapshot)) => serde_json::json!({
                        "id":id.get().to_string(),
                        "state":format!("{:?}",snapshot.state),
                    }),
                    _ => serde_json::json!({"id":id.get().to_string(),"state":"unknown"}),
                },
            )
            .collect();
        serde_json::json!({
            "outputs":self.output_safe_records(),
            "resources":self.resource_records(),
            "controllers":controllers,
            "unfinished_managed_workers":self.runtime.unfinished_component_workers(),
        })
    }

    // Pending identities are already accepted facts, never predictions of
    // their terminal domain or physical outcome.
    fn frozen_stop_summary(&self, at: Duration) -> serde_json::Value {
        let pending:Vec<_>=self.pending_operations.iter().map(|((scope,seq),operation)|
            serde_json::json!({"scope":scope,"request_seq":seq.to_string(),
                "command":operation.command,"accepted_at_ns":operation.accepted_at.as_nanos().to_string(),
                "accepted_recorded":operation.accepted_recorded})).collect();
        serde_json::json!({"requested_stop_at_ns":at.as_nanos().to_string(),
            "pending_operations":pending})
    }

    /// Fence producers before Rust begins a new required safe procedure.
    pub fn begin_shutdown(&mut self, clock: &impl Clock) -> Result<(), Error> {
        if self.stopping {
            return Ok(());
        }
        self.stopping = true;
        self.runtime
            .command(Command::QuiesceManaged { at: clock.now() })?;
        self.observe(clock.now(), None)?;
        let identities: Vec<_> = self.plan.controllers.iter().map(|(id, _, _)| *id).collect();
        for id in identities {
            let QueryResult::Controller(snapshot) = self.runtime.query(Query::Controller(id))?
            else {
                unreachable!()
            };
            if matches!(
                snapshot.state,
                ControllerState::Warming | ControllerState::Running
            ) {
                self.runtime.command(Command::PauseController {
                    controller: id,
                    at: clock.now(),
                })?;
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
            }
        }
        for actuator in self.outputs.clone() {
            self.runtime.command(Command::Output {
                actuator,
                command: OutputCommand::RequestSafe,
                at: clock.now(),
            })?;
            self.observe(clock.now(), None)?;
        }
        self.plan.safety.next_due = clock.now();
        Ok(())
    }

    /// Copy output evidence and worker cleanup status without progressing either.
    pub fn shutdown_status(&self) -> ShutdownStatus {
        let safe_confirmed=self.outputs.iter().all(|actuator|matches!(self.runtime.query(Query::Output(*actuator)),
            Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed && snapshot.lease.is_none()));
        let unfinished_workers = self.runtime.unfinished_component_workers();
        let unfinished_transports = self
            .resources
            .len()
            .saturating_sub(self.closed_resources.len());
        let transports_closed = unfinished_transports == 0;
        let recorder_state = self.recording_status.as_ref().map(|status| status.state);
        let recorder_flushed = self.recording_status.as_ref().is_none_or(|status| {
            status.state == RecordingState::Closed
                && status.terminal_seal_committed
                && status.first_error.is_none()
                && status.outstanding_records == 0
        });
        let recorder_error = recorder_state == Some(RecordingState::Failed);
        let recorder_unfinished = self
            .recording_status
            .as_ref()
            .is_some_and(|status| !status.worker_closed);
        ShutdownStatus {
            safe_confirmed,
            unfinished_workers,
            unfinished_transports,
            transports_closed,
            fatal_error: false,
            recorder_flushed,
            recorder_unfinished,
            recorder_error,
            exit_success: safe_confirmed
                && unfinished_workers == 0
                && transports_closed
                && recorder_flushed
                && !recorder_error,
        }
    }

    /// Replace a schedule only through a trusted local composition decision.
    /// Network requests cannot access this seam or choose controller cadence.
    pub fn replace_plan(&mut self, plan: SchedulePlan) -> Result<(), Error> {
        if plan.plants.len() > 8
            || plan.metakon_reads.len() > 64
            || plan.references.len() > 8
            || plan.controllers.len() > 8
            || plan.sources.len() + plan.transforms.len() > 8
        {
            return Err(Error::InvalidConfiguration("M6 host schedule limit"));
        }
        self.plan = plan;
        Ok(())
    }

    /// Serialize a local domain Command on this owner; snapshots are separate.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        self.command_with_cause(command, None)
    }

    /// Serialize one command and publish resulting facts with optional causing ID.
    pub fn command_with_cause(
        &mut self,
        command: Command,
        cause: Option<(String, u64)>,
    ) -> Result<CommandResult, Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        self.poll_recorder(self.last_now);
        let start = match &command {
            Command::StartController { controller, .. }
            | Command::ResumeController { controller, .. } => Some(*controller),
            _ => None,
        };
        let profile_binding = match &command {
            Command::Output {
                actuator,
                command: OutputCommand::BindProfile(profile),
                ..
            } => Some((*actuator, profile.clone())),
            _ => None,
        };
        let outcome = self.runtime.command(command);
        if outcome.is_ok()
            && let Some((actuator, profile)) = profile_binding
        {
            if let Some((_, active)) = self
                .active_safety_profiles
                .iter_mut()
                .find(|(id, _)| *id == actuator)
            {
                *active = profile;
            } else {
                self.active_safety_profiles.push((actuator, profile));
                self.outputs.push(actuator);
            }
        }
        let at = self.last_now;
        self.observe(at, cause.as_ref().map(|(s, n)| (s.as_str(), *n)))?;
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        let outcome = outcome?;
        if let Some(controller) = start
            && let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(controller))?
        {
            let QueryResult::Latest(sample) =
                self.runtime.query(Query::GetLatestSignal(config.input))?
            else {
                unreachable!()
            };
            self.consumed.insert(controller, sample);
        }
        Ok(outcome)
    }

    /// Borrow the owner-local bounded semantic publication ring.
    pub const fn event_log(&self) -> &EventLog {
        &self.events
    }
    /// Borrow the ring mutably only from the serialized service owner.
    pub fn event_log_mut(&mut self) -> &mut EventLog {
        &mut self.events
    }
    /// Install the OS process identity before listener readiness.
    pub fn set_boot_id(&mut self, boot: &str) {
        self.events.set_boot_id(boot);
    }
    fn observe(&mut self, at: Duration, cause: Option<(&str, u64)>) -> Result<(), Error> {
        self.events
            .observe(&self.runtime, at, cause)
            .map_err(event_domain_error)
    }

    /// Return an owned pure domain snapshot without polling clocks or workers.
    pub fn query(&self, query: Query) -> Result<QueryResult, Error> {
        self.runtime.query(query)
    }

    /// Drive one bounded turn, checking safety before lower-priority units.
    pub fn service(&mut self, clock: &impl Clock) -> Result<ServiceReport, Error> {
        let now = clock.now();
        self.poll_recorder(now);
        let result = self.service_inner(clock);
        self.admit_recording_facts(clock.now());
        self.poll_recorder(clock.now());
        result
    }

    fn service_inner(&mut self, clock: &impl Clock) -> Result<ServiceReport, Error> {
        let mut report = ServiceReport::default();
        let now = clock.now();
        if now < self.last_now {
            return Err(Error::InvalidConfiguration(
                "scheduler clock moved backward",
            ));
        }
        self.last_now = now;
        if let Some(skipped) = self.plan.safety.take(now)? {
            self.runtime.command(if self.stopping {
                Command::ServiceSafety { at: clock.now() }
            } else {
                Command::PollComponents { at: clock.now() }
            })?;
            self.observe(clock.now(), None)?;
            report.safety += 1;
            report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
        }
        if self.stopping && self.outputs_safe_for_transport_close() {
            for resource in self.resources.clone() {
                if self.closed_resources.contains(&resource) {
                    continue;
                }
                if self.runtime.shutdown_transport(resource, clock.now())?
                    == lab_core::transport::TransportShutdown::Complete
                {
                    self.closed_resources.insert(resource);
                }
            }
        }
        if self.stopping {
            return Ok(report);
        }
        if self.configuration_quiesced {
            return Ok(report);
        }
        for (plant, slot) in &mut self.plan.plants {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                let outcome = self.runtime.command(Command::RefreshMeasurement {
                    instrument: *plant,
                    parameter: lab_core::TEMPERATURE,
                    at: clock.now(),
                });
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                if let Err(error) = outcome
                    && !matches!(error, Error::MeasurementUnavailable { .. })
                {
                    return Err(error);
                }
                report.measurements += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for read in &mut self.plan.metakon_reads {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if self
                .runtime
                .metakon_binding(read.instrument)
                .is_some_and(|binding| {
                    self.reconnect_quiesced_resources
                        .contains(&binding.resource)
                })
            {
                continue;
            }
            if let Some(skipped) = read.slot.take(now)? {
                let outcome = self.runtime.command(Command::QueueMetakonRead {
                    instrument: read.instrument,
                    parameter: read.parameter,
                    at: clock.now(),
                    queue_ttl: read.queue_ttl,
                    timeout: read.timeout,
                });
                if let Err(error) = outcome
                    && !matches!(error, Error::Transport(_))
                {
                    return Err(error);
                }
                report.measurements += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for (reference, slot) in &mut self.plan.references {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                self.runtime.command(Command::EvaluateReference {
                    reference: *reference,
                    at: clock.now(),
                })?;
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                report.references += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for (controller, input, slot) in &mut self.plan.controllers {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
                let QueryResult::Controller(state) =
                    self.runtime.query(Query::Controller(*controller))?
                else {
                    unreachable!()
                };
                if !matches!(
                    state.state,
                    ControllerState::Warming | ControllerState::Running
                ) {
                    continue;
                }
                let QueryResult::Latest(latest) =
                    self.runtime.query(Query::GetLatestSignal(*input))?
                else {
                    unreachable!()
                };
                if latest
                    .as_ref()
                    .is_some_and(|sample| sample.quality() == SampleQuality::Good)
                    && latest != self.consumed.get(controller).cloned().flatten()
                {
                    self.runtime.command(Command::TickController {
                        controller: *controller,
                        at: clock.now(),
                    })?;
                    self.events
                        .observe(&self.runtime, clock.now(), None)
                        .map_err(event_domain_error)?;
                    self.consumed.insert(*controller, latest);
                    report.controller_ticks += 1;
                }
            }
        }
        for (source, slot) in &mut self.plan.sources {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
                let status = self.runtime.query(Query::Component(*source));
                if !matches!(status,Ok(QueryResult::Component(s)) if s.state!=ComponentState::Failed && s.pending.is_none())
                {
                    continue;
                }
                let outcome = self.runtime.command(Command::InvokeComponent {
                    component: *source,
                    at: clock.now(),
                });
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                if let Err(error) = outcome
                    && !matches!(
                        error,
                        Error::Component(ComponentError::Busy | ComponentError::InputUnavailable)
                    )
                {
                    return Err(error);
                }
            }
        }
        for (transform, input) in &self.plan.transforms {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            let status = self.runtime.query(Query::Component(*transform));
            if !matches!(status,Ok(QueryResult::Component(s)) if s.state!=ComponentState::Failed && s.pending.is_none())
            {
                continue;
            }
            let latest = match self.runtime.query(Query::GetLatestSignal(*input))? {
                QueryResult::Latest(s) => s,
                _ => unreachable!(),
            };
            if latest.is_none() || latest == self.consumed_managed.get(transform).cloned().flatten()
            {
                continue;
            }
            let outcome = self.runtime.command(Command::InvokeComponent {
                component: *transform,
                at: clock.now(),
            });
            self.events
                .observe(&self.runtime, clock.now(), None)
                .map_err(event_domain_error)?;
            if let Err(error) = outcome
                && !matches!(
                    error,
                    Error::Component(ComponentError::Busy | ComponentError::InputUnavailable)
                )
            {
                return Err(error);
            }
            self.consumed_managed.insert(*transform, latest);
        }
        Ok(report)
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
