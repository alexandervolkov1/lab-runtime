//! Recorder lifecycle and durable-history completion delivery.
//!
//! [`crate::recorder_api`] owns public Recorder DTO validation/projection. This
//! module only correlates bounded asynchronous Recorder work with admitted
//! Application operations and connection-local pages; it performs no SQL and owns
//! no experiment state.

use super::{
    Application, CompletedHistory, HistoryCache, HistoryContinuation, MAX_HISTORY_CURSORS,
    PendingHistory, PendingRecording, RetainedHistoryCursor,
    common::{domain_code, operation_reply, operation_state},
    operations::recorded_terminal,
};
use crate::{
    host::Clock,
    recorder::{AnnotationRecord, HistoryFilter, OperationRecord, RecordingState},
    recorder_api,
    service::ServiceHost,
    sessions::{Mutation, OperationState},
    wire::WireRequestId,
};
use lab_core::{InstrumentId, ParameterId};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) struct AdmittedRecorderRequest<'a> {
    pub(super) connection: u64,
    pub(super) payload: &'a Mutation,
    pub(super) scope: &'a str,
    pub(super) request_id: &'a WireRequestId,
    pub(super) message: &'a str,
    pub(super) now: Duration,
    pub(super) accepted: &'a Value,
}

impl Application {
    pub(super) fn recording_query(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        operation: &str,
        args: &Value,
    ) -> Option<Result<Value, &'static str>> {
        let result = match operation {
            "history_page" => {
                let token = match args.get("page_token").and_then(Value::as_str) {
                    Some(token) => token,
                    None => return Some(Err("invalid_args")),
                };
                let page = match self.history_pages.get(&connection) {
                    Some(page) => page,
                    None => return Some(Err("history_page_expired")),
                };
                if page.token != token || service.clock().now() >= page.expires {
                    Err("history_page_expired")
                } else {
                    Ok(page.page.clone())
                }
            }
            "history_release" => {
                let token = match args.get("page_token").and_then(Value::as_str) {
                    Some(token) => token,
                    None => return Some(Err("invalid_args")),
                };
                if self
                    .history_pages
                    .get(&connection)
                    .is_some_and(|page| page.token == token)
                {
                    self.history_pages.remove(&connection);
                    Ok(json!({"released":true}))
                } else {
                    Err("history_page_expired")
                }
            }
            "recording_status" => Ok(recorder_api::status_json(
                service.owner().recording_status(),
                service.owner().recording_database_id(),
                service.owner().recording_policy(),
                service.boot_id(),
            )),
            _ => return None,
        };
        Some(result)
    }

    pub(super) fn handle_recorder_mutation(
        &mut self,
        service: &mut ServiceHost,
        request: AdmittedRecorderRequest<'_>,
    ) -> Option<Vec<Value>> {
        let AdmittedRecorderRequest {
            connection,
            payload,
            scope,
            request_id,
            message,
            now,
            accepted,
        } = request;
        if let Mutation::ExperimentAnnotate { name, data_json } = payload {
            let annotation = AnnotationRecord {
                scope: scope.to_owned(),
                request_seq: request_id.seq,
                name: name.clone(),
                data_json: data_json.clone(),
                at: now,
            };
            let outcome = service.owner_mut().annotate(annotation).map_or_else(
                |error| OperationState::Failed(domain_code(error).into()),
                |sequence| {
                    OperationState::Completed(
                        json!({"record_seq":sequence.to_string(),"durability":"pending"})
                            .to_string(),
                    )
                },
            );
            self.sessions
                .complete(
                    scope,
                    request_id.seq,
                    outcome.clone(),
                    service.clock().now(),
                )
                .expect("admitted bounded annotation operation");
            return Some(vec![
                accepted.clone(),
                operation_reply(message, request_id, outcome),
            ]);
        }
        if let Mutation::HistoryReadRuns {
            database_id,
            max_records,
            cursor,
        } = payload
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
            return Some(match scheduled {
                Ok(job) => {
                    self.pending_history.insert(
                        connection,
                        PendingHistory {
                            scope: scope.to_owned(),
                            seq: request_id.seq,
                            msg: message.to_owned(),
                            job,
                            submitted: now,
                            runs: true,
                        },
                    );
                    vec![accepted.clone()]
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(scope, request_id.seq, failed.clone(), now)
                        .expect("admitted run discovery operation");
                    vec![
                        accepted.clone(),
                        operation_reply(message, request_id, failed),
                    ]
                }
            });
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
        } = payload
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
            return Some(match scheduled {
                Ok(job) => {
                    self.pending_history.insert(
                        connection,
                        PendingHistory {
                            scope: scope.to_owned(),
                            seq: request_id.seq,
                            msg: message.to_owned(),
                            job,
                            submitted: now,
                            runs: false,
                        },
                    );
                    vec![accepted.clone()]
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(scope, request_id.seq, failed.clone(), now)
                        .expect("admitted history read operation");
                    vec![
                        accepted.clone(),
                        operation_reply(message, request_id, failed),
                    ]
                }
            });
        }
        if matches!(
            payload,
            Mutation::RecordingStart { .. } | Mutation::RecordingStop { .. }
        ) {
            let (command, data) = match payload {
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
                scope: scope.to_owned(),
                request_seq: request_id.seq,
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
                match payload {
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
            return Some(match action {
                Ok(()) => {
                    self.pending_recording = Some(PendingRecording {
                        connection,
                        scope: scope.to_owned(),
                        seq: request_id.seq,
                        msg: message.to_owned(),
                        start: matches!(payload, Mutation::RecordingStart { .. }),
                        run_no: saved_run,
                        interval_no: saved_interval,
                    });
                    vec![accepted.clone()]
                }
                Err(code) => {
                    let failed = OperationState::Failed(code.into());
                    self.sessions
                        .complete(scope, request_id.seq, failed.clone(), service.clock().now())
                        .expect("admitted recording operation");
                    let (phase, data) = recorded_terminal(&failed);
                    let terminal_at = service.clock().now();
                    service.owner_mut().record_operation(OperationRecord {
                        scope: scope.to_owned(),
                        request_seq: request_id.seq,
                        command,
                        phase,
                        data,
                        outcome_basis: "domain_result",
                        at: terminal_at,
                    });
                    vec![
                        accepted.clone(),
                        operation_reply(message, request_id, failed),
                    ]
                }
            });
        }
        None
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
        let request_id = WireRequestId {
            scope: pending.scope,
            seq: pending.seq,
        };
        vec![(
            pending.connection,
            operation_reply(&pending.msg, &request_id, result),
        )]
    }

    /// Reconcile completed bounded history jobs and retain immutable pages.
    /// This only takes worker results; SQL ran on the Recorder thread.
    pub fn poll_history(&mut self, service: &mut ServiceHost) -> Vec<(u64, Value)> {
        for job in self.orphan_history.drain(..) {
            service.owner_mut().cancel_history(job);
        }
        let now = service.clock().now();
        self.expire_projections_at(now);
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
                            let request_id = WireRequestId {
                                scope: pending.scope,
                                seq: pending.seq,
                            };
                            replies.push((
                                connection,
                                operation_reply(&pending.msg, &request_id, failed),
                            ));
                        }
                        continue;
                    };
                    let token = format!("history-page-{}", self.next_token);
                    let cursor_token =
                        has_cursor.then(|| format!("history-cursor-{}", self.next_token + 1));
                    self.next_token = following;
                    let encoded = match &page {
                        CompletedHistory::Measurements(page) => {
                            recorder_api::history_page_json(page, cursor_token.as_deref())
                        }
                        CompletedHistory::Runs(page) => {
                            recorder_api::runs_page_json(page, cursor_token.as_deref())
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
                let request_id = WireRequestId {
                    scope: pending.scope,
                    seq: pending.seq,
                };
                replies.push((
                    connection,
                    operation_reply(&pending.msg, &request_id, terminal),
                ));
            }
        }
        replies
    }
}
