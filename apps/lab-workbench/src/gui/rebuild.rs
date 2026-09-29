//! One bounded projection/bootstrap coordinator for the minimal GUI.

use crate::{
    client::types::{CommandSendError, EventCursor, HelloState, ReplyKind},
    client::{ClientHandle, ClientUpdate},
    model::WorkbenchModel,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_REBUILD_REFERENCES: usize = 64;
const MAX_REBUILD_CONTROLLERS: usize = 64;
const MAX_REBUILD_RESOURCES: usize = 64;

pub(crate) trait RebuildClient {
    fn connect(&self, scope: Option<String>) -> Result<u64, CommandSendError>;
    fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError>;
    fn subscribe(&self, after: EventCursor, filter: Value) -> Result<u64, CommandSendError>;
}

impl RebuildClient for ClientHandle {
    fn connect(&self, scope: Option<String>) -> Result<u64, CommandSendError> {
        ClientHandle::connect(self, scope)
    }

    fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        ClientHandle::query(self, op, args)
    }

    fn subscribe(&self, after: EventCursor, filter: Value) -> Result<u64, CommandSendError> {
        ClientHandle::subscribe(self, after, filter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Task {
    Discovery,
    DiscoveryPage,
    Measurements,
    MeasurementsPage,
    Reference,
    Controller,
    ControllerRefresh,
    Resource,
    ResourceRefresh,
    Recorder,
    ConfigurationProperties,
    ConfigurationPage,
    Fence,
    Subscribe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RebuildPhase {
    Idle,
    AwaitingHello { command_id: u64 },
    Waiting { command_id: u64, task: Task },
    CatchingUp { barrier: EventCursor },
    Complete,
    Failed,
}

/// Serializes the finite M14.4 discovery/current/reference/subscription barrier.
pub(crate) struct RebuildCoordinator {
    phase: RebuildPhase,
    base_cursor: Option<EventCursor>,
    references: VecDeque<String>,
    seen_references: BTreeSet<String>,
    controllers: VecDeque<String>,
    seen_controllers: BTreeSet<String>,
    resources: VecDeque<String>,
    seen_resources: BTreeSet<String>,
    optional_operations: BTreeSet<String>,
    configuration_refresh_only: bool,
    configuration_dirty: bool,
    dirty_controllers: BTreeMap<String, Value>,
    dirty_resources: BTreeMap<String, Value>,
    settling_rebuild: bool,
}

impl Default for RebuildCoordinator {
    fn default() -> Self {
        Self {
            phase: RebuildPhase::Idle,
            base_cursor: None,
            references: VecDeque::new(),
            seen_references: BTreeSet::new(),
            controllers: VecDeque::new(),
            seen_controllers: BTreeSet::new(),
            resources: VecDeque::new(),
            seen_resources: BTreeSet::new(),
            optional_operations: BTreeSet::new(),
            configuration_refresh_only: false,
            configuration_dirty: false,
            dirty_controllers: BTreeMap::new(),
            dirty_resources: BTreeMap::new(),
            settling_rebuild: false,
        }
    }
}

impl RebuildCoordinator {
    /// Consumes an already-model-applied update and advances at most one command.
    pub(crate) fn after_update(
        &mut self,
        update: &ClientUpdate,
        model: &mut WorkbenchModel,
        client: &impl RebuildClient,
    ) {
        match update {
            ClientUpdate::Hello(hello) => self.begin(hello, model, client),
            ClientUpdate::State(
                crate::client::types::ConnectionState::Disconnected
                | crate::client::types::ConnectionState::Stale
                | crate::client::types::ConnectionState::Stopped,
            ) => {
                self.phase = RebuildPhase::Idle;
            }
            ClientUpdate::ResnapshotRequired {
                connection_lost, ..
            } => {
                if *connection_lost {
                    match client.connect(model.recovery.scope.clone()) {
                        Ok(command_id) => {
                            self.phase = RebuildPhase::AwaitingHello { command_id };
                        }
                        Err(error) => {
                            self.fail(model, &format!("cannot reconnect for rebuild: {error}"));
                        }
                    }
                } else if let Some(hello) = model.hello.clone() {
                    self.begin(&hello, model, client);
                }
            }
            ClientUpdate::LocalRejected { command_id, reason }
                if matches!(
                    self.phase,
                    RebuildPhase::AwaitingHello {
                        command_id: expected
                    } if expected == *command_id
                ) =>
            {
                self.fail(model, &format!("reconnect rejected: {reason}"));
            }
            ClientUpdate::Reply {
                command_id,
                kind,
                envelope,
                ..
            } => self.handle_reply(*command_id, *kind, envelope, model, client),
            ClientUpdate::Event { envelope, .. } => {
                match envelope.get("kind").and_then(Value::as_str) {
                    Some("configuration") => {
                        self.configuration_dirty = true;
                    }
                    Some("controller") => {
                        if let Some(controller) =
                            envelope.pointer("/target/id").and_then(Value::as_str)
                            && !model.controller_detail_is_fresh(controller)
                            && hello_has(model, "controller")
                        {
                            if self.dirty_controllers.len() == MAX_REBUILD_CONTROLLERS
                                && !self.dirty_controllers.contains_key(controller)
                            {
                                self.fail(model, "controller detail-refresh bound exceeded");
                                return;
                            }
                            self.dirty_controllers.insert(
                                controller.to_owned(),
                                envelope
                                    .pointer("/data/revision")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            );
                        }
                    }
                    Some("resource") => {
                        if let Some(resource) =
                            envelope.pointer("/target/id").and_then(Value::as_str)
                            && !model.resource_detail_is_fresh(resource)
                            && hello_has(model, "resource")
                        {
                            if self.dirty_resources.len() == MAX_REBUILD_RESOURCES
                                && !self.dirty_resources.contains_key(resource)
                            {
                                self.fail(model, "resource detail-refresh bound exceeded");
                                return;
                            }
                            self.dirty_resources.insert(
                                resource.to_owned(),
                                envelope
                                    .pointer("/data/generation")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            );
                        }
                    }
                    _ => {}
                }
                if self.phase == RebuildPhase::Complete {
                    self.service_dirty(client, model);
                } else {
                    self.maybe_complete(model, client);
                }
            }
            ClientUpdate::SubscriptionProgress(_) => self.maybe_complete(model, client),
            _ => {}
        }
    }

    fn begin(
        &mut self,
        hello: &HelloState,
        model: &mut WorkbenchModel,
        client: &impl RebuildClient,
    ) {
        let required = [
            "discover",
            "discovery_page",
            "measurements_current",
            "measurements_page",
            "reference",
            "subscribe",
        ];
        if required
            .iter()
            .any(|required| !hello.operations.iter().any(|op| op == required))
        {
            self.fail(
                model,
                "Runtime hello lacks a required M14.4 projection operation",
            );
            return;
        }
        model.observations.begin_rebuild();
        model.refresh_unresolved();
        self.base_cursor = None;
        self.references.clear();
        self.seen_references.clear();
        self.controllers.clear();
        self.seen_controllers.clear();
        self.resources.clear();
        self.seen_resources.clear();
        self.optional_operations = hello.operations.iter().cloned().collect();
        self.configuration_refresh_only = false;
        self.configuration_dirty = false;
        self.dirty_controllers.clear();
        self.dirty_resources.clear();
        self.settling_rebuild = false;
        self.issue_query(client, model, Task::Discovery, "discover", json!({}));
    }

    fn handle_reply(
        &mut self,
        command_id: u64,
        kind: ReplyKind,
        envelope: &Value,
        model: &mut WorkbenchModel,
        client: &impl RebuildClient,
    ) {
        let RebuildPhase::Waiting {
            command_id: expected,
            task,
        } = self.phase.clone()
        else {
            return;
        };
        if command_id != expected {
            return;
        }
        self.phase = RebuildPhase::Idle;
        if kind != ReplyKind::Result && !is_optional_task(&task) {
            self.fail(model, "projection rebuild received an Application error");
            return;
        }
        if kind != ReplyKind::Result {
            self.advance_after_optional(task, model, client);
            return;
        }
        let Some(result) = envelope.get("result") else {
            self.fail(model, "projection rebuild result omitted its body");
            return;
        };
        match task {
            Task::Discovery | Task::DiscoveryPage => {
                if self.base_cursor.is_none() {
                    self.base_cursor = projection_cursor(result);
                    if self.base_cursor.is_none() {
                        self.fail(model, "discovery projection omitted its revision cursor");
                        return;
                    }
                }
                if !self.collect_detail_ids(result, model) {
                    return;
                }
                if let Some((projection, index)) = next_page(result) {
                    self.issue_query(
                        client,
                        model,
                        Task::DiscoveryPage,
                        "discovery_page",
                        json!({"projection":projection,"index":index}),
                    );
                } else {
                    self.issue_query(
                        client,
                        model,
                        Task::Measurements,
                        "measurements_current",
                        json!({}),
                    );
                }
            }
            Task::Measurements | Task::MeasurementsPage => {
                if let Some((projection, index)) = next_page(result) {
                    self.issue_query(
                        client,
                        model,
                        Task::MeasurementsPage,
                        "measurements_page",
                        json!({"projection":projection,"index":index}),
                    );
                } else {
                    self.issue_next_detail_or_fence(client, model);
                }
            }
            Task::Reference | Task::Controller | Task::Resource => {
                self.issue_next_detail_or_fence(client, model)
            }
            Task::ControllerRefresh | Task::ResourceRefresh => self.service_dirty(client, model),
            Task::Recorder => self.issue_configuration_or_fence(client, model),
            Task::ConfigurationProperties | Task::ConfigurationPage => {
                if let Some((projection, index)) = next_page(result) {
                    if hello_has(model, "configuration_page") {
                        self.issue_query(
                            client,
                            model,
                            Task::ConfigurationPage,
                            "configuration_page",
                            json!({"projection":projection,"index":index}),
                        );
                    } else {
                        model.observations.mark_matching_stale(|identity| {
                            matches!(
                                identity,
                                crate::presentation::RuntimeRef::ConfigurationProperty { .. }
                            )
                        });
                        self.finish_configuration(client, model);
                    }
                } else {
                    self.finish_configuration(client, model);
                }
            }
            Task::Fence => {
                let Some(barrier) = projection_cursor(result) else {
                    self.fail(model, "rebuild fence omitted its revision cursor");
                    return;
                };
                let Some(base) = self.base_cursor.clone() else {
                    self.fail(model, "rebuild has no initial event cursor");
                    return;
                };
                match client.subscribe(
                    base,
                    json!({"kinds":["signal","reference","controller","resource","recorder","configuration"],"targets":[]}),
                ) {
                    Ok(command_id) => {
                        self.base_cursor = Some(barrier);
                        self.phase = RebuildPhase::Waiting {
                            command_id,
                            task: Task::Subscribe,
                        };
                    }
                    Err(error) => self.fail(model, &format!("cannot subscribe: {error}")),
                }
            }
            Task::Subscribe => {
                let barrier = self
                    .base_cursor
                    .clone()
                    .expect("fence stored before subscribe");
                self.phase = RebuildPhase::CatchingUp { barrier };
                self.maybe_complete(model, client);
            }
        }
    }

    fn collect_detail_ids(&mut self, result: &Value, model: &mut WorkbenchModel) -> bool {
        let Some(records) = result.get("records").and_then(Value::as_array) else {
            self.fail(model, "discovery projection omitted records");
            return false;
        };
        for record in records {
            let Some(kind) = record.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let id = record
                .pointer("/id/id")
                .or_else(|| record.get("id"))
                .and_then(Value::as_str);
            let Some(id) = id else {
                continue;
            };
            let (seen, queue, limit, name) = match kind {
                "reference" => (
                    &mut self.seen_references,
                    &mut self.references,
                    MAX_REBUILD_REFERENCES,
                    "Reference",
                ),
                "controller" => (
                    &mut self.seen_controllers,
                    &mut self.controllers,
                    MAX_REBUILD_CONTROLLERS,
                    "controller",
                ),
                "resource" => (
                    &mut self.seen_resources,
                    &mut self.resources,
                    MAX_REBUILD_RESOURCES,
                    "resource",
                ),
                _ => continue,
            };
            if seen.insert(id.to_owned()) {
                if queue.len() == limit {
                    self.fail(model, &format!("{name} rebuild bound exceeded"));
                    return false;
                }
                queue.push_back(id.to_owned());
            }
        }
        true
    }

    fn issue_next_detail_or_fence(
        &mut self,
        client: &impl RebuildClient,
        model: &mut WorkbenchModel,
    ) {
        if let Some(reference) = self.references.pop_front() {
            self.issue_query(
                client,
                model,
                Task::Reference,
                "reference",
                json!({"reference":reference}),
            );
        } else if self.optional_operations.contains("controller")
            && let Some(controller) = self.controllers.pop_front()
        {
            self.issue_query(
                client,
                model,
                Task::Controller,
                "controller",
                json!({"controller":controller}),
            );
        } else if self.optional_operations.contains("resource")
            && let Some(resource) = self.resources.pop_front()
        {
            self.issue_query(
                client,
                model,
                Task::Resource,
                "resource",
                json!({"resource":resource}),
            );
        } else if self.optional_operations.contains("recording_status") {
            self.optional_operations.remove("recording_status");
            self.issue_query(client, model, Task::Recorder, "recording_status", json!({}));
        } else {
            self.issue_configuration_or_fence(client, model);
        }
    }

    fn issue_configuration_or_fence(
        &mut self,
        client: &impl RebuildClient,
        model: &mut WorkbenchModel,
    ) {
        if self.optional_operations.remove("configuration_properties") {
            self.issue_query(
                client,
                model,
                Task::ConfigurationProperties,
                "configuration_properties",
                json!({}),
            );
        } else {
            self.issue_fence(client, model);
        }
    }

    fn issue_fence(&mut self, client: &impl RebuildClient, model: &mut WorkbenchModel) {
        // This second frozen projection is an event-sequence fence. Once the
        // subscription has advanced from the first discovery cursor through this
        // cursor, every concurrent change has been applied or forces a gap.
        self.issue_query(client, model, Task::Fence, "discover", json!({}));
    }

    fn finish_configuration(&mut self, client: &impl RebuildClient, model: &mut WorkbenchModel) {
        if self.configuration_refresh_only {
            model.refresh_unresolved();
            if self.configuration_dirty {
                self.start_configuration_refresh(client, model);
            } else {
                self.configuration_refresh_only = false;
                self.service_dirty(client, model);
            }
        } else {
            self.issue_fence(client, model);
        }
    }

    fn advance_after_optional(
        &mut self,
        task: Task,
        model: &mut WorkbenchModel,
        client: &impl RebuildClient,
    ) {
        match task {
            Task::Controller | Task::Resource => self.issue_next_detail_or_fence(client, model),
            Task::ControllerRefresh | Task::ResourceRefresh => self.service_dirty(client, model),
            Task::Recorder => self.issue_configuration_or_fence(client, model),
            Task::ConfigurationProperties | Task::ConfigurationPage => {
                self.finish_configuration(client, model)
            }
            _ => self.fail(model, "unexpected optional rebuild failure"),
        }
    }

    fn issue_query(
        &mut self,
        client: &impl RebuildClient,
        model: &mut WorkbenchModel,
        task: Task,
        op: &str,
        args: Value,
    ) {
        match client.query(op, args) {
            Ok(command_id) => self.phase = RebuildPhase::Waiting { command_id, task },
            Err(error) => self.fail(model, &format!("cannot submit rebuild query: {error}")),
        }
    }

    fn maybe_complete(&mut self, model: &mut WorkbenchModel, client: &impl RebuildClient) {
        let RebuildPhase::CatchingUp { barrier } = &self.phase else {
            return;
        };
        let caught_up =
            model.recovery.event_cursor.as_ref().is_some_and(|cursor| {
                cursor.boot_id == barrier.boot_id && cursor.seq >= barrier.seq
            });
        if caught_up {
            self.settling_rebuild = true;
            self.service_dirty(client, model);
        }
    }

    fn service_dirty(&mut self, client: &impl RebuildClient, model: &mut WorkbenchModel) {
        if matches!(self.phase, RebuildPhase::Waiting { .. }) {
            return;
        }
        while let Some((controller, revision)) = self
            .dirty_controllers
            .iter()
            .next()
            .map(|(controller, revision)| (controller.clone(), revision.clone()))
        {
            self.dirty_controllers.remove(&controller);
            if model.controller_detail_is_fresh(&controller)
                && model.controller_detail_revision(&controller) == Some(&revision)
            {
                continue;
            }
            model.mark_controller_detail_stale(&controller);
            self.issue_query(
                client,
                model,
                Task::ControllerRefresh,
                "controller",
                json!({"controller":controller}),
            );
            return;
        }
        while let Some((resource, generation)) = self
            .dirty_resources
            .iter()
            .next()
            .map(|(resource, generation)| (resource.clone(), generation.clone()))
        {
            self.dirty_resources.remove(&resource);
            if model.resource_detail_is_fresh(&resource)
                && model.resource_detail_generation(&resource) == Some(&generation)
            {
                continue;
            }
            model.mark_resource_detail_stale(&resource);
            self.issue_query(
                client,
                model,
                Task::ResourceRefresh,
                "resource",
                json!({"resource":resource}),
            );
            return;
        }
        if self.configuration_dirty && hello_has(model, "configuration_properties") {
            self.start_configuration_refresh(client, model);
        } else {
            self.configuration_dirty = false;
            self.configuration_refresh_only = false;
            self.phase = RebuildPhase::Complete;
            if self.settling_rebuild {
                self.settling_rebuild = false;
                model.complete_rebuild();
            }
        }
    }

    fn start_configuration_refresh(
        &mut self,
        client: &impl RebuildClient,
        model: &mut WorkbenchModel,
    ) {
        self.configuration_dirty = false;
        self.configuration_refresh_only = true;
        model.observations.mark_matching_stale(|identity| {
            matches!(
                identity,
                crate::presentation::RuntimeRef::ConfigurationProperty { .. }
            )
        });
        self.issue_query(
            client,
            model,
            Task::ConfigurationProperties,
            "configuration_properties",
            json!({}),
        );
    }

    fn fail(&mut self, model: &mut WorkbenchModel, reason: &str) {
        self.phase = RebuildPhase::Failed;
        model.client_error = Some(reason.to_owned());
    }
}

fn is_optional_task(task: &Task) -> bool {
    matches!(
        task,
        Task::Controller
            | Task::ControllerRefresh
            | Task::Resource
            | Task::ResourceRefresh
            | Task::Recorder
            | Task::ConfigurationProperties
            | Task::ConfigurationPage
    )
}

fn hello_has(model: &WorkbenchModel, operation: &str) -> bool {
    model
        .hello
        .as_ref()
        .is_some_and(|hello| hello.operations.iter().any(|item| item == operation))
}

fn projection_cursor(result: &Value) -> Option<EventCursor> {
    let revision = result.get("revision")?;
    Some(EventCursor {
        boot_id: revision.get("boot_id")?.as_str()?.to_owned(),
        seq: revision.get("event_seq")?.as_str()?.parse().ok()?,
    })
}

fn next_page(result: &Value) -> Option<(String, String)> {
    let index = result.get("next_index")?.as_str()?.to_owned();
    let projection = result.get("projection")?.as_str()?.to_owned();
    Some((projection, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::types::ConnectionState, model::Freshness, presentation::PresentationDocument,
    };
    use std::{cell::RefCell, collections::VecDeque};

    #[derive(Clone, Debug, PartialEq)]
    enum Sent {
        Connect(u64, Option<String>),
        Query(u64, String, Value),
        Subscribe(u64, EventCursor, Value),
    }

    #[derive(Default)]
    struct FakeClient {
        next: RefCell<u64>,
        sent: RefCell<VecDeque<Sent>>,
    }

    impl RebuildClient for FakeClient {
        fn connect(&self, scope: Option<String>) -> Result<u64, CommandSendError> {
            let id = next_id(&self.next);
            self.sent.borrow_mut().push_back(Sent::Connect(id, scope));
            Ok(id)
        }

        fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
            let id = next_id(&self.next);
            self.sent
                .borrow_mut()
                .push_back(Sent::Query(id, op.to_owned(), args));
            Ok(id)
        }

        fn subscribe(&self, after: EventCursor, filter: Value) -> Result<u64, CommandSendError> {
            let id = next_id(&self.next);
            self.sent
                .borrow_mut()
                .push_back(Sent::Subscribe(id, after, filter));
            Ok(id)
        }
    }

    fn next_id(next: &RefCell<u64>) -> u64 {
        let mut value = next.borrow_mut();
        *value += 1;
        *value
    }

    fn hello() -> HelloState {
        HelloState {
            boot_id: "boot".into(),
            scope: "scope".into(),
            next_seq: 1,
            operations: vec![
                "discover",
                "discovery_page",
                "measurements_current",
                "measurements_page",
                "reference",
                "subscribe",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot".into(),
                seq: 1,
            },
        }
    }

    fn operator_hello() -> HelloState {
        let mut value = hello();
        value.operations.extend(
            [
                "controller",
                "controller_configure_pid",
                "resource",
                "recording_status",
                "configuration_properties",
                "configuration_page",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        value
    }

    fn reply(command_id: u64, op: &str, result: Value) -> ClientUpdate {
        ClientUpdate::Reply {
            command_id,
            msg_id: command_id.to_string(),
            op: op.into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":result}),
            recovery: None,
        }
    }

    fn apply(
        coordinator: &mut RebuildCoordinator,
        model: &mut WorkbenchModel,
        client: &FakeClient,
        update: ClientUpdate,
    ) {
        model.apply_client_update(update.clone());
        coordinator.after_update(&update, model, client);
    }

    #[test]
    fn global_freshness_requires_snapshot_subscription_and_cursor_barrier() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        let mut coordinator = RebuildCoordinator::default();
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Hello(hello()),
        );
        assert_eq!(model.observations.freshness, Freshness::Rebuilding);

        let Sent::Query(discover, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                discover,
                "discover",
                json!({
                    "projection":"d","revision":{"boot_id":"boot","event_seq":"2"},
                    "records":[{"kind":"reference","id":{"id":"1"}}],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Query(measurements, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "measurements_current");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                measurements,
                &op,
                json!({
                    "projection":"m","revision":{"boot_id":"boot","event_seq":"3"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Query(reference, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "reference");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                reference,
                &op,
                json!({
                    "reference":"1","revision":"1","value":10.0
                }),
            ),
        );
        let Sent::Query(fence, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                fence,
                &op,
                json!({
                    "projection":"f","revision":{"boot_id":"boot","event_seq":"5"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Subscribe(subscribe, after, _) = client.sent.borrow_mut().pop_front().unwrap()
        else {
            panic!()
        };
        assert_eq!(after.seq, 2);
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                subscribe,
                "subscribe",
                json!({
                    "subscription":"s","accepted_cursor":{"boot_id":"boot","seq":"2"}
                }),
            ),
        );
        assert_eq!(model.observations.freshness, Freshness::Rebuilding);

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::SubscriptionProgress(json!({"cursor":{"boot_id":"boot","seq":"5"}})),
        );
        assert_eq!(model.observations.freshness, Freshness::Fresh);
    }

    #[test]
    fn disconnect_and_gap_never_claim_global_freshness() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.observations.complete_rebuild();
        let mut coordinator = RebuildCoordinator::default();
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::State(ConnectionState::Disconnected),
        );
        assert_eq!(model.observations.freshness, Freshness::Stale);
        model.apply_client_update(ClientUpdate::Hello(hello()));
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::ResnapshotRequired {
                reason: "event_gap".into(),
                envelope: None,
                connection_lost: false,
            },
        );
        assert_eq!(model.observations.freshness, Freshness::Rebuilding);
        assert!(matches!(
            client.sent.borrow_mut().pop_front(),
            Some(Sent::Query(_, ref op, _)) if op == "discover"
        ));
    }

    #[test]
    fn transport_lost_resnapshot_reconnects_before_hello_restarts_the_barrier() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(hello()));
        model.complete_rebuild();
        let mut coordinator = RebuildCoordinator::default();

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::ResnapshotRequired {
                reason: "ordered_update_queue_full".into(),
                envelope: None,
                connection_lost: true,
            },
        );
        assert_eq!(model.connection, ConnectionState::Stale);
        assert!(matches!(
            client.sent.borrow_mut().pop_front(),
            Some(Sent::Connect(_, Some(ref scope))) if scope == "scope"
        ));
        assert!(
            client.sent.borrow().is_empty(),
            "stale transport was queried"
        );

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::State(ConnectionState::Reattaching),
        );
        assert!(client.sent.borrow().is_empty());
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Hello(hello()),
        );

        let Sent::Query(discover, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                discover,
                &op,
                json!({
                    "projection":"d","revision":{"boot_id":"boot","event_seq":"2"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Query(measurements, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "measurements_current");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                measurements,
                &op,
                json!({
                    "projection":"m","revision":{"boot_id":"boot","event_seq":"3"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Query(fence, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                fence,
                &op,
                json!({
                    "projection":"f","revision":{"boot_id":"boot","event_seq":"5"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Subscribe(subscribe, after, _) = client.sent.borrow_mut().pop_front().unwrap()
        else {
            panic!()
        };
        assert_eq!(after.seq, 2);
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                subscribe,
                "subscribe",
                json!({
                    "subscription":"s","accepted_cursor":{"boot_id":"boot","seq":"2"}
                }),
            ),
        );
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::SubscriptionProgress(json!({
                "cursor":{"boot_id":"boot","seq":"5"}
            })),
        );
        assert_eq!(model.observations.freshness, Freshness::Fresh);
    }

    #[test]
    fn operator_rebuild_queries_bounded_optional_domains_before_one_aggregate_subscription() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        let mut coordinator = RebuildCoordinator::default();
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Hello(operator_hello()),
        );

        let Sent::Query(discover, _, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                discover,
                "discover",
                json!({
                    "projection":"d","revision":{"boot_id":"boot","event_seq":"2"},
                    "records":[
                        {"kind":"reference","id":{"id":"1"}},
                        {"kind":"controller","id":{"id":"2"}},
                        {"kind":"resource","id":{"id":"3"}}
                    ],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Query(measurements, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "measurements_current");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                measurements,
                &op,
                json!({
                    "projection":"m","revision":{"boot_id":"boot","event_seq":"3"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );

        for (expected_op, result) in [
            (
                "reference",
                json!({"reference":"1","kind":"ramp","revision":"1","target":1.0,"rate":1.0}),
            ),
            (
                "controller",
                json!({"controller":"2","state":"ready","revision":"1","config":{"pid":{"kp":1.0,"ki":0.0,"kd":0.0,"output_min":0.0,"output_max":100.0}}}),
            ),
            (
                "resource",
                json!({"resource":"3","state":"idle","binding_generation":"1","capabilities":{"reconnect":true}}),
            ),
            (
                "recording_status",
                json!({"state":"idle","active_run":null}),
            ),
            (
                "configuration_properties",
                json!({"projection":"p","records":[],"next_index":null,"complete":true}),
            ),
        ] {
            let Sent::Query(id, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
                panic!()
            };
            assert_eq!(op, expected_op);
            apply(
                &mut coordinator,
                &mut model,
                &client,
                reply(id, &op, result),
            );
        }
        let Sent::Query(fence, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                fence,
                &op,
                json!({
                    "projection":"f","revision":{"boot_id":"boot","event_seq":"5"},
                    "records":[],"next_index":null,"complete":true
                }),
            ),
        );
        let Sent::Subscribe(_, _, filter) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(
            filter["kinds"],
            json!([
                "signal",
                "reference",
                "controller",
                "resource",
                "recorder",
                "configuration"
            ])
        );
        assert!(client.sent.borrow().is_empty());
    }

    #[test]
    fn configuration_event_refreshes_properties_without_a_second_subscription() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(operator_hello()));
        model.complete_rebuild();
        let mut coordinator = RebuildCoordinator {
            phase: RebuildPhase::Complete,
            ..Default::default()
        };

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
                envelope: json!({"kind":"configuration","target":{"id":"runtime"},"data":{"revision":"2"}}),
            },
        );
        let Sent::Query(query, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "configuration_properties");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                query,
                &op,
                json!({
                    "projection":"p","records":[],"next_index":null,"complete":true
                }),
            ),
        );
        assert_eq!(coordinator.phase, RebuildPhase::Complete);
        assert!(
            client.sent.borrow().is_empty(),
            "configuration refresh created another subscription"
        );
    }

    #[test]
    fn changed_controller_revision_requests_one_full_detail_refresh() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(operator_hello()));
        model.apply_client_update(reply(
            90,
            "controller",
            json!({"controller":"2","state":"ready","revision":"7",
                "config":{"pid":{"kp":1.0,"ki":0.0,"kd":0.0,
                "output_min":0.0,"output_max":10.0}}}),
        ));
        model.complete_rebuild();
        let mut coordinator = RebuildCoordinator {
            phase: RebuildPhase::Complete,
            ..Default::default()
        };

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
                envelope: json!({"kind":"controller","target":{"id":"2"},
                    "data":{"state":"running","status":"valid","failure":null,"active":true,
                        "paused":false,"revision":"7","last_tick":"1","latest_output":1.0}}),
            },
        );
        assert!(client.sent.borrow().is_empty());
        assert!(model.controller_detail_is_fresh("2"));
        assert_eq!(
            model.observations.entities[&crate::presentation::RuntimeRef::Controller {
                controller: "2".into()
            }]
                .value["config"]["pid"]["kp"],
            1.0
        );

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 7,
                },
                envelope: json!({"kind":"controller","target":{"id":"2"},
                    "data":{"state":"paused","status":"valid","failure":null,"active":false,
                        "paused":true,"revision":"8","last_tick":"2","latest_output":0.0}}),
            },
        );
        assert!(!model.controller_detail_is_fresh("2"));
        let Sent::Query(refresh, op, args) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "controller");
        assert_eq!(args, json!({"controller":"2"}));
        assert!(client.sent.borrow().is_empty());

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 8,
                },
                envelope: json!({"kind":"controller","target":{"id":"2"},
                    "data":{"state":"paused","status":"valid","failure":null,"active":false,
                        "paused":true,"revision":"8","last_tick":"3","latest_output":0.0}}),
            },
        );
        assert!(client.sent.borrow().is_empty());

        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                refresh,
                "controller",
                json!({"controller":"2","state":"paused","revision":"8",
                    "config":{"pid":{"kp":2.0,"ki":0.0,"kd":0.0,
                    "output_min":0.0,"output_max":10.0}}}),
            ),
        );
        assert!(model.controller_detail_is_fresh("2"));
        assert_eq!(coordinator.phase, RebuildPhase::Complete);
        assert!(client.sent.borrow().is_empty());
    }

    #[test]
    fn changed_resource_generation_coalesces_one_full_detail_refresh() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(operator_hello()));
        model.apply_client_update(reply(
            90,
            "resource",
            json!({"resource":"3","state":"idle","binding_generation":"4",
                "transport_generation":"9","capabilities":{"reconnect":true},
                "configuration_revision":"2","deployment":{"port":"COM5"}}),
        ));
        model.complete_rebuild();
        let mut coordinator = RebuildCoordinator {
            phase: RebuildPhase::Complete,
            ..Default::default()
        };

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
                envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
                    "state":"in_flight","queue_len":1,"generation":"9","active":"12",
                    "latest":null}}),
            },
        );
        assert!(client.sent.borrow().is_empty());
        assert!(model.resource_detail_is_fresh("3"));

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 7,
                },
                envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
                    "state":"recovering","queue_len":0,"generation":"10","active":null,
                    "latest":null}}),
            },
        );
        assert!(!model.resource_detail_is_fresh("3"));
        let Sent::Query(refresh, op, args) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "resource");
        assert_eq!(args, json!({"resource":"3"}));

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 8,
                },
                envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
                    "state":"recovering","queue_len":0,"generation":"10","active":null,
                    "latest":null}}),
            },
        );
        assert!(client.sent.borrow().is_empty());

        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                refresh,
                "resource",
                json!({"resource":"3","state":"idle","binding_generation":"5",
                    "transport_generation":"10","capabilities":{"reconnect":true},
                    "configuration_revision":"3","deployment":{"port":"COM5"}}),
            ),
        );
        assert!(model.resource_detail_is_fresh("3"));
        assert_eq!(
            model.observations.entities[&crate::presentation::RuntimeRef::Resource {
                resource: "3".into()
            }]
                .value["binding_generation"],
            "5"
        );
        assert_eq!(coordinator.phase, RebuildPhase::Complete);
        assert!(client.sent.borrow().is_empty());
    }

    #[test]
    fn resource_event_without_advertised_query_retains_partial_fact_without_refresh() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(hello()));
        model.apply_client_update(reply(
            90,
            "resource",
            json!({"resource":"3","state":"idle","binding_generation":"4",
                "transport_generation":"9","capabilities":{"reconnect":true}}),
        ));
        model.complete_rebuild();
        let mut coordinator = RebuildCoordinator {
            phase: RebuildPhase::Complete,
            ..Default::default()
        };
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
                envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
                    "state":"recovering","queue_len":0,"generation":"10","active":null,
                    "latest":null}}),
            },
        );
        assert!(!model.resource_detail_is_fresh("3"));
        assert_eq!(
            model.observations.entities[&crate::presentation::RuntimeRef::Resource {
                resource: "3".into()
            }]
                .value["transport_generation"],
            "10"
        );
        assert!(client.sent.borrow().is_empty());
    }

    #[test]
    fn configuration_events_coalesce_and_catch_up_before_global_fresh() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(operator_hello()));
        let property = crate::presentation::RuntimeRef::ConfigurationProperty {
            owner: crate::presentation::ConfigurationOwner::Instrument {
                instrument: "1".into(),
            },
            property: "poll_ms".into(),
        };
        model.observations.observe(
            property.clone(),
            json!({"owner":{"kind":"instrument","id":"1"},"property":"poll_ms",
                "value_type":"integer","current":1,"access":"read_write","revision":"1"}),
            None,
        );
        model.observations.freshness = Freshness::Rebuilding;
        let mut coordinator = RebuildCoordinator {
            phase: RebuildPhase::CatchingUp {
                barrier: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
            },
            ..Default::default()
        };

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 6,
                },
                envelope: json!({"kind":"configuration","target":{"id":"runtime"},
                    "data":{"revision":"2"}}),
            },
        );
        assert_eq!(model.observations.freshness, Freshness::Rebuilding);
        assert_eq!(
            model.observations.entities[&property].freshness,
            Freshness::Stale
        );
        let Sent::Query(first, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "configuration_properties");

        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Event {
                cursor: EventCursor {
                    boot_id: "boot".into(),
                    seq: 7,
                },
                envelope: json!({"kind":"configuration","target":{"id":"runtime"},
                    "data":{"revision":"3"}}),
            },
        );
        assert!(
            client.sent.borrow().is_empty(),
            "second event must coalesce"
        );
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                first,
                &op,
                json!({"projection":"a","records":[{"owner":{"kind":"instrument","id":"1"},
                    "property":"poll_ms","value_type":"integer","current":2,
                    "access":"read_write","revision":"2"}],"next_index":null,"complete":true}),
            ),
        );
        assert_eq!(model.observations.freshness, Freshness::Rebuilding);
        assert_eq!(
            model.observations.entities[&property].freshness,
            Freshness::Stale
        );
        let Sent::Query(second, second_op, _) = client.sent.borrow_mut().pop_front().unwrap()
        else {
            panic!()
        };
        assert_eq!(second_op, "configuration_properties");
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                second,
                &second_op,
                json!({"projection":"b","records":[{"owner":{"kind":"instrument","id":"1"},
                    "property":"poll_ms","value_type":"integer","current":3,
                    "access":"read_write","revision":"3"}],"next_index":null,"complete":true}),
            ),
        );
        assert_eq!(model.observations.freshness, Freshness::Fresh);
        assert_eq!(
            model.observations.entities[&property].freshness,
            Freshness::Fresh
        );
        assert_eq!(model.observations.entities[&property].value["current"], 3);
        assert!(client.sent.borrow().is_empty());
    }

    #[test]
    fn unsupported_optional_detail_operations_are_never_issued() {
        let client = FakeClient::default();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        let mut coordinator = RebuildCoordinator::default();
        apply(
            &mut coordinator,
            &mut model,
            &client,
            ClientUpdate::Hello(hello()),
        );
        let Sent::Query(discover, _, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                discover,
                "discover",
                json!({"projection":"d","revision":{"boot_id":"boot","event_seq":"2"},
                    "records":[{"kind":"controller","id":{"id":"2"}},
                    {"kind":"resource","id":{"id":"3"}}],"next_index":null,"complete":true}),
            ),
        );
        let Sent::Query(measurements, _, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        apply(
            &mut coordinator,
            &mut model,
            &client,
            reply(
                measurements,
                "measurements_current",
                json!({"projection":"m","revision":{"boot_id":"boot","event_seq":"3"},
                    "records":[],"next_index":null,"complete":true}),
            ),
        );
        let Sent::Query(_, op, _) = client.sent.borrow_mut().pop_front().unwrap() else {
            panic!()
        };
        assert_eq!(op, "discover", "unsupported optional detail was issued");
    }
}
