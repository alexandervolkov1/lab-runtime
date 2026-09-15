//! Bounded recent observations, not durable experiment history.
//! A sample couples identity, value-or-failure, units and monotonic time. The
//! buffer owns ordering and oldest-first eviction; latest is its back element.

use crate::{Error, MeasurementFailure, SignalId, Unit, Value};
use std::{collections::VecDeque, time::Duration};

/// Maximum configured samples retained per signal; full history belongs to a future recorder.
pub const MAX_HISTORY_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether an observation contains a usable value, independent of its age.
pub enum SampleQuality {
    /// A validated value exists; consumers must additionally check freshness.
    Good,
    /// The measurement attempt failed and contains no successful value.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
enum Reading {
    Good(Value),
    Unavailable(MeasurementFailure),
}

/// An immutable observation: a failed measurement cannot carry a successful value.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Identity of the associated measurement stream, when one exists.
    signal: SignalId,
    /// Engineering unit attached to the declared value; no implicit conversion.
    unit: Unit,
    /// Elapsed monotonic runtime time of this explicit measurement attempt.
    at: Duration,
    /// Original observation/model time; processing delay never renews freshness.
    freshness_at: Duration,
    reading: Reading,
}

impl Sample {
    pub(crate) fn good(signal: SignalId, unit: Unit, at: Duration, value: Value) -> Self {
        Self {
            signal,
            unit,
            at,
            freshness_at: at,
            reading: Reading::Good(value),
        }
    }
    pub(crate) fn unavailable(
        signal: SignalId,
        unit: Unit,
        at: Duration,
        reason: MeasurementFailure,
    ) -> Self {
        Self {
            signal,
            unit,
            at,
            freshness_at: at,
            reading: Reading::Unavailable(reason),
        }
    }
    /// Construct a Rust-validated derived reading with the captured input lineage.
    pub(crate) fn derived_good(
        signal: SignalId,
        unit: Unit,
        at: Duration,
        freshness_at: Duration,
        value: Value,
    ) -> Result<Self, Error> {
        if freshness_at > at {
            return Err(Error::InvalidConfiguration(
                "derived freshness is after publication",
            ));
        }
        if let Value::Float(number) = &value
            && !number.is_finite()
        {
            return Err(Error::NonFinite);
        }
        Ok(Self {
            signal,
            unit,
            at,
            freshness_at,
            reading: Reading::Good(value),
        })
    }
    /// Return the immutable stream identity captured with this observation.
    pub fn signal(&self) -> SignalId {
        self.signal
    }
    /// Return the engineering unit captured at observation time.
    pub fn unit(&self) -> Unit {
        self.unit
    }
    /// Return the monotonic time of this attempt; a failed attempt also has its own timestamp.
    pub fn at(&self) -> Duration {
        self.at
    }
    /// Source/model observation time used for control freshness and filter sample intervals.
    pub fn freshness_at(&self) -> Duration {
        self.freshness_at
    }
    /// Report whether a value exists; Good alone does not establish freshness for control.
    pub fn quality(&self) -> SampleQuality {
        match self.reading {
            Reading::Good(_) => SampleQuality::Good,
            Reading::Unavailable(_) => SampleQuality::Unavailable,
        }
    }
    /// Borrow the successful observed value, or None for a failed attempt; never substitute stale data.
    pub fn value(&self) -> Option<&Value> {
        match &self.reading {
            Reading::Good(value) => Some(value),
            Reading::Unavailable(_) => None,
        }
    }
    /// Return the unavailable reason, or None for a good observation.
    pub fn failure(&self) -> Option<MeasurementFailure> {
        match self.reading {
            Reading::Good(_) => None,
            Reading::Unavailable(reason) => Some(reason),
        }
    }
}

pub(crate) struct SignalBuffer {
    id: SignalId,
    capacity: usize,
    samples: VecDeque<Sample>,
}

impl SignalBuffer {
    pub(crate) fn new(id: SignalId, capacity: usize) -> Result<Self, Error> {
        if !(1..=MAX_HISTORY_CAPACITY).contains(&capacity) {
            return Err(Error::InvalidConfiguration(
                "history capacity must be in 1..=4096",
            ));
        }
        Ok(Self {
            id,
            capacity,
            samples: VecDeque::with_capacity(capacity),
        })
    }

    pub(crate) fn latest(&self) -> Option<&Sample> {
        self.samples.back()
    }
    pub(crate) fn window(&self) -> Vec<Sample> {
        self.samples.iter().cloned().collect()
    }

    pub(crate) fn check_time(&self, at: Duration) -> Result<(), Error> {
        if let Some(previous) = self.latest()
            && at <= previous.at
        {
            return Err(Error::NonMonotonicTime {
                signal: self.id,
                previous: previous.at,
                requested: at,
            });
        }
        Ok(())
    }

    pub(crate) fn push(&mut self, sample: Sample) -> Result<(), Error> {
        if sample.signal != self.id {
            return Err(Error::UnknownSignal(sample.signal));
        }
        self.check_time(sample.at)?;
        // Evict before push: even the transient length never exceeds the configured bound.
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        Ok(())
    }
    /// A trusted failure may replace a same-time Good tail with Unavailable only.
    /// Normal Good publication and M1 Refresh still require strictly increasing time.
    pub(crate) fn invalidate(
        &mut self,
        at: Duration,
        reason: MeasurementFailure,
    ) -> Result<(), Error> {
        if self.latest().is_some_and(|last| at < last.at()) {
            return Err(Error::NonMonotonicTime {
                signal: self.id,
                previous: self.latest().expect("checked above").at(),
                requested: at,
            });
        }
        let unit = self
            .latest()
            .map(Sample::unit)
            .ok_or(Error::InvalidConfiguration(
                "cannot invalidate a signal without an installed unit",
            ))?;
        let failure = Sample::unavailable(self.id, unit, at, reason);
        if self.latest().is_some_and(|last| last.at() == at) {
            self.samples.pop_back();
            self.samples.push_back(failure);
            Ok(())
        } else {
            self.push(failure)
        }
    }
}

#[cfg(test)]
mod managed_provenance_tests {
    use super::*;
    use crate::{InstrumentId, ParameterId};

    #[test]
    fn delayed_derived_sample_retains_source_freshness() {
        let id = SignalId::new(InstrumentId::new(15), ParameterId::new(1));
        let derived = Sample::derived_good(
            id,
            Unit::CELSIUS,
            Duration::from_secs(3),
            Duration::from_secs(1),
            Value::Float(42.0),
        )
        .unwrap();
        assert_eq!(derived.at(), Duration::from_secs(3));
        assert_eq!(derived.freshness_at(), Duration::from_secs(1));
        assert_eq!(
            Sample::good(
                id,
                Unit::CELSIUS,
                Duration::from_secs(1),
                Value::Float(42.0)
            )
            .freshness_at(),
            Duration::from_secs(1)
        );
        assert!(
            Sample::derived_good(
                id,
                Unit::CELSIUS,
                Duration::from_secs(1),
                Duration::from_secs(2),
                Value::Float(42.0)
            )
            .is_err()
        );
    }

    #[test]
    fn equal_time_failure_can_replace_good_but_cannot_restore_good() {
        let id = SignalId::new(InstrumentId::new(15), ParameterId::new(1));
        let mut buffer = SignalBuffer::new(id, 2).unwrap();
        let good = Sample::good(
            id,
            Unit::CELSIUS,
            Duration::from_secs(3),
            Value::Float(42.0),
        );
        buffer.push(good.clone()).unwrap();
        buffer
            .invalidate(Duration::from_secs(3), MeasurementFailure::ComponentFailure)
            .unwrap();
        assert_eq!(good.value(), Some(&Value::Float(42.0)));
        assert_eq!(
            buffer.latest().unwrap().quality(),
            SampleQuality::Unavailable
        );
        assert!(buffer.latest().unwrap().value().is_none());
        assert_eq!(buffer.window().len(), 1);
        assert!(buffer.push(good).is_err());
        assert!(
            buffer
                .invalidate(Duration::from_secs(3), MeasurementFailure::ComponentFailure)
                .is_ok()
        );
    }
}
