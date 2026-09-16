//! Headless service startup prepares a safe owner before exposing the listener.
//!
//! Entropy failure, malformed CLI or failed safe profile prevents readiness.
//! The network reactor is a separate adapter added after this host foundation.

use crate::recorder::{
    ConfigurationLifecycleRecord, RecorderLimits, RecorderWorker, RecordingPolicy, TimeAnchor,
};
use crate::{
    configuration::{FlowControlDto, ParityDto, RecordingPolicyDto, load_runtime_toml},
    deployment::{
        ApplyError, ApplyPort, ApplyResult, DeploymentLifecycle, StageError, StagedConfiguration,
    },
    host::{Clock, HostCore, ShutdownStatus, SystemClock},
    serial::{ComSettings, ComTransport, SerialFlowControl, SerialParity},
};
use lab_core::managed::ComponentError;
use lab_core::{
    Error as DomainError,
    transport::{ByteTransport, ResourceId},
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
                        .prepare_configured_transport_replacement(resource_id)
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
        if !host.shutdown_status().safe_confirmed {
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
        }
        if let Some(deployment) = loaded.as_ref()
            && !deployment.effective().dto.managed_components.is_empty()
        {
            let init_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let supervisor = loop {
                match lab_lua::LuaSupervisor::new() {
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
                match lab_lua::LuaSupervisor::new() {
                    Ok(supervisor) => break supervisor,
                    Err(ComponentError::Busy) if std::time::Instant::now() < init_deadline => {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(DomainError::from(error).into()),
                }
            };
            host.install_component_executor(Box::new(supervisor))?;
            host.stage_standard_lua(clock.now())?;
            while !host.source_lua_initialized() {
                if std::time::Instant::now() >= init_deadline {
                    let _ = host.begin_shutdown(&clock);
                    return Err(io::Error::other("managed Source init deadline").into());
                }
                host.service(&clock)?;
                std::thread::yield_now();
            }
            host.stage_standard_filter(clock.now())?;
            while !host.standard_lua_initialized() {
                if std::time::Instant::now() >= init_deadline {
                    let _ = host.begin_shutdown(&clock);
                    return Err(io::Error::other("managed startup init deadline").into());
                }
                host.service(&clock)?;
                std::thread::yield_now();
            }
            host.activate_standard_lua(clock.now())?;
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
        })
    }

    /// Raise the producer stop barrier before subsequent network or worker work.
    pub fn request_shutdown(&mut self) -> Result<(), DomainError> {
        if self.stopping_since.is_some() {
            return Ok(());
        }
        self.stopping_since = Some(std::time::Instant::now());
        let clock = self.clock;
        if let Err(error) = self.host.begin_shutdown(&clock) {
            self.fatal = true;
            return Err(error);
        }
        let at = self.clock.now();
        self.host
            .event_log_mut()
            .host_state(at, "stopping", serde_json::json!({}))
            .map_err(|_| {
                self.fatal = true;
                DomainError::InvalidConfiguration("host event limit")
            })?;
        Ok(())
    }
    /// Record a fatal owner fault and enter the same bounded evidence-preserving
    /// shutdown path; a failed stop step still leaves the grace state active.
    pub fn request_fatal_shutdown(&mut self) {
        self.fatal = true;
        let _ = self.request_shutdown();
    }
    /// Stop barrier is raised before further client mutation admission.
    pub const fn is_stopping(&self) -> bool {
        self.stopping_since.is_some()
    }

    /// Progress trusted safe work once; never sleep or join on the owner lane.
    /// The caller keeps sweeping clients/requests between these bounded turns.
    pub fn shutdown_step(&mut self) -> Result<Option<ShutdownStatus>, DomainError> {
        if let Some(terminal) = self.terminal {
            return Ok(Some(terminal));
        }
        let Some(started) = self.stopping_since else {
            return Ok(None);
        };
        let clock = self.clock;
        if self.host.service(&clock).is_err() {
            self.fatal = true;
        }
        let mut status = self.host.shutdown_status();
        status.fatal_error = self.fatal;
        status.exit_success &= !self.fatal;
        let now = std::time::Instant::now();
        let safety_finished = if status.safe_confirmed {
            self.safe_since.get_or_insert(now);
            status.unfinished_workers == 0
                || self.safe_since.is_some_and(|safe_at| {
                    now.duration_since(safe_at) >= std::time::Duration::from_millis(200)
                })
        } else {
            now.duration_since(started) >= std::time::Duration::from_secs(2)
        };
        if safety_finished {
            self.recorder_flush_since.get_or_insert(now);
            self.host.shutdown_recorder_step(self.clock.now());
            status = self.host.shutdown_status();
            status.fatal_error = self.fatal;
            status.exit_success &= !self.fatal;
            let flush_expired = self
                .recorder_flush_since
                .is_some_and(|at| now.duration_since(at) >= std::time::Duration::from_secs(2));
            if status.recorder_flushed || flush_expired {
                self.terminal = Some(status);
            }
        }
        Ok(self.terminal)
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

    /// Reload, validate, stage and atomically commit a live-safe deployment diff.
    pub fn reload_configuration(
        &mut self,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        let staged = self.stage_configuration()?;
        self.apply_staged_configuration_kind(
            staged.id(),
            staged.base_revision(),
            "reload_configuration",
        )
    }

    /// Load, validate and retain exactly one immutable candidate without active mutation.
    pub fn stage_configuration(&mut self) -> Result<StagedConfiguration, LifecycleOperationError> {
        let path = self
            .configuration_path
            .as_deref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let candidate = load_runtime_toml(path)
            .map_err(|_| LifecycleOperationError::InvalidCandidate)?
            .reuse_unchanged_managed_sources(
                self.deployment
                    .as_ref()
                    .ok_or(LifecycleOperationError::ConfigurationDisabled)?
                    .active(),
            )
            .map_err(|_| LifecycleOperationError::InvalidCandidate)?;
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let staged = lifecycle
            .stage(candidate, self.clock.now())
            .map_err(|error| match error {
                StageError::Busy | StageError::CounterExhausted | StageError::DeadlineOverflow => {
                    LifecycleOperationError::Conflict
                }
            })?;
        Ok(staged)
    }

    /// Apply one retained candidate under its explicit identity/revision fence.
    pub fn apply_staged_configuration(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        self.apply_staged_configuration_kind(candidate_id, expected_revision, "apply_configuration")
    }

    fn apply_staged_configuration_kind(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
        operation_kind: &'static str,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let apply_at = self.clock.now();
        let mut port = LiveApplyPort {
            host: &mut self.host,
            active: &active,
            at: apply_at,
            recording_fence: None,
            clock: self.clock,
            operation_id: candidate_id,
            operation_kind,
            base_revision: expected_revision,
            activation_generation: None,
            postcommit_recording_failure: false,
            prepared_bindings: BTreeMap::new(),
        };
        let applied = lifecycle.apply(candidate_id, expected_revision, self.clock.now(), &mut port);
        let recording_fence = port.recording_fence;
        let activation_generation = port.activation_generation;
        let postcommit_recording_failure = port.postcommit_recording_failure;
        match applied {
            Ok(ApplyResult::Applied { revision }) => {
                if postcommit_recording_failure {
                    return Err(LifecycleOperationError::OwnerFailure);
                }
                let (reopen_required, submitted_at) = recording_fence
                    .expect("successful apply crosses the recording fence before commit");
                let generation = activation_generation
                    .expect("successful apply reserves activation before commit");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match self.host.live_activation_committed(
                        generation,
                        reopen_required,
                        submitted_at,
                        self.clock.now(),
                    ) {
                        Ok(true) => {
                            self.host.end_configuration_quiesce();
                            break;
                        }
                        Ok(false) if std::time::Instant::now() < deadline => {
                            let _ = self.host.service(&self.clock);
                            std::thread::yield_now();
                        }
                        _ => {
                            self.host.configuration_recording_failed(self.clock.now());
                            self.host.end_configuration_quiesce();
                            return Err(LifecycleOperationError::OwnerFailure);
                        }
                    }
                }
                Ok(ReloadConfigurationResult { revision })
            }
            Ok(ApplyResult::FailedBeforeCommit) => {
                self.host.end_configuration_quiesce();
                Err(LifecycleOperationError::RequiresSafeBarrier)
            }
            Err(ApplyError::RestartRequired) => Err(LifecycleOperationError::InvalidCandidate),
            Err(ApplyError::OwnerFailure) => Err(LifecycleOperationError::OwnerFailure),
            Err(ApplyError::UnknownCandidate | ApplyError::Conflict | ApplyError::Expired) => {
                Err(LifecycleOperationError::Conflict)
            }
        }
    }

    fn begin_recorded_lifecycle(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        self.begin_recorded_lifecycle_with_scope(operation_kind, deployment, true)
    }

    fn begin_resource_recorded_lifecycle(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        self.begin_recorded_lifecycle_with_scope(operation_kind, deployment, false)
    }

    fn begin_recorded_lifecycle_with_scope(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
        global_quiesced: bool,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        let operation_id = self.next_lifecycle_operation;
        self.next_lifecycle_operation = operation_id
            .checked_add(1)
            .ok_or(LifecycleOperationError::Conflict)?;
        let revision = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .revision();
        let at = self.clock.now();
        let reopen_required = if global_quiesced {
            if !self
                .host
                .enter_configuration_safe_barrier(at)
                .map_err(|_| LifecycleOperationError::OwnerFailure)?
            {
                return Err(LifecycleOperationError::RequiresSafeBarrier);
            }
            let reopen_required = self.host.begin_configuration_recording_fence(at);
            self.host.begin_configuration_quiesce();
            reopen_required
        } else {
            false
        };
        let record = ConfigurationLifecycleRecord {
            operation_id,
            operation_kind,
            base_revision: revision,
            committed_revision: revision,
            toml_hash: deployment.toml_hash(),
            affected: configuration_affected(deployment, revision, revision),
            reason: None,
            at,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let generation = loop {
            let reservation = self
                .host
                .try_reserve_configuration_activation(&record, self.clock.now())
                .map_err(|_| LifecycleOperationError::RecordingUnavailable);
            match reservation {
                Err(error) => {
                    if global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(error);
                }
                Ok(Some(generation)) => break generation,
                Ok(None) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        if global_quiesced {
                            self.host.end_configuration_quiesce();
                        }
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                Ok(None) => {
                    if global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
        };
        Ok(PendingRecordedLifecycle {
            record,
            generation,
            reopen_required,
            submitted_at: at,
            global_quiesced,
        })
    }

    fn cancel_recorded_lifecycle(&mut self, pending: PendingRecordedLifecycle) {
        let _ = self
            .host
            .cancel_configuration_activation(pending.generation);
        if pending.global_quiesced {
            self.host.end_configuration_quiesce();
        }
    }

    fn finish_recorded_lifecycle(
        &mut self,
        pending: PendingRecordedLifecycle,
    ) -> Result<(), LifecycleOperationError> {
        if self
            .host
            .commit_reserved_configuration_activation(pending.generation, pending.record)
            .is_err()
        {
            self.host.configuration_recording_failed(self.clock.now());
            if pending.global_quiesced {
                self.host.end_configuration_quiesce();
            }
            return Err(LifecycleOperationError::RecordingUnavailable);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match self.host.live_activation_committed(
                pending.generation,
                pending.reopen_required,
                pending.submitted_at,
                self.clock.now(),
            ) {
                Ok(true) => {
                    if pending.global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Ok(());
                }
                Ok(false) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        self.host.configuration_recording_failed(self.clock.now());
                        if pending.global_quiesced {
                            self.host.end_configuration_quiesce();
                        }
                        return Err(LifecycleOperationError::RecordingUnavailable);
                    }
                    std::thread::yield_now();
                }
                _ => {
                    self.host.configuration_recording_failed(self.clock.now());
                    if pending.global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(LifecycleOperationError::RecordingUnavailable);
                }
            }
        }
    }

    /// Reload managed sources independently of deployment and model restart.
    pub fn reload_managed_scripts(&mut self) -> Result<(), LifecycleOperationError> {
        let candidate = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .reload_managed_sources()
            .map_err(|_| LifecycleOperationError::InvalidCandidate)?;
        let count = candidate.effective().dto.managed_components.len();
        if count == 0 {
            return Err(LifecycleOperationError::NoManagedComponents);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut prepared = Vec::with_capacity(count);
        for index in 0..count {
            let id = self
                .host
                .stage_configured_component(&candidate, index, true, self.clock.now())
                .map_err(|_| LifecycleOperationError::InvalidCandidate)?;
            while !self.host.component_prepared(id) {
                if std::time::Instant::now() >= deadline {
                    self.host.discard_prepared_components();
                    return Err(LifecycleOperationError::OwnerFailure);
                }
                if self.host.service(&self.clock).is_err()
                    || (!self.host.component_prepare_pending() && !self.host.component_prepared(id))
                {
                    self.host.discard_prepared_components();
                    return Err(LifecycleOperationError::InvalidCandidate);
                }
                std::thread::yield_now();
            }
            prepared.push(id);
        }
        let pending = self.begin_recorded_lifecycle("reload_managed_scripts", &candidate)?;
        if self
            .host
            .commit_prepared_components(prepared, self.clock.now())
            .is_err()
            || self
                .host
                .activate_configured_components(&candidate, self.clock.now())
                .is_err()
        {
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        if self
            .deployment
            .as_mut()
            .expect("checked above")
            .commit_managed_sources(candidate)
            .is_err()
        {
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::Conflict);
        }
        self.finish_recorded_lifecycle(pending)
    }

    /// Restart configured native models without rereading TOML or managed sources.
    pub fn restart_virtual_models(
        &mut self,
    ) -> Result<RestartModelsResult, LifecycleOperationError> {
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        let managed = active.effective().dto.managed_components.len();
        let native = active
            .effective()
            .dto
            .instruments
            .iter()
            .filter(|instrument| {
                matches!(
                    instrument,
                    crate::configuration::InstrumentDto::ThermalPlant { .. }
                )
            })
            .count();
        if native == 0 && managed == 0 {
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let mut prepared = Vec::with_capacity(managed);
        if managed != 0 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            for index in 0..managed {
                let id = self
                    .host
                    .stage_configured_component(&active, index, true, self.clock.now())
                    .map_err(|_| LifecycleOperationError::OwnerFailure)?;
                while !self.host.component_prepared(id) {
                    if std::time::Instant::now() >= deadline
                        || self.host.service(&self.clock).is_err()
                        || (!self.host.component_prepare_pending()
                            && !self.host.component_prepared(id))
                    {
                        self.host.discard_prepared_components();
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                prepared.push(id);
            }
        }
        let pending = self.begin_recorded_lifecycle("restart_virtual_models", &active)?;
        let mut models = 0usize;
        let mut generation = 0u64;
        if native != 0 {
            match self
                .host
                .restart_configured_models(&active, self.clock.now())
            {
                Ok((count, latest)) => {
                    models += count;
                    generation = generation.max(latest);
                }
                Err(_) => {
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
        }
        if managed != 0 {
            if self
                .host
                .commit_prepared_components(prepared, self.clock.now())
                .is_err()
                || self
                    .host
                    .activate_configured_components(&active, self.clock.now())
                    .is_err()
            {
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            models += managed;
            for component in &active.effective().dto.managed_components {
                if let Ok(lab_core::QueryResult::Component(snapshot)) = self.host.query(
                    lab_core::Query::Component(lab_core::managed::ComponentId::new(component.id)),
                ) {
                    generation = generation.max(snapshot.generation);
                }
            }
        }
        self.finish_recorded_lifecycle(pending)?;
        Ok(RestartModelsResult { models, generation })
    }

    /// Explicitly retire and reopen one configured read-only COM resource.
    /// Port enumeration never calls this operation and no write retry is implied.
    pub fn reconnect_resource(
        &mut self,
        resource_id: u64,
        expected_binding_generation: u64,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        let resource = active
            .effective()
            .dto
            .resources
            .iter()
            .find(|resource| resource.id == resource_id)
            .ok_or(LifecycleOperationError::InvalidCandidate)?
            .clone();
        let instrument = active
            .effective()
            .dto
            .instruments
            .iter()
            .find_map(|instrument| match instrument {
                crate::configuration::InstrumentDto::Metakon {
                    id,
                    resource_id: bound,
                    ..
                } if *bound == resource_id => Some(*id),
                _ => None,
            })
            .ok_or(LifecycleOperationError::InvalidCandidate)?;
        let current = self
            .host
            .configured_binding_generation(instrument)
            .ok_or(LifecycleOperationError::Conflict)?;
        if current != expected_binding_generation {
            return Err(LifecycleOperationError::Conflict);
        }
        let generation = current
            .checked_add(1)
            .ok_or(LifecycleOperationError::Conflict)?;
        let pending = self.begin_resource_recorded_lifecycle("reconnect_resource", &active)?;
        let resource_key = ResourceId::new(resource_id);
        if self
            .host
            .begin_configured_resource_reconnect(resource_key)
            .is_err()
        {
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(resource.recovery_timeout_ms);
        loop {
            match self
                .host
                .prepare_configured_transport_replacement(resource_key)
            {
                Ok(true) => break,
                Ok(false) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        self.cancel_recorded_lifecycle(pending);
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                _ => {
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
        }
        let settings = match com_settings(&resource, generation) {
            Ok(settings) => settings,
            Err(error) => {
                self.cancel_recorded_lifecycle(pending);
                return Err(error);
            }
        };
        let adapter =
            ComTransport::open_windows(settings).map_err(|_| LifecycleOperationError::OwnerFailure);
        let Ok(adapter) = adapter else {
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        };
        if self
            .host
            .rebind_configured_transport(resource_key, Box::new(adapter), self.clock.now())
            .is_err()
        {
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        if self
            .host
            .begin_configured_probes_for_resource(resource_key, self.clock.now())
            .is_err()
        {
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(resource.recovery_timeout_ms);
        loop {
            match self.host.configured_probes_ready_for_resource(resource_key) {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) => {
                    let _ =
                        self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
            if std::time::Instant::now() >= deadline {
                let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            if self.host.service(&self.clock).is_err() {
                let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            std::thread::yield_now();
        }
        if self.finish_recorded_lifecycle(pending).is_err() {
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        if self
            .host
            .activate_configured_resource_after_reconnect(resource_key, self.clock.now())
            .is_err()
        {
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        Ok(ReconnectResourceResult {
            resource_id,
            binding_generation: generation,
        })
    }

    fn retire_failed_reconnect(
        &mut self,
        resource: ResourceId,
        timeout_ms: u64,
    ) -> Result<(), LifecycleOperationError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match self
                .host
                .retire_failed_configured_reconnect(resource, self.clock.now())
            {
                Ok(true) => return Ok(()),
                Ok(false) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                _ => return Err(LifecycleOperationError::OwnerFailure),
            }
        }
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
