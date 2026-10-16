//! Process lifecycle around the serialized [`crate::host::HostCore`] owner.
//!
//! [`crate::service::ServiceHost`] owns the system clock, Application listener, deployment lifecycle,
//! resource reconnect candidates and finite shutdown coordination. It orchestrates
//! external workers/adapters and Runtime progress through `HostCore`; it does not own
//! a second copy of experiment state. Entropy failure, malformed configuration,
//! failed safe profile or failed startup Recorder/transport work prevents readiness.
//! The network reactor remains a separate adapter.
//!
//! # Implementation map
//!
//! - the parent module validates startup and owns the process resources;
//! - `configuration` stages/applies validated deployment candidates;
//! - `reconnect` advances retire/open/probe/rebind under one generation fence;
//! - `shutdown` gives safety, transport and Recorder cleanup bounded owner turns.
//!
//! Native scheduling remains in [`crate::host`], Application request/session state
//! remains in [`crate::application`], and experiment authority remains in
//! [`lab_core::Runtime`].

mod configuration;
#[cfg(test)]
mod property_lifecycle_tests;
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
        ApplyError, ApplyPort, ApplyResult, ConsumedSimpleCandidate, DeploymentLifecycle,
        StageError, StagedConfiguration,
    },
    host::{Clock, HostCore, PreparedSimpleOverlayPublication, ShutdownStatus, SystemClock},
    managed_executor::ManagedExecutor,
    protocol::ProtocolFeatures,
    serial::{
        ComOpenStatus, ComSettings, ComState, ComTransport, SerialError, SerialFlowControl,
        SerialParity,
    },
    simple_device::SimpleDeviceCandidate,
    websocket::WebSocketOptions,
};
use lab_core::managed::ComponentError;
use lab_core::{
    Error as DomainError, InstrumentId,
    output::ActuatorId,
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

/// Strict trusted startup options; default binary execution remains finite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceOptions {
    bind: Ipv4Addr,
    port: u16,
    websocket: Option<WebSocketOptions>,
    recording: Option<RecordingOptions>,
    config: Option<PathBuf>,
}
impl ServiceOptions {
    /// Accept the fixed virtual profile, TCP address, and optional local Recorder.
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        if args.len() >= 3 && args[0] == "--serve" && args[1] == "--config" {
            if args[2].is_empty() {
                return Err("configuration path must not be empty".into());
            }
            let mut index = 3;
            let bind = parse_tcp_bind(args, &mut index)?;
            if index != args.len() {
                return Err("unknown configuration startup option".into());
            }
            return Ok(Self {
                bind,
                port: 0,
                websocket: None,
                recording: None,
                config: Some(PathBuf::from(args[2])),
            });
        }
        if args.len() < 5
            || args[0] != "--serve"
            || args[1] != "--profile"
            || args[2] != "virtual-demo"
        {
            return Err("expected --serve --config PATH [--bind <IPv4> --allow-remote-tcp] or --serve --profile virtual-demo [--bind <IPv4> --allow-remote-tcp] --port <0..65535> [--record-db <absolute-local-path>] [--record-policy required|best-effort] [--ws-port <0..65535> --ws-origin <exact-origin> ...]".into());
        }
        let mut index = 3;
        let bind = parse_tcp_bind(args, &mut index)?;
        if args.get(index) != Some(&"--port") {
            return Err("expected --port after profile bind options".into());
        }
        let port = args
            .get(index + 1)
            .ok_or_else(|| "port requires a value".to_string())?
            .parse::<u16>()
            .map_err(|_| "port must be an integer in 0..65535".to_string())?;
        index += 2;
        let recording = if args.get(index) == Some(&"--record-db") {
            let value = args
                .get(index + 1)
                .copied()
                .ok_or_else(|| "recording policy requires a database path".to_string())?;
            if value.is_empty() {
                return Err("recording policy requires a database path".into());
            }
            let path = PathBuf::from(value);
            if !path.is_absolute() || value.starts_with("\\\\") || value.starts_with("//") {
                return Err("recording database path must be local and absolute".into());
            }
            index += 2;
            let policy = if args.get(index) == Some(&"--record-policy") {
                let value = args
                    .get(index + 1)
                    .copied()
                    .ok_or_else(|| "unknown recording policy".to_string())?;
                index += 2;
                match value {
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
        let websocket = if index == args.len() {
            None
        } else {
            if args.get(index) != Some(&"--ws-port") {
                return Err("unknown service option".into());
            }
            let port = args
                .get(index + 1)
                .ok_or_else(|| "ws-port requires a value".to_string())?
                .parse::<u16>()
                .map_err(|_| "ws-port must be an integer in 0..65535".to_string())?;
            index += 2;
            let mut origins = Vec::new();
            while index < args.len() {
                if args.get(index) != Some(&"--ws-origin") {
                    return Err("unknown WebSocket service option".into());
                }
                origins.push(
                    args.get(index + 1)
                        .ok_or_else(|| "ws-origin requires a value".to_string())?
                        .to_string(),
                );
                index += 2;
            }
            Some(WebSocketOptions::new(port, origins).map_err(str::to_owned)?)
        };
        Ok(Self {
            bind,
            port,
            websocket,
            recording,
            config: None,
        })
    }

    /// Requested numeric IPv4 address for the TCP Application listener.
    pub const fn bind_address(&self) -> Ipv4Addr {
        self.bind
    }

    /// Requested TCP port; zero delegates selection to the OS.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Optional validated IPv4-loopback WebSocket endpoint configuration.
    pub const fn websocket(&self) -> Option<&WebSocketOptions> {
        self.websocket.as_ref()
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

// This is a trusted process-start choice, never a deployment DTO/reload setting.
// An opt-in selects one numeric interface; it does not authenticate TCP clients.
fn parse_tcp_bind(args: &[&str], index: &mut usize) -> Result<Ipv4Addr, String> {
    if args.get(*index) != Some(&"--bind") {
        return Ok(Ipv4Addr::LOCALHOST);
    }
    let address: Ipv4Addr = args
        .get(*index + 1)
        .ok_or("bind requires an IPv4 address")?
        .parse()
        .map_err(|_| "bind must be a numeric IPv4 address")?;
    *index += 2;
    let allow_remote = args.get(*index) == Some(&"--allow-remote-tcp");
    if allow_remote {
        *index += 1;
    }
    if address.is_unspecified()
        || address.is_broadcast()
        || address.is_multicast()
        || address.octets()[0] == 0
        || address.octets()[0] >= 240
    {
        return Err(
            "TCP bind requires one unicast interface address; wildcard is forbidden".into(),
        );
    }
    if !address.is_loopback() && !allow_remote {
        return Err("non-loopback TCP bind requires --allow-remote-tcp (trusted LAN only; no TLS/authentication)".into());
    }
    Ok(address)
}

/// Process-lifecycle owner around one serialized [`HostCore`].
///
/// `ServiceHost` owns the listener, monotonic clock, deployment lifecycle,
/// reconnect candidates and shutdown deadline. It advances those resources in
/// bounded turns and exposes `HostCore` to the Application facade, but never caches
/// a second authoritative experiment state. Socket framing and client delivery are
/// handled outside this type; OS and SQLite errors are translated before reaching
/// the public Application error taxonomy.
pub struct ServiceHost {
    host: HostCore,
    clock: SystemClock,
    listener: TcpListener,
    bound: SocketAddr,
    websocket: Option<WebSocketEndpoint>,
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
    pending_simple_apply: Option<PendingSimpleConfigurationApply>,
    quarantined_simple_output: Option<QuarantinedSimpleOutput>,
    api_simple_overlay_active: bool,
    #[cfg(test)]
    reconnect_cleanup_retire_failure: bool,
    #[cfg(test)]
    reconnect_cleanup_service_failure: bool,
}

const SIMPLE_APPLY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

struct PendingSimpleConfigurationApply {
    candidate: ConsumedSimpleCandidate,
    phase: SimpleApplyPhase,
    deadline: std::time::Duration,
    actuator: Option<ActuatorId>,
    recording: Option<PendingSimpleRecording>,
    quiesced: bool,
    publication: Option<PreparedSimpleOverlayPublication>,
    committed_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SimpleApplyPhase {
    Prepare,
    ReserveRecording,
    PrepareTopology,
    EstablishSafe,
    Commit,
    ConfirmRecording,
}

struct PendingSimpleRecording {
    record: ConfigurationLifecycleRecord,
    generation: Option<u64>,
    reopen_required: bool,
    submitted_at: std::time::Duration,
}

struct QuarantinedSimpleOutput {
    candidate_id: u64,
    resource: ResourceId,
    actuator: ActuatorId,
    binding_generation: u64,
    mapping_revision: u64,
    acknowledged: bool,
    readback_verified: bool,
}

/// One terminal owner outcome polled by Application for session correlation.
pub(crate) struct SimpleApplyCompletion {
    pub(crate) candidate_id: u64,
    pub(crate) result: Result<u64, LifecycleOperationError>,
}

struct WebSocketEndpoint {
    listener: TcpListener,
    bound: SocketAddr,
    allowed_origins: Vec<String>,
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
    /// A bounded lifecycle slot or retained owner is already occupied.
    Capacity,
    /// Candidate referenced no configured physical resource.
    UnknownResource,
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
    /// Candidate safe evidence failed after entering the output lifecycle.
    OutputRejected,
}

/// Completed semantic result of staging one process-local SimpleDevice candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedSimpleDeviceResult {
    /// Shared deployment candidate metadata.
    pub staged: StagedConfiguration,
    /// Stable definition identity.
    pub definition_id: String,
    /// Nonzero definition version.
    pub definition_version: u32,
    /// Canonical normalized definition identity.
    pub normalized_sha256: [u8; 32],
    /// Stable instrument identities that would be published on apply.
    pub instruments: Vec<u64>,
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
    /// The worker exists and the actual serial port open/readback is pending.
    ActualPortOpening,
    /// The actual serial port open/readback failed or timed out.
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
    /// Bounded internal cleanup failure observed after the reconnect failed.
    pub cleanup_failure: Option<&'static str>,
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
            "cleanup_failure": self.cleanup_failure,
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
                let adapter = ComTransport::open_serial(
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
                .apply_configuration(
                    self.active,
                    candidate,
                    self.clock.now(),
                    lifecycle.committed_revision,
                )
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
            simple_device_provisioning: deployment.is_some_and(|active| {
                active.effective().dto.resources.iter().any(|resource| {
                    matches!(
                        resource.kind,
                        crate::configuration::ResourceKindDto::WindowsComReadOnly
                            | crate::configuration::ResourceKindDto::WindowsCom
                    ) && !active.effective().dto.instruments.iter().any(|instrument| {
                        matches!(instrument,
                            crate::configuration::InstrumentDto::Metakon { resource_id, .. }
                                if *resource_id == resource.id)
                    })
                })
            }),
            resource_reconnect: deployment
                .is_some_and(|active| !active.effective().dto.resources.is_empty()),
            emulator_publication: self.host.emulator_target_count() != 0,
            virtual_model_lifecycle: self.host.virtual_model_count() != 0,
        }
    }

    /// Bind a trusted already-safe host fixture, without changing its
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
        let listener =
            TcpListener::bind(SocketAddrV4::new(options.bind_address(), options.port()))?;
        listener.set_nonblocking(true)?;
        let websocket = bind_websocket(options.websocket())?;
        if let Some(recording) = options.recording() {
            let anchor = TimeAnchor::capture(|| clock.now(), || Ok(std::time::SystemTime::now()))?;
            let worker = match RecorderWorker::open_with_boot_clock(
                &recording.path,
                RecorderLimits::default(),
                &boot_id,
                anchor,
                clock,
            ) {
                Ok(worker) => worker,
                Err(error) => {
                    tracing::error!(
                        event = "recorder_archive_open_failed",
                        archive = %recording.path.display(),
                        detail = %error,
                        "Recorder archive could not be opened"
                    );
                    return Err(error.into());
                }
            };
            host.attach_recorder(worker, recording.policy, clock.now())?;
        }
        await_recorder_activation(&mut host)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            host,
            clock,
            listener,
            bound,
            websocket,
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
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        })
    }
    /// Validate identity/profile, then bind the configured IPv4 address in that order.
    pub fn startup(options: ServiceOptions) -> Result<Self, Box<dyn Error>> {
        let configuration_path = options.configuration_path().map(PathBuf::from);
        let loaded = options
            .configuration_path()
            .map(load_runtime_toml)
            .transpose()?;
        let (bind, port, configured_recording, websocket_options) =
            if let Some(deployment) = loaded.as_ref() {
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
                let websocket = dto
                    .server
                    .websocket
                    .enabled
                    .then(|| {
                        WebSocketOptions::new(
                            dto.server.websocket.port,
                            dto.server.websocket.allowed_origins.clone(),
                        )
                    })
                    .transpose()
                    .map_err(io::Error::other)?;
                (
                    options.bind_address(),
                    dto.server.port,
                    recording,
                    websocket,
                )
            } else {
                (
                    options.bind_address(),
                    options.port(),
                    options.recording().cloned(),
                    options.websocket().cloned(),
                )
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
                let adapter = ComTransport::open_serial(settings)
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
                    }
                    | crate::configuration::InstrumentDto::SimpleDevice {
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
        let listener = TcpListener::bind(SocketAddrV4::new(bind, port))?;
        listener.set_nonblocking(true)?;
        let websocket = bind_websocket(websocket_options.as_ref())?;
        if let Some(recording) = configured_recording.as_ref() {
            let worker = match RecorderWorker::open_with_boot_clock(
                &recording.path,
                RecorderLimits::default(),
                &boot_id,
                boot_anchor.expect("recording startup captured its boot anchor"),
                clock,
            ) {
                Ok(worker) => worker,
                Err(error) => {
                    tracing::error!(
                        event = "recorder_archive_open_failed",
                        archive = %recording.path.display(),
                        detail = %error,
                        "Recorder archive could not be opened"
                    );
                    return Err(error.into());
                }
            };
            host.attach_recorder(worker, recording.policy, clock.now())?;
        }
        await_recorder_activation(&mut host)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            host,
            clock,
            listener,
            bound,
            websocket,
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
            pending_simple_apply: None,
            quarantined_simple_output: None,
            api_simple_overlay_active: false,
            #[cfg(test)]
            reconnect_cleanup_retire_failure: false,
            #[cfg(test)]
            reconnect_cleanup_service_failure: false,
        })
    }

    /// Actual bound TCP Application endpoint.
    pub const fn bound_address(&self) -> SocketAddr {
        self.bound
    }
    /// Fresh 128-bit process identity; old scopes/cursors cannot attach after restart.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }
    /// One bounded JSON readiness line for a process harness; caller prints it once.
    pub fn ready_line(&self) -> String {
        let mut ready =
            serde_json::json!({"boot_id":self.boot_id,"port":self.bound.port(),"state":"ready"});
        if let Some(websocket) = &self.websocket {
            ready["websocket"] = serde_json::json!({
                "port":websocket.bound.port(),
                "path":crate::websocket::APPLICATION_PATH,
                "subprotocol":crate::websocket::APPLICATION_SUBPROTOCOL,
            });
        }
        ready.to_string()
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

    /// Borrow the optional nonblocking WebSocket listener for the network reactor.
    pub fn websocket_listener(&self) -> Option<&TcpListener> {
        self.websocket.as_ref().map(|endpoint| &endpoint.listener)
    }

    /// Actual optional IPv4-loopback WebSocket address.
    pub fn websocket_bound_address(&self) -> Option<SocketAddr> {
        self.websocket.as_ref().map(|endpoint| endpoint.bound)
    }

    /// Validated exact browser Origin allowlist for the optional endpoint.
    pub fn websocket_allowed_origins(&self) -> &[String] {
        self.websocket
            .as_ref()
            .map_or(&[], |endpoint| endpoint.allowed_origins.as_slice())
    }
}

fn bind_websocket(
    options: Option<&WebSocketOptions>,
) -> Result<Option<WebSocketEndpoint>, io::Error> {
    let Some(options) = options else {
        return Ok(None);
    };
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, options.port()))?;
    listener.set_nonblocking(true)?;
    let bound = listener.local_addr()?;
    Ok(Some(WebSocketEndpoint {
        listener,
        bound,
        allowed_origins: options.allowed_origins().to_vec(),
    }))
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
mod reconnect_preparation_tests;
