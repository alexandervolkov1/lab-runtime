//! Native time-based references independent of measurement arrival.

use crate::Unit;
use std::time::Duration;

/// Validation or monotonic-time failure for a Reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceError {
    /// Value, target or rate is NaN/infinite, or rate is not positive.
    InvalidConfiguration,
    /// Requested Runtime time moved backwards.
    InvalidTime,
}

/// One finite desired value at explicit monotonic Runtime time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReferenceValue {
    /// Desired engineering value.
    pub value: f64,
    /// Exact engineering-unit identity.
    pub unit: Unit,
    /// Runtime monotonic time used to evaluate the Reference.
    pub at: Duration,
}

/// Constant Reference with explicit unit and time ordering.
pub struct FixedReference {
    value: f64,
    unit: Unit,
    last_at: Option<Duration>,
}

impl FixedReference {
    /// Create a finite constant Reference.
    pub fn new(value: f64, unit: Unit) -> Result<Self, ReferenceError> {
        if !value.is_finite() {
            return Err(ReferenceError::InvalidConfiguration);
        }
        Ok(Self {
            value,
            unit,
            last_at: None,
        })
    }

    /// Evaluate without any wall-clock dependency; equal timestamps are allowed.
    pub fn value_at(&mut self, at: Duration) -> Result<ReferenceValue, ReferenceError> {
        if self.last_at.is_some_and(|previous| at < previous) {
            return Err(ReferenceError::InvalidTime);
        }
        self.last_at = Some(at);
        Ok(ReferenceValue {
            value: self.value,
            unit: self.unit,
            at,
        })
    }
}

/// Bounded Ramp state used for diagnostics and continuity checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampSnapshot {
    /// Current finite value at `last_at`.
    pub current: f64,
    /// Target at which progress clamps.
    pub target: f64,
    /// Positive absolute engineering units per second.
    pub rate: f64,
    /// Exact engineering-unit identity.
    pub unit: Unit,
    /// Last committed monotonic Runtime time.
    pub last_at: Duration,
}

/// Linear monotonic-time Reference supporting upward and downward targets.
pub struct RampReference {
    state: RampSnapshot,
}

impl RampReference {
    /// Create a ramp starting at `start` and `at`.
    pub fn new(
        start: f64,
        target: f64,
        rate: f64,
        unit: Unit,
        at: Duration,
    ) -> Result<Self, ReferenceError> {
        validate_ramp(start, target, rate)?;
        Ok(Self {
            state: RampSnapshot {
                current: start,
                target,
                rate,
                unit,
                last_at: at,
            },
        })
    }

    /// Advance toward the target with exact clamp and no measurement dependency.
    pub fn value_at(&mut self, at: Duration) -> Result<ReferenceValue, ReferenceError> {
        let current = self.candidate_at(at)?;
        self.state.current = current;
        self.state.last_at = at;
        Ok(ReferenceValue {
            value: current,
            unit: self.state.unit,
            at,
        })
    }

    /// Change target/rate at a continuous point on the old trajectory.
    pub fn retune(&mut self, target: f64, rate: f64, at: Duration) -> Result<(), ReferenceError> {
        validate_ramp(self.state.current, target, rate)?;
        let current = self.candidate_at(at)?;
        self.state = RampSnapshot {
            current,
            target,
            rate,
            unit: self.state.unit,
            last_at: at,
        };
        Ok(())
    }

    /// Return current state without advancing Runtime time.
    pub const fn snapshot(&self) -> RampSnapshot {
        self.state
    }

    fn candidate_at(&self, at: Duration) -> Result<f64, ReferenceError> {
        if at < self.state.last_at {
            return Err(ReferenceError::InvalidTime);
        }
        let distance = self.state.target - self.state.current;
        let maximum_step = self.state.rate * (at - self.state.last_at).as_secs_f64();
        let step = distance.signum() * distance.abs().min(maximum_step);
        let candidate = self.state.current + step;
        if candidate.is_finite() {
            Ok(candidate)
        } else {
            Err(ReferenceError::InvalidConfiguration)
        }
    }
}

fn validate_ramp(start: f64, target: f64, rate: f64) -> Result<(), ReferenceError> {
    if start.is_finite() && target.is_finite() && rate.is_finite() && rate > 0.0 {
        Ok(())
    } else {
        Err(ReferenceError::InvalidConfiguration)
    }
}
