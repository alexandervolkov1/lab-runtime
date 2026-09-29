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

/// Freshness of the full controller configuration/detail projection.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ControllerDetail {
    /// Revision for which the full query supplied config, bindings, and policy.
    pub(crate) revision: Value,
    /// Full detail is mutation-authoritative only while this is Fresh.
    pub(crate) freshness: Freshness,
}

/// Freshness of full resource reconnect/configuration detail.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResourceDetail {
    /// Transport generation for which the full query supplied reconnect metadata.
    pub(crate) transport_generation: Value,
    /// Full detail is mutation-authoritative only while this is Fresh.
    pub(crate) freshness: Freshness,
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
    /// Full controller-detail authority, separate from narrow lifecycle events.
    pub(crate) controller_details: BTreeMap<String, ControllerDetail>,
    /// Full resource-detail authority, separate from narrow transport events.
    pub(crate) resource_details: BTreeMap<String, ResourceDetail>,
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
        for detail in self.controller_details.values_mut() {
            detail.freshness = Freshness::Stale;
        }
        for detail in self.resource_details.values_mut() {
            detail.freshness = Freshness::Stale;
        }
    }

    /// Starts a new observation epoch while preserving stale entity projections.
    ///
    /// Live display points are cleared because a rebuild follows lost event
    /// continuity; joining the prior and later points would fabricate a line.
    pub(crate) fn begin_rebuild(&mut self) {
        self.freshness = Freshness::Rebuilding;
        for observation in self.entities.values_mut() {
            observation.freshness = Freshness::Stale;
        }
        for detail in self.controller_details.values_mut() {
            detail.freshness = Freshness::Stale;
        }
        for detail in self.resource_details.values_mut() {
            detail.freshness = Freshness::Stale;
        }
        self.live.clear();
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

    /// Replaces one controller with a complete query projection.
    pub(crate) fn observe_controller_full(&mut self, controller: String, value: Value) {
        let revision = value.get("revision").cloned().unwrap_or(Value::Null);
        self.observe(
            RuntimeRef::Controller {
                controller: controller.clone(),
            },
            value,
            None,
        );
        self.controller_details.insert(
            controller,
            ControllerDetail {
                revision,
                freshness: Freshness::Fresh,
            },
        );
    }

    /// Merges only event-owned dynamic controller fields and invalidates full detail
    /// when the event announces another revision.
    pub(crate) fn observe_controller_event(
        &mut self,
        controller: String,
        event: Value,
        cursor: u64,
    ) {
        let event_revision = event.get("revision").cloned().unwrap_or(Value::Null);
        let target = RuntimeRef::Controller {
            controller: controller.clone(),
        };
        let existing = self.entities.get_mut(&target);
        if let Some(observation) = existing {
            if let (Some(current), Some(update)) =
                (observation.value.as_object_mut(), event.as_object())
            {
                for field in [
                    "state",
                    "status",
                    "failure",
                    "active",
                    "paused",
                    "revision",
                    "last_tick",
                    "latest_output",
                ] {
                    if let Some(value) = update.get(field) {
                        current.insert(field.to_owned(), value.clone());
                    }
                }
            } else {
                observation.value = event;
            }
            observation.freshness = Freshness::Fresh;
            observation.cursor = Some(cursor);
        } else {
            self.observe(target, event, Some(cursor));
        }
        if let Some(detail) = self.controller_details.get_mut(&controller)
            && detail.revision != event_revision
        {
            detail.freshness = Freshness::Stale;
        }
    }

    /// Whether full controller configuration corresponds to the current revision.
    pub(crate) fn controller_detail_is_fresh(&self, controller: &str) -> bool {
        self.controller_details
            .get(controller)
            .is_some_and(|detail| detail.freshness == Freshness::Fresh)
    }

    /// Revision covered by the last complete controller query.
    pub(crate) fn controller_detail_revision(&self, controller: &str) -> Option<&Value> {
        self.controller_details
            .get(controller)
            .map(|detail| &detail.revision)
    }

    /// Invalidates full controller detail while retaining it for stale display.
    pub(crate) fn mark_controller_detail_stale(&mut self, controller: &str) {
        if let Some(detail) = self.controller_details.get_mut(controller) {
            detail.freshness = Freshness::Stale;
        }
    }

    /// Replaces one resource with a complete query projection.
    pub(crate) fn observe_resource_full(&mut self, resource: String, value: Value) {
        let transport_generation = value
            .get("transport_generation")
            .cloned()
            .unwrap_or(Value::Null);
        self.observe(
            RuntimeRef::Resource {
                resource: resource.clone(),
            },
            value,
            None,
        );
        self.resource_details.insert(
            resource,
            ResourceDetail {
                transport_generation,
                freshness: Freshness::Fresh,
            },
        );
    }

    /// Merges event-owned transport facts without replacing full reconnect detail.
    pub(crate) fn observe_resource_event(&mut self, resource: String, event: Value, cursor: u64) {
        let event_generation = event.get("generation").cloned().unwrap_or(Value::Null);
        let target = RuntimeRef::Resource {
            resource: resource.clone(),
        };
        if let Some(observation) = self.entities.get_mut(&target) {
            if let (Some(current), Some(update)) =
                (observation.value.as_object_mut(), event.as_object())
            {
                for field in ["state", "queue_len", "active", "latest"] {
                    if let Some(value) = update.get(field) {
                        current.insert(field.to_owned(), value.clone());
                    }
                }
                if let Some(generation) = update.get("generation") {
                    current.insert("transport_generation".to_owned(), generation.clone());
                }
            } else {
                observation.value = event;
            }
            observation.freshness = Freshness::Fresh;
            observation.cursor = Some(cursor);
        } else {
            self.observe(target, event, Some(cursor));
        }
        if let Some(detail) = self.resource_details.get_mut(&resource)
            && detail.transport_generation != event_generation
        {
            detail.freshness = Freshness::Stale;
        }
    }

    /// Whether full reconnect metadata corresponds to the current transport generation.
    pub(crate) fn resource_detail_is_fresh(&self, resource: &str) -> bool {
        self.resource_details
            .get(resource)
            .is_some_and(|detail| detail.freshness == Freshness::Fresh)
    }

    /// Transport generation covered by the last complete resource query.
    pub(crate) fn resource_detail_generation(&self, resource: &str) -> Option<&Value> {
        self.resource_details
            .get(resource)
            .map(|detail| &detail.transport_generation)
    }

    /// Retains cached resource metadata for display but removes reconnect authority.
    pub(crate) fn mark_resource_detail_stale(&mut self, resource: &str) {
        if let Some(detail) = self.resource_details.get_mut(resource) {
            detail.freshness = Freshness::Stale;
        }
    }

    /// Marks one client-selected subset stale without changing unrelated projections.
    pub(crate) fn mark_matching_stale(&mut self, predicate: impl Fn(&RuntimeRef) -> bool) {
        for (identity, observation) in &mut self.entities {
            if predicate(identity) {
                observation.freshness = Freshness::Stale;
            }
        }
    }
}
