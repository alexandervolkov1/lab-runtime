//! Native time-based references independent of measurement arrival.

use crate::Unit;
use std::time::Duration;

/// Stable Runtime-local identity of a configured Reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReferenceId(u64);

impl ReferenceId {
    /// Construct an identity. Zero is valid because this is an opaque key, not a sentinel.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric representation for diagnostics and stable ordering.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Data-only construction contract for one Runtime-owned Reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReferenceConfig {
    /// A time-independent finite target.
    Fixed {
        /// Stable Reference identity.
        id: ReferenceId,
        /// Constant engineering value.
        value: f64,
        /// Exact engineering-unit identity.
        unit: Unit,
    },
    /// A target moving linearly in explicit monotonic Runtime time.
    Ramp {
        /// Stable Reference identity.
        id: ReferenceId,
        /// Value at the creation time.
        start: f64,
        /// Finite target at which the ramp clamps.
        target: f64,
        /// Positive absolute engineering units per second.
        rate: f64,
        /// Exact engineering-unit identity.
        unit: Unit,
        /// Initial monotonic Runtime time.
        at: Duration,
    },
}

impl ReferenceConfig {
    pub(crate) const fn id(self) -> ReferenceId {
        match self {
            Self::Fixed { id, .. } | Self::Ramp { id, .. } => id,
        }
    }
}

/// Validation or monotonic-time failure for a Reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceError {
    /// Value, target or rate is NaN/infinite, or rate is not positive.
    InvalidConfiguration,
    /// Requested Runtime time moved backwards.
    InvalidTime,
    /// The expected configuration revision is not the committed revision.
    RevisionConflict,
    /// A Fixed Reference has no Ramp target or slope to retune.
    NotRamp,
    /// Configuration revision cannot advance without reusing an old value.
    RevisionExhausted,
}

/// One atomically committed Ramp configuration after a continuous retune.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetunedRamp {
    /// Value and target at the trusted command time.
    pub state: RampSnapshot,
    /// New configuration revision; ordinary evaluations do not increment it.
    pub revision: u64,
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

/// Bounded diagnostic view of a Runtime-owned Reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReferenceSnapshot {
    /// Constant reference and its most recent evaluation time.
    Fixed {
        /// Stable Reference identity.
        id: ReferenceId,
        /// Constant finite engineering value.
        value: f64,
        /// Exact engineering-unit identity.
        unit: Unit,
        /// Last evaluation time, or None before first use.
        last_at: Option<Duration>,
        /// Revision of the immutable Fixed configuration.
        revision: u64,
    },
    /// Ramp state after its most recent evaluation.
    Ramp {
        /// Stable Reference identity.
        id: ReferenceId,
        /// Bounded ramp state.
        state: RampSnapshot,
        /// Revision of target/rate configuration, independent of progress.
        revision: u64,
    },
}

pub(crate) enum RuntimeReference {
    Fixed {
        id: ReferenceId,
        reference: FixedReference,
        revision: u64,
    },
    Ramp {
        id: ReferenceId,
        reference: RampReference,
        revision: u64,
    },
}

impl RuntimeReference {
    pub(crate) fn new(config: ReferenceConfig) -> Result<Self, ReferenceError> {
        match config {
            ReferenceConfig::Fixed { id, value, unit } => Ok(Self::Fixed {
                id,
                reference: FixedReference::new(value, unit)?,
                revision: 1,
            }),
            ReferenceConfig::Ramp {
                id,
                start,
                target,
                rate,
                unit,
                at,
            } => Ok(Self::Ramp {
                id,
                reference: RampReference::new(start, target, rate, unit, at)?,
                revision: 1,
            }),
        }
    }

    pub(crate) const fn unit(&self) -> Unit {
        match self {
            Self::Fixed { reference, .. } => reference.unit,
            Self::Ramp { reference, .. } => reference.state.unit,
        }
    }

    pub(crate) const fn revision(&self) -> u64 {
        match self {
            Self::Fixed { revision, .. } | Self::Ramp { revision, .. } => *revision,
        }
    }

    /// Replace a complete stopped-graph Reference without carrying progress.
    pub(crate) fn reconfigure(
        &mut self,
        config: ReferenceConfig,
        expected_revision: u64,
    ) -> Result<ReferenceSnapshot, ReferenceError> {
        let current_id = match self {
            Self::Fixed { id, .. } | Self::Ramp { id, .. } => *id,
        };
        if config.id() != current_id || self.revision() != expected_revision {
            return Err(ReferenceError::RevisionConflict);
        }
        let next = expected_revision
            .checked_add(1)
            .ok_or(ReferenceError::RevisionExhausted)?;
        let mut candidate = Self::new(config)?;
        match &mut candidate {
            Self::Fixed { revision, .. } | Self::Ramp { revision, .. } => *revision = next,
        }
        *self = candidate;
        Ok(self.snapshot())
    }

    pub(crate) fn value_at(&mut self, at: Duration) -> Result<ReferenceValue, ReferenceError> {
        match self {
            Self::Fixed { reference, .. } => reference.value_at(at),
            Self::Ramp { reference, .. } => reference.value_at(at),
        }
    }

    /// Validate revision and the complete new trajectory before committing either.
    pub(crate) fn retune(
        &mut self,
        target: f64,
        rate: f64,
        expected_revision: u64,
        at: Duration,
    ) -> Result<RetunedRamp, ReferenceError> {
        let Self::Ramp {
            reference,
            revision,
            ..
        } = self
        else {
            return Err(ReferenceError::NotRamp);
        };
        if *revision != expected_revision {
            return Err(ReferenceError::RevisionConflict);
        }
        let next = revision
            .checked_add(1)
            .ok_or(ReferenceError::RevisionExhausted)?;
        reference.retune(target, rate, at)?;
        *revision = next;
        Ok(RetunedRamp {
            state: reference.snapshot(),
            revision: next,
        })
    }

    pub(crate) const fn snapshot(&self) -> ReferenceSnapshot {
        match self {
            Self::Fixed {
                id,
                reference,
                revision,
            } => ReferenceSnapshot::Fixed {
                id: *id,
                value: reference.value,
                unit: reference.unit,
                last_at: reference.last_at,
                revision: *revision,
            },
            Self::Ramp {
                id,
                reference,
                revision,
            } => ReferenceSnapshot::Ramp {
                id: *id,
                state: reference.snapshot(),
                revision: *revision,
            },
        }
    }
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
