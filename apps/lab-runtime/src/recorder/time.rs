//! Immutable UTC anchors for display; all control and record ordering remain monotonic.

use super::StorageError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One bracketed sample of the process monotonic clock and actual UTC.
/// A later UTC adjustment never changes an earlier boot mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeAnchor {
    before: Duration,
    wall_us: Option<i64>,
    after: Duration,
    unavailable_reason: Option<String>,
}

impl TimeAnchor {
    /// Construct a valid anchor from checked signed Unix microseconds.
    pub fn valid(before: Duration, wall_us: i64, after: Duration) -> Result<Self, StorageError> {
        Self::bracket(before, after)?;
        Ok(Self {
            before,
            wall_us: Some(wall_us),
            after,
            unavailable_reason: None,
        })
    }

    /// Preserve an unsuccessful later wall read without inventing an actual UTC.
    pub fn unavailable(
        before: Duration,
        reason: &str,
        after: Duration,
    ) -> Result<Self, StorageError> {
        Self::bracket(before, after)?;
        if reason.is_empty() || reason.len() > 128 {
            return Err(StorageError("invalid bounded UTC failure reason".into()));
        }
        Ok(Self {
            before,
            wall_us: None,
            after,
            unavailable_reason: Some(reason.to_owned()),
        })
    }

    /// Capture a real or deterministic UTC read between two calls to the same
    /// process monotonic clock. The wall provider may report failure.
    pub fn capture(
        monotonic: impl Fn() -> Duration,
        wall: impl Fn() -> Result<SystemTime, &'static str>,
    ) -> Result<Self, StorageError> {
        let before = monotonic();
        let actual = wall();
        let after = monotonic();
        match actual {
            Ok(time) => Self::valid(before, unix_microseconds(time)?, after),
            Err(reason) => Self::unavailable(before, reason, after),
        }
    }

    fn bracket(before: Duration, after: Duration) -> Result<(), StorageError> {
        if after < before || after.as_nanos() > u64::MAX as u128 {
            return Err(StorageError("invalid monotonic clock bracket".into()));
        }
        Ok(())
    }

    /// Monotonic time before the actual wall read.
    pub const fn before(&self) -> Duration {
        self.before
    }

    /// Monotonic time after the actual wall read; mapping uses this endpoint.
    pub const fn after(&self) -> Duration {
        self.after
    }

    /// Actual signed Unix microseconds, absent only on a failed later wall read.
    pub const fn wall_us(&self) -> Option<i64> {
        self.wall_us
    }

    /// Explicit bounded reason for an unavailable actual wall read.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    /// Uncertainty span of the monotonic bracket in nanoseconds.
    pub fn uncertainty_ns(&self) -> u64 {
        u64::try_from(self.after.as_nanos() - self.before.as_nanos())
            .expect("validated bracket is within version-one range")
    }

    /// Map a fact's monotonic time using this fixed anchor. The result is a
    /// display estimate, not a device timestamp or a fresh UTC observation.
    pub fn estimate_us(&self, at: Duration) -> Result<i64, StorageError> {
        let wall = self
            .wall_us
            .ok_or_else(|| StorageError("UTC estimate has no valid anchor".into()))?;
        let at = i128::try_from(at.as_nanos())
            .map_err(|_| StorageError("monotonic fact time exceeds range".into()))?;
        let after = i128::try_from(self.after.as_nanos())
            .map_err(|_| StorageError("monotonic anchor exceeds range".into()))?;
        let delta_us = (at - after).div_euclid(1_000);
        let estimate = i128::from(wall) + delta_us;
        i64::try_from(estimate)
            .map_err(|_| StorageError("UTC display estimate exceeds signed range".into()))
    }
}

fn unix_microseconds(time: SystemTime) -> Result<i64, StorageError> {
    let value = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_micros()),
        Err(error) => i128::try_from(error.duration().as_micros()).map(|value| -value),
    }
    .map_err(|_| StorageError("actual UTC exceeds signed range".into()))?;
    i64::try_from(value).map_err(|_| StorageError("actual UTC exceeds signed range".into()))
}
