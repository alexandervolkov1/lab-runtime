//! The long-running owner will call this bounded scheduler independently of clients.
//!
//! This module keeps one mutable Runtime owner and explicit periods. It does not
//! infer a cadence from a native controller's failure threshold. A skipped slot
//! receives one actual-time opportunity, never a replay at an old deadline.

use crate::events::{EventError, EventLog};
use lab_core::{
    Command, CommandResult, Error, InstrumentId, Query, QueryResult, Runtime, Sample,
    SampleQuality, SignalId, Unit,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    managed::{
        ComponentDefinition, ComponentError, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, ComponentState, PlainData, PlainValue,
    },
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const PLANT: InstrumentId = InstrumentId::new(1);
const REFERENCE: ReferenceId = ReferenceId::new(1);
const CONTROLLER: ControllerId = ControllerId::new(1);
const LUA_SOURCE: ComponentId = ComponentId::new(201);
const LUA_FILTER: ComponentId = ComponentId::new(202);

/// Trusted elapsed-time source; production and deterministic tests share the interface.
pub trait Clock {
    /// Return nondecreasing elapsed monotonic Runtime time.
    fn now(&self) -> Duration;
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
    references: Vec<(ReferenceId, Periodic)>,
    controllers: Vec<(ControllerId, SignalId, Periodic)>,
    sources: Vec<(ComponentId, Periodic)>,
    transforms: Vec<(ComponentId, SignalId)>,
}
impl SchedulePlan {
    fn virtual_demo() -> Self {
        Self {
            safety: Periodic::new(Duration::from_millis(10)),
            plants: vec![(PLANT, Periodic::new(Duration::from_millis(100)))],
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
    /// Safe evidence and worker cleanup both finished.
    pub exit_success: bool,
}

/// Sole mutable Core owner plus one explicit schedule; callers serialize commands.
pub struct HostCore {
    runtime: Runtime,
    events: EventLog,
    plan: SchedulePlan,
    consumed: BTreeMap<ControllerId, Option<Sample>>,
    consumed_managed: BTreeMap<ComponentId, Option<Sample>>,
    components: Vec<(ComponentId, &'static str)>,
    last_now: Duration,
    stopping: bool,
}
impl HostCore {
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
        runtime.command(Command::Output {
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
            last_now: Duration::ZERO,
            stopping: false,
        })
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

    /// Stage only the fixed trusted Lua Source; Core permits one staged init at a time.
    pub fn stage_standard_lua(&mut self, at: Duration) -> Result<(), Error> {
        if !self.components.is_empty() {
            return Err(Error::InvalidConfiguration("Lua profile already staged"));
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
                name: format!("Lua observation {}", id.get()),
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
                manifest: manifest(LUA_SOURCE, ComponentKind::Source, 1),
                source: lab_lua::fixtures::VIRTUAL_MODEL_SOURCE.into(),
                config,
            },
            replaces: None,
            at,
        })?;
        self.events.track_component(LUA_SOURCE);
        self.components.push((LUA_SOURCE, "source"));
        Ok(())
    }

    /// Stage the fixed one-input Transform only after Source init has committed.
    pub fn stage_standard_filter(&mut self, at: Duration) -> Result<(), Error> {
        if !self.source_lua_initialized() {
            return Err(Error::InvalidConfiguration("Lua Source init incomplete"));
        }
        let manifest = ComponentManifest {
            schema_version: 1,
            id: LUA_FILTER,
            instrument: InstrumentId::new(LUA_FILTER.get()),
            name: "Lua moving mean".into(),
            parameter: lab_core::TEMPERATURE,
            kind: ComponentKind::Transform {
                input: SignalId::new(InstrumentId::new(LUA_SOURCE.get()), lab_core::TEMPERATURE),
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
                source: lab_lua::fixtures::MOVING_MEAN_SOURCE.into(),
                config: PlainData::default(),
            },
            replaces: None,
            at,
        })?;
        self.events.track_component(LUA_FILTER);
        self.components.push((LUA_FILTER, "transform"));
        Ok(())
    }
    /// The first real init result committed without any pending callback.
    pub fn source_lua_initialized(&self) -> bool {
        matches!(self.runtime.query(Query::Component(LUA_SOURCE)),Ok(QueryResult::Component(s))
            if matches!(s.state,ComponentState::Warming|ComponentState::Ready) && s.pending.is_none())
    }

    /// True only after both real init callbacks have committed and no job waits.
    pub fn standard_lua_initialized(&self) -> bool {
        [LUA_SOURCE,LUA_FILTER].iter().all(|id|matches!(self.runtime.query(Query::Component(*id)),
            Ok(QueryResult::Component(snapshot)) if matches!(snapshot.state,ComponentState::Warming|ComponentState::Ready) && snapshot.pending.is_none()))
    }

    /// Start trusted managed cadence after bounded startup init completes.
    pub fn activate_standard_lua(&mut self, at: Duration) -> Result<(), Error> {
        if !self.standard_lua_initialized() {
            return Err(Error::InvalidConfiguration("managed init incomplete"));
        }
        let mut source = Periodic::new(Duration::from_millis(200));
        source.next_due = at;
        self.plan.sources.push((LUA_SOURCE, source));
        self.plan.transforms.push((
            LUA_FILTER,
            SignalId::new(InstrumentId::new(LUA_SOURCE.get()), lab_core::TEMPERATURE),
        ));
        Ok(())
    }

    /// Fixed trusted component identities/kinds for discovery and typed status.
    pub fn component_catalog(&self) -> &[(ComponentId, &'static str)] {
        &self.components
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
        let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
        self.runtime.command(Command::Output {
            actuator,
            command: OutputCommand::RequestSafe,
            at: clock.now(),
        })?;
        self.observe(clock.now(), None)?;
        self.plan.safety.next_due = clock.now();
        Ok(())
    }

    /// Copy output evidence and worker cleanup status without progressing either.
    pub fn shutdown_status(&self) -> ShutdownStatus {
        let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
        let safe_confirmed = match self.runtime.query(Query::Output(actuator)) {
            Ok(QueryResult::Output(snapshot)) => {
                snapshot.safe_confirmed && snapshot.lease.is_none()
            }
            _ => false,
        };
        let unfinished_workers = self.runtime.unfinished_component_workers();
        ShutdownStatus {
            safe_confirmed,
            unfinished_workers,
            exit_success: safe_confirmed && unfinished_workers == 0,
        }
    }

    /// Replace a schedule only through a trusted local composition decision.
    /// Network requests cannot access this seam or choose controller cadence.
    pub fn replace_plan(&mut self, plan: SchedulePlan) -> Result<(), Error> {
        if plan.plants.len() > 8
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
        let start = match &command {
            Command::StartController { controller, .. }
            | Command::ResumeController { controller, .. } => Some(*controller),
            _ => None,
        };
        let outcome = self.runtime.command(command);
        let at = self.last_now;
        self.observe(at, cause.as_ref().map(|(s, n)| (s.as_str(), *n)))?;
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
        if self.stopping {
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
