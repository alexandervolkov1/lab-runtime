//! Headless service startup prepares a safe owner before exposing the listener.
//!
//! Entropy failure, malformed CLI or failed safe profile prevents readiness.
//! The network reactor is a separate adapter added after this host foundation.

use crate::recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, TimeAnchor};
use crate::{
    configuration::{FlowControlDto, ParityDto, RecordingPolicyDto, load_runtime_toml},
    deployment::{ApplyError, ApplyPort, ApplyResult, DeploymentLifecycle, StageError},
    host::{Clock, HostCore, ShutdownStatus, SystemClock},
    serial::{ComSettings, ComTransport, SerialFlowControl, SerialParity},
};
use lab_core::managed::ComponentError;
use lab_core::{
    Error as DomainError,
    transport::{ByteTransport, ResourceId},
};
use std::{
    collections::BTreeMap,
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

struct LiveApplyPort<'a> {
    host: &'a mut HostCore,
    active: &'a crate::configuration::FrozenDeployment,
    at: std::time::Duration,
}
impl ApplyPort for LiveApplyPort<'_> {
    fn enter_safe_barrier(&mut self, at: std::time::Duration) -> Result<bool, ApplyError> {
        self.at = at;
        self.host
            .enter_configuration_safe_barrier(at)
            .map_err(|_| ApplyError::OwnerFailure)
    }

    fn prepare_bindings(&mut self) -> Result<(), ApplyError> {
        Err(ApplyError::OwnerFailure)
    }

    fn commit_configuration(
        &mut self,
        candidate: &crate::configuration::FrozenDeployment,
    ) -> Result<(), ApplyError> {
        self.host
            .apply_configuration(self.active, candidate, self.at)
            .map_err(|_| ApplyError::OwnerFailure)
    }
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
        let staged = lifecycle
            .stage(candidate, self.clock.now())
            .map_err(|error| match error {
                StageError::Busy | StageError::CounterExhausted | StageError::DeadlineOverflow => {
                    LifecycleOperationError::Conflict
                }
            })?;
        let submitted_at = self.clock.now();
        let reopen_required = self.host.begin_configuration_recording_fence(submitted_at);
        let apply_at = self.clock.now();
        let mut port = LiveApplyPort {
            host: &mut self.host,
            active: &active,
            at: apply_at,
        };
        let applied = lifecycle.apply(
            staged.id(),
            staged.base_revision(),
            self.clock.now(),
            &mut port,
        );
        match applied {
            Ok(ApplyResult::Applied { revision }) => {
                let generation = match self.host.request_live_activation() {
                    Ok(generation) => generation,
                    Err(_) => {
                        self.host.configuration_recording_failed(self.clock.now());
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                };
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match self.host.live_activation_committed(
                        generation,
                        reopen_required,
                        submitted_at,
                        self.clock.now(),
                    ) {
                        Ok(true) => break,
                        Ok(false) if std::time::Instant::now() < deadline => {
                            let _ = self.host.service(&self.clock);
                            std::thread::yield_now();
                        }
                        _ => {
                            self.host.configuration_recording_failed(self.clock.now());
                            return Err(LifecycleOperationError::OwnerFailure);
                        }
                    }
                }
                Ok(ReloadConfigurationResult { revision })
            }
            Ok(ApplyResult::FailedBeforeCommit) => {
                Err(LifecycleOperationError::RequiresSafeBarrier)
            }
            Err(ApplyError::RestartRequired) => Err(LifecycleOperationError::InvalidCandidate),
            Err(ApplyError::OwnerFailure) => Err(LifecycleOperationError::OwnerFailure),
            Err(ApplyError::UnknownCandidate | ApplyError::Conflict | ApplyError::Expired) => {
                Err(LifecycleOperationError::Conflict)
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
        self.host
            .commit_prepared_components(prepared, self.clock.now())
            .map_err(|_| LifecycleOperationError::OwnerFailure)?;
        self.host
            .activate_configured_components(&candidate, self.clock.now())
            .map_err(|_| LifecycleOperationError::OwnerFailure)?;
        self.deployment
            .as_mut()
            .expect("checked above")
            .commit_managed_sources(candidate)
            .map_err(|_| LifecycleOperationError::Conflict)?;
        Ok(())
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
        let (models, generation) = self
            .host
            .restart_configured_models(&active, self.clock.now())
            .map_err(|_| LifecycleOperationError::OwnerFailure)?;
        Ok(RestartModelsResult { models, generation })
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
