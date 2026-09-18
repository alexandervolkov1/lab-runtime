//! The serialized service owner maps fixed wire operations to pure Core queries
//! or bounded mutations. Accepted records precede dispatch; terminal records
//! precede any attempt to send a reply.

use crate::recorder::{
    AnnotationRecord, HistoryCursor, HistoryFilter, HistoryPage, OperationRecord, RecordingState,
    RunsCursor, RunsPage, bounded_annotation_data,
};
use crate::{
    configuration::PropertyValue,
    configuration_api,
    host::Clock,
    measurements::{current_json, sample_json, signal_id_json},
    protocol::{self, PublicError},
    recorder_api,
    service::{LifecycleOperationError, ServiceHost},
    sessions::{
        Admission, EmulatorPublication, Mutation, OperationState, PropertyMutationValue,
        SessionError, SessionStore,
    },
    wire::{WireRequest, WireRequestId, decimal_u64},
};
use lab_core::control::{
    ControllerError, ControllerId, ControllerSnapshot, ControllerState, PidConfig,
};
use lab_core::managed::{ComponentId, ComponentState};
use lab_core::output::{
    ActuatorId, DispatchOutcome, OutputError, OutputOwner, OutputSnapshot, OutputState,
};
use lab_core::processing::EmaConfig;
use lab_core::reference::{ReferenceConfig, ReferenceId, ReferenceSnapshot};
use lab_core::transport::TransportError;
use lab_core::{
    AccessMode, Command, CommandResult, Error, InstrumentId, ParameterDescriptor, ParameterId,
    ParameterRole, Query, QueryResult, SignalId, ValueSpec, WriteEffect,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

struct FrozenSnapshot {
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
    snapshots: BTreeMap<u64, FrozenSnapshot>,
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
            snapshots: BTreeMap::new(),
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
        self.snapshots.remove(&connection);
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
    /// Expire frozen connection snapshots at a trusted monotonic owner instant.
    pub fn expire_snapshots_at(&mut self, now: Duration) {
        self.snapshots.retain(|_, s| now < s.expires);
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

    /// Complete one accepted recording lifecycle operation only after its
    /// storage barrier has committed or the Recorder has failed.
    pub fn poll_recording(&mut self, service: &mut ServiceHost) -> Vec<(u64, Value)> {
        let Some(pending) = self.pending_recording.as_ref() else {
            return Vec::new();
        };
        let Some(status) = service.owner().recording_status() else {
            return Vec::new();
        };
        let done = if pending.start {
            matches!(
                status.state,
                RecordingState::Recording | RecordingState::Failed
            )
        } else {
            matches!(status.state, RecordingState::Idle | RecordingState::Failed)
        };
        if !done {
            return Vec::new();
        }
        let pending = self.pending_recording.take().expect("checked above");
        let run_no = if pending.start {
            status.run_no
        } else {
            pending.run_no
        };
        let interval_no = if pending.start {
            status.interval_no
        } else {
            pending.interval_no
        };
        let result = if status.state == RecordingState::Failed {
            OperationState::Failed("recording_failed".into())
        } else {
            let completion = if pending.start {
                json!({"start_committed":true,"fact_admission_open":true,
                    "provenance_committed":true})
            } else {
                json!({"accepted_facts_drained":true,"interval_sealed":true,
                    "run_sealed":true,"transaction_committed":true,
                    "writer_closed":false,"archive_boot_sealed":false})
            };
            OperationState::Completed(
                json!({
                    "database_id":service.owner().recording_database_id(),
                    "run_id":{"boot_id":service.boot_id(),
                        "run_no":run_no.map(|number| number.to_string())},
                    "interval_id":{"boot_id":service.boot_id(),
                        "interval_no":interval_no.map(|number| number.to_string())},
                    "completion":completion
                })
                .to_string(),
            )
        };
        self.sessions
            .complete(
                &pending.scope,
                pending.seq,
                result.clone(),
                service.clock().now(),
            )
            .expect("retained recording lifecycle operation");
        let (phase, data) = recorded_terminal(&result);
        let terminal_at = service.clock().now();
        service.owner_mut().record_operation(OperationRecord {
            scope: pending.scope.clone(),
            request_seq: pending.seq,
            command: if pending.start {
                "recording_start"
            } else {
                "recording_stop"
            },
            phase,
            data,
            outcome_basis: "domain_result",
            at: terminal_at,
        });
        let published = service.clock().now();
        let _ = service.owner_mut().event_log_mut().operation_terminal(
            published,
            &pending.scope,
            pending.seq,
            operation_state(result.clone()),
        );
        if self.clients.get(&pending.connection) != Some(&pending.scope) {
            return Vec::new();
        }
        let rid = WireRequestId {
            scope: pending.scope,
            seq: pending.seq,
        };
        vec![(
            pending.connection,
            operation_reply(&pending.msg, &rid, result),
        )]
    }

    /// Reconcile completed bounded history jobs and retain immutable pages.
    /// This only takes worker results; SQL ran on the Recorder thread.
    pub fn poll_history(&mut self, service: &mut ServiceHost) -> Vec<(u64, Value)> {
        for job in self.orphan_history.drain(..) {
            service.owner_mut().cancel_history(job);
        }
        let now = service.clock().now();
        self.expire_snapshots_at(now);
        let ids: Vec<u64> = self.pending_history.keys().copied().collect();
        let mut replies = Vec::new();
        for connection in ids {
            let Some(pending) = self.pending_history.get(&connection) else {
                continue;
            };
            let expired = pending
                .submitted
                .checked_add(Duration::from_secs(2))
                .is_none_or(|deadline| now >= deadline);
            let outcome = if expired {
                service.owner_mut().cancel_history(pending.job);
                Some(Err("history_timeout"))
            } else if pending.runs {
                service
                    .owner_mut()
                    .try_take_runs(pending.job)
                    .map(|result| {
                        result
                            .map(CompletedHistory::Runs)
                            .map_err(|_| "history_failed")
                    })
            } else {
                service
                    .owner_mut()
                    .try_take_history(pending.job)
                    .map(|result| {
                        result
                            .map(|page| CompletedHistory::Measurements(Box::new(page)))
                            .map_err(|_| "history_failed")
                    })
            };
            let Some(outcome) = outcome else { continue };
            let pending = self
                .pending_history
                .remove(&connection)
                .expect("listed above");
            let terminal = match outcome {
                Err(code) => OperationState::Failed(code.into()),
                Ok(page) => {
                    let has_cursor = match &page {
                        CompletedHistory::Measurements(page) => page.next_cursor.is_some(),
                        CompletedHistory::Runs(page) => page.next_cursor.is_some(),
                    };
                    let Some(following) =
                        self.next_token.checked_add(if has_cursor { 2 } else { 1 })
                    else {
                        let failed = OperationState::Failed("history_token_exhausted".into());
                        self.sessions
                            .complete(&pending.scope, pending.seq, failed.clone(), now)
                            .expect("retained history read operation");
                        let _ = service.owner_mut().event_log_mut().operation_terminal(
                            now,
                            &pending.scope,
                            pending.seq,
                            operation_state(failed.clone()),
                        );
                        if self.clients.get(&connection) == Some(&pending.scope) {
                            let rid = WireRequestId {
                                scope: pending.scope,
                                seq: pending.seq,
                            };
                            replies.push((connection, operation_reply(&pending.msg, &rid, failed)));
                        }
                        continue;
                    };
                    let token = format!("history-page-{}", self.next_token);
                    let cursor_token =
                        has_cursor.then(|| format!("history-cursor-{}", self.next_token + 1));
                    self.next_token = following;
                    let encoded = match &page {
                        CompletedHistory::Measurements(page) => {
                            history_page_json(page, cursor_token.as_deref())
                        }
                        CompletedHistory::Runs(page) => {
                            runs_page_json(page, cursor_token.as_deref())
                        }
                    };
                    if serde_json::to_vec(&encoded).is_ok_and(|bytes| bytes.len() <= 8 * 1024) {
                        let next_cursor = match page {
                            CompletedHistory::Measurements(page) => {
                                page.next_cursor.map(RetainedHistoryCursor::Measurements)
                            }
                            CompletedHistory::Runs(page) => {
                                page.next_cursor.map(RetainedHistoryCursor::Runs)
                            }
                        };
                        if let Some(cursor) = next_cursor {
                            let token = cursor_token.expect("cursor was present");
                            self.history_cursors
                                .retain(|_, retained| retained.connection != connection);
                            self.history_cursors.insert(
                                token.clone(),
                                HistoryContinuation {
                                    token,
                                    connection,
                                    scope: pending.scope.clone(),
                                    cursor,
                                    expires: now + Duration::from_secs(30),
                                },
                            );
                        }
                        self.history_pages.insert(
                            connection,
                            HistoryCache {
                                token: token.clone(),
                                page: encoded,
                                expires: now + Duration::from_secs(5),
                            },
                        );
                        OperationState::Completed(
                            json!({"page_token":token,
                            "durability":"checkpoint_frozen"})
                            .to_string(),
                        )
                    } else {
                        OperationState::Failed("history_page_oversize".into())
                    }
                }
            };
            self.sessions
                .complete(&pending.scope, pending.seq, terminal.clone(), now)
                .expect("retained history read operation");
            let _ = service.owner_mut().event_log_mut().operation_terminal(
                now,
                &pending.scope,
                pending.seq,
                operation_state(terminal.clone()),
            );
            if self.clients.get(&connection) == Some(&pending.scope) {
                let rid = WireRequestId {
                    scope: pending.scope,
                    seq: pending.seq,
                };
                replies.push((connection, operation_reply(&pending.msg, &rid, terminal)));
            }
        }
        replies
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
            self.handle_query(service, connection, &request)
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

    fn handle_query(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        request: &WireRequest,
    ) -> Result<Value, &'static str> {
        let args = &request.args;
        let scope = self.clients.get(&connection).ok_or("hello_required")?;
        let owner = service.owner();
        let result = match request.op.as_str() {
            "history_page" => {
                let token = args
                    .get("page_token")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let page = self
                    .history_pages
                    .get(&connection)
                    .ok_or("history_page_expired")?;
                if page.token != token || service.clock().now() >= page.expires {
                    return Err("history_page_expired");
                }
                page.page.clone()
            }
            "history_release" => {
                let token = args
                    .get("page_token")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if self
                    .history_pages
                    .get(&connection)
                    .is_some_and(|page| page.token == token)
                {
                    self.history_pages.remove(&connection);
                    json!({"released":true})
                } else {
                    return Err("history_page_expired");
                }
            }
            "recording_status" => recorder_api::status_json(
                owner.recording_status(),
                owner.recording_database_id(),
                owner.recording_policy(),
                service.boot_id(),
            ),
            "discovery_page" | "measurements_page" | "configuration_page" => {
                let token = args
                    .get("projection")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let index = id_field(args, "index")? as usize;
                let expected = match request.op.as_str() {
                    "discovery_page" => "discovery",
                    "measurements_page" => "measurements",
                    _ => "configuration",
                };
                self.projection_page(service, connection, token, index, expected)?
            }
            "subscribe" => {
                if self.subscriptions.contains_key(&connection) {
                    return Err("subscription_busy");
                }
                let after = args.get("after").ok_or("invalid_args")?;
                let boot = after
                    .get("boot_id")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if boot != service.boot_id() {
                    return Err("instance_changed");
                }
                let seq = id_field(after, "seq")?;
                match owner.event_log().scan_after(seq, 1) {
                    Ok(_) => {}
                    Err(crate::events::EventError::Gap) => return Err("event_gap"),
                    Err(crate::events::EventError::Future) => return Err("invalid_cursor"),
                    Err(_) => return Err("event_error"),
                }
                let filter = args.get("filter").ok_or("invalid_args")?;
                let kinds = filter
                    .get("kinds")
                    .and_then(Value::as_array)
                    .ok_or("invalid_args")?;
                let targets = filter
                    .get("targets")
                    .and_then(Value::as_array)
                    .ok_or("invalid_args")?;
                if kinds.len() > 8 || targets.len() > 16 {
                    return Err("invalid_args");
                }
                let mut selected = Vec::new();
                for kind in kinds {
                    let name = kind.as_str().ok_or("invalid_args")?;
                    if ![
                        "signal",
                        "controller",
                        "reference",
                        "component",
                        "output",
                        "operation",
                        "host",
                        "recorder",
                        "resource",
                        "configuration",
                    ]
                    .contains(&name)
                    {
                        return Err("invalid_args");
                    }
                    selected.push(name.to_string());
                }
                let selected_targets = targets
                    .iter()
                    .map(parse_filter_target)
                    .collect::<Result<Vec<_>, _>>()?;
                let token = self.issue_token(service.boot_id())?;
                self.subscriptions.insert(
                    connection,
                    Subscription {
                        token: token.clone(),
                        scan: seq,
                        kinds: selected,
                        targets: selected_targets,
                    },
                );
                json!({"subscription":token,"accepted_cursor":{"boot_id":service.boot_id(),"seq":seq.to_string()}})
            }
            "unsubscribe" => {
                let token = args
                    .get("subscription")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let removed = self
                    .subscriptions
                    .get(&connection)
                    .is_some_and(|s| s.token == token);
                if removed {
                    self.subscriptions.remove(&connection);
                }
                json!({"removed":removed})
            }
            "reference" => {
                let id = id_field(args, "reference")?;
                match owner
                    .query(Query::Reference(ReferenceId::new(id)))
                    .map_err(domain_code)?
                {
                    QueryResult::Reference(snap) => reference_json(id, snap),
                    _ => return Err("internal_error"),
                }
            }
            "resource" => configuration_api::resource_json(service, id_field(args, "resource")?)?,
            "configuration_status" => configuration_api::status_json(service),
            "configuration_properties" => self.begin_projection(
                service,
                connection,
                "configuration",
                configuration_api::property_records(service)?,
            )?,
            "component" => {
                let id = id_field(args, "component")?;
                let QueryResult::Component(snapshot) = owner
                    .query(Query::Component(ComponentId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let kind = owner
                    .component_catalog()
                    .iter()
                    .find(|(candidate, _)| candidate.get() == id)
                    .map(|(_, kind)| *kind)
                    .ok_or("unknown_component")?;
                json!({"component":id.to_string(),"instrument":snapshot.instrument.get().to_string(),"kind":kind,
                    "implementation":snapshot.implementation.as_str(),
                    "generation":snapshot.generation.to_string(),"revision":snapshot.revision.to_string(),
                    "state":match snapshot.state{ComponentState::Warming=>"warming",ComponentState::Ready=>"ready",ComponentState::Failed=>"failed"},
                    "good_steps":snapshot.good_steps,"pending":snapshot.pending.is_some(),"diagnostics":snapshot.diagnostics})
            }
            "controller" => {
                let id = id_field(args, "controller")?;
                let QueryResult::Controller(snap) = owner
                    .query(Query::Controller(ControllerId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let QueryResult::ControllerConfig(config) = owner
                    .query(Query::ControllerConfig(ControllerId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                controller_projection_json(snap, config)
            }
            "describe" => {
                let id = id_field(args, "instrument")?;
                let QueryResult::Descriptor(descriptor) = owner
                    .query(Query::DescribeInstrument(InstrumentId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let parameters: Vec<_> = descriptor.parameters.iter().map(parameter_json).collect();
                json!({"id":descriptor.id.get().to_string(),"name":descriptor.name,"parameters":parameters})
            }
            "output" => {
                let actuator = args.get("actuator").ok_or("invalid_args")?;
                let instrument = id_field(actuator, "instrument")?;
                let parameter = id_field(actuator, "parameter")?;
                let QueryResult::Output(snapshot) = owner
                    .query(Query::Output(ActuatorId::new(
                        InstrumentId::new(instrument),
                        ParameterId::new(parameter),
                    )))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                output_json(snapshot)
            }
            "latest" => {
                let signal = args.get("signal").ok_or("invalid_args")?;
                let instrument = id_field(signal, "instrument")?;
                let parameter = id_field(signal, "parameter")?;
                match owner
                    .query(Query::GetLatestSignal(SignalId::new(
                        InstrumentId::new(instrument),
                        ParameterId::new(parameter),
                    )))
                    .map_err(domain_code)?
                {
                    QueryResult::Latest(sample) => {
                        let QueryResult::Descriptor(descriptor) = owner
                            .query(Query::DescribeInstrument(InstrumentId::new(instrument)))
                            .map_err(domain_code)?
                        else {
                            return Err("internal_error");
                        };
                        let descriptor = descriptor
                            .parameter(ParameterId::new(parameter))
                            .ok_or("unknown_parameter")?;
                        current_json(
                            descriptor.signal.ok_or("unknown_signal")?,
                            descriptor.unit,
                            sample.as_ref(),
                            owner.signal_generation(SignalId::new(
                                InstrumentId::new(instrument),
                                ParameterId::new(parameter),
                            )),
                        )
                    }
                    _ => return Err("internal_error"),
                }
            }
            "operation_status" => {
                let rid = parse_rid(args.get("request_id").ok_or("invalid_args")?)?;
                if rid.scope != *scope {
                    return Err("scope_unknown");
                }
                operation_status(self.sessions.status(scope, rid.seq))
            }
            "discover" => {
                let QueryResult::Instruments(instruments) =
                    owner.query(Query::Discover).map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let mut records = Vec::new();
                for descriptor in instruments {
                    let kind = owner.instrument_kind(descriptor.id);
                    records.push(json!({"kind":"instrument","id":descriptor.id.get().to_string(),
                        "name":descriptor.name,"implementation_kind":kind,
                        "identity_class":if kind=="physical"{"physical"}else{"virtual"},
                        "capabilities":{"readable":true,"current":descriptor.parameters.iter().any(|p|p.signal.is_some()),
                            "recent_history":descriptor.parameters.iter().any(|p|p.signal.is_some()),
                            "live_subscription":descriptor.parameters.iter().any(|p|p.signal.is_some()),
                            "emulator_publication":descriptor.parameters.iter().any(|p|p.signal.is_some_and(|s|owner.emulator_writable(s))),
                            "properties":descriptor.parameters.iter().any(|p|p.role==ParameterRole::Configuration)}}));
                    for parameter in descriptor.parameters {
                        if let Some(signal) = parameter.signal {
                            let QueryResult::Latest(latest) = owner
                                .query(Query::GetLatestSignal(signal))
                                .map_err(domain_code)?
                            else {
                                return Err("internal_error");
                            };
                            records.push(json!({"kind":"signal","id":signal_id_json(signal),
                                "instrument":descriptor.id.get().to_string(),"name":parameter.name,
                                "signal_kind":match parameter.role {ParameterRole::Measurement=>"measurement",_=>"diagnostic"},
                                "value_type":value_type_name(&parameter.value_spec),
                                "unit":{"id":parameter.unit.id(),"symbol":parameter.unit.symbol()},
                                "quality":latest.as_ref().map_or("unavailable",|s|quality_name(s.quality())),
                                "generation":owner.signal_generation(signal).to_string(),
                                "capabilities":{"readable":true,"current":true,"recent_history":true,
                                    "durable_history":owner.recording_database_id().is_some(),"live_subscription":true,
                                    "emulator_publication":owner.emulator_writable(signal)},
                                "emulator":if owner.emulator_writable(signal) {json!({"writable":true,
                                    "states":["good","unavailable"],"timing":"runtime_receipt",
                                    "expected_generation":owner.signal_generation(signal).to_string(),
                                    "value_constraints":value_constraints(&parameter.value_spec)})} else {Value::Null}}));
                        }
                    }
                }
                for (id, component_kind) in owner.component_catalog() {
                    let QueryResult::Component(snapshot) =
                        owner.query(Query::Component(*id)).map_err(domain_code)?
                    else {
                        return Err("internal_error");
                    };
                    records.push(json!({"kind":"component","id":id.get().to_string(),
                        "component_kind":component_kind,"instrument":snapshot.instrument.get().to_string(),
                        "implementation":snapshot.implementation.as_str(),
                        "generation":snapshot.generation.to_string(),"revision":snapshot.revision.to_string()}));
                }
                records.extend(
                    owner
                        .event_log()
                        .snapshot_records()
                        .into_iter()
                        .filter(|record| {
                            matches!(
                                record["kind"].as_str(),
                                Some("controller" | "reference" | "output")
                            )
                        })
                        .map(|record| {
                            json!({"kind":record["kind"],"id":record["target"],
                                "state":record["data"]})
                        }),
                );
                records.extend(owner.resource_records().into_iter().filter_map(|record| {
                    let id = record["target"]["id"].as_str()?.parse().ok()?;
                    let state =
                        configuration_api::resource_json(service, id).unwrap_or_else(|_| {
                            json!({"resource":id.to_string(),"state":record["data"]["state"],
                            "binding_generation":Value::Null,
                            "transport_generation":record["data"]["generation"]})
                        });
                    Some(json!({"kind":"resource","id":id.to_string(),"state":state}))
                }));
                if let Ok(properties) = configuration_api::property_records(service) {
                    records.extend(properties);
                }
                records.sort_by_key(|record| record.to_string());
                self.begin_projection(service, connection, "discovery", records)?
            }
            "measurements_current" => {
                let QueryResult::Instruments(instruments) =
                    owner.query(Query::Discover).map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let mut records = Vec::new();
                for descriptor in instruments {
                    for parameter in descriptor.parameters.iter().filter(|p| p.signal.is_some()) {
                        let signal = parameter.signal.expect("filtered above");
                        let QueryResult::Latest(latest) = owner
                            .query(Query::GetLatestSignal(signal))
                            .map_err(domain_code)?
                        else {
                            return Err("internal_error");
                        };
                        records.push(current_json(
                            signal,
                            parameter.unit,
                            latest.as_ref(),
                            owner.signal_generation(signal),
                        ));
                    }
                }
                self.begin_projection(service, connection, "measurements", records)?
            }
            "measurement_window" => {
                let signal = args.get("signal").ok_or("invalid_args")?;
                let signal = SignalId::new(
                    InstrumentId::new(id_field(signal, "instrument")?),
                    ParameterId::new(id_field(signal, "parameter")?),
                );
                let limit = args
                    .get("max_records")
                    .and_then(Value::as_u64)
                    .ok_or("invalid_args")? as usize;
                if !(1..=128).contains(&limit) {
                    return Err("invalid_args");
                }
                let QueryResult::Window(window) = owner
                    .query(Query::GetSignalWindow(signal))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let total = window.len();
                let start = total.saturating_sub(limit);
                let rows = window[start..]
                    .iter()
                    .map(|sample| sample_json(sample, owner.signal_generation(signal)))
                    .collect::<Vec<_>>();
                let result = json!({"signal":signal_id_json(signal),"ordering":"oldest_first",
                    "source":"runtime_recent","capacity":total.to_string(),"truncated":start>0,"rows":rows});
                if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > 8 * 1024) {
                    return Err("response_too_large");
                }
                result
            }
            _ => return Err("unsupported_operation"),
        };
        Ok(result)
    }

    fn issue_token(&mut self, boot: &str) -> Result<String, &'static str> {
        let counter = self.next_token;
        self.next_token = self.next_token.checked_add(1).ok_or("counter_exhausted")?;
        Ok(format!("{boot}:{counter}"))
    }
    fn begin_projection(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        kind: &'static str,
        records: Vec<Value>,
    ) -> Result<Value, &'static str> {
        let token = self.issue_token(service.boot_id())?;
        self.snapshots.insert(
            connection,
            FrozenSnapshot {
                token: token.clone(),
                kind,
                cursor: service.owner().event_log().latest_cursor(),
                records,
                expires: service.clock().now() + Duration::from_secs(5),
            },
        );
        self.projection_page(service, connection, &token, 0, kind)
    }
    fn projection_page(
        &self,
        service: &ServiceHost,
        connection: u64,
        token: &str,
        index: usize,
        kind: &str,
    ) -> Result<Value, &'static str> {
        let snapshot = self.snapshots.get(&connection).ok_or("snapshot_expired")?;
        if snapshot.token != token
            || snapshot.kind != kind
            || service.clock().now() >= snapshot.expires
            || index > snapshot.records.len()
        {
            return Err("snapshot_expired");
        }
        let mut records = Vec::new();
        let mut next = index;
        while next < snapshot.records.len() && records.len() < 64 {
            let candidate = &snapshot.records[next];
            let mut trial = records.clone();
            trial.push(candidate.clone());
            if serde_json::to_vec(&trial).map_or(true, |bytes| bytes.len() > 8 * 1024) {
                break;
            }
            records.push(candidate.clone());
            next += 1;
        }
        if next == index && next < snapshot.records.len() {
            return Err("snapshot_capacity");
        }
        Ok(
            json!({"projection":token,"revision":{"boot_id":service.boot_id(),
            "event_seq":snapshot.cursor.to_string()},"records":records,
            "next_index":(next<snapshot.records.len()).then(||next.to_string()),"complete":next==snapshot.records.len()}),
        )
    }

    /// Offer at most four matching retained semantic events after scanning 32.
    /// A filtered scan emits progress so a client can advance its applied cursor.
    pub fn pump_events(&mut self, service: &ServiceHost, connection: u64) -> Vec<Value> {
        let Some(sub) = self.subscriptions.get_mut(&connection) else {
            return Vec::new();
        };
        let initial_scan = sub.scan;
        let events = match service.owner().event_log().scan_after(sub.scan, 32) {
            Ok(events) => events,
            Err(_) => {
                self.subscriptions.remove(&connection);
                let mut response = json!({"v":protocol::PROTOCOL_VERSION,
                    "type":"error","accepted":false});
                PublicError::from_code("event_gap").apply_to(&mut response);
                return vec![response];
            }
        };
        let mut offered = Vec::new();
        let mut scanned = sub.scan;
        for event in events {
            let seq = event["seq"]
                .as_str()
                .and_then(decimal_u64)
                .unwrap_or(scanned);
            scanned = seq;
            if (sub.kinds.is_empty()
                || event["kind"]
                    .as_str()
                    .is_some_and(|k| sub.kinds.iter().any(|s| s == k)))
                && (sub.targets.is_empty()
                    || sub
                        .targets
                        .iter()
                        .any(|t| event["kind"] == t.kind && event["target"] == t.target))
            {
                offered.push(event);
                if offered.len() >= 4 {
                    break;
                }
            }
        }
        sub.scan = scanned;
        if offered.is_empty() && scanned > initial_scan {
            offered.push(
                json!({"v":protocol::PROTOCOL_VERSION,"type":"subscription_progress","subscription":sub.token,
            "boot_id":service.boot_id(),"seq":scanned.to_string()}),
            );
        }
        offered
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
        let payload = match typed_mutation(&request.op, &request.args) {
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
        if let Mutation::ExperimentAnnotate { name, data_json } = &payload {
            let annotation = AnnotationRecord {
                scope: scope.clone(),
                request_seq: rid.seq,
                name: name.clone(),
                data_json: data_json.clone(),
                at: now,
            };
            let outcome = service.owner_mut().annotate(annotation).map_or_else(
                |error| OperationState::Failed(domain_code(error).into()),
                |sequence| {
                    OperationState::Completed(
                        json!({
                            "record_seq":sequence.to_string(),"durability":"pending"
                        })
                        .to_string(),
                    )
                },
            );
            self.sessions
                .complete(&scope, rid.seq, outcome.clone(), service.clock().now())
                .expect("admitted bounded annotation operation");
            return vec![accepted, operation_reply(&msg, &rid, outcome)];
        }
        if let Mutation::HistoryReadRuns {
            database_id,
            max_records,
            cursor,
        } = &payload
        {
            let valid_database =
                service.owner().recording_database_id() == Some(database_id.as_str());
            let busy = self.pending_history.contains_key(&connection)
                || self.history_pages.contains_key(&connection)
                || self.pending_history.len() + self.history_pages.len() >= 8
                || (!self
                    .history_cursors
                    .values()
                    .any(|retained| retained.connection == connection)
                    && self.history_cursors.len() >= MAX_HISTORY_CURSORS);
            let retained_cursor = if let Some(token) = cursor {
                self.history_cursors
                    .get(token)
                    .filter(|retained| {
                        retained.token == *token
                            && retained.scope == scope
                            && service.clock().now() < retained.expires
                    })
                    .and_then(|retained| match &retained.cursor {
                        RetainedHistoryCursor::Runs(cursor) => Some(cursor.clone()),
                        RetainedHistoryCursor::Measurements(_) => None,
                    })
            } else {
                None
            };
            let scheduled = if !valid_database {
                Err("history_database_unknown")
            } else if busy {
                Err("history_busy")
            } else if cursor.is_some() && retained_cursor.is_none() {
                Err("history_cursor_expired")
            } else {
                service
                    .owner_mut()
                    .request_runs(retained_cursor, *max_records as usize)
                    .map_err(|_| "history_busy")
            };
            match scheduled {
                Ok(job) => {
                    self.pending_history.insert(
                        connection,
                        PendingHistory {
                            scope,
                            seq: rid.seq,
                            msg,
                            job,
                            submitted: now,
                            runs: true,
                        },
                    );
                    return vec![accepted];
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(&scope, rid.seq, failed.clone(), now)
                        .expect("admitted run discovery operation");
                    return vec![accepted, operation_reply(&msg, &rid, failed)];
                }
            }
        }
        if let Mutation::HistoryReadMeasurements {
            database_id,
            boot_id,
            run_no,
            instrument,
            parameter,
            from_ns,
            to_ns,
            max_records,
            cursor,
        } = &payload
        {
            let valid_database =
                service.owner().recording_database_id() == Some(database_id.as_str());
            let busy = self.pending_history.contains_key(&connection)
                || self.history_pages.contains_key(&connection)
                || self.pending_history.len() + self.history_pages.len() >= 8
                || (!self
                    .history_cursors
                    .values()
                    .any(|retained| retained.connection == connection)
                    && self.history_cursors.len() >= MAX_HISTORY_CURSORS);
            let retained_cursor = if let Some(token) = cursor {
                self.history_cursors
                    .get(token)
                    .filter(|retained| {
                        retained.token == *token
                            && retained.scope == scope
                            && service.clock().now() < retained.expires
                    })
                    .and_then(|retained| match &retained.cursor {
                        RetainedHistoryCursor::Measurements(cursor) => Some(cursor.clone()),
                        RetainedHistoryCursor::Runs(_) => None,
                    })
            } else {
                None
            };
            let cursor_valid = cursor.is_none() || retained_cursor.is_some();
            let scheduled = if !valid_database {
                Err("history_database_unknown")
            } else if busy {
                Err("history_busy")
            } else if !cursor_valid {
                Err("history_cursor_expired")
            } else {
                let filter = HistoryFilter {
                    boot_id: boot_id.clone(),
                    run_no: *run_no,
                    instrument: InstrumentId::new(*instrument),
                    parameter: ParameterId::new(*parameter),
                    from: Duration::from_nanos(*from_ns),
                    to: Duration::from_nanos(*to_ns),
                };
                if retained_cursor.as_ref().is_some_and(|previous| {
                    previous.filter().boot_id != filter.boot_id
                        || previous.filter().run_no != filter.run_no
                }) {
                    Err("history_archive_mismatch")
                } else if retained_cursor
                    .as_ref()
                    .is_some_and(|previous| previous.filter() != &filter)
                {
                    Err("history_cursor_mismatch")
                } else {
                    service
                        .owner_mut()
                        .request_history(filter, retained_cursor, *max_records as usize)
                        .map_err(|_| "history_busy")
                }
            };
            match scheduled {
                Ok(job) => {
                    self.pending_history.insert(
                        connection,
                        PendingHistory {
                            scope,
                            seq: rid.seq,
                            msg,
                            job,
                            submitted: now,
                            runs: false,
                        },
                    );
                    return vec![accepted];
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(&scope, rid.seq, failed.clone(), now)
                        .expect("admitted history read operation");
                    return vec![accepted, operation_reply(&msg, &rid, failed)];
                }
            }
        }
        if matches!(
            &payload,
            Mutation::RecordingStart { .. } | Mutation::RecordingStop { .. }
        ) {
            let (command, data) = match &payload {
                Mutation::RecordingStart { label } => {
                    ("recording_start", json!({"label":label}).to_string())
                }
                Mutation::RecordingStop { boot_id, run_no } => (
                    "recording_stop",
                    json!({"boot_id":boot_id,"run_no":run_no.to_string()}).to_string(),
                ),
                _ => unreachable!(),
            };
            service.owner_mut().record_operation(OperationRecord {
                scope: scope.clone(),
                request_seq: rid.seq,
                command,
                phase: "accepted",
                data,
                outcome_basis: "application_admission",
                at: now,
            });
            let saved_run = service
                .owner()
                .recording_status()
                .and_then(|status| status.run_no);
            let saved_interval = service
                .owner()
                .recording_status()
                .and_then(|status| status.interval_no);
            let recorder_state = service
                .owner()
                .recording_status()
                .map(|status| status.state);
            let action: Result<(), &'static str> = if self.pending_recording.is_some() {
                Err("busy")
            } else {
                match &payload {
                    Mutation::RecordingStart { .. }
                        if matches!(
                            recorder_state,
                            None | Some(RecordingState::Failed | RecordingState::Closed)
                        ) =>
                    {
                        Err("recording_unavailable")
                    }
                    Mutation::RecordingStart { .. }
                        if recorder_state != Some(RecordingState::Idle) =>
                    {
                        Err("invalid_state")
                    }
                    Mutation::RecordingStart { label } => service
                        .owner_mut()
                        .start_recording(label, now)
                        .map_err(domain_code),
                    Mutation::RecordingStop { boot_id, run_no } => {
                        if matches!(
                            recorder_state,
                            None | Some(RecordingState::Failed | RecordingState::Closed)
                        ) {
                            Err("recording_unavailable")
                        } else if recorder_state != Some(RecordingState::Recording) {
                            Err("invalid_state")
                        } else if boot_id != service.boot_id() || saved_run != Some(*run_no) {
                            Err("revision_conflict")
                        } else {
                            service
                                .owner_mut()
                                .stop_recording_at(now)
                                .map_err(domain_code)
                        }
                    }
                    _ => unreachable!(),
                }
            };
            match action {
                Ok(()) => {
                    self.pending_recording = Some(PendingRecording {
                        connection,
                        scope,
                        seq: rid.seq,
                        msg,
                        start: matches!(payload, Mutation::RecordingStart { .. }),
                        run_no: saved_run,
                        interval_no: saved_interval,
                    });
                    return vec![accepted];
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(&scope, rid.seq, failed.clone(), service.clock().now())
                        .expect("admitted recording operation");
                    let (phase, data) = recorded_terminal(&failed);
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
                    return vec![accepted, operation_reply(&msg, &rid, failed)];
                }
            }
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

// Stable, language-neutral command fields are selected from the admitted typed
// mutation. The storage encoding is not the wire request and cannot supply ACK
// or safe evidence. History selections remain process-local to avoid recursion.
fn recorded_intent(mutation: &Mutation) -> Option<(&'static str, String)> {
    let (command, fields) = match mutation {
        Mutation::ConfigureReferenceFixed {
            reference,
            expected_revision,
            value,
        } => (
            "reference_configure",
            json!({"reference":reference.to_string(),"expected_revision":expected_revision.to_string(),
                "kind":"fixed","value":value}),
        ),
        Mutation::ConfigureReferenceRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => (
            "reference_configure",
            json!({"reference":reference.to_string(),"expected_revision":expected_revision.to_string(),
                "kind":"ramp","target":target,"rate":rate}),
        ),
        Mutation::RetuneRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => (
            "reference_retune",
            json!({"reference":reference.to_string(),
                "expected_revision":expected_revision.to_string(),"target":target,"rate":rate}),
        ),
        Mutation::ConfigurePid {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
        } => (
            "controller_configure",
            json!({"controller":controller.to_string(),
                "expected_revision":expected_revision.to_string(),"kp":kp,"ki":ki,"kd":kd,
                "output_min":output_min,"output_max":output_max}),
        ),
        Mutation::ConfigureController {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
            ema_time_constant_ns,
            ema_warmup_samples,
            max_input_age_ns,
            max_tick_gap_ns,
            lease_lifetime_ns,
            proposal_ttl_ns,
        } => (
            "controller_configure",
            json!({"controller":controller.to_string(),"expected_revision":expected_revision.to_string(),
                "pid":{"kp":kp,"ki":ki,"kd":kd,"output_min":output_min,"output_max":output_max},
                "ema":{"time_constant_ns":ema_time_constant_ns.to_string(),"warmup_samples":ema_warmup_samples.to_string()},
                "max_input_age_ns":max_input_age_ns.to_string(),"max_tick_gap_ns":max_tick_gap_ns.to_string(),
                "lease_lifetime_ns":lease_lifetime_ns.to_string(),"proposal_ttl_ns":proposal_ttl_ns.to_string()}),
        ),
        Mutation::Start { controller } => (
            "controller_start",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::Pause { controller } => (
            "controller_pause",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::Resume { controller } => (
            "controller_resume",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::ResetFailed { controller } => (
            "controller_reset_failed",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::ReloadConfiguration => ("reload_configuration", json!({})),
        Mutation::StageConfiguration => ("stage_configuration", json!({})),
        Mutation::ApplyConfiguration {
            candidate_id,
            expected_revision,
        } => (
            "apply_configuration",
            json!({"candidate_id":candidate_id.to_string(),
                "expected_revision":expected_revision.to_string()}),
        ),
        Mutation::ConfigureProperty {
            target_kind,
            target_id,
            property,
            value,
            expected_revision,
        } => (
            "property_configure",
            json!({"target":{"kind":target_kind,"id":target_id.to_string()},
                "property":property,"value":match value {
                    PropertyMutationValue::Integer(value)=>json!(value),
                    PropertyMutationValue::Text(value)=>json!(value)},
                "expected_revision":expected_revision.to_string()}),
        ),
        Mutation::PublishEmulatorMeasurement {
            instrument,
            parameter,
            expected_generation,
            publication,
        } => (
            "emulator_publish",
            json!({"signal":{"instrument":instrument.to_string(),"parameter":parameter.to_string()},
                "expected_generation":expected_generation.to_string(),
                "state":match publication {EmulatorPublication::Good(_)=>"good",EmulatorPublication::Unavailable=>"unavailable"},
                "value":match publication {EmulatorPublication::Good(value)=>json!(value),EmulatorPublication::Unavailable=>Value::Null}}),
        ),
        Mutation::RestartVirtualModels => ("virtual_models_restart", json!({})),
        Mutation::ReconnectResource {
            resource,
            expected_binding_generation,
        } => (
            "reconnect_resource",
            json!({"resource":resource.to_string(),
                "expected_binding_generation":expected_binding_generation.to_string()}),
        ),
        Mutation::Shutdown => ("shutdown", json!({})),
        Mutation::RecordingStart { .. }
        | Mutation::RecordingStop { .. }
        | Mutation::ExperimentAnnotate { .. }
        | Mutation::HistoryReadMeasurements { .. }
        | Mutation::HistoryReadRuns { .. } => return None,
    };
    Some((command, fields.to_string()))
}

fn recorded_terminal(outcome: &OperationState) -> (&'static str, String) {
    match outcome {
        OperationState::Completed(result) => (
            "completed",
            json!({"result":serde_json::from_str::<Value>(result).unwrap_or(Value::Null)})
                .to_string(),
        ),
        OperationState::Failed(code) => ("failed", json!({"code":code}).to_string()),
        OperationState::FailedWithResult { code, detail } => (
            "failed",
            json!({"code":code,"detail":serde_json::from_str::<Value>(detail)
                .unwrap_or(Value::Null)})
            .to_string(),
        ),
        OperationState::Accepted => unreachable!("terminal fact requires domain outcome"),
    }
}

fn typed_mutation(op: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match op {
        "reference_configure" => {
            let reference = id_field(args, "reference")?;
            let expected_revision = id_field(args, "expected_revision")?;
            match args.get("kind").and_then(Value::as_str) {
                Some("fixed") if args.get("target").is_none() && args.get("rate").is_none() => {
                    Mutation::ConfigureReferenceFixed {
                        reference,
                        expected_revision,
                        value: float_field(args, "value")?,
                    }
                }
                Some("ramp") if args.get("value").is_none() => Mutation::ConfigureReferenceRamp {
                    reference,
                    expected_revision,
                    target: float_field(args, "target")?,
                    rate: float_field(args, "rate")?,
                },
                _ => return Err("invalid_args"),
            }
        }
        "reference_retune" => Mutation::RetuneRamp {
            reference: id_field(args, "reference")?,
            expected_revision: id_field(args, "expected_revision")?,
            target: float_field(args, "target")?,
            rate: float_field(args, "rate")?,
        },
        "controller_configure_pid" => {
            let pid = args.get("pid").ok_or("invalid_args")?;
            Mutation::ConfigurePid {
                controller: id_field(args, "controller")?,
                expected_revision: id_field(args, "expected_revision")?,
                kp: float_field(pid, "kp")?,
                ki: float_field(pid, "ki")?,
                kd: float_field(pid, "kd")?,
                output_min: float_field(pid, "output_min")?,
                output_max: float_field(pid, "output_max")?,
            }
        }
        "controller_configure" => {
            let pid = args.get("pid").ok_or("invalid_args")?;
            let ema = args.get("ema").ok_or("invalid_args")?;
            Mutation::ConfigureController {
                controller: id_field(args, "controller")?,
                expected_revision: id_field(args, "expected_revision")?,
                kp: float_field(pid, "kp")?,
                ki: float_field(pid, "ki")?,
                kd: float_field(pid, "kd")?,
                output_min: float_field(pid, "output_min")?,
                output_max: float_field(pid, "output_max")?,
                ema_time_constant_ns: id_field(ema, "time_constant_ns")?,
                ema_warmup_samples: id_field(ema, "warmup_samples")?,
                max_input_age_ns: id_field(args, "max_input_age_ns")?,
                max_tick_gap_ns: id_field(args, "max_tick_gap_ns")?,
                lease_lifetime_ns: id_field(args, "lease_lifetime_ns")?,
                proposal_ttl_ns: id_field(args, "proposal_ttl_ns")?,
            }
        }
        "controller_start" => Mutation::Start {
            controller: id_field(args, "controller")?,
        },
        "controller_pause" => Mutation::Pause {
            controller: id_field(args, "controller")?,
        },
        "controller_resume" => Mutation::Resume {
            controller: id_field(args, "controller")?,
        },
        "controller_reset_failed" => Mutation::ResetFailed {
            controller: id_field(args, "controller")?,
        },
        "recording_start" => {
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if label.trim().is_empty() || label.len() > 128 {
                return Err("invalid_args");
            }
            Mutation::RecordingStart {
                label: label.to_owned(),
            }
        }
        "recording_stop" => {
            let run = args.get("run_id").ok_or("invalid_args")?;
            let boot_id = run
                .get("boot_id")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if boot_id.len() != 32
                || !boot_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("invalid_args");
            }
            Mutation::RecordingStop {
                boot_id: boot_id.to_owned(),
                run_no: id_field(run, "run_no")?,
            }
        }
        "experiment_annotate" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            let data = args.get("data").ok_or("invalid_args")?;
            if name.trim().is_empty() || name.len() > 64 || !bounded_annotation_data(data) {
                return Err("invalid_args");
            }
            Mutation::ExperimentAnnotate {
                name: name.to_owned(),
                data_json: data.to_string(),
            }
        }
        "history_read" => {
            if args.get("mode").and_then(Value::as_str) == Some("runs") {
                let database_id = args
                    .get("database_id")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if !lower_hex_id(database_id) {
                    return Err("invalid_args");
                }
                let max = args
                    .get("max_records")
                    .and_then(Value::as_u64)
                    .ok_or("invalid_args")?;
                if !(1..=32).contains(&max) {
                    return Err("invalid_args");
                }
                let cursor = match args.get("cursor") {
                    Some(Value::Null) => None,
                    Some(Value::String(token)) if !token.is_empty() && token.len() <= 128 => {
                        Some(token.clone())
                    }
                    _ => return Err("invalid_args"),
                };
                Mutation::HistoryReadRuns {
                    database_id: database_id.to_owned(),
                    max_records: max as u8,
                    cursor,
                }
            } else if args.get("mode").and_then(Value::as_str) == Some("measurements") {
                let database_id = args
                    .get("database_id")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if !lower_hex_id(database_id) {
                    return Err("invalid_args");
                }
                let run = args.get("run_id").ok_or("invalid_args")?;
                let boot_id = args
                    .get("boot_id")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if !lower_hex_id(boot_id) {
                    return Err("invalid_args");
                }
                if run.get("boot_id").and_then(Value::as_str) != Some(boot_id) {
                    return Err("invalid_args");
                }
                let signal = args.get("signal").ok_or("invalid_args")?;
                let from_ns = id_field(args, "from_ns")?;
                let to_ns = id_field(args, "to_ns")?;
                if from_ns >= to_ns {
                    return Err("invalid_args");
                }
                let max = args
                    .get("max_records")
                    .and_then(Value::as_u64)
                    .ok_or("invalid_args")?;
                if !(1..=128).contains(&max) {
                    return Err("invalid_args");
                }
                let cursor = match args.get("cursor") {
                    Some(Value::Null) => None,
                    Some(Value::String(token)) if token.len() <= 128 && !token.is_empty() => {
                        Some(token.clone())
                    }
                    _ => return Err("invalid_args"),
                };
                Mutation::HistoryReadMeasurements {
                    database_id: database_id.to_owned(),
                    boot_id: boot_id.to_owned(),
                    run_no: id_field(run, "run_no")?,
                    instrument: id_field(signal, "instrument")?,
                    parameter: id_field(signal, "parameter")?,
                    from_ns,
                    to_ns,
                    max_records: max as u16,
                    cursor,
                }
            } else {
                return Err("unsupported_history_mode");
            }
        }
        "runtime_shutdown" => Mutation::Shutdown,
        "reload_configuration" => Mutation::ReloadConfiguration,
        "stage_configuration" => Mutation::StageConfiguration,
        "apply_configuration" => Mutation::ApplyConfiguration {
            candidate_id: id_field(args, "candidate_id")?,
            expected_revision: id_field(args, "expected_revision")?,
        },
        "property_configure" => {
            let target = args.get("target").ok_or("invalid_args")?;
            let target_kind = target
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            let property = args
                .get("property")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if target_kind.len() > protocol::SEMANTIC_NAME_LIMIT
                || property.is_empty()
                || property.len() > protocol::SEMANTIC_NAME_LIMIT
            {
                return Err("invalid_args");
            }
            let value = match args.get("value").ok_or("invalid_args")? {
                Value::Number(value) => {
                    PropertyMutationValue::Integer(value.as_i64().ok_or("invalid_args")?)
                }
                Value::String(value) => PropertyMutationValue::Text(value.clone()),
                _ => return Err("invalid_args"),
            };
            Mutation::ConfigureProperty {
                target_kind: target_kind.to_owned(),
                target_id: id_field(target, "id")?,
                property: property.to_owned(),
                value,
                expected_revision: id_field(args, "expected_revision")?,
            }
        }
        "emulator_publish" => {
            let signal = args.get("signal").ok_or("invalid_args")?;
            let publication = match args.get("state").and_then(Value::as_str) {
                Some("good") => EmulatorPublication::Good(float_field(args, "value")?),
                Some("unavailable") if args.get("value").is_none() => {
                    EmulatorPublication::Unavailable
                }
                _ => return Err("invalid_args"),
            };
            Mutation::PublishEmulatorMeasurement {
                instrument: id_field(signal, "instrument")?,
                parameter: id_field(signal, "parameter")?,
                expected_generation: id_field(args, "expected_generation")?,
                publication,
            }
        }
        "virtual_models_restart" => Mutation::RestartVirtualModels,
        "reconnect_resource" => Mutation::ReconnectResource {
            resource: id_field(args, "resource")?,
            expected_binding_generation: id_field(args, "expected_binding_generation")?,
        },
        _ => return Err("unsupported_operation"),
    })
}
fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    rid: &WireRequestId,
) -> Result<Value, Error> {
    let at = service.clock().now();
    let changed_reference = match &mutation {
        Mutation::RetuneRamp { reference, .. }
        | Mutation::ConfigureReferenceFixed { reference, .. }
        | Mutation::ConfigureReferenceRamp { reference, .. } => Some(*reference),
        _ => None,
    };
    let command = match mutation {
        Mutation::ConfigureReferenceFixed {
            reference,
            expected_revision,
            value,
        } => {
            let QueryResult::Reference(snapshot) = service
                .owner()
                .query(Query::Reference(ReferenceId::new(reference)))?
            else {
                return Err(Error::InvalidConfiguration("unexpected Reference query"));
            };
            let unit = match snapshot {
                ReferenceSnapshot::Fixed { unit, .. } => unit,
                ReferenceSnapshot::Ramp { state, .. } => state.unit,
            };
            Command::ReconfigureReference {
                reference: ReferenceId::new(reference),
                config: ReferenceConfig::Fixed {
                    id: ReferenceId::new(reference),
                    value,
                    unit,
                },
                expected_revision,
            }
        }
        Mutation::ConfigureReferenceRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => {
            let QueryResult::Reference(snapshot) = service
                .owner()
                .query(Query::Reference(ReferenceId::new(reference)))?
            else {
                return Err(Error::InvalidConfiguration("unexpected Reference query"));
            };
            let (start, unit) = match snapshot {
                ReferenceSnapshot::Fixed { value, unit, .. } => (value, unit),
                ReferenceSnapshot::Ramp { state, .. } => (state.current, state.unit),
            };
            Command::ReconfigureReference {
                reference: ReferenceId::new(reference),
                config: ReferenceConfig::Ramp {
                    id: ReferenceId::new(reference),
                    start,
                    target,
                    rate,
                    unit,
                    at,
                },
                expected_revision,
            }
        }
        Mutation::RetuneRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => Command::RetuneRampReference {
            reference: ReferenceId::new(reference),
            expected_revision,
            target,
            rate,
            at,
        },
        Mutation::ConfigurePid {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
        } => Command::ConfigureControllerPid {
            controller: ControllerId::new(controller),
            expected_revision,
            pid: PidConfig {
                kp,
                ki,
                kd,
                output_min,
                output_max,
            },
        },
        Mutation::ConfigureController {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
            ema_time_constant_ns,
            ema_warmup_samples,
            max_input_age_ns,
            max_tick_gap_ns,
            lease_lifetime_ns,
            proposal_ttl_ns,
        } => {
            let id = ControllerId::new(controller);
            let QueryResult::ControllerConfig(current) =
                service.owner().query(Query::ControllerConfig(id))?
            else {
                return Err(Error::InvalidConfiguration("unexpected controller query"));
            };
            let warmup_samples = usize::try_from(ema_warmup_samples)
                .map_err(|_| Error::Controller(ControllerError::InvalidConfiguration))?;
            Command::ReconfigureController {
                controller: id,
                config: lab_core::control::NativeControllerConfig {
                    id,
                    input: current.input,
                    output: current.output,
                    reference: current.reference,
                    ema: EmaConfig {
                        time_constant: Duration::from_nanos(ema_time_constant_ns),
                        warmup_samples,
                        unit: current.ema.unit,
                    },
                    pid: PidConfig {
                        kp,
                        ki,
                        kd,
                        output_min,
                        output_max,
                    },
                    max_input_age: Duration::from_nanos(max_input_age_ns),
                    max_tick_gap: Duration::from_nanos(max_tick_gap_ns),
                    lease_lifetime: Duration::from_nanos(lease_lifetime_ns),
                    proposal_ttl: Duration::from_nanos(proposal_ttl_ns),
                },
                expected_revision,
            }
        }
        Mutation::Start { controller } => Command::StartController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Pause { controller } => Command::PauseController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Resume { controller } => Command::ResumeController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::ResetFailed { controller } => Command::ResetFailedController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Shutdown => {
            return Err(Error::InvalidConfiguration(
                "shutdown dispatch is host-only",
            ));
        }
        Mutation::ReloadConfiguration => {
            return service
                .reload_configuration()
                .map(|result| json!({"revision":result.revision.to_string()}))
                .map_err(lifecycle_domain_error);
        }
        Mutation::StageConfiguration => {
            return service
                .stage_configuration()
                .map(|staged| {
                    let effects: Vec<_> = staged
                        .diff()
                        .effects()
                        .iter()
                        .map(|effect| effect.as_str())
                        .collect();
                    json!({"candidate_id":staged.id().to_string(),
                        "base_revision":staged.base_revision().to_string(),
                        "expires_at_ns":staged.expires_at().as_nanos().to_string(),
                        "effects":effects})
                })
                .map_err(lifecycle_domain_error);
        }
        Mutation::ApplyConfiguration {
            candidate_id,
            expected_revision,
        } => {
            return service
                .apply_staged_configuration(candidate_id, expected_revision)
                .map(|result| json!({"revision":result.revision.to_string()}))
                .map_err(lifecycle_domain_error);
        }
        Mutation::ConfigureProperty {
            target_kind,
            target_id,
            property,
            value,
            expected_revision,
        } => {
            let value = match value {
                PropertyMutationValue::Integer(value) => PropertyValue::Integer(value),
                PropertyMutationValue::Text(value) => PropertyValue::Text(value),
            };
            return service
                .configure_property(&target_kind, target_id, &property, value, expected_revision)
                .map(|result| {
                    json!({"target":{"kind":target_kind,"id":target_id.to_string()},
                    "property":property,"revision":result.revision.to_string(),
                    "persisted_to_deployment_source":false})
                })
                .map_err(lifecycle_domain_error);
        }
        Mutation::PublishEmulatorMeasurement {
            instrument,
            parameter,
            expected_generation,
            publication,
        } => {
            let signal = SignalId::new(InstrumentId::new(instrument), ParameterId::new(parameter));
            let value = match publication {
                EmulatorPublication::Good(value) => Some(lab_core::Value::Float(value)),
                EmulatorPublication::Unavailable => None,
            };
            let sample = service.owner_mut().publish_emulator_measurement(
                signal,
                value,
                expected_generation,
                at,
                Some((rid.scope.clone(), rid.seq)),
            )?;
            let generation = service.owner().signal_generation(signal);
            return Ok(json!({"signal":signal_id_json(signal),
                "generation":generation.to_string(),
                "state":if sample.quality()==lab_core::SampleQuality::Good{"good"}else{"unavailable"},
                "committed_at_ns":nanos(sample.at())}));
        }
        Mutation::RestartVirtualModels => {
            return service
                .restart_virtual_models()
                .map(|result| {
                    json!({"models":result.models.to_string(),
                    "generation":result.generation.to_string()})
                })
                .map_err(lifecycle_domain_error);
        }
        Mutation::ReconnectResource {
            resource,
            expected_binding_generation,
        } => {
            return service
                .reconnect_resource(resource, expected_binding_generation)
                .map(|result| {
                    json!({"resource":result.resource_id.to_string(),
                        "binding_generation":result.binding_generation.to_string()})
                })
                .map_err(lifecycle_domain_error);
        }
        Mutation::RecordingStart { .. }
        | Mutation::RecordingStop { .. }
        | Mutation::ExperimentAnnotate { .. }
        | Mutation::HistoryReadMeasurements { .. }
        | Mutation::HistoryReadRuns { .. } => {
            return Err(Error::InvalidConfiguration(
                "recording dispatch is host-only",
            ));
        }
    };
    match service
        .owner_mut()
        .command_with_cause(command, Some((rid.scope.clone(), rid.seq)))?
    {
        CommandResult::ReferenceRetuned(retuned) => Ok(
            json!({"reference":changed_reference.ok_or(Error::InvalidConfiguration("unexpected retune identity"))?.to_string(),"revision":retuned.revision.to_string(),
            "value":retuned.state.current,"target":retuned.state.target,"rate":retuned.state.rate,
            "committed_at":nanos(retuned.state.last_at)}),
        ),
        CommandResult::ReferenceConfigured(snapshot) => Ok(reference_json(
            changed_reference
                .ok_or(Error::InvalidConfiguration("unexpected Reference identity"))?,
            snapshot,
        )),
        CommandResult::ControllerUpdated(snapshot) => Ok(controller_json(snapshot)),
        _ => Err(Error::InvalidConfiguration("unexpected command result")),
    }
}
fn parse_rid(value: &Value) -> Result<WireRequestId, &'static str> {
    let scope = value
        .get("scope")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    let seq = value
        .get("seq")
        .and_then(Value::as_str)
        .and_then(decimal_u64)
        .ok_or("invalid_args")?;
    Ok(WireRequestId {
        scope: scope.into(),
        seq,
    })
}
fn id_field(args: &Value, field: &str) -> Result<u64, &'static str> {
    args.get(field)
        .and_then(Value::as_str)
        .and_then(decimal_u64)
        .ok_or("invalid_args")
}
fn float_field(args: &Value, field: &str) -> Result<f64, &'static str> {
    args.get(field)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or("invalid_args")
}
fn value_constraints(spec: &lab_core::ValueSpec) -> Value {
    match spec {
        lab_core::ValueSpec::Float { min, max } => json!({"minimum":min,"maximum":max}),
        lab_core::ValueSpec::Integer { min, max } => json!({"minimum":min,"maximum":max}),
        _ => Value::Null,
    }
}
fn parse_filter_target(value: &Value) -> Result<FilterTarget, &'static str> {
    let object = value.as_object().ok_or("invalid_args")?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    let target = match kind {
        "instrument" | "controller" | "reference" | "component" | "resource" => {
            if object.len() != 2 || !object.contains_key("id") {
                return Err("invalid_args");
            }
            json!({"id":id_field(value,"id")?.to_string()})
        }
        "configuration" => {
            if object.len() != 2 || value.get("id").and_then(Value::as_str) != Some("runtime") {
                return Err("invalid_args");
            }
            json!({"id":"runtime"})
        }
        "signal" | "output" => {
            if object.len() != 3
                || !object.contains_key("instrument")
                || !object.contains_key("parameter")
            {
                return Err("invalid_args");
            }
            json!({"instrument":id_field(value,"instrument")?.to_string(),"parameter":id_field(value,"parameter")?.to_string()})
        }
        _ => return Err("invalid_args"),
    };
    Ok(FilterTarget {
        kind: kind.into(),
        target,
    })
}
pub(crate) fn controller_json(s: ControllerSnapshot) -> Value {
    json!({"controller":s.id.get().to_string(),"kind":"pid","state":state_name(s.state),
        "status":if s.state==ControllerState::Failed{"failed"}else{"valid"},
        "failure":if s.state==ControllerState::Failed{Some("latched")}else{None},
        "active":matches!(s.state,ControllerState::Warming|ControllerState::Running),
        "paused":s.state==ControllerState::Paused,
        "revision":s.config_revision.to_string(),"last_tick":s.last_tick.map(nanos),
        "latest_output":s.latest_output.map(|v|v.output)})
}
pub(crate) fn controller_projection_json(
    snapshot: ControllerSnapshot,
    config: lab_core::control::NativeControllerConfig,
) -> Value {
    let mut view = controller_json(snapshot);
    view["bindings"] = json!({
        "input":{"instrument":config.input.instrument().get().to_string(),
            "parameter":config.input.parameter().get().to_string()},
        "reference":{"id":config.reference.get().to_string()},
        "output":{"instrument":config.output.instrument().get().to_string(),
            "parameter":config.output.parameter().get().to_string()}
    });
    view["config"] = json!({
        "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
            "output_min":config.pid.output_min,"output_max":config.pid.output_max},
        "max_input_age_ns":nanos(config.max_input_age),"max_tick_gap_ns":nanos(config.max_tick_gap),
        "lease_lifetime_ns":nanos(config.lease_lifetime),"proposal_ttl_ns":nanos(config.proposal_ttl),
        "ema":{"time_constant_ns":nanos(config.ema.time_constant),
            "warmup_samples":config.ema.warmup_samples.to_string(),
            "unit":{"id":config.ema.unit.id(),"symbol":config.ema.unit.symbol()}}
    });
    view
}
fn parameter_json(p: &ParameterDescriptor) -> Value {
    let spec = match &p.value_spec {
        ValueSpec::Float { min, max } => json!({"type":"float","min":min,"max":max}),
        ValueSpec::Integer { min, max } => {
            json!({"type":"integer","min":min.to_string(),"max":max.to_string()})
        }
        ValueSpec::Boolean => json!({"type":"boolean"}),
        ValueSpec::Text { max_bytes } => json!({"type":"text","max_bytes":max_bytes}),
        ValueSpec::Enum { choices } => json!({"type":"enum","choices":choices}),
    };
    json!({"id":p.id.get().to_string(),"name":p.name,"unit":{"id":p.unit.id(),"symbol":p.unit.symbol()},
        "value_spec":spec,"access":match p.access {AccessMode::ReadOnly=>"read_only",AccessMode::ReadWrite=>"read_write",AccessMode::WriteOnly=>"write_only"},
        "role":match p.role {ParameterRole::Measurement=>"measurement",ParameterRole::Configuration=>"configuration",ParameterRole::Actuator=>"actuator",ParameterRole::Action=>"action",ParameterRole::Diagnostic=>"diagnostic"},
        "write_effect":match p.write_effect {WriteEffect::None=>"none",WriteEffect::ConfigurationOnly=>"configuration_only",WriteEffect::OutputAffecting=>"output_affecting"},
        "signal":p.signal.map(|s|json!({"instrument":s.instrument().get().to_string(),"parameter":s.parameter().get().to_string()}))})
}
fn value_type_name(spec: &ValueSpec) -> &'static str {
    match spec {
        ValueSpec::Float { .. } => "float",
        ValueSpec::Integer { .. } => "integer",
        ValueSpec::Boolean => "boolean",
        ValueSpec::Text { .. } => "text",
        ValueSpec::Enum { .. } => "enum",
    }
}
pub(crate) fn output_json(s: OutputSnapshot) -> Value {
    let owner = s.lease.map(|l| match l.owner() {
        OutputOwner::Manual(id) => json!({"kind":"manual","id":id.to_string()}),
        OutputOwner::Automatic(id) => json!({"kind":"automatic","id":id.to_string()}),
    });
    let observation = |o: Option<lab_core::output::OutputObservation>| {
        o.map(|v| json!({"value":v.value,"at":nanos(v.at)}))
    };
    json!({"state":match s.state {OutputState::Unverified=>"unverified",OutputState::SafePending=>"safe_pending",
            OutputState::Disarmed=>"disarmed",OutputState::ArmedManual=>"armed_manual",OutputState::ArmedAuto=>"armed_auto",OutputState::FaultLatched=>"fault_latched"},
        "owner":owner,"instance":s.lease.map(|l|l.instance().to_string()),"epoch":s.epoch.to_string(),"lease_expires_at":s.lease.map(|l|nanos(l.expires())),
        "fault_latched":s.fault_latched,"safe_confirmed":s.safe_confirmed,"pending":s.pending,
        "in_flight":s.in_flight.is_some(),"requested":s.requested,"sent":observation(s.sent),
        "acknowledged":observation(s.acknowledged),"readback":observation(s.readback),
        "reported_readback":observation(s.reported_readback),
        "readback_failure":s.readback_failure.map(|failure|match failure {
            lab_core::output::OutputReadbackFailure::Mismatch=>"mismatch",
            lab_core::output::OutputReadbackFailure::Unavailable=>"unavailable"}),
        "outcome":s.outcome.map(|o|match o {DispatchOutcome::Acknowledged=>"acknowledged",DispatchOutcome::ReadbackVerified=>"readback_verified",
            DispatchOutcome::Failed=>"failed",DispatchOutcome::Ambiguous=>"ambiguous"})})
}
pub(crate) fn reference_json(id: u64, s: ReferenceSnapshot) -> Value {
    match s {
        ReferenceSnapshot::Fixed {
            value,
            unit,
            last_at,
            revision,
            ..
        } => json!({"reference":id.to_string(),"kind":"fixed","value":value,
            "revision":revision.to_string(),"status":"valid","configurable":true,
            "last_at":last_at.map(nanos),"last_evaluated_at_ns":last_at.map(nanos),
            "unit":{"id":unit.id().to_string(),"symbol":unit.symbol()}}),
        ReferenceSnapshot::Ramp {
            state, revision, ..
        } => json!({"reference":id.to_string(),"kind":"ramp","value":state.current,
            "target":state.target,"rate":state.rate,"revision":revision.to_string(),
            "status":"valid","configurable":true,"last_at":nanos(state.last_at),
            "last_evaluated_at_ns":nanos(state.last_at),
            "unit":{"id":state.unit.id().to_string(),"symbol":state.unit.symbol()}}),
    }
}
pub(crate) fn state_name(s: ControllerState) -> &'static str {
    match s {
        ControllerState::Created => "created",
        ControllerState::Ready => "ready",
        ControllerState::Warming => "warming",
        ControllerState::Running => "running",
        ControllerState::Paused => "paused",
        ControllerState::Failed => "failed",
    }
}
pub(crate) fn quality_name(s: lab_core::SampleQuality) -> &'static str {
    match s {
        lab_core::SampleQuality::Good => "good",
        lab_core::SampleQuality::Unavailable => "unavailable",
    }
}
pub(crate) fn nanos(at: std::time::Duration) -> String {
    at.as_nanos().to_string()
}
fn operation_status(a: Admission) -> Value {
    match a {
        Admission::Known(s) => operation_state(s),
        _ => json!({"state":"outcome_unknown"}),
    }
}
fn operation_state(s: OperationState) -> Value {
    match s {
        OperationState::Accepted => json!({"state":"accepted"}),
        OperationState::Completed(text) => {
            json!({"state":"completed","result":serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)})
        }
        OperationState::Failed(code) => {
            let mut value = json!({"state":"failed"});
            PublicError::from_internal_code(&code).apply_to(&mut value);
            value
        }
        OperationState::FailedWithResult { code, detail } => {
            let mut value = json!({"state":"failed",
                "result":serde_json::from_str::<Value>(&detail).unwrap_or(Value::Null)});
            PublicError::from_internal_code(&code).apply_to(&mut value);
            value
        }
    }
}
fn operation_reply(msg: &str, rid: &WireRequestId, s: OperationState) -> Value {
    let mut value = json!({"v":protocol::PROTOCOL_VERSION,"msg_id":msg,
        "type":"operation","request_id":{"scope":rid.scope,"seq":rid.seq.to_string()}});
    let state = operation_state(s);
    for (key, value_part) in state.as_object().expect("fixed state object") {
        value[key] = value_part.clone();
    }
    value
}
fn error_reply(msg: &str, code: &str) -> Value {
    let mut value = json!({"v":protocol::PROTOCOL_VERSION,"msg_id":msg,
        "type":"error","accepted":false});
    PublicError::from_internal_code(code).apply_to(&mut value);
    value
}
fn session_code(e: SessionError) -> &'static str {
    match e {
        SessionError::InstanceChanged => "instance_changed",
        SessionError::ScopeInUse => "scope_in_use",
        SessionError::ScopeCapacity => "busy",
        _ => "scope_unknown",
    }
}
fn domain_code(e: Error) -> &'static str {
    match e {
        Error::Controller(ControllerError::RevisionConflict) => "revision_conflict",
        Error::Controller(ControllerError::UnknownController) => "unknown_controller",
        Error::Controller(ControllerError::UnknownReference) => "unknown_reference",
        Error::Controller(
            ControllerError::DuplicateController
            | ControllerError::DuplicateReference
            | ControllerError::InvalidConfiguration
            | ControllerError::InvalidTickTime
            | ControllerError::Algorithm
            | ControllerError::RevisionExhausted,
        ) => "invalid_configuration",
        Error::Controller(ControllerError::InvalidState) => "invalid_state",
        Error::Controller(ControllerError::InputUnavailable) => "input_unavailable",
        Error::Controller(ControllerError::StaleInput) => "stale_input",
        Error::Controller(ControllerError::Output) => "output_rejected",
        Error::Output(OutputError::UnknownActuator) => "unknown_actuator",
        Error::Output(OutputError::Busy) => "busy",
        Error::Output(OutputError::Expired) => "timeout",
        Error::Output(_) => "output_rejected",
        Error::UnknownInstrument(_) => "unknown_instrument",
        Error::UnknownParameter { .. } => "unknown_parameter",
        Error::UnknownSignal(_) => "unknown_signal",
        Error::Transport(TransportError::UnknownResource) => "unknown_resource",
        Error::Transport(
            TransportError::QueueFull
            | TransportError::ResourceLimit
            | TransportError::ResourceBusy,
        ) => "busy",
        Error::Transport(
            TransportError::InvalidTransaction
            | TransportError::InvalidTime
            | TransportError::CounterExhausted
            | TransportError::DuplicateResource,
        ) => "invalid_configuration",
        Error::Transport(TransportError::ResourceUnavailable) => "transport_unavailable",
        Error::InvalidConfiguration(_) => "invalid_configuration",
        Error::RecordingUnavailable => "recording_unavailable",
        _ => "domain_rejected",
    }
}
fn lifecycle_domain_error(error: LifecycleOperationError) -> Error {
    match error {
        LifecycleOperationError::RecordingUnavailable => Error::RecordingUnavailable,
        LifecycleOperationError::TransportUnavailable => {
            Error::Transport(lab_core::transport::TransportError::ResourceUnavailable)
        }
        LifecycleOperationError::Conflict => Error::Controller(ControllerError::RevisionConflict),
        _ => Error::InvalidConfiguration("lifecycle operation failed"),
    }
}
fn lower_hex_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn history_page_json(page: &HistoryPage, cursor_token: Option<&str>) -> Value {
    let rows: Vec<Value> = page
        .rows
        .iter()
        .map(|row| {
            let value = match &row.value {
                Some(lab_core::Value::Float(value)) => json!({"kind":"float","value":value}),
                Some(lab_core::Value::Integer(value)) => {
                    json!({"kind":"integer","value":value.to_string()})
                }
                Some(lab_core::Value::Boolean(value)) => json!({"kind":"boolean","value":value}),
                Some(lab_core::Value::Text(value)) => json!({"kind":"text","value":value}),
                Some(lab_core::Value::Enum(value)) => json!({"kind":"enum","value":value}),
                None => Value::Null,
            };
            let lineage = row.lineage.as_ref().map(|input| json!({
                "signal":{"instrument":input.signal.instrument().get().to_string(),
                          "parameter":input.signal.parameter().get().to_string()},
                "value":input.value,"unit":input.unit,
                "published_at_ns":nanos(input.published_at),
                "observed_at_ns":nanos(input.observed_at),
                "source_generation":input.source_generation.to_string(),
                "source_revision":input.source_revision.to_string(),
                "source_state_revision":input.source_state_revision.map(|revision|revision.to_string())}));
            json!({"record_seq":row.record_sequence.to_string(),
            "published_at_ns":nanos(row.published_at),
            "observed_at_ns":nanos(row.observed_at),"unit":row.unit,
            "quality":row.quality,"failure":row.failure,"value":value,
            "generation":row.generation.to_string(),"revision":row.revision.to_string(),
            "state_revision":row.state_revision.map(|revision|revision.to_string()),
            "lineage":lineage})
        })
        .collect();
    json!({"rows":rows,"watermark":page.watermark.to_string(),
        "coverage":page.coverage,"next_cursor":cursor_token,
        "loss":page.loss.as_ref().map(|loss| json!({
            "reason":loss.reason,
            "first_missing_fact_seq":loss.first_missing_fact.map(|id|id.to_string()),
            "known_missing_count":loss.known_missing_count.map(|id|id.to_string()),
            "last_accepted_fact_seq":loss.last_accepted_fact.map(|id|id.to_string()),
            "cutoff_monotonic_ns":nanos(loss.cutoff),
            "last_confirmed_record_seq":loss.last_confirmed_record.to_string()})),
        "has_more":cursor_token.is_some(),"raw":true})
}

fn runs_page_json(page: &RunsPage, cursor_token: Option<&str>) -> Value {
    let runs: Vec<Value> = page
        .runs
        .iter()
        .map(|run| {
            json!({
                "run_id":{"boot_id":run.boot_id,"run_no":run.run_no.to_string()},
                "label":run.label,"policy":run.policy,"state":run.state,
                "coverage":run.coverage,
            })
        })
        .collect();
    json!({"runs":runs,"next_cursor":cursor_token,
        "has_more":cursor_token.is_some(),"mode":"runs"})
}

#[cfg(test)]
mod lifecycle_error_tests {
    use super::*;

    #[test]
    fn recorder_lifecycle_failure_uses_existing_recording_unavailable_wire_code() {
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::RecordingUnavailable
            )),
            "recording_unavailable"
        );
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::InvalidCandidate
            )),
            "invalid_configuration"
        );
    }

    #[test]
    fn reconnect_transport_failure_and_stale_generation_keep_distinct_public_codes() {
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::TransportUnavailable
            )),
            "transport_unavailable"
        );
        assert_eq!(
            domain_code(lifecycle_domain_error(LifecycleOperationError::Conflict)),
            "revision_conflict"
        );
    }
}
