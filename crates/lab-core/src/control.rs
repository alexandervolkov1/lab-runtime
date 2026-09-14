//! Native controller algorithms. They calculate proposals but have no output access.

use crate::{
    SignalId,
    output::{ActuatorId, OutputLease},
    processing::{Ema, EmaConfig, EmaSnapshot},
    reference::ReferenceId,
};
use std::time::Duration;

/// Stable Runtime-local identity of one native controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControllerId(u64);

impl ControllerId {
    /// Construct an opaque identity; zero has no sentinel meaning.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return its numeric representation for diagnostics and owner identity.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Explicit lifecycle of a Runtime-owned native controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerState {
    /// Registered but not yet checked against Runtime descriptors.
    Created,
    /// Descriptor contracts are valid; no output authority is held.
    Ready,
    /// Fresh input drives bounded proposals through OutputAuthority.
    Running,
    /// Deliberately stopped with output returned to its safe procedure.
    Paused,
    /// A timing, input, algorithm or output failure latched safe output.
    Failed,
}

/// Data-only construction contract for a native EMA/PID control loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeControllerConfig {
    /// Stable controller identity.
    pub id: ControllerId,
    /// Measurement stream consumed by the loop.
    pub input: SignalId,
    /// Actuator accessed only through central output authority.
    pub output: ActuatorId,
    /// Independent time-based target source.
    pub reference: ReferenceId,
    /// Native input-filter policy.
    pub ema: EmaConfig,
    /// Native controller mathematics and output limits.
    pub pid: PidConfig,
    /// Exclusive freshness threshold for the latest measurement attempt.
    pub max_input_age: Duration,
    /// Largest accepted interval between successful controller ticks.
    pub max_tick_gap: Duration,
    /// Requested bounded automatic-output lease lifetime.
    pub lease_lifetime: Duration,
    /// Lifetime of each generated proposal.
    pub proposal_ttl: Duration,
}

/// Controller registration, lifecycle or fail-safe reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerError {
    /// No controller exists for the requested identity.
    UnknownController,
    /// Registration would replace an existing controller identity.
    DuplicateController,
    /// No Reference exists for the configured identity.
    UnknownReference,
    /// Registration would replace an existing Reference identity.
    DuplicateReference,
    /// The lifecycle transition is not valid from the current state.
    InvalidState,
    /// Durations, descriptors, units or algorithm bounds are inconsistent.
    InvalidConfiguration,
    /// The latest attempt is absent, unavailable or not a finite float.
    InputUnavailable,
    /// The most recent usable input is too old to authorize output.
    StaleInput,
    /// Monotonic controller time did not advance or exceeded its gap bound.
    InvalidTickTime,
    /// Native processing or PID mathematics rejected the update.
    Algorithm,
    /// Central output authority rejected acquisition, proposal or delivery.
    Output,
}

impl From<ControllerError> for crate::Error {
    fn from(error: ControllerError) -> Self {
        Self::Controller(error)
    }
}

/// PID configuration/update failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PidError {
    /// Gains/limits are nonfinite or output limits are not ordered.
    InvalidConfiguration,
    /// Measurement, setpoint or a calculated diagnostic is nonfinite.
    NonFinite,
    /// A subsequent update did not advance monotonic time.
    InvalidTime,
}

/// Finite gains and inclusive output limits for one native PID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PidConfig {
    /// Proportional gain.
    pub kp: f64,
    /// Integral gain applied using actual elapsed seconds.
    pub ki: f64,
    /// Derivative gain applied to measurement change, not setpoint change.
    pub kd: f64,
    /// Inclusive lower output bound.
    pub output_min: f64,
    /// Inclusive upper output bound.
    pub output_max: f64,
}

/// Diagnostics and output from one successfully committed PID update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PidUpdate {
    /// Proportional contribution.
    pub p: f64,
    /// Committed integral contribution after conditional anti-windup.
    pub i: f64,
    /// Derivative-on-measurement contribution.
    pub d: f64,
    /// Sum before final output clamp, after anti-windup choice.
    pub unconstrained: f64,
    /// Inclusive-limit-clamped proposal value.
    pub output: f64,
    /// Monotonic time of this update.
    pub at: Duration,
}

/// Bounded private-memory snapshot for diagnostics and atomicity tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PidSnapshot {
    /// Integral contribution retained by the algorithm.
    pub integral: f64,
    /// Previous measurement used only for derivative calculation.
    pub previous_measurement: Option<f64>,
    /// Previous successful update time.
    pub last_at: Option<Duration>,
    /// Last successful diagnostics/output.
    pub latest: Option<PidUpdate>,
}

/// Deterministic PID kernel with derivative on measurement and conditional anti-windup.
pub struct Pid {
    config: PidConfig,
    state: PidSnapshot,
}

/// Bounded diagnostic view; it carries no ability to produce output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerSnapshot {
    /// Stable controller identity.
    pub id: ControllerId,
    /// Current explicit lifecycle state.
    pub state: ControllerState,
    /// Current central authority token, only while Running.
    pub lease: Option<OutputLease>,
    /// Native filter memory retained for diagnosis.
    pub ema: EmaSnapshot,
    /// Native PID memory retained for diagnosis.
    pub pid: PidSnapshot,
    /// Last successfully initialized or executed controller time.
    pub last_tick: Option<Duration>,
    /// Most recent successfully delivered PID proposal.
    pub latest_output: Option<PidUpdate>,
}

pub(crate) struct NativeController {
    pub(crate) config: NativeControllerConfig,
    pub(crate) state: ControllerState,
    pub(crate) ema: Ema,
    pub(crate) pid: Pid,
    pub(crate) lease: Option<OutputLease>,
    pub(crate) last_tick: Option<Duration>,
    pub(crate) latest_output: Option<PidUpdate>,
}

impl NativeController {
    pub(crate) fn new(config: NativeControllerConfig) -> Result<Self, ControllerError> {
        if config.max_input_age.is_zero()
            || config.max_tick_gap.is_zero()
            || config.lease_lifetime.is_zero()
            || config.proposal_ttl.is_zero()
        {
            return Err(ControllerError::InvalidConfiguration);
        }
        let ema = Ema::new(config.ema).map_err(|_| ControllerError::InvalidConfiguration)?;
        let pid = Pid::new(config.pid).map_err(|_| ControllerError::InvalidConfiguration)?;
        Ok(Self {
            config,
            state: ControllerState::Created,
            ema,
            pid,
            lease: None,
            last_tick: None,
            latest_output: None,
        })
    }

    pub(crate) fn reset_algorithms(&mut self) {
        self.ema.reset();
        self.pid.reset();
        self.latest_output = None;
    }

    pub(crate) fn snapshot(&self) -> ControllerSnapshot {
        ControllerSnapshot {
            id: self.config.id,
            state: self.state,
            lease: self.lease,
            ema: self.ema.snapshot(),
            pid: self.pid.snapshot(),
            last_tick: self.last_tick,
            latest_output: self.latest_output,
        }
    }
}

impl Pid {
    /// Create a PID with zero integral and no fabricated previous sample.
    pub fn new(config: PidConfig) -> Result<Self, PidError> {
        if ![
            config.kp,
            config.ki,
            config.kd,
            config.output_min,
            config.output_max,
        ]
        .into_iter()
        .all(f64::is_finite)
            || config.output_min >= config.output_max
        {
            return Err(PidError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            state: PidSnapshot {
                integral: 0.0,
                previous_measurement: None,
                last_at: None,
                latest: None,
            },
        })
    }

    /// Calculate and atomically commit one update using actual monotonic `dt`.
    pub fn update(
        &mut self,
        measurement: f64,
        setpoint: f64,
        at: Duration,
    ) -> Result<PidUpdate, PidError> {
        if !measurement.is_finite() || !setpoint.is_finite() {
            return Err(PidError::NonFinite);
        }
        let error = setpoint - measurement;
        let p = self.config.kp * error;
        let (candidate_integral, d) = match (self.state.previous_measurement, self.state.last_at) {
            (None, None) => (self.state.integral, 0.0),
            (Some(previous_measurement), Some(previous_at)) if at > previous_at => {
                let dt = (at - previous_at).as_secs_f64();
                (
                    self.state.integral + self.config.ki * error * dt,
                    -self.config.kd * (measurement - previous_measurement) / dt,
                )
            }
            (Some(_), Some(_)) => return Err(PidError::InvalidTime),
            _ => unreachable!("PID previous measurement and time are committed together"),
        };
        let candidate_unconstrained = p + candidate_integral + d;
        let pushes_upper = candidate_unconstrained > self.config.output_max && error > 0.0;
        let pushes_lower = candidate_unconstrained < self.config.output_min && error < 0.0;
        let integral = if pushes_upper || pushes_lower {
            self.state.integral
        } else {
            candidate_integral
        };
        let unconstrained = p + integral + d;
        let output = unconstrained.clamp(self.config.output_min, self.config.output_max);
        if ![error, p, integral, d, unconstrained, output]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err(PidError::NonFinite);
        }
        let update = PidUpdate {
            p,
            i: integral,
            d,
            unconstrained,
            output,
            at,
        };
        self.state = PidSnapshot {
            integral,
            previous_measurement: Some(measurement),
            last_at: Some(at),
            latest: Some(update),
        };
        Ok(update)
    }

    /// Return a copy of bounded algorithm memory without advancing it.
    pub const fn snapshot(&self) -> PidSnapshot {
        self.state
    }

    /// Reinitialize memory explicitly for controller start/resume.
    pub fn reset(&mut self) {
        self.state = PidSnapshot {
            integral: 0.0,
            previous_measurement: None,
            last_at: None,
            latest: None,
        };
    }
}
