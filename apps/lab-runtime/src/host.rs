//! The long-running owner will call this bounded scheduler independently of clients.
//!
//! This module keeps one mutable Runtime owner and explicit periods. It does not
//! infer a cadence from a native controller's failure threshold. A skipped slot
//! receives one actual-time opportunity, never a replay at an old deadline.

use lab_core::{
    Command, CommandResult, Error, InstrumentId, Query, QueryResult, Runtime, Sample,
    SampleQuality, SignalId, Unit,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    managed::ComponentExecutor,
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
    plan: SchedulePlan,
    consumed: BTreeMap<ControllerId, Option<Sample>>,
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
        Ok(Self {
            runtime,
            plan: SchedulePlan::virtual_demo(),
            consumed: BTreeMap::new(),
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

    /// Fence producers before Rust begins a new required safe procedure.
    pub fn begin_shutdown(&mut self, clock: &impl Clock) -> Result<(), Error> {
        if self.stopping {
            return Ok(());
        }
        self.stopping = true;
        self.runtime
            .command(Command::QuiesceManaged { at: clock.now() })?;
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
            }
        }
        let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
        self.runtime.command(Command::Output {
            actuator,
            command: OutputCommand::RequestSafe,
            at: clock.now(),
        })?;
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
        if plan.plants.len() > 8 || plan.references.len() > 8 || plan.controllers.len() > 8 {
            return Err(Error::InvalidConfiguration("M6 host schedule limit"));
        }
        self.plan = plan;
        Ok(())
    }

    /// Serialize a local domain Command on this owner; snapshots are separate.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        let start = match &command {
            Command::StartController { controller, .. }
            | Command::ResumeController { controller, .. } => Some(*controller),
            _ => None,
        };
        let outcome = self.runtime.command(command)?;
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
                    self.consumed.insert(*controller, latest);
                    report.controller_ticks += 1;
                }
            }
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
