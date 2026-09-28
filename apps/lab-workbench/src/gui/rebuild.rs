//! One bounded projection/bootstrap coordinator for the minimal GUI.

use crate::{
    client::types::{CommandSendError, EventCursor, HelloState, ReplyKind},
    client::{ClientHandle, ClientUpdate},
    model::WorkbenchModel,
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};

const MAX_REBUILD_REFERENCES: usize = 64;

pub(super) trait RebuildClient {
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
}

impl Default for RebuildCoordinator {
    fn default() -> Self {
        Self {
            phase: RebuildPhase::Idle,
            base_cursor: None,
            references: VecDeque::new(),
            seen_references: BTreeSet::new(),
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
            ClientUpdate::Event { .. } | ClientUpdate::SubscriptionProgress(_) => {
                self.maybe_complete(model)
            }
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
        if kind != ReplyKind::Result {
            self.fail(model, "projection rebuild received an Application error");
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
                if !self.collect_references(result, model) {
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
                    self.issue_next_reference_or_fence(client, model);
                }
            }
            Task::Reference => self.issue_next_reference_or_fence(client, model),
            Task::Fence => {
                let Some(barrier) = projection_cursor(result) else {
                    self.fail(model, "rebuild fence omitted its revision cursor");
                    return;
                };
                let Some(base) = self.base_cursor.clone() else {
                    self.fail(model, "rebuild has no initial event cursor");
                    return;
                };
                match client.subscribe(base, json!({"kinds":["signal","reference"],"targets":[]})) {
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
                self.maybe_complete(model);
            }
        }
    }

    fn collect_references(&mut self, result: &Value, model: &mut WorkbenchModel) -> bool {
        let Some(records) = result.get("records").and_then(Value::as_array) else {
            self.fail(model, "discovery projection omitted records");
            return false;
        };
        for record in records {
            if record.get("kind").and_then(Value::as_str) != Some("reference") {
                continue;
            }
            let id = record
                .pointer("/id/id")
                .or_else(|| record.get("id"))
                .and_then(Value::as_str);
            let Some(id) = id else {
                continue;
            };
            if self.seen_references.insert(id.to_owned()) {
                if self.references.len() == MAX_REBUILD_REFERENCES {
                    self.fail(model, "reference rebuild bound exceeded");
                    return false;
                }
                self.references.push_back(id.to_owned());
            }
        }
        true
    }

    fn issue_next_reference_or_fence(
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
        } else {
            // This second frozen projection is an event-sequence fence. Once the
            // subscription has advanced from the first discovery cursor through
            // this cursor, every change concurrent with the snapshots is applied.
            self.issue_query(client, model, Task::Fence, "discover", json!({}));
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

    fn maybe_complete(&mut self, model: &mut WorkbenchModel) {
        let RebuildPhase::CatchingUp { barrier } = &self.phase else {
            return;
        };
        let caught_up =
            model.recovery.event_cursor.as_ref().is_some_and(|cursor| {
                cursor.boot_id == barrier.boot_id && cursor.seq >= barrier.seq
            });
        if caught_up {
            model.complete_rebuild();
            self.phase = RebuildPhase::Complete;
        }
    }

    fn fail(&mut self, model: &mut WorkbenchModel, reason: &str) {
        self.phase = RebuildPhase::Failed;
        model.client_error = Some(reason.to_owned());
    }
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
}
