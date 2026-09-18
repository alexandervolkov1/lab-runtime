//! Process lifecycle around the serialized [`crate::host::HostCore`] owner.
//!
//! [`crate::service::ServiceHost`] owns the system clock, loopback listener, deployment lifecycle,
//! resource reconnect candidates and finite shutdown coordination. It orchestrates
//! external workers/adapters and Runtime progress through `HostCore`; it does not own
//! a second copy of experiment state. Entropy failure, malformed configuration,
//! failed safe profile or failed startup Recorder/transport work prevents readiness.
//! The network reactor remains a separate adapter.

mod configuration;
mod reconnect;
mod shutdown;

use crate::recorder::{
    ConfigurationLifecycleRecord, RecorderLimits, RecorderWorker, RecordingPolicy, TimeAnchor,
};
use crate::{
    configuration::{
        FlowControlDto, ParityDto, PropertyValue, RecordingPolicyDto, load_runtime_toml,
    },
    deployment::{
        ApplyError, ApplyPort, ApplyResult, DeploymentLifecycle, StageError, StagedConfiguration,
    },
    host::{Clock, HostCore, ShutdownStatus, SystemClock},
    managed_executor::ManagedExecutor,
    protocol::ProtocolFeatures,
    serial::{
        ComOpenStatus, ComSettings, ComState, ComTransport, SerialError, SerialFlowControl,
        SerialParity,
    },
};
use lab_core::managed::ComponentError;
use lab_core::{
    Error as DomainError,
    transport::{ByteTransport, ExecutorState, ResourceId},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener},
    path::PathBuf,
};

/// Local SQLite recording configuration selected before Runtime readiness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingOptions {
    /// Absolute local database path; UNC/network paths are not accepted in M7.
    pub path: PathBuf,
    /// Stable startup policy; clients cannot switch it during a run.
    pub policy: RecordingPolicy,
}

/// Strict virtual-only service options; default binary execution remains finite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceOptions {
    port: u16,
    recording: Option<RecordingOptions>,
    config: Option<PathBuf>,
}
impl ServiceOptions {
    /// Accept the fixed virtual profile, loopback port, and optional local Recorder.
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        if args.len() == 3 && args[0] == "--serve" && args[1] == "--config" {
            if args[2].is_empty() {
                return Err("configuration path must not be empty".into());
            }
            return Ok(Self {
                port: 0,
                recording: None,
                config: Some(PathBuf::from(args[2])),
            });
        }
        if !matches!(args.len(), 5 | 7 | 9)
            || args[0] != "--serve"
            || args[1] != "--profile"
            || args[2] != "virtual-demo"
            || args[3] != "--port"
        {
            return Err("expected --serve --profile virtual-demo --port <0..65535> [--record-db <absolute-local-path> [--record-policy required|best-effort]]".into());
        }
        let port = args[4]
            .parse::<u16>()
            .map_err(|_| "port must be an integer in 0..65535".to_string())?;
        let recording = if args.len() > 5 {
            if args[5] != "--record-db" || args[6].is_empty() {
                return Err("recording policy requires a database path".into());
            }
            let path = PathBuf::from(args[6]);
            if !path.is_absolute() || args[6].starts_with("\\\\") || args[6].starts_with("//") {
                return Err("recording database path must be local and absolute".into());
            }
            let policy = if args.len() == 9 {
                if args[7] != "--record-policy" {
                    return Err("unknown recording option".into());
                }
                match args[8] {
                    "required" => RecordingPolicy::Required,
                    "best-effort" => RecordingPolicy::BestEffort,
                    _ => return Err("unknown recording policy".into()),
                }
            } else {
                RecordingPolicy::Required
            };
            Some(RecordingOptions { path, policy })
        } else {
            None
        };
        Ok(Self {
            port,
            recording,
            config: None,
        })
    }

    /// Requested loopback TCP port; zero delegates selection to the OS.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Selected Recorder path/policy, if durability is enabled for this host.
    pub fn recording(&self) -> Option<&RecordingOptions> {
        self.recording.as_ref()
    }

    /// Selected declarative deployment path, if profile startup is not used.
    pub fn configuration_path(&self) -> Option<&std::path::Path> {
        self.config.as_deref()
    }
}

/// Safe owner and bound listener; neither socket nor serialization enters Core.
pub struct ServiceHost {
    host: HostCore,
    clock: SystemClock,
    listener: TcpListener,
    bound: SocketAddr,
    boot_id: String,
    stopping_since: Option<std::time::Instant>,
    safe_since: Option<std::time::Instant>,
    recorder_flush_since: Option<std::time::Instant>,
    terminal: Option<ShutdownStatus>,
    fatal: bool,
    deployment: Option<DeploymentLifecycle>,
    configuration_path: Option<PathBuf>,
    next_lifecycle_operation: u64,
    reconnect_diagnostic: Option<ReconnectDiagnostic>,
    quarantined_reconnect_candidate: Option<(ResourceId, ComTransport)>,
}

/// Bounded lifecycle-operation failure exposed without leaking filesystem details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleOperationError {
    /// This process was started with the legacy compiled profile.
    ConfigurationDisabled,
    /// Candidate loading or validation failed before active mutation.
    InvalidCandidate,
    /// The one staged-candidate slot or revision fence rejected the request.
    Conflict,
    /// The requested diff needs a lifecycle not supported by this operation.
    RequiresSafeBarrier,
    /// No managed component exists in the active deployment.
    NoManagedComponents,
    /// The authoritative owner rejected the model transition.
    OwnerFailure,
    /// Required lifecycle provenance could not be admitted or confirmed.
    RecordingUnavailable,
    /// A bounded transport retire/open/prepare step could not establish a replacement.
    TransportUnavailable,
}

/// Bounded stages of one explicit configured-resource reconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconnectStage {
    /// A safety barrier was evaluated; resource-only read reconnect requires none.
    SafeBarrier,
    /// Recorder lifecycle capacity and activation generation are being reserved.
    RecorderReservation,
    /// Retirement was requested from the old executor/worker.
    RetireOldBegin,
    /// The old executor has not yet proved worker/handle completion.
    RetireOldPending,
    /// The finite old-retirement deadline expired.
    RetireOldTimeout,
    /// The old executor rejected retirement progress before its deadline.
    RetireOldFailed,
    /// Candidate serial settings are being validated and frozen.
    ReplacementSettings,
    /// The bounded replacement worker is being spawned.
    ReplacementWorkerSpawn,
    /// The worker exists and the actual Windows port open/readback is pending.
    ActualPortOpening,
    /// The actual Windows port open/readback failed or timed out.
    ReplacementPortOpenFailed,
    /// The actual OS port open and configured-settings readback were confirmed.
    ReplacementPortReady,
    /// The ready candidate is being transferred to the authoritative owner.
    ReplacementInstall,
    /// Core rebind/generation fencing has been crossed.
    CoreRebind,
    /// The resource-specific compatibility probe is being enqueued.
    CompatibilityProbeEnqueue,
    /// The resource-specific compatibility probe is pending.
    ProbeWaiting,
    /// The resource-specific compatibility probe failed or timed out.
    ProbeFailed,
    /// The applied lifecycle is being enqueued to Recorder.
    LifecycleRecorderCommit,
    /// The exact lifecycle receipt is awaiting durable confirmation.
    LifecycleDurability,
    /// Probe, lifecycle durability and ordinary-acquisition release completed.
    Complete,
}

impl ReconnectStage {
    /// Stable lower-snake-case representation used by public diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SafeBarrier => "safe_barrier",
            Self::RecorderReservation => "recorder_reservation",
            Self::RetireOldBegin => "retire_old_begin",
            Self::RetireOldPending => "retire_old_pending",
            Self::RetireOldTimeout => "retire_old_timeout",
            Self::RetireOldFailed => "retire_old_failed",
            Self::ReplacementSettings => "replacement_settings",
            Self::ReplacementWorkerSpawn => "replacement_worker_spawn",
            Self::ActualPortOpening => "actual_windows_port_opening",
            Self::ReplacementPortOpenFailed => "replacement_port_open_failed",
            Self::ReplacementPortReady => "replacement_port_ready",
            Self::ReplacementInstall => "replacement_install",
            Self::CoreRebind => "core_rebind",
            Self::CompatibilityProbeEnqueue => "compatibility_probe_enqueue",
            Self::ProbeWaiting => "probe_waiting",
            Self::ProbeFailed => "probe_failed_or_timeout",
            Self::LifecycleRecorderCommit => "lifecycle_recorder_commit",
            Self::LifecycleDurability => "lifecycle_durability",
            Self::Complete => "complete",
        }
    }
}

/// One replace-in-place diagnostic; no OS error string or unbounded history is retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconnectDiagnostic {
    /// Stable logical resource identity.
    pub resource_id: u64,
    /// Binding generation visible before the reconnect.
    pub current_generation: u64,
    /// Checked generation intended for the replacement.
    pub target_generation: u64,
    /// Latest reached or failed bounded stage.
    pub stage: ReconnectStage,
    /// Latest bounded COM worker state when a candidate existed.
    pub com_state: Option<ComState>,
    /// Latest typed serial error class, without an OS-owned string.
    pub serial_error: Option<SerialError>,
    /// Whether the old worker/adapter retirement boundary completed.
    pub old_worker_finished: bool,
    /// Whether the replacement worker thread was spawned.
    pub replacement_worker_spawned: bool,
    /// Whether actual OS port open plus configured settings readback completed.
    pub os_port_open_confirmed: bool,
    /// Open attempts performed by the one bounded candidate worker.
    pub open_attempts: usize,
    /// Whether Core installed the replacement and crossed the generation fence.
    pub core_rebind_crossed: bool,
}

impl ReconnectDiagnostic {
    /// Return a bounded wire/storage summary with stable enum spellings.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "resource": self.resource_id.to_string(),
            "current_generation": self.current_generation.to_string(),
            "target_generation": self.target_generation.to_string(),
            "stage": self.stage.as_str(),
            "com_state": self.com_state.map(com_state_name),
            "serial_error": self.serial_error.map(serial_error_name),
            "old_worker_finished": self.old_worker_finished,
            "replacement_worker_spawned": self.replacement_worker_spawned,
            "os_port_open_confirmed": self.os_port_open_confirmed,
            "open_attempts": self.open_attempts,
            "core_rebind_crossed": self.core_rebind_crossed,
        })
    }
}

const fn com_state_name(state: ComState) -> &'static str {
    match state {
        ComState::Opening => "opening",
        ComState::Online => "online",
        ComState::Offline => "offline",
        ComState::Closing => "closing",
        ComState::Closed => "closed",
    }
}

const fn serial_error_name(error: SerialError) -> &'static str {
    match error {
        SerialError::InvalidSettings => "invalid_settings",
        SerialError::Disconnected => "disconnected",
        SerialError::Timeout => "timeout",
        SerialError::Other => "other",
    }
}

const fn executor_com_state(state: ExecutorState) -> ComState {
    match state {
        ExecutorState::Idle | ExecutorState::InFlight => ComState::Online,
        ExecutorState::Recovering | ExecutorState::Offline => ComState::Offline,
    }
}

/// Successful configuration activation identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReloadConfigurationResult {
    /// Newly committed nonzero configuration revision.
    pub revision: u64,
}

/// Successful native-model restart summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestartModelsResult {
    /// Number of native models reset in this bounded operation.
    pub models: usize,
    /// Latest generation committed by this operation.
    pub generation: u64,
}

/// Successful explicit read-only resource reconnect summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectResourceResult {
    /// Stable logical resource identity retained across the new handle session.
    pub resource_id: u64,
    /// New physical binding generation fencing all old bytes/results.
    pub binding_generation: u64,
}

struct PendingRecordedLifecycle {
    record: ConfigurationLifecycleRecord,
    generation: Option<u64>,
    reopen_required: bool,
    submitted_at: std::time::Duration,
    global_quiesced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordedLifecycleFailureStage {
    Commit,
    Durability,
}

struct LiveApplyPort<'a> {
    host: &'a mut HostCore,
    active: &'a crate::configuration::FrozenDeployment,
    at: std::time::Duration,
    recording_fence: Option<(bool, std::time::Duration)>,
    clock: SystemClock,
    operation_id: u64,
    operation_kind: &'static str,
    base_revision: u64,
    activation_generation: Option<Option<u64>>,
    postcommit_recording_failure: bool,
    prepared_bindings: BTreeMap<ResourceId, Box<dyn ByteTransport>>,
}
impl LiveApplyPort<'_> {
    fn ensure_recording_fence(&mut self, at: std::time::Duration) {
        if self.recording_fence.is_none() {
            self.recording_fence = Some((self.host.begin_configuration_recording_fence(at), at));
        }
    }
}
impl ApplyPort for LiveApplyPort<'_> {
    fn enter_safe_barrier(&mut self, at: std::time::Duration) -> Result<bool, ApplyError> {
        self.at = at;
        self.ensure_recording_fence(at);
        self.host
            .enter_configuration_safe_barrier(at)
            .map_err(|_| ApplyError::OwnerFailure)
    }

    fn prepare_bindings(
        &mut self,
        candidate: &crate::configuration::FrozenDeployment,
    ) -> Result<(), ApplyError> {
        self.host.begin_configuration_quiesce();
        let result = (|| {
            let old = &self.active.effective().dto;
            let new = &candidate.effective().dto;
            let mut affected = BTreeSet::new();
            for (old_resource, new_resource) in old.resources.iter().zip(&new.resources) {
                if old_resource != new_resource {
                    affected.insert(ResourceId::new(old_resource.id));
                    affected.insert(ResourceId::new(new_resource.id));
                }
            }
            for (old_instrument, new_instrument) in old.instruments.iter().zip(&new.instruments) {
                if old_instrument == new_instrument {
                    continue;
                }
                if let crate::configuration::InstrumentDto::Metakon { resource_id, .. } =
                    old_instrument
                {
                    affected.insert(ResourceId::new(*resource_id));
                }
                if let crate::configuration::InstrumentDto::Metakon { resource_id, .. } =
                    new_instrument
                {
                    affected.insert(ResourceId::new(*resource_id));
                }
            }
            for resource_id in affected {
                let resource = new
                    .resources
                    .iter()
                    .find(|resource| resource.id == resource_id.get())
                    .ok_or(ApplyError::OwnerFailure)?;
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(resource.recovery_timeout_ms);
                loop {
                    match self
                        .host
                        .prepare_configured_transport_replacement(resource_id, self.clock.now())
                    {
                        Ok(true) => break,
                        Ok(false) if std::time::Instant::now() < deadline => {
                            self.host
                                .service(&self.clock)
                                .map_err(|_| ApplyError::OwnerFailure)?;
                            std::thread::yield_now();
                        }
                        _ => return Err(ApplyError::OwnerFailure),
                    }
                }
                let generation = self
                    .host
                    .configured_resource_generation(resource_id)
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(ApplyError::OwnerFailure)?;
                let adapter = ComTransport::open_windows(
                    com_settings(resource, generation).map_err(|_| ApplyError::OwnerFailure)?,
                )
                .map_err(|_| ApplyError::OwnerFailure)?;
                self.prepared_bindings
                    .insert(resource_id, Box::new(adapter));
            }
            Ok(())
        })();
        if result.is_err() {
            self.host.end_configuration_quiesce();
        }
        result
    }

    fn commit_configuration(
        &mut self,
        candidate: &crate::configuration::FrozenDeployment,
    ) -> Result<(), ApplyError> {
        self.at = self.clock.now();
        self.ensure_recording_fence(self.at);
        self.host.begin_configuration_quiesce();
        let lifecycle = ConfigurationLifecycleRecord {
            operation_id: self.operation_id,
            operation_kind: self.operation_kind,
            base_revision: self.base_revision,
            committed_revision: self
                .base_revision
                .checked_add(1)
                .ok_or(ApplyError::OwnerFailure)?,
            toml_hash: candidate.toml_hash(),
            affected: configuration_affected(
                candidate,
                self.base_revision,
                self.base_revision.saturating_add(1),
            ),
            reason: None,
            at: self.at,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let reservation = loop {
            match self
                .host
                .try_reserve_configuration_activation(&lifecycle, self.clock.now())
                .map_err(|_| ApplyError::OwnerFailure)?
            {
                Some(reservation) => break reservation,
                None if std::time::Instant::now() < deadline => {
                    self.host
                        .service(&self.clock)
                        .map_err(|_| ApplyError::OwnerFailure)?;
                    std::thread::yield_now();
                }
                None => {
                    self.host.end_configuration_quiesce();
                    return Err(ApplyError::OwnerFailure);
                }
            }
        };
        let prepared = std::mem::take(&mut self.prepared_bindings);
        let mut commit_failed = false;
        for (resource, adapter) in prepared {
            if self
                .host
                .rebind_configured_transport_from_configuration(
                    candidate,
                    resource,
                    adapter,
                    self.clock.now(),
                )
                .is_err()
            {
                commit_failed = true;
                break;
            }
        }
        if !commit_failed
            && self
                .host
                .apply_configuration(self.active, candidate, self.clock.now())
                .is_err()
        {
            commit_failed = true;
        }
        if !commit_failed
            && !candidate.effective().dto.resources.is_empty()
            && self.host.begin_configured_probes(self.clock.now()).is_err()
        {
            commit_failed = true;
        }
        if commit_failed {
            let _ = self.host.cancel_configuration_activation(reservation);
            self.host.end_configuration_quiesce();
            return Err(ApplyError::OwnerFailure);
        }
        self.activation_generation = Some(reservation);
        if self
            .host
            .commit_reserved_configuration_activation(reservation, lifecycle)
            .is_err()
        {
            self.host.configuration_recording_failed(self.clock.now());
            self.host.end_configuration_quiesce();
            self.postcommit_recording_failure = true;
        } else if reservation.is_none() {
            self.host.end_configuration_quiesce();
        }
        Ok(())
    }
}

fn configuration_affected(
    candidate: &crate::configuration::FrozenDeployment,
    base_revision: u64,
    committed: u64,
) -> Vec<String> {
    let dto = &candidate.effective().dto;
    let mut affected = Vec::with_capacity(
        dto.resources.len()
            + dto.instruments.len()
            + dto.managed_components.len()
            + dto.references.len()
            + dto.controllers.len()
            + dto.safe_profiles.len(),
    );
    affected.extend(dto.resources.iter().map(|item| {
        format!(
            "resource:{}:deployment:{base_revision}->{committed}",
            item.id
        )
    }));
    affected.extend(dto.instruments.iter().map(|item| {
        format!(
            "instrument:{}:deployment:{base_revision}->{committed}",
            item.id()
        )
    }));
    affected.extend(dto.managed_components.iter().map(|item| {
        format!(
            "component:{}:deployment:{base_revision}->{committed}",
            item.id
        )
    }));
    affected.extend(dto.references.iter().map(|item| {
        format!(
            "reference:{}:deployment:{base_revision}->{committed}",
            item.id
        )
    }));
    affected.extend(dto.controllers.iter().map(|item| {
        format!(
            "controller:{}:deployment:{base_revision}->{committed}",
            item.id
        )
    }));
    affected.extend(dto.safe_profiles.iter().map(|item| {
        format!(
            "actuator:{}:{}:deployment:{base_revision}->{committed}",
            item.instrument_id, item.parameter_id
        )
    }));
    affected
}
impl ServiceHost {
    /// Actual optional protocol domains supported by this frozen composition.
    pub fn protocol_features(&self) -> ProtocolFeatures {
        let deployment = self.deployment.as_ref().map(DeploymentLifecycle::active);
        ProtocolFeatures {
            recorder: self.host.recording_status().is_some(),
            configuration: deployment.is_some(),
            resource_reconnect: deployment
                .is_some_and(|active| !active.effective().dto.resources.is_empty()),
            emulator_publication: self.host.emulator_target_count() != 0,
            virtual_model_lifecycle: self.host.virtual_model_count() != 0,
        }
    }

    /// Bind a trusted already-safe host fixture on loopback, without changing its
    /// Core composition. This is local test/deployment wiring, never a wire op.
    pub fn startup_from_trusted_host(
        options: ServiceOptions,
        mut host: HostCore,
    ) -> Result<Self, Box<dyn Error>> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|e| io::Error::other(format!("OS boot entropy unavailable: {e}")))?;
        let boot_id = host
            .recording_boot_id()
            .map(str::to_owned)
            .unwrap_or_else(|| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        host.set_boot_id(&boot_id);
        if !host.shutdown_status().safe_confirmed {
            return Err(io::Error::other("fixture safe evidence unavailable").into());
        }
        let clock = SystemClock::new();
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, options.port()))?;
        listener.set_nonblocking(true)?;
        if let Some(recording) = options.recording() {
            let anchor = TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now()))?;
            let worker = RecorderWorker::open_with_boot_clock(
                &recording.path,
                RecorderLimits::default(),
                &boot_id,
                anchor,
                clock,
            )?;
            host.attach_recorder(worker, recording.policy, clock.now())?;
        }
        await_recorder_activation(&mut host)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            host,
            clock,
            listener,
            bound,
            boot_id,
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: None,
            configuration_path: None,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
        })
    }
    /// Validate identity/profile, then bind only IPv4 loopback in that order.
    pub fn startup(options: ServiceOptions) -> Result<Self, Box<dyn Error>> {
        let configuration_path = options.configuration_path().map(PathBuf::from);
        let loaded = options
            .configuration_path()
            .map(load_runtime_toml)
            .transpose()?;
        let (port, configured_recording) = if let Some(deployment) = loaded.as_ref() {
            let dto = &deployment.effective().dto;
            let recording = if dto.recording.enabled {
                Some(RecordingOptions {
                    path: deployment.resolved_path(&dto.recording.path)?,
                    policy: match dto.recording.policy {
                        RecordingPolicyDto::BestEffort => RecordingPolicy::BestEffort,
                        RecordingPolicyDto::Required => RecordingPolicy::Required,
                    },
                })
            } else {
                None
            };
            (dto.server.port, recording)
        } else {
            (options.port(), options.recording().cloned())
        };
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|error| io::Error::other(format!("OS boot entropy unavailable: {error}")))?;
        let mut boot_id = String::with_capacity(32);
        for byte in bytes {
            use std::fmt::Write;
            write!(&mut boot_id, "{byte:02x}")?;
        }
        let clock = SystemClock::new();
        let boot_anchor = if configured_recording.is_some() {
            Some(TimeAnchor::capture(
                || clock.now(),
                || Ok(std::time::SystemTime::now()),
            )?)
        } else {
            None
        };
        let mut host = if let Some(deployment) = loaded.as_ref() {
            let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
            for resource in &deployment.effective().dto.resources {
                let settings = ComSettings::new(
                    resource.id,
                    &resource.port,
                    resource.baud_rate,
                    resource.data_bits,
                    match resource.parity {
                        ParityDto::None => SerialParity::None,
                        ParityDto::Odd => SerialParity::Odd,
                        ParityDto::Even => SerialParity::Even,
                    },
                    resource.stop_bits,
                    match resource.flow_control {
                        FlowControlDto::None => SerialFlowControl::None,
                        FlowControlDto::Software => SerialFlowControl::Software,
                        FlowControlDto::Hardware => SerialFlowControl::Hardware,
                    },
                    std::time::Duration::from_millis(resource.read_timeout_ms),
                    std::time::Duration::from_millis(resource.write_timeout_ms),
                    1,
                )
                .map_err(|_| io::Error::other("invalid configured COM settings"))?;
                let adapter = ComTransport::open_windows(settings)
                    .map_err(|_| io::Error::other("COM worker could not start"))?;
                transports.insert(ResourceId::new(resource.id), Box::new(adapter));
            }
            HostCore::configured_with_transports(deployment, transports)?
        } else {
            HostCore::virtual_demo()?
        };
        host.set_boot_id(&boot_id);
        if !host.shutdown_status().safe_confirmed
            && loaded
                .as_ref()
                .is_none_or(|deployment| deployment.effective().dto.resources.is_empty())
        {
            return Err(io::Error::other("startup safe evidence unavailable").into());
        }
        if let Some(deployment) = loaded.as_ref()
            && !deployment.effective().dto.resources.is_empty()
        {
            host.begin_configured_probes(clock.now())?;
            let maximum_ms = deployment
                .effective()
                .dto
                .resources
                .iter()
                .map(|resource| resource.open_timeout_ms)
                .max()
                .unwrap_or(1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(maximum_ms);
            while !host.configured_probes_ready()? {
                if std::time::Instant::now() >= deadline {
                    let _ = host.begin_shutdown(&clock);
                    return Err(io::Error::other("configured COM probe deadline").into());
                }
                host.service(&clock)?;
                std::thread::yield_now();
            }
            host.request_configured_physical_safe(clock.now())?;
            let output_maximum_ms = deployment
                .effective()
                .dto
                .instruments
                .iter()
                .filter_map(|instrument| match instrument {
                    crate::configuration::InstrumentDto::Metakon {
                        queue_timeout_ms,
                        transaction_timeout_ms,
                        ..
                    } => Some(
                        queue_timeout_ms
                            .saturating_add(transaction_timeout_ms.saturating_mul(2))
                            .saturating_add(500),
                    ),
                    _ => None,
                })
                .max()
                .unwrap_or(1);
            let output_deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(output_maximum_ms);
            while !host.configured_physical_outputs_safe()? {
                if std::time::Instant::now() >= output_deadline {
                    let _ = host.begin_shutdown(&clock);
                    return Err(io::Error::other("configured physical safe deadline").into());
                }
                host.service(&clock)?;
                std::thread::yield_now();
            }
            host.prepare_configured_physical_controllers()?;
        }
        if let Some(deployment) = loaded.as_ref()
            && !deployment.effective().dto.managed_components.is_empty()
        {
            let init_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let supervisor = loop {
                match ManagedExecutor::new() {
                    Ok(supervisor) => break supervisor,
                    Err(ComponentError::Busy) if std::time::Instant::now() < init_deadline => {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(DomainError::from(error).into()),
                }
            };
            host.install_component_executor(Box::new(supervisor))?;
            for index in 0..deployment.effective().dto.managed_components.len() {
                let id = host.stage_configured_component(deployment, index, false, clock.now())?;
                while !host.component_initialized(id) {
                    if std::time::Instant::now() >= init_deadline {
                        let _ = host.begin_shutdown(&clock);
                        return Err(io::Error::other("configured managed init deadline").into());
                    }
                    host.service(&clock)?;
                    std::thread::yield_now();
                }
            }
            host.activate_configured_components(deployment, clock.now())?;
        } else if loaded.is_none() {
            let init_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let supervisor = loop {
                match ManagedExecutor::new() {
                    Ok(supervisor) => break supervisor,
                    Err(ComponentError::Busy) if std::time::Instant::now() < init_deadline => {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(DomainError::from(error).into()),
                }
            };
            host.install_component_executor(Box::new(supervisor))?;
            host.stage_standard_components(clock.now())?;
            while !host.standard_components_initialized() {
                if std::time::Instant::now() >= init_deadline {
                    let _ = host.begin_shutdown(&clock);
                    return Err(io::Error::other("managed startup init deadline").into());
                }
                host.service(&clock)?;
                std::thread::yield_now();
            }
            host.activate_standard_components(clock.now())?;
        }
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        if let Some(recording) = configured_recording.as_ref() {
            let worker = RecorderWorker::open_with_boot_clock(
                &recording.path,
                RecorderLimits::default(),
                &boot_id,
                boot_anchor.expect("recording startup captured its boot anchor"),
                clock,
            )?;
            host.attach_recorder(worker, recording.policy, clock.now())?;
        }
        await_recorder_activation(&mut host)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            host,
            clock,
            listener,
            bound,
            boot_id,
            stopping_since: None,
            safe_since: None,
            recorder_flush_since: None,
            terminal: None,
            fatal: false,
            deployment: loaded.map(DeploymentLifecycle::new),
            configuration_path,
            next_lifecycle_operation: 1,
            reconnect_diagnostic: None,
            quarantined_reconnect_candidate: None,
        })
    }

    /// OS-selected loopback endpoint; no wildcard or external interface is bound.
    pub const fn bound_address(&self) -> SocketAddr {
        self.bound
    }
    /// Fresh 128-bit process identity; old scopes/cursors cannot attach after restart.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }
    /// One bounded JSON readiness line for a process harness; caller prints it once.
    pub fn ready_line(&self) -> String {
        serde_json::json!({"boot_id":self.boot_id,"port":self.bound.port(),"state":"ready"})
            .to_string()
    }
    /// Borrow the committed owner only on the owning service thread.
    pub fn owner(&self) -> &HostCore {
        &self.host
    }
    /// Borrow the owner mutably only from the serialized service loop.
    pub fn owner_mut(&mut self) -> &mut HostCore {
        &mut self.host
    }
    /// Current immutable loaded deployment, absent for the legacy virtual profile.
    pub const fn loaded_configuration(&self) -> Option<&DeploymentLifecycle> {
        self.deployment.as_ref()
    }

    /// Latest bounded reconnect stage for post-failure reconciliation.
    pub const fn reconnect_diagnostic(&self) -> Option<&ReconnectDiagnostic> {
        self.reconnect_diagnostic.as_ref()
    }
    /// Return the one monotonic process clock used by the owner.
    pub const fn clock(&self) -> &SystemClock {
        &self.clock
    }
    /// Copy the same Instant origin to drive an owner mutably in one thread.
    pub const fn clock_copy(&self) -> SystemClock {
        self.clock
    }
    /// Borrow the nonblocking listener only for the separate network reactor.
    pub const fn listener(&self) -> &TcpListener {
        &self.listener
    }
}
fn com_settings(
    resource: &crate::configuration::ResourceDto,
    binding_generation: u64,
) -> Result<ComSettings, LifecycleOperationError> {
    ComSettings::new(
        resource.id,
        &resource.port,
        resource.baud_rate,
        resource.data_bits,
        match resource.parity {
            ParityDto::None => SerialParity::None,
            ParityDto::Odd => SerialParity::Odd,
            ParityDto::Even => SerialParity::Even,
        },
        resource.stop_bits,
        match resource.flow_control {
            FlowControlDto::None => SerialFlowControl::None,
            FlowControlDto::Software => SerialFlowControl::Software,
            FlowControlDto::Hardware => SerialFlowControl::Hardware,
        },
        std::time::Duration::from_millis(resource.read_timeout_ms),
        std::time::Duration::from_millis(resource.write_timeout_ms),
        binding_generation,
    )
    .map_err(|_| LifecycleOperationError::InvalidCandidate)
}

// Listener readiness waits for the active frozen composition to commit. This
// bounded startup wait happens before the owner serves any client or controller;
// no steady-state Runtime turn blocks on storage.
fn await_recorder_activation(host: &mut HostCore) -> Result<(), Box<dyn Error>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if host.recording_activation_committed()? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other("recorder activation startup deadline").into());
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod reconnect_preparation_tests {
    use super::*;
    use crate::{
        application::Application,
        configuration::{ArtifactReader, ConfigurationError, parse_runtime_toml},
        configuration_api,
        recorder::RecordingState,
        serial::{OpenRetryPolicy, SerialDevice},
        wire::{decode_frame, encode_frame},
    };
    use lab_core::{
        metakon::crc,
        transport::{RecoveryStatus, TransportIoError, TransportShutdown},
    };
    use std::{
        collections::{BTreeMap, VecDeque},
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    const DEFINITION: &[u8] = br#"{
 "schema_version":1,"profile":"metakon-5x3-v1","id":11,"name":"Metakon",
 "parameters":[
  {"id":1,"name":"channel_type","value_type":"integer","unit":{"id":"1","symbol":"1"},"min":0,"max":255,"role":"diagnostic","access":"read_only","operation":"channel_type","scale":1,"write_effect":"none"},
  {"id":2,"name":"temperature","value_type":"float","unit":{"id":"degC","symbol":"C"},"min":-99.9,"max":999.9,"role":"measurement","access":"read_only","operation":"temperature","scale":0.1,"write_effect":"none"}
 ]}"#;

    const CONFIG: &[u8] = br#"schema_version=1
[runtime]
key="bench"
display_name="Bench"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=true
path="history.sqlite"
policy="required"
[[resources]]
id=7
key="bus"
kind="windows_com_read_only"
port="COM3"
baud_rate=9600
data_bits=8
parity="none"
stop_bits=1
flow_control="none"
read_timeout_ms=1
write_timeout_ms=1
open_timeout_ms=100
recovery_timeout_ms=100
[[instruments]]
id=11
key="temperature"
kind="metakon"
definition="metakon.json"
resource_id=7
address=1
poll_period_ms=100
queue_timeout_ms=50
transaction_timeout_ms=50
"#;

    struct Reader;

    impl ArtifactReader for Reader {
        fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
            Ok(DEFINITION.to_vec())
        }
    }

    struct OldTransport {
        shutdown_calls: Arc<AtomicUsize>,
        never_finishes: bool,
    }

    impl ByteTransport for OldTransport {
        fn try_write(&mut self, _: &[u8]) -> Result<usize, TransportIoError> {
            Err(TransportIoError::Disconnected)
        }

        fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
            Ok(0)
        }

        fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
            Ok(RecoveryStatus::Pending)
        }

        fn try_shutdown(&mut self) -> TransportShutdown {
            self.shutdown_calls.fetch_add(1, Ordering::AcqRel);
            if self.never_finishes {
                TransportShutdown::Pending
            } else {
                TransportShutdown::Complete
            }
        }
    }

    struct ProbeDevice {
        readable: VecDeque<u8>,
        channel_type: u8,
    }

    impl SerialDevice for ProbeDevice {
        fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
            let mut response = vec![1, 0, 0, 0, 0x41, self.channel_type];
            response.push(crc(&response));
            self.readable.extend(response);
            Ok(bytes.len())
        }

        fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
            let count = maximum.min(self.readable.len());
            Ok(self.readable.drain(..count).collect())
        }
    }

    fn temporary_database() -> PathBuf {
        let mut entropy = [0u8; 12];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::temp_dir().join(format!("lab-runtime-m8-reconnect-{suffix}.sqlite"))
    }

    fn service_with_old_transport(never_finishes: bool) -> (ServiceHost, PathBuf) {
        let deployment = parse_runtime_toml(CONFIG, Path::new("C:\\m8-test"), &mut Reader)
            .expect("test deployment");
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let mut transports: BTreeMap<ResourceId, Box<dyn ByteTransport>> = BTreeMap::new();
        transports.insert(
            ResourceId::new(7),
            Box::new(OldTransport {
                shutdown_calls,
                never_finishes,
            }),
        );
        let mut host = HostCore::configured_with_transports(&deployment, transports).unwrap();
        let boot_id = "0123456789abcdef0123456789abcdef".to_owned();
        host.set_boot_id(&boot_id);
        let clock = SystemClock::new();
        host.begin_configured_probes(clock.now()).unwrap();
        loop {
            host.service(&clock).unwrap();
            if host.resource_records()[0]["data"]["state"] == "offline" {
                break;
            }
            std::thread::yield_now();
        }
        let database = temporary_database();
        let anchor =
            TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now())).unwrap();
        let recorder = RecorderWorker::open_with_boot_clock(
            &database,
            RecorderLimits::default(),
            &boot_id,
            anchor,
            clock,
        )
        .unwrap();
        host.attach_recorder(recorder, RecordingPolicy::Required, clock.now())
            .unwrap();
        await_recorder_activation(&mut host).unwrap();
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let bound = listener.local_addr().unwrap();
        (
            ServiceHost {
                host,
                clock,
                listener,
                bound,
                boot_id,
                stopping_since: None,
                safe_since: None,
                recorder_flush_since: None,
                terminal: None,
                fatal: false,
                deployment: Some(DeploymentLifecycle::new(deployment)),
                configuration_path: None,
                next_lifecycle_operation: 1,
                reconnect_diagnostic: None,
                quarantined_reconnect_candidate: None,
            },
            database,
        )
    }

    fn assert_required_healthy_and_no_outputs(service: &ServiceHost) {
        let status = service.owner().recording_status().unwrap();
        assert_ne!(status.state, RecordingState::Failed);
        assert!(status.first_error.is_none());
        assert!(service.owner().output_safe_records().is_empty());
    }

    fn shutdown_and_remove(mut service: ServiceHost, database: PathBuf) -> ShutdownStatus {
        service.request_shutdown().unwrap();
        let status = loop {
            if let Some(status) = service.shutdown_step().unwrap() {
                break status;
            }
            std::thread::yield_now();
        };
        drop(service);
        let _ = std::fs::remove_file(&database);
        let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
        status
    }

    #[test]
    fn semantic_resource_identity_matches_discovery_current_and_event_without_raw_handle() {
        let (mut service, database) = service_with_old_transport(false);
        let mut application = Application::new(service.boot_id()).unwrap();
        let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
        let hello = application.handle(
            &mut service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
        );
        assert_eq!(hello[0]["type"], "result");
        let current = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":"resource","op":"resource",
                "args":{"resource":"7"}}),
            ),
        );
        let state = &current[0]["result"];
        assert_eq!(state["resource"], "7");
        assert_eq!(state["identity_class"], "physical");
        assert_eq!(state["physical"], true);
        assert_eq!(state["virtual"], false);
        assert_eq!(state["binding_generation"], "1");
        assert_eq!(state["instruments"], serde_json::json!(["11"]));
        assert!(state.get("handle").is_none());

        let discovery = application.handle(
            &mut service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"discover","op":"discover","args":{}})),
        );
        let resource = discovery[0]["result"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["kind"] == "resource")
            .unwrap();
        assert_eq!(resource["id"], "7");
        assert_eq!(resource["state"]["resource"], "7");

        let snapshot = configuration_api::resource_json(&service, 7).unwrap();
        let at = service.clock().now();
        service
            .owner_mut()
            .event_log_mut()
            .resource_state(at, 7, snapshot)
            .unwrap();
        let event = service
            .owner()
            .event_log()
            .scan_after(0, 1024)
            .unwrap()
            .into_iter()
            .find(|event| event["kind"] == "resource")
            .unwrap();
        assert_eq!(event["target"]["id"], "7");
        assert_eq!(event["data"]["resource"], "7");

        drop(application);
        let status = shutdown_and_remove(service, database);
        assert!(status.safe_confirmed);
    }

    #[test]
    fn physical_signal_never_advertises_or_accepts_emulator_publication() {
        let (mut service, database) = service_with_old_transport(false);
        let signal = lab_core::SignalId::new(
            lab_core::InstrumentId::new(11),
            lab_core::ParameterId::new(2),
        );
        let before = service
            .owner()
            .query(lab_core::Query::GetLatestSignal(signal))
            .unwrap();
        assert!(!service.owner().emulator_writable(signal));
        assert!(!service.protocol_features().emulator_publication);

        let mut application = Application::new(service.boot_id()).unwrap();
        let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
        let hello = application.handle(
            &mut service,
            1,
            request(serde_json::json!({"v":1,"msg_id":"hello","op":"hello",
                "args":{"scope":null}})),
        );
        assert!(
            !hello[0]["result"]["operations"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("emulator_publish"))
        );
        let rejected = application.handle(
            &mut service,
            1,
            request(
                serde_json::json!({"v":1,"msg_id":"publish","op":"emulator_publish",
                "request_id":{"scope":hello[0]["result"]["scope"],"seq":"1"},
                "args":{"signal":{"instrument":"11","parameter":"2"},"state":"good",
                    "value":25.0,"expected_generation":"1"}}),
            ),
        );
        assert_eq!(rejected[0]["code"], "unsupported_operation");
        assert_eq!(
            service
                .owner()
                .query(lab_core::Query::GetLatestSignal(signal))
                .unwrap(),
            before
        );
        assert_required_healthy_and_no_outputs(&service);
        drop(application);
        let status = shutdown_and_remove(service, database);
        assert!(status.safe_confirmed);
    }

    #[test]
    fn reconnect_retirement_timeout_reports_exact_pre_rebind_stage() {
        let (mut service, database) = service_with_old_transport(true);

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
            Err(LifecycleOperationError::TransportUnavailable)
        );
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::RetireOldTimeout);
        assert!(!diagnostic.old_worker_finished);
        assert!(!diagnostic.replacement_worker_spawned);
        assert!(!diagnostic.os_port_open_confirmed);
        assert!(!diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(!status.transports_closed);
        assert!(!status.exit_success);
    }

    #[test]
    fn reconnect_worker_spawn_failure_keeps_old_generation_and_recorder_healthy() {
        let (mut service, database) = service_with_old_transport(false);

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
            Err(LifecycleOperationError::TransportUnavailable)
        );
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::ReplacementWorkerSpawn);
        assert_eq!(diagnostic.serial_error, Some(SerialError::Other));
        assert!(diagnostic.old_worker_finished);
        assert!(!diagnostic.replacement_worker_spawned);
        assert!(!diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
    }

    #[test]
    fn stale_expected_generation_is_conflict_without_prior_failure_diagnostics() {
        let (mut service, database) = service_with_old_transport(false);
        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, |_, _| Err(SerialError::Other)),
            Err(LifecycleOperationError::TransportUnavailable)
        );
        assert!(service.reconnect_diagnostic().is_some());

        assert_eq!(
            service.reconnect_resource_with_factory(7, 99, |_, _| Err(SerialError::Other)),
            Err(LifecycleOperationError::Conflict)
        );
        assert!(service.reconnect_diagnostic().is_none());
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
    }

    #[test]
    fn asynchronous_candidate_open_failure_never_crosses_generation_fence() {
        let (mut service, database) = service_with_old_transport(false);

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, |settings, open_deadline| {
                ComTransport::with_retrying_device_factory(
                    settings,
                    OpenRetryPolicy::transient_until(open_deadline),
                    || Err(SerialError::Disconnected),
                )
            }),
            Err(LifecycleOperationError::TransportUnavailable)
        );
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
        assert_eq!(diagnostic.serial_error, Some(SerialError::Disconnected));
        assert!(diagnostic.replacement_worker_spawned);
        assert!(!diagnostic.os_port_open_confirmed);
        assert!(!diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
    }

    #[test]
    fn unfinished_candidate_open_is_quarantined_until_worker_finishes() {
        let (mut service, database) = service_with_old_transport(false);
        let release = Arc::new(AtomicBool::new(false));
        let worker_release = release.clone();

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, move |settings, _| {
                ComTransport::with_device_factory(settings, move || {
                    while !worker_release.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    Ok(Box::new(ProbeDevice {
                        readable: VecDeque::new(),
                        channel_type: 3,
                    }))
                })
            }),
            Err(LifecycleOperationError::TransportUnavailable)
        );
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
        assert_eq!(diagnostic.serial_error, Some(SerialError::Timeout));
        assert!(diagnostic.replacement_worker_spawned);
        assert!(!diagnostic.os_port_open_confirmed);
        assert!(!diagnostic.core_rebind_crossed);
        assert!(service.quarantined_reconnect_candidate.is_some());
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        release.store(true, Ordering::Release);
        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
        assert!(status.exit_success);
    }

    #[test]
    fn failed_probe_is_distinct_and_keeps_installed_generation_quiesced() {
        let (mut service, database) = service_with_old_transport(false);

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, |settings, _| {
                ComTransport::with_device_factory(settings, || {
                    Ok(Box::new(ProbeDevice {
                        readable: VecDeque::new(),
                        channel_type: 2,
                    }))
                })
            }),
            Err(LifecycleOperationError::OwnerFailure)
        );
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::ProbeFailed);
        assert!(diagnostic.os_port_open_confirmed);
        assert!(diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(2));
        assert!(
            service
                .owner()
                .configured_resource_reconnect_quiesced(ResourceId::new(7))
        );
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
        assert!(status.exit_success);
    }

    #[test]
    fn ready_candidate_installs_once_then_probe_and_lifecycle_release_acquisition() {
        let (mut service, database) = service_with_old_transport(false);

        let result = service
            .reconnect_resource_with_factory(7, 1, |settings, _| {
                ComTransport::with_device_factory(settings, || {
                    Ok(Box::new(ProbeDevice {
                        readable: VecDeque::new(),
                        channel_type: 3,
                    }))
                })
            })
            .unwrap_or_else(|error| panic!("{error:?}: {:?}", service.reconnect_diagnostic()));
        assert_eq!(result.binding_generation, 2);
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::Complete);
        assert!(diagnostic.old_worker_finished);
        assert!(diagnostic.replacement_worker_spawned);
        assert!(diagnostic.os_port_open_confirmed);
        assert!(diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(2));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
        assert!(status.exit_success);
    }

    #[test]
    fn transient_disconnected_open_retries_before_same_deadline_and_rebinds_once() {
        let (mut service, database) = service_with_old_transport(false);
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();

        let result = service
            .reconnect_resource_with_factory(7, 1, move |settings, open_deadline| {
                ComTransport::with_retrying_device_factory(
                    settings,
                    OpenRetryPolicy::for_test(open_deadline, Duration::ZERO, 4),
                    move || {
                        if factory_attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                            Err(SerialError::Disconnected)
                        } else {
                            Ok(Box::new(ProbeDevice {
                                readable: VecDeque::new(),
                                channel_type: 3,
                            }))
                        }
                    },
                )
            })
            .unwrap_or_else(|error| panic!("{error:?}: {:?}", service.reconnect_diagnostic()));

        assert_eq!(attempts.load(Ordering::Acquire), 2);
        assert_eq!(result.binding_generation, 2);
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::Complete);
        assert_eq!(diagnostic.open_attempts, 2);
        assert_eq!(diagnostic.serial_error, Some(SerialError::Disconnected));
        assert!(diagnostic.old_worker_finished);
        assert!(diagnostic.os_port_open_confirmed);
        assert!(diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(2));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
        assert!(status.exit_success);
    }

    #[test]
    fn invalid_settings_open_is_terminal_without_generation_or_probe() {
        let (mut service, database) = service_with_old_transport(false);
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();

        assert_eq!(
            service.reconnect_resource_with_factory(7, 1, move |settings, open_deadline| {
                ComTransport::with_retrying_device_factory(
                    settings,
                    OpenRetryPolicy::transient_until(open_deadline),
                    move || {
                        factory_attempts.fetch_add(1, Ordering::AcqRel);
                        Err(SerialError::InvalidSettings)
                    },
                )
            }),
            Err(LifecycleOperationError::TransportUnavailable)
        );

        assert_eq!(attempts.load(Ordering::Acquire), 1);
        let diagnostic = service.reconnect_diagnostic().unwrap();
        assert_eq!(diagnostic.stage, ReconnectStage::ReplacementPortOpenFailed);
        assert_eq!(diagnostic.serial_error, Some(SerialError::InvalidSettings));
        assert_eq!(diagnostic.open_attempts, 1);
        assert!(!diagnostic.os_port_open_confirmed);
        assert!(!diagnostic.core_rebind_crossed);
        assert_eq!(service.owner().configured_binding_generation(11), Some(1));
        assert_required_healthy_and_no_outputs(&service);

        let status = shutdown_and_remove(service, database);
        assert!(status.transports_closed);
        assert!(status.recorder_flushed);
        assert!(status.exit_success);
    }
}
