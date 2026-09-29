//! Renderer-neutral Workbench observational and presentation owner.
#![allow(
    dead_code,
    unused_imports,
    reason = "M14.3 model API precedes the M14.4 GUI consumer"
)]

mod command;
mod operator;
mod projections;

pub(crate) use command::{
    LabCommand, UiCommand, UiCommandError, WorkbenchCommand, apply_ui_command,
};
pub(crate) use operator::{
    ControllerLifecycleIntent, OperatorIntent, OperatorIntentError, OperatorWarning,
    OperatorWorkflow, OperatorWorkflowState, PROPERTY_TEXT_BYTES, PidCandidate,
    PropertyMutationCandidate, RECORDING_LABEL_BYTES,
};
pub(crate) use projections::{Freshness, LIVE_TRACE_POINTS, LivePoint, RuntimeObservations};

use crate::{
    client::types::{
        ClientUpdate, ConnectionState, EventCursor, HelloState, KnownAdmission, MAX_IN_FLIGHT,
        MutationIdentity, RecoveryRecord, ReplyKind,
    },
    presentation::{ConfigurationOwner, PresentationDocument, RuntimeRef},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Maximum retained operator actions, including current nonterminal work.
pub(crate) const MAX_OPERATOR_ACTIONS: usize = 64;

const OPERATOR_ACTION_CAPACITY_ERROR: &str = "operator action history is full of nonterminal work";

/// Client recovery/session observations, separate from presentation persistence.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ClientRecoveryState {
    /// Last attached Runtime boot identity.
    pub(crate) boot_id: Option<String>,
    /// Last retained Application scope.
    pub(crate) scope: Option<String>,
    /// Last authoritative `hello.next_seq` observation.
    pub(crate) next_seq: Option<u64>,
    /// Last usable process event cursor.
    pub(crate) event_cursor: Option<EventCursor>,
    /// Current bounded projection of all worker-owned recovery records.
    pub(crate) mutations: Vec<RecoveryRecord>,
    /// Mutation identities for which the worker requested authoritative reconciliation.
    pub(crate) reconciliation_required: Vec<MutationIdentity>,
}

/// Caller-local command correlation identity, distinct from mutation sequence identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CommandId(u64);

impl CommandId {
    fn new(value: u64) -> Self {
        Self(value)
    }
}

/// State of one operator action from intent through authoritative outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperatorActionState {
    /// Intent exists but has not received authoritative admission.
    PendingAdmission,
    /// Runtime reported accepted/in progress.
    Accepted,
    /// Runtime reported terminal completion.
    Completed,
    /// Runtime reported terminal failure or public rejection.
    Failed,
    /// Transport loss made the outcome require reconciliation.
    Unknown,
}

/// Single client-owned observable state consumed later by GUI and Steel adapters.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkbenchModel {
    /// Last observed client worker connection state.
    pub(crate) connection: ConnectionState,
    /// Last complete hello for discovery/limits and session observation.
    pub(crate) hello: Option<HelloState>,
    /// Rebuildable observations and display buffers.
    pub(crate) observations: RuntimeObservations,
    /// Session/dedup reconciliation aid; never presentation state.
    pub(crate) recovery: ClientRecoveryState,
    /// Persistent client-owned presentation.
    pub(crate) presentation: PresentationDocument,
    /// Runtime references not proven by a fresh observation in the current view.
    pub(crate) unresolved: BTreeSet<RuntimeRef>,
    /// Pending/terminal client action display state keyed by command ID.
    pub(crate) actions: BTreeMap<CommandId, OperatorActionState>,
    /// Bounded insertion order used to retire only the oldest terminal action.
    action_order: VecDeque<CommandId>,
    /// Last visible client-local failure.
    pub(crate) client_error: Option<String>,
    /// Durable recovery-journal failure; separately gates mutation controls.
    pub(crate) recovery_problem: Option<String>,
}

impl WorkbenchModel {
    /// Creates a model with no fabricated Runtime observations.
    pub(crate) fn new(presentation: PresentationDocument) -> Self {
        let mut model = Self {
            connection: ConnectionState::Disconnected,
            hello: None,
            observations: RuntimeObservations::default(),
            recovery: ClientRecoveryState::default(),
            presentation,
            unresolved: BTreeSet::new(),
            actions: BTreeMap::new(),
            action_order: VecDeque::new(),
            client_error: None,
            recovery_problem: None,
        };
        model.refresh_unresolved();
        model
    }

    /// Deterministically consumes one ordered update from the single client owner.
    pub(crate) fn apply_client_update(&mut self, update: ClientUpdate) {
        match update {
            ClientUpdate::State(state) => self.apply_connection_state(state),
            ClientUpdate::Hello(hello) => self.apply_hello(hello),
            ClientUpdate::Reply {
                command_id,
                op,
                kind,
                envelope,
                ..
            } => self.apply_reply(command_id, &op, kind, envelope),
            ClientUpdate::RecoveryState { records } => {
                self.apply_recovery_projection(records);
            }
            ClientUpdate::Event { cursor, envelope } => {
                self.recovery.event_cursor = Some(cursor.clone());
                self.apply_event(cursor.seq, envelope);
            }
            ClientUpdate::SubscriptionProgress(envelope) => {
                if let Some(cursor) = parse_cursor(&envelope) {
                    self.recovery.event_cursor = Some(cursor);
                }
            }
            ClientUpdate::ReferenceBootstrap {
                command_id: _,
                snapshot,
                subsequent_events,
            } => {
                self.observe_reference(snapshot, None);
                for event in subsequent_events {
                    let seq = parse_cursor(&event).map(|cursor| cursor.seq);
                    self.apply_event(seq.unwrap_or_default(), event);
                }
                self.refresh_unresolved();
            }
            ClientUpdate::ReconciliationRequired { records } => {
                if records.len() <= MAX_IN_FLIGHT {
                    self.recovery.reconciliation_required = records
                        .into_iter()
                        .filter(|record| requires_reconciliation(record.admission))
                        .map(|record| record.identity)
                        .collect();
                } else {
                    self.client_error = Some("worker recovery notification exceeded bound".into());
                }
            }
            ClientUpdate::ResnapshotRequired {
                reason,
                connection_lost,
                ..
            } => {
                if connection_lost {
                    self.connection = ConnectionState::Stale;
                    self.observations.mark_stale();
                } else {
                    self.observations.begin_rebuild();
                }
                self.refresh_unresolved();
                self.client_error = Some(reason);
            }
            ClientUpdate::RecoveryJournalProblem { reason } => {
                self.observations.mark_stale();
                self.refresh_unresolved();
                self.recovery_problem = Some(reason.clone());
                self.client_error = Some(reason);
            }
            ClientUpdate::LocalRejected { command_id, reason } => {
                if let Some(action) = self.actions.get_mut(&CommandId::new(command_id)) {
                    *action = OperatorActionState::Failed;
                }
                self.client_error = Some(reason);
            }
            ClientUpdate::TransportFailure { reason } => {
                self.observations.mark_stale();
                self.refresh_unresolved();
                self.client_error = Some(reason);
            }
            ClientUpdate::WorkerStopped => {
                self.connection = ConnectionState::Stopped;
                self.observations.mark_stale();
                self.refresh_unresolved();
            }
        }
    }

    /// Adds one point to a bounded display-only signal window.
    pub(crate) fn push_live_point(
        &mut self,
        source: RuntimeRef,
        point: LivePoint,
    ) -> Result<(), &'static str> {
        self.observations
            .live
            .entry(source)
            .or_default()
            .push(point)
    }

    /// Applies one validated presentation mutation and refreshes unresolved targets.
    pub(crate) fn apply_ui_command(&mut self, command: UiCommand) -> Result<(), UiCommandError> {
        command::apply_ui_command(&mut self.presentation, command)?;
        self.refresh_unresolved();
        Ok(())
    }

    /// Marks one submitted lab intent pending authoritative admission/result.
    pub(crate) fn track_operator_intent(&mut self, command_id: u64) -> Result<(), &'static str> {
        let command_id = CommandId::new(command_id);
        if self.actions.contains_key(&command_id) {
            self.client_error = Some("operator command ID was already tracked".into());
            return Err("operator command ID was already tracked");
        }
        if self.actions.len() == MAX_OPERATOR_ACTIONS {
            let Some(position) = self.action_order.iter().position(|candidate| {
                self.actions
                    .get(candidate)
                    .is_some_and(|state| state.is_terminal())
            }) else {
                self.client_error = Some(OPERATOR_ACTION_CAPACITY_ERROR.into());
                return Err(OPERATOR_ACTION_CAPACITY_ERROR);
            };
            let retired = self
                .action_order
                .remove(position)
                .expect("located terminal action remains in bounded order");
            self.actions.remove(&retired);
        }
        self.actions
            .insert(command_id, OperatorActionState::PendingAdmission);
        self.action_order.push_back(command_id);
        debug_assert!(self.actions.len() <= MAX_OPERATOR_ACTIONS);
        debug_assert_eq!(self.actions.len(), self.action_order.len());
        Ok(())
    }

    /// Whether one new operator intent can be retained without losing active state.
    pub(crate) fn operator_action_capacity_available(&self) -> bool {
        self.actions.len() < MAX_OPERATOR_ACTIONS
            || self.actions.values().any(|state| state.is_terminal())
    }

    /// Returns one caller-local action state without crossing into mutation identity.
    pub(crate) fn action_state(&self, command_id: u64) -> Option<OperatorActionState> {
        self.actions.get(&CommandId::new(command_id)).copied()
    }

    /// Explicitly completes a model-owner-driven multi-projection rebuild.
    pub(crate) fn complete_rebuild(&mut self) {
        self.observations.complete_rebuild();
        self.refresh_unresolved();
    }

    /// Full controller config/PID is authoritative only from a current full query.
    pub(crate) fn controller_detail_is_fresh(&self, controller: &str) -> bool {
        self.observations.controller_detail_is_fresh(controller)
    }

    /// Revision covered by the last complete controller detail query.
    pub(crate) fn controller_detail_revision(&self, controller: &str) -> Option<&Value> {
        self.observations.controller_detail_revision(controller)
    }

    /// Retains cached controller config for display but removes mutation authority.
    pub(crate) fn mark_controller_detail_stale(&mut self, controller: &str) {
        self.observations.mark_controller_detail_stale(controller);
    }

    /// Full resource reconnect detail is authoritative only from a current query.
    pub(crate) fn resource_detail_is_fresh(&self, resource: &str) -> bool {
        self.observations.resource_detail_is_fresh(resource)
    }

    /// Transport generation covered by the last complete resource detail query.
    pub(crate) fn resource_detail_generation(&self, resource: &str) -> Option<&Value> {
        self.observations.resource_detail_generation(resource)
    }

    /// Retains resource metadata for display but removes reconnect authority.
    pub(crate) fn mark_resource_detail_stale(&mut self, resource: &str) {
        self.observations.mark_resource_detail_stale(resource);
    }

    /// Recomputes unresolved presentation references without changing the document.
    pub(crate) fn refresh_unresolved(&mut self) {
        self.unresolved = self
            .presentation
            .runtime_refs()
            .filter(|reference| {
                !self
                    .observations
                    .entities
                    .get(*reference)
                    .is_some_and(|observation| observation.freshness == Freshness::Fresh)
            })
            .cloned()
            .collect();
    }

    fn apply_connection_state(&mut self, state: ConnectionState) {
        self.connection = state;
        match state {
            ConnectionState::Disconnected | ConnectionState::Stale | ConnectionState::Stopped => {
                self.observations.mark_stale();
            }
            ConnectionState::Connecting
            | ConnectionState::AwaitingHello
            | ConnectionState::Reattaching => self.observations.begin_rebuild(),
            ConnectionState::Ready => {}
            ConnectionState::Stopping => self.observations.mark_stale(),
        }
        self.refresh_unresolved();
    }

    fn apply_hello(&mut self, hello: HelloState) {
        let changed_boot = self
            .recovery
            .boot_id
            .as_ref()
            .is_some_and(|boot| boot != &hello.boot_id);
        if changed_boot {
            self.observations.mark_stale();
        }
        self.recovery.boot_id = Some(hello.boot_id.clone());
        self.recovery.scope = Some(hello.scope.clone());
        self.recovery.next_seq = Some(hello.next_seq);
        self.recovery.event_cursor = Some(hello.event_latest.clone());
        self.hello = Some(hello);
        self.connection = ConnectionState::Ready;
        self.observations.begin_rebuild();
        self.refresh_unresolved();
    }

    fn apply_reply(&mut self, command_id: u64, op: &str, kind: ReplyKind, envelope: Value) {
        if kind != ReplyKind::Result
            && let Some(action) = self.actions.get_mut(&CommandId::new(command_id))
        {
            *action = match kind {
                ReplyKind::MutationAccepted => OperatorActionState::Accepted,
                ReplyKind::MutationCompleted => OperatorActionState::Completed,
                ReplyKind::MutationFailed | ReplyKind::PublicError => OperatorActionState::Failed,
                ReplyKind::Result => unreachable!("Result was excluded above"),
            };
        }
        if kind != ReplyKind::Result {
            return;
        }
        let Some(result) = envelope.get("result").cloned() else {
            return;
        };
        match op {
            "reference" => self.observe_reference(result, None),
            "controller" => {
                if let Some(controller) = result.get("controller").and_then(Value::as_str) {
                    self.observations
                        .observe_controller_full(controller.to_owned(), result);
                }
            }
            "resource" => {
                if let Some(resource) = result.get("resource").and_then(Value::as_str) {
                    self.observations
                        .observe_resource_full(resource.to_owned(), result);
                }
            }
            "component" => self.observe_named(result, "component", |id| RuntimeRef::Component {
                component: id,
            }),
            "recording_status" => self
                .observations
                .observe(RuntimeRef::Recorder, result, None),
            "discover" | "discovery_page" => self.apply_discovery(result),
            "latest" | "measurements_current" | "measurements_page" | "measurement_window" => {
                self.apply_measurements(result)
            }
            "configuration_properties" | "configuration_page" => {
                self.apply_configuration_properties(result)
            }
            _ => {}
        }
        self.refresh_unresolved();
    }

    fn apply_recovery_projection(&mut self, records: Vec<RecoveryRecord>) {
        if records.len() > MAX_IN_FLIGHT {
            self.client_error = Some("worker recovery projection exceeded bound".into());
            return;
        }
        self.recovery.mutations = records;
        self.recovery.reconciliation_required.retain(|identity| {
            self.recovery.mutations.iter().any(|record| {
                record.identity == *identity && requires_reconciliation(record.admission)
            })
        });
    }

    fn apply_event(&mut self, cursor: u64, envelope: Value) {
        let kind = envelope.get("kind").and_then(Value::as_str);
        let target = envelope.get("target");
        let data = envelope.get("data").cloned().unwrap_or(Value::Null);
        let signal_point = (kind == Some("signal"))
            .then(|| live_point_from_signal(&data))
            .flatten();
        let reference = match kind {
            Some("reference") => target
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .map(|id| RuntimeRef::Reference {
                    reference: id.to_owned(),
                }),
            Some("controller") => {
                if let Some(controller) = target
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                {
                    self.observations.observe_controller_event(
                        controller.to_owned(),
                        data.clone(),
                        cursor,
                    );
                }
                None
            }
            Some("resource") => {
                if let Some(resource) = target
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                {
                    self.observations.observe_resource_event(
                        resource.to_owned(),
                        data.clone(),
                        cursor,
                    );
                }
                None
            }
            Some("component") => target
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .map(|id| RuntimeRef::Component {
                    component: id.to_owned(),
                }),
            Some("recorder") => Some(RuntimeRef::Recorder),
            Some("configuration") => {
                self.observations.mark_matching_stale(|identity| {
                    matches!(identity, RuntimeRef::ConfigurationProperty { .. })
                });
                None
            }
            Some("signal" | "measurement") => target.and_then(parse_signal_ref),
            _ => None,
        };
        if let Some(reference) = reference {
            self.observations
                .observe(reference.clone(), data, Some(cursor));
            if let Some(point) = signal_point {
                let _ = self.push_live_point(reference, point);
            }
        }
        self.refresh_unresolved();
    }

    fn observe_reference(&mut self, value: Value, cursor: Option<u64>) {
        if let Some(reference) = value.get("reference").and_then(Value::as_str) {
            self.observations.observe(
                RuntimeRef::Reference {
                    reference: reference.to_owned(),
                },
                value.clone(),
                cursor,
            );
        }
    }

    fn observe_named(&mut self, value: Value, key: &str, build: impl FnOnce(String) -> RuntimeRef) {
        if let Some(id) = value.get(key).and_then(Value::as_str) {
            self.observations.observe(build(id.to_owned()), value, None);
        }
    }

    fn apply_discovery(&mut self, value: Value) {
        let records = value
            .as_array()
            .or_else(|| value.get("records").and_then(Value::as_array));
        let Some(records) = records else {
            return;
        };
        for record in records {
            let Some(kind) = record.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let identity = match kind {
                "instrument" => record
                    .get("id")
                    .and_then(entity_id)
                    .map(|instrument| RuntimeRef::Instrument { instrument }),
                "signal" => record.get("id").and_then(parse_signal_ref),
                "component" => record
                    .get("id")
                    .and_then(entity_id)
                    .map(|component| RuntimeRef::Component { component }),
                "resource" => record
                    .get("id")
                    .and_then(entity_id)
                    .map(|resource| RuntimeRef::Resource { resource }),
                "controller" => record
                    .get("id")
                    .and_then(entity_id)
                    .map(|controller| RuntimeRef::Controller { controller }),
                "reference" => record
                    .get("id")
                    .and_then(entity_id)
                    .map(|reference| RuntimeRef::Reference { reference }),
                _ => None,
            };
            if let Some(identity) = identity {
                // Discovery descriptors establish identity, but a later event-sequence
                // fence must not overwrite a fresher current projection obtained in
                // the same rebuild (for example a signal sample or Reference state).
                if !self
                    .observations
                    .entities
                    .get(&identity)
                    .is_some_and(|observation| observation.freshness == Freshness::Fresh)
                {
                    self.observations.observe(identity, record.clone(), None);
                }
            }
        }
    }

    fn apply_measurements(&mut self, value: Value) {
        if let Some(records) = value
            .get("records")
            .and_then(Value::as_array)
            .or_else(|| value.as_array())
        {
            for record in records {
                if let Some(signal) = record.get("signal").and_then(parse_signal_ref) {
                    self.observations.observe(signal, record.clone(), None);
                }
            }
        } else if let Some(signal) = value.get("signal").and_then(parse_signal_ref) {
            self.observations.observe(signal, value, None);
        }
    }

    fn apply_configuration_properties(&mut self, value: Value) {
        let Some(records) = value.get("records").and_then(Value::as_array) else {
            return;
        };
        for record in records {
            let Some(owner) = record.get("owner") else {
                continue;
            };
            let Some(id) = owner.get("id").and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            let owner = match owner.get("kind").and_then(Value::as_str) {
                Some("instrument") => ConfigurationOwner::Instrument { instrument: id },
                Some("component") => ConfigurationOwner::Component { component: id },
                Some("resource") => ConfigurationOwner::Resource { resource: id },
                _ => continue,
            };
            let Some(property) = record.get("property").and_then(Value::as_str) else {
                continue;
            };
            self.observations.observe(
                RuntimeRef::ConfigurationProperty {
                    owner,
                    property: property.to_owned(),
                },
                record.clone(),
                None,
            );
        }
    }
}

impl OperatorActionState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

fn parse_signal_ref(value: &Value) -> Option<RuntimeRef> {
    Some(RuntimeRef::Signal {
        instrument: value.get("instrument")?.as_str()?.to_owned(),
        parameter: value.get("parameter")?.as_str()?.to_owned(),
    })
}

fn live_point_from_signal(value: &Value) -> Option<LivePoint> {
    if value.get("quality").and_then(Value::as_str) != Some("good") {
        return None;
    }
    let sample = value.get("value")?.as_f64()?;
    let observed_ns = value
        .get("observed_at_ns")?
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())?;
    let point = LivePoint {
        time_seconds: observed_ns / 1_000_000_000.0,
        value: sample,
    };
    (point.time_seconds.is_finite() && point.value.is_finite()).then_some(point)
}

fn entity_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .or_else(|| value.get("id").and_then(Value::as_str))
        .map(str::to_owned)
}

fn parse_cursor(value: &Value) -> Option<EventCursor> {
    let cursor = value.get("cursor").unwrap_or(value);
    let boot_id = cursor.get("boot_id")?.as_str()?.to_owned();
    let seq = cursor.get("seq")?.as_str()?.parse().ok()?;
    Some(EventCursor { boot_id, seq })
}

fn requires_reconciliation(admission: KnownAdmission) -> bool {
    matches!(
        admission,
        KnownAdmission::Pending | KnownAdmission::Accepted | KnownAdmission::Ambiguous
    )
}

#[cfg(test)]
mod tests;
