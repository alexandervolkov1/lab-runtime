//! Native controller algorithms. They calculate proposals but have no output access.

use std::time::Duration;

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
