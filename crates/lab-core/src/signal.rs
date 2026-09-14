use crate::{Error, MeasurementFailure, SignalId, Unit, Value};
use std::{collections::VecDeque, time::Duration};

pub const MAX_HISTORY_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleQuality {
    Good,
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
    signal: SignalId,
    unit: Unit,
    at: Duration,
    reading: Reading,
}

impl Sample {
    pub(crate) fn good(signal: SignalId, unit: Unit, at: Duration, value: Value) -> Self {
        Self {
            signal,
            unit,
            at,
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
            reading: Reading::Unavailable(reason),
        }
    }
    pub fn signal(&self) -> SignalId {
        self.signal
    }
    pub fn unit(&self) -> Unit {
        self.unit
    }
    pub fn at(&self) -> Duration {
        self.at
    }
    pub fn quality(&self) -> SampleQuality {
        match self.reading {
            Reading::Good(_) => SampleQuality::Good,
            Reading::Unavailable(_) => SampleQuality::Unavailable,
        }
    }
    pub fn value(&self) -> Option<&Value> {
        match &self.reading {
            Reading::Good(value) => Some(value),
            Reading::Unavailable(_) => None,
        }
    }
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
}
