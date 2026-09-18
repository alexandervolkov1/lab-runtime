//! Accepted local Application API facade and bounded delivery owner.
//!
//! `server` decodes bounded NDJSON with [`crate::wire`]; [`crate::protocol`] supplies
//! the fixed 42-operation registry, capabilities, limits and public error taxonomy.
//! [`crate::application::Application`] owns sessions, deduplication, connection delivery and asynchronous
//! operation correlation, then routes semantic work to focused modules:
//!
//! - `discovery` — discovery, instrument descriptions and component projections;
//! - [`crate::measurements`] — current and recent in-memory measurements;
//! - `references` and `controllers` — native control-facing operations;
//! - [`crate::recorder_api`] — Recorder DTO validation and projections;
//! - [`crate::configuration_api`] — resources and deployment properties;
//! - `virtuals` — virtual publication and native-model lifecycle;
//! - `delivery` — frozen pages, subscriptions and replay delivery.
//!
//! `operations` is the single typed mutation router below the authoritative
//! registry. Domain modules do not duplicate session admission or construct public
//! error envelopes. Runtime remains the sole authoritative experiment owner.
//!
//! Frozen projections are connection-local delivery pages, not the removed M9C
//! `runtime_snapshot` operation family and not experiment authority. Accepted
//! operation records precede mutation dispatch; terminal records precede any attempt
//! to offer a reply to the network.

pub(crate) mod common;
mod controllers;
mod delivery;
mod discovery;
mod operations;
pub(crate) mod projections;
mod recording;
mod references;
mod virtuals;

use self::{
    common::{
        domain_code, error_reply, operation_reply, operation_state, operation_status,
        parse_request_id, session_code,
    },
    operations::{dispatch, recorded_intent, recorded_terminal},
    recording::AdmittedRecorderRequest,
};

use crate::recorder::{HistoryCursor, HistoryPage, OperationRecord, RunsCursor, RunsPage};
use crate::{
    configuration_api,
    host::Clock,
    protocol,
    service::ServiceHost,
    sessions::{Admission, Mutation, OperationState, SessionError, SessionStore},
    wire::{WireRequest, WireRequestId},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

struct FrozenProjection {
    token: String,
    kind: &'static str,
    cursor: u64,
    records: Vec<Value>,
    expires: Duration,
}
struct Subscription {
    token: String,
    scan: u64,
    kinds: Vec<String>,
    targets: Vec<FilterTarget>,
}
struct FilterTarget {
    kind: String,
    target: Value,
}
struct PendingShutdown {
    connection: u64,
    scope: String,
    seq: u64,
    msg: String,
}
struct PendingRecording {
    connection: u64,
    scope: String,
    seq: u64,
    msg: String,
    start: bool,
    run_no: Option<u64>,
    interval_no: Option<u64>,
}
struct PendingHistory {
    scope: String,
    seq: u64,
    msg: String,
    job: u64,
    submitted: Duration,
    runs: bool,
}
struct HistoryCache {
    token: String,
    page: Value,
    expires: Duration,
}
struct HistoryContinuation {
    token: String,
    connection: u64,
    scope: String,
    cursor: RetainedHistoryCursor,
    expires: Duration,
}
pub(crate) const MAX_HISTORY_CURSORS: usize = 8;
enum RetainedHistoryCursor {
    Measurements(HistoryCursor),
    Runs(RunsCursor),
}
enum CompletedHistory {
    Measurements(Box<HistoryPage>),
    Runs(RunsPage),
}

/// Single-owner fixed API state; a TCP connection carries no domain authority.
pub struct Application {
    sessions: SessionStore,
    clients: BTreeMap<u64, String>,
    projections: BTreeMap<u64, FrozenProjection>,
    subscriptions: BTreeMap<u64, Subscription>,
    next_token: u64,
    pending_shutdown: Option<PendingShutdown>,
    pending_recording: Option<PendingRecording>,
    pending_history: BTreeMap<u64, PendingHistory>,
    history_pages: BTreeMap<u64, HistoryCache>,
    history_cursors: BTreeMap<String, HistoryContinuation>,
    orphan_history: Vec<u64>,
}
impl Application {
    /// Create bounded process-local coordination state for one fresh boot.
    pub fn new(boot_id: &str) -> Result<Self, SessionError> {
        Ok(Self {
            sessions: SessionStore::new(boot_id)?,
            clients: BTreeMap::new(),
            projections: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            next_token: 1,
            pending_shutdown: None,
            pending_recording: None,
            pending_history: BTreeMap::new(),
            history_pages: BTreeMap::new(),
            history_cursors: BTreeMap::new(),
            orphan_history: Vec::new(),
        })
    }

    /// Detach a connection while retaining already admitted operation outcomes.
    pub fn detach(&mut self, service: &ServiceHost, connection: u64) {
        self.clients.remove(&connection);
        self.projections.remove(&connection);
        self.subscriptions.remove(&connection);
        self.history_pages.remove(&connection);
        self.history_cursors
            .retain(|_, cursor| cursor.connection != connection);
        if let Some(pending) = self.pending_history.remove(&connection) {
            let terminal = OperationState::Failed("client_disconnected".into());
            self.sessions
                .complete(&pending.scope, pending.seq, terminal, service.clock().now())
                .expect("detached admitted history operation");
            self.orphan_history.push(pending.job);
        }
        self.sessions.detach(connection, service.clock().now());
    }
    /// Expire frozen connection projections at a trusted monotonic owner instant.
    pub fn expire_projections_at(&mut self, now: Duration) {
        self.projections
            .retain(|_, projection| now < projection.expires);
        self.history_pages.retain(|_, page| now < page.expires);
        self.history_cursors
            .retain(|_, cursor| now < cursor.expires);
    }
    /// Store the safe/cleanup terminal outcome before any network offer.
    pub fn finish_shutdown(
        &mut self,
        service: &mut ServiceHost,
        status: crate::host::ShutdownStatus,
    ) -> Vec<(u64, Value)> {
        let Some(pending) = self.pending_shutdown.take() else {
            return Vec::new();
        };
        let result = json!({"safe_confirmed":status.safe_confirmed,"unfinished_workers":status.unfinished_workers,
            "unfinished_transports":status.unfinished_transports,
            "transports_closed":status.transports_closed,
            "fatal_error":status.fatal_error,
            "cleanup_complete":status.unfinished_workers==0 && status.transports_closed,
            "recorder_flushed":status.recorder_flushed,
            "recorder_unfinished":status.recorder_unfinished,
            "recorder_error":status.recorder_error,
            "exit_success":status.exit_success,
            "outputs":service.owner().output_safe_records()});
        let state = if status.exit_success {
            OperationState::Completed(result.to_string())
        } else {
            OperationState::FailedWithResult {
                code: if status.fatal_error {
                    "fatal_owner_error"
                } else if status.recorder_error {
                    "recording_flush_failed"
                } else if status.recorder_unfinished {
                    "recording_flush_timeout"
                } else if status.safe_confirmed {
                    "cleanup_incomplete"
                } else {
                    "safe_unconfirmed"
                }
                .into(),
                detail: result.to_string(),
            }
        };
        self.sessions
            .complete(
                &pending.scope,
                pending.seq,
                state.clone(),
                service.clock().now(),
            )
            .expect("retained admitted shutdown record");
        let event_state = operation_state(state.clone());
        let published = service.clock().now();
        let owner = service.owner_mut();
        if owner
            .event_log_mut()
            .host_state(
                published,
                if status.exit_success {
                    "stopped"
                } else {
                    "failed"
                },
                result.clone(),
            )
            .is_ok()
        {
            let _ = owner.event_log_mut().operation_terminal(
                published,
                &pending.scope,
                pending.seq,
                event_state,
            );
        }
        if self.clients.get(&pending.connection) != Some(&pending.scope) {
            return Vec::new();
        }
        let rid = WireRequestId {
            scope: pending.scope,
            seq: pending.seq,
        };
        vec![(
            pending.connection,
            operation_reply(&pending.msg, &rid, state),
        )]
    }

    /// Process one parsed request in owner order. Replies are owned and bounded by
    /// the network encoder; a disconnected client cannot roll back a mutation.
    pub fn handle(
        &mut self,
        service: &mut ServiceHost,
        connection: u64,
        request: WireRequest,
    ) -> Vec<Value> {
        let msg = request.msg_id.clone();
        let result = if request.op == "hello" {
            if service.is_stopping() {
                Err("shutdown_in_progress")
            } else if self.clients.contains_key(&connection) {
                Err("already_hello")
            } else {
                let old = request.args.get("scope").and_then(Value::as_str);
                if request
                    .args
                    .get("scope")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
                {
                    Err("invalid_args")
                } else {
                    self.sessions
                        .open(old, connection, service.clock().now())
                        .map_err(session_code)
                        .map(|opened| {
                            self.clients.insert(connection, opened.scope.clone());
                            let features = service.protocol_features();
                            json!({"boot_id":service.boot_id(),"v":protocol::PROTOCOL_VERSION,
                                "protocol":{"id":protocol::PROTOCOL_ID,
                                    "version":protocol::PROTOCOL_VERSION},
                                "application":{"api_version":protocol::APPLICATION_API_VERSION,
                                    "package_version":env!("CARGO_PKG_VERSION")},
                                "scope":opened.scope,
                                "next_seq":opened.next_seq.to_string(),"state":"ready",
                                "capabilities":protocol::capabilities(features),
                                "operations":protocol::supported_operations(features),
                                "limits":protocol::limits(),
                                "event_oldest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().oldest_cursor().to_string()},
                                "event_latest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().latest_cursor().to_string()}})
                        })
                }
            }
        } else if !self.clients.contains_key(&connection) {
            Err("hello_required")
        } else if protocol::operation_spec(&request.op).is_none_or(|operation| {
            !protocol::operation_supported(operation, service.protocol_features())
        }) {
            Err("unsupported_operation")
        } else if request.request_id.is_some() {
            return self.handle_mutation(service, connection, request);
        } else {
            self.handle_query_routed(service, connection, &request)
        };
        vec![match result {
            Ok(value) => json!({"v":protocol::PROTOCOL_VERSION,"msg_id":msg,
                "type":"result","result":value}),
            Err("event_gap") => {
                let mut response = error_reply(&msg, "event_gap");
                response["oldest"] = json!({"boot_id":service.boot_id(),"seq":service.owner().event_log().oldest_cursor().to_string()});
                response["latest"] = json!({"boot_id":service.boot_id(),"seq":service.owner().event_log().latest_cursor().to_string()});
                response
            }
            Err(code) => error_reply(&msg, code),
        }]
    }

    fn handle_query_routed(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        request: &WireRequest,
    ) -> Result<Value, &'static str> {
        let args = &request.args;
        let scope = self
            .clients
            .get(&connection)
            .ok_or("hello_required")?
            .clone();
        if let Some(result) = self.delivery_query(service, connection, &request.op, args) {
            return result;
        }
        if let Some(result) = self.recording_query(service, connection, &request.op, args) {
            return result;
        }
        match request.op.as_str() {
            "reference" => references::query(service, args),
            "controller" => controllers::query_controller(service, args),
            "output" => controllers::query_output(service, args),
            "describe" => discovery::describe(service, args),
            "component" => discovery::component(service, args),
            "discover" => self.begin_projection(
                service,
                connection,
                "discovery",
                discovery::records(service)?,
            ),
            "latest" => crate::measurements::latest(service, args),
            "measurements_current" => self.begin_projection(
                service,
                connection,
                "measurements",
                crate::measurements::current_records(service)?,
            ),
            "measurement_window" => crate::measurements::window(service, args),
            "resource" => {
                configuration_api::resource_json(service, common::id_field(args, "resource")?)
            }
            "configuration_status" => Ok(configuration_api::status_json(service)),
            "configuration_properties" => self.begin_projection(
                service,
                connection,
                "configuration",
                configuration_api::property_records(service)?,
            ),
            "operation_status" => {
                let request_id = parse_request_id(args.get("request_id").ok_or("invalid_args")?)?;
                if request_id.scope != scope {
                    return Err("scope_unknown");
                }
                Ok(operation_status(
                    self.sessions.status(&scope, request_id.seq),
                ))
            }
            _ => Err("unsupported_operation"),
        }
    }

    fn handle_mutation(
        &mut self,
        service: &mut ServiceHost,
        connection: u64,
        request: WireRequest,
    ) -> Vec<Value> {
        let msg = request.msg_id;
        let Some(scope) = self.clients.get(&connection).cloned() else {
            return vec![error_reply(&msg, "hello_required")];
        };
        let Some(rid) = request.request_id else {
            return vec![error_reply(&msg, "invalid_args")];
        };
        if rid.scope != scope {
            return vec![error_reply(&msg, "scope_unknown")];
        }
        let payload = match operations::decode(&request.op, &request.args) {
            Ok(payload) => payload,
            Err(code) => return vec![error_reply(&msg, code)],
        };
        if service.is_stopping() && self.sessions.next_seq(&scope).ok() == Some(rid.seq) {
            return vec![error_reply(
                &msg,
                if matches!(&payload, Mutation::Shutdown) {
                    "shutdown_in_progress"
                } else {
                    "shutdown_before_execution"
                },
            )];
        }
        let now = service.clock().now();
        match self.sessions.admit(&scope, rid.seq, payload.clone(), now) {
            Admission::Known(state) => return vec![operation_reply(&msg, &rid, state)],
            Admission::Conflict => return vec![error_reply(&msg, "request_id_conflict")],
            Admission::Unknown => return vec![error_reply(&msg, "outcome_unknown")],
            Admission::Gap => return vec![error_reply(&msg, "sequence_gap")],
            Admission::Busy => return vec![error_reply(&msg, "busy")],
            Admission::ScopeUnknown => return vec![error_reply(&msg, "scope_unknown")],
            Admission::Accepted => {}
        }
        let accepted = operation_reply(&msg, &rid, OperationState::Accepted);
        if let Some(replies) = self.handle_recorder_mutation(
            service,
            AdmittedRecorderRequest {
                connection,
                payload: &payload,
                scope: &scope,
                request_id: &rid,
                message: &msg,
                now,
                accepted: &accepted,
            },
        ) {
            return replies;
        }
        if let Some((command, data)) = recorded_intent(&payload) {
            service.owner_mut().record_operation(OperationRecord {
                scope: scope.clone(),
                request_seq: rid.seq,
                command,
                phase: "accepted",
                data,
                outcome_basis: "application_admission",
                at: now,
            });
        }
        if matches!(&payload, Mutation::Shutdown) {
            match service.request_shutdown() {
                Ok(()) => {
                    self.pending_shutdown = Some(PendingShutdown {
                        connection,
                        scope,
                        seq: rid.seq,
                        msg,
                    });
                    return vec![accepted];
                }
                Err(error) => {
                    let failed = OperationState::Failed(domain_code(error).into());
                    self.sessions
                        .complete(&scope, rid.seq, failed.clone(), service.clock().now())
                        .expect("admitted shutdown record");
                    return vec![accepted, operation_reply(&msg, &rid, failed)];
                }
            }
        }
        let recorded_command = recorded_intent(&payload).map(|(command, _)| command);
        let reconnect_resource = match &payload {
            Mutation::ReconnectResource { resource, .. } => Some(*resource),
            _ => None,
        };
        let reconnect_operation = reconnect_resource.is_some();
        let configuration_operation = matches!(
            &payload,
            Mutation::ReloadConfiguration
                | Mutation::StageConfiguration
                | Mutation::ApplyConfiguration { .. }
                | Mutation::ConfigureProperty { .. }
        );
        let outcome = dispatch(service, payload, &rid).map_or_else(
            |error| {
                let code = domain_code(error);
                if reconnect_operation && let Some(diagnostic) = service.reconnect_diagnostic() {
                    OperationState::FailedWithResult {
                        code: code.into(),
                        detail: diagnostic.to_json().to_string(),
                    }
                } else {
                    OperationState::Failed(code.into())
                }
            },
            |value| OperationState::Completed(value.to_string()),
        );
        // The terminal record is committed even if the connection closes before delivery.
        self.sessions
            .complete(&scope, rid.seq, outcome.clone(), service.clock().now())
            .expect("bounded typed terminal record for admitted scope");
        if let Some(command) = recorded_command {
            let (phase, data) = recorded_terminal(&outcome);
            let terminal_at = service.clock().now();
            service.owner_mut().record_operation(OperationRecord {
                scope: scope.clone(),
                request_seq: rid.seq,
                command,
                phase,
                data,
                outcome_basis: "domain_result",
                at: terminal_at,
            });
        }
        let lifecycle_at = service.clock().now();
        if let Some(resource) = reconnect_resource
            && let Ok(state) = configuration_api::resource_json(service, resource)
        {
            let _ =
                service
                    .owner_mut()
                    .event_log_mut()
                    .resource_state(lifecycle_at, resource, state);
        }
        if configuration_operation && matches!(outcome, OperationState::Completed(_)) {
            let state = configuration_api::status_json(service);
            let _ = service
                .owner_mut()
                .event_log_mut()
                .configuration_state(lifecycle_at, state);
        }
        let event_state = operation_state(outcome.clone());
        let published_at = service.clock().now();
        if service
            .owner_mut()
            .event_log_mut()
            .operation_terminal(published_at, &scope, rid.seq, event_state)
            .is_err()
        {
            let _ = service.request_shutdown();
        }
        vec![accepted, operation_reply(&msg, &rid, outcome)]
    }
}
