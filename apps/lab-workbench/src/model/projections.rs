//! Rebuildable observations and bounded live display data.

use crate::presentation::RuntimeRef;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// Maximum live display points retained for one trace.
pub(crate) const LIVE_TRACE_POINTS: usize = 4_096;

/// Freshness of a rebuildable observation set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Freshness {
    /// No observation has been obtained in this client lifetime.
    #[default]
    Unknown,
    /// A reconnect/resnapshot rebuild is in progress.
    Rebuilding,
    /// Values are based on the current attached boot/session/event stream.
    Fresh,
    /// Values are cached and must not be presented as current.
    Stale,
}

/// Last observed plain Application value and its client-side freshness.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Observation {
    /// Cached plain Application value.
    pub(crate) value: Value,
    /// Current client-side freshness classification.
    pub(crate) freshness: Freshness,
    /// Event cursor associated with the observation, when event-derived.
    pub(crate) cursor: Option<u64>,
}

/// One display-only live point; it is not Recorder data or physical evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LivePoint {
    /// Client/display timestamp expressed in seconds.
    pub(crate) time_seconds: f64,
    /// Observed numeric value.
    pub(crate) value: f64,
}

/// Bounded display window for one Runtime source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LiveTraceBuffer {
    points: VecDeque<LivePoint>,
    dropped: u64,
}

impl LiveTraceBuffer {
    /// Appends one finite display point and drops the oldest at capacity.
    pub(crate) fn push(&mut self, point: LivePoint) -> Result<(), &'static str> {
        if !point.time_seconds.is_finite() || !point.value.is_finite() {
            return Err("live point must be finite");
        }
        if self.points.len() == LIVE_TRACE_POINTS {
            self.points.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.points.push_back(point);
        Ok(())
    }

    /// Current ordered display points.
    pub(crate) fn points(&self) -> &VecDeque<LivePoint> {
        &self.points
    }

    /// Count of locally truncated display points.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// All Runtime observations owned by the Workbench cache, never by presentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeObservations {
    /// Observed entity projections by stable identity.
    pub(crate) entities: BTreeMap<RuntimeRef, Observation>,
    /// Bounded live display buffers by stable identity.
    pub(crate) live: BTreeMap<RuntimeRef, LiveTraceBuffer>,
    /// Overall freshness of the observation set.
    pub(crate) freshness: Freshness,
}

impl RuntimeObservations {
    /// Marks every cached observation stale without deleting it.
    pub(crate) fn mark_stale(&mut self) {
        self.freshness = Freshness::Stale;
        for observation in self.entities.values_mut() {
            observation.freshness = Freshness::Stale;
        }
    }

    /// Marks the cache as rebuilding while preserving visibly stale values.
    pub(crate) fn begin_rebuild(&mut self) {
        self.freshness = Freshness::Rebuilding;
        for observation in self.entities.values_mut() {
            observation.freshness = Freshness::Stale;
        }
    }

    /// Marks an explicitly completed multi-projection rebuild fresh.
    pub(crate) fn complete_rebuild(&mut self) {
        self.freshness = Freshness::Fresh;
    }

    /// Inserts or replaces one fresh observed projection.
    pub(crate) fn observe(&mut self, target: RuntimeRef, value: Value, cursor: Option<u64>) {
        self.entities.insert(
            target,
            Observation {
                value,
                freshness: Freshness::Fresh,
                cursor,
            },
        );
    }
}
