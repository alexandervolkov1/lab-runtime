//! Small native signal-processing algorithms with explicit quality and time.

use crate::{SampleQuality, Unit};
use std::time::Duration;

/// Validation or update failure for a native processing component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessingError {
    /// Time constant or warm-up policy is invalid.
    InvalidConfiguration,
    /// Input/output was NaN or infinite.
    NonFinite,
    /// Input unit differs from the configured canonical identity.
    WrongUnit,
    /// Quality and optional value contradict each other.
    InvalidQuality,
    /// A good sample did not advance monotonic time.
    InvalidTime,
}

/// Validated EMA configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmaConfig {
    /// Positive exponential time constant.
    pub time_constant: Duration,
    /// Number of good samples required before control may consume the output.
    pub warmup_samples: usize,
    /// Exact input/output engineering-unit identity.
    pub unit: Unit,
}

/// Whether the current call produced control-usable output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmaStatus {
    /// A finite value exists, but fewer configured good samples were processed.
    WarmingUp,
    /// A finite value exists and warm-up is complete.
    Ready,
    /// This input attempt was unavailable; retained state was not presented as fresh.
    Unavailable,
}

/// Result of one EMA input attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmaUpdate {
    /// Filtered value for Good input, or None for an unavailable attempt.
    pub value: Option<f64>,
    /// Warm-up/quality status of this attempt.
    pub status: EmaStatus,
    /// Input monotonic time.
    pub at: Duration,
    /// Exact configured unit.
    pub unit: Unit,
}

/// Bounded state snapshot used for diagnostics and atomicity tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmaSnapshot {
    /// Last filtered value from a Good input.
    pub value: Option<f64>,
    /// Last Good input time; unavailable attempts do not move it.
    pub last_at: Option<Duration>,
    /// Saturating count of Good inputs, bounded at the warm-up target.
    pub good_samples: usize,
}

/// Exponential moving average supporting uneven monotonic intervals.
pub struct Ema {
    config: EmaConfig,
    state: EmaSnapshot,
}

impl Ema {
    /// Create an empty filter; no value exists before the first Good input.
    pub fn new(config: EmaConfig) -> Result<Self, ProcessingError> {
        if config.time_constant.is_zero() || config.warmup_samples == 0 {
            return Err(ProcessingError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            state: EmaSnapshot {
                value: None,
                last_at: None,
                good_samples: 0,
            },
        })
    }

    /// Process one typed attempt using `alpha = -expm1(-dt / tau)`.
    ///
    /// Unavailable input returns Unavailable without mutating retained filter state.
    /// Every candidate is validated before committing value/time/count together.
    pub fn update(
        &mut self,
        value: Option<f64>,
        quality: SampleQuality,
        unit: Unit,
        at: Duration,
    ) -> Result<EmaUpdate, ProcessingError> {
        if unit != self.config.unit {
            return Err(ProcessingError::WrongUnit);
        }
        match (quality, value) {
            (SampleQuality::Unavailable, None) => {
                if self.state.last_at.is_some_and(|previous| at <= previous) {
                    return Err(ProcessingError::InvalidTime);
                }
                Ok(EmaUpdate {
                    value: None,
                    status: EmaStatus::Unavailable,
                    at,
                    unit,
                })
            }
            (SampleQuality::Good, Some(value)) if value.is_finite() => {
                let candidate = match (self.state.value, self.state.last_at) {
                    (None, None) => value,
                    (Some(previous_value), Some(previous_at)) if at > previous_at => {
                        let elapsed = (at - previous_at).as_secs_f64();
                        let tau = self.config.time_constant.as_secs_f64();
                        let alpha = -(-elapsed / tau).exp_m1();
                        previous_value + alpha * (value - previous_value)
                    }
                    (Some(_), Some(_)) => return Err(ProcessingError::InvalidTime),
                    _ => unreachable!("EMA value and time are committed together"),
                };
                if !candidate.is_finite() {
                    return Err(ProcessingError::NonFinite);
                }
                let good_samples = self
                    .state
                    .good_samples
                    .saturating_add(1)
                    .min(self.config.warmup_samples);
                self.state = EmaSnapshot {
                    value: Some(candidate),
                    last_at: Some(at),
                    good_samples,
                };
                Ok(EmaUpdate {
                    value: Some(candidate),
                    status: if good_samples == self.config.warmup_samples {
                        EmaStatus::Ready
                    } else {
                        EmaStatus::WarmingUp
                    },
                    at,
                    unit,
                })
            }
            (SampleQuality::Good, Some(_)) => Err(ProcessingError::NonFinite),
            _ => Err(ProcessingError::InvalidQuality),
        }
    }

    /// Replace the time constant while preserving accumulated value/time/warm-up.
    pub fn retune(&mut self, time_constant: Duration) -> Result<(), ProcessingError> {
        if time_constant.is_zero() {
            return Err(ProcessingError::InvalidConfiguration);
        }
        self.config.time_constant = time_constant;
        Ok(())
    }

    /// Return a copy of bounded mathematical state without advancing the filter.
    pub const fn snapshot(&self) -> EmaSnapshot {
        self.state
    }

    /// Reset value, time and warm-up count for explicit controller reinitialization.
    pub fn reset(&mut self) {
        self.state = EmaSnapshot {
            value: None,
            last_at: None,
            good_samples: 0,
        };
    }
}
