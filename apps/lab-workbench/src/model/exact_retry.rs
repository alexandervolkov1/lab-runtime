//! Renderer-neutral confirmation and correlation for worker-owned Exact Retry.

use super::{RecoveryStatusTracker, WorkbenchModel};
use crate::client::{
    ClientHandle,
    types::{
        ClientUpdate, CommandSendError, ConnectionState, KnownAdmission, MutationIdentity,
        RecoveryRecord, ReplyKind,
    },
};

/// Fixed operator-facing meaning of Exact Retry; it deliberately makes no safety claim.
pub(crate) const EXACT_RETRY_WARNING: &str = "Exact Retry resends the same request_id and the same worker-retained payload. If Runtime already admitted that identity, deduplication must not execute it twice. If it was never admitted, this may be its first execution. If Runtime no longer retains the outcome, the result may remain outcome_unknown.";
const EXACT_RETRY_MESSAGE_BYTES: usize = 512;
const TRUNCATION_MARKER: &str = "… [truncated]";

/// Immutable worker-projected evidence reviewed before an Exact Retry submission.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreparedExactRetry {
    pub(crate) record: RecoveryRecord,
    pub(crate) boot_id: String,
    pub(crate) scope: String,
}

impl PreparedExactRetry {
    /// Revalidates every authority input without rebasing the prepared record.
    pub(crate) fn is_current(
        &self,
        model: &WorkbenchModel,
        status: &RecoveryStatusTracker,
    ) -> bool {
        model.connection == ConnectionState::Ready
            && model.recovery_problem.is_none()
            && model.recovery.quarantined.is_empty()
            && model.hello.as_ref().is_some_and(|hello| {
                hello.boot_id == self.boot_id
                    && hello.scope == self.scope
                    && self.record.boot_id == hello.boot_id
                    && self.record.identity.scope == hello.scope
            })
            && model
                .recovery
                .mutations
                .iter()
                .any(|record| record == &self.record)
            && unresolved(self.record.admission)
            && !status.is_pending(&self.record.identity)
    }
}

/// One bounded, renderer-neutral Exact Retry workflow; no attempt history is retained.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum ExactRetryState {
    #[default]
    Idle,
    AwaitingConfirmation(PreparedExactRetry),
    Submitted {
        command_id: u64,
        prepared: PreparedExactRetry,
    },
    Accepted {
        command_id: u64,
        prepared: PreparedExactRetry,
    },
    Completed {
        prepared: PreparedExactRetry,
    },
    Failed {
        prepared: PreparedExactRetry,
    },
    OutcomeUnknown {
        prepared: PreparedExactRetry,
    },
    LocalFailure {
        prepared: PreparedExactRetry,
        reason: String,
    },
    ApplicationFailure {
        prepared: PreparedExactRetry,
        code: String,
    },
    Interrupted {
        prepared: PreparedExactRetry,
    },
    DraftStale {
        prepared: PreparedExactRetry,
    },
}

/// Local preparation/submission failure; it never changes worker recovery authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExactRetryError {
    WorkflowBusy,
    ClientNotReady,
    HelloMissing,
    RecoveryJournalProblem,
    QuarantineUnresolved,
    RecordMissing,
    SessionMismatch,
    TerminalRecord,
    StatusPending,
    DraftStale,
    LocalSubmission(CommandSendError),
}

impl std::fmt::Display for ExactRetryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkflowBusy => formatter.write_str("another Exact Retry workflow is active"),
            Self::ClientNotReady => formatter.write_str("Application client is not Ready"),
            Self::HelloMissing => formatter.write_str("authoritative hello is unavailable"),
            Self::RecoveryJournalProblem => {
                formatter.write_str("recovery journal authority is unavailable")
            }
            Self::QuarantineUnresolved => {
                formatter.write_str("quarantined recovery evidence blocks Exact Retry")
            }
            Self::RecordMissing => formatter.write_str("active recovery record is unavailable"),
            Self::SessionMismatch => {
                formatter.write_str("recovery record is not attached to the current session")
            }
            Self::TerminalRecord => formatter.write_str("terminal evidence is not retryable"),
            Self::StatusPending => {
                formatter.write_str("Check Status is already pending for this identity")
            }
            Self::DraftStale => formatter.write_str("Exact Retry evidence changed; review again"),
            Self::LocalSubmission(error) => {
                write!(formatter, "Exact Retry submission failed: {error}")
            }
        }
    }
}

impl std::error::Error for ExactRetryError {}

/// Narrow submission boundary: callers can provide only the retained identity.
pub(crate) trait ExactRetrySubmitter {
    fn retry_exact(&self, identity: MutationIdentity) -> Result<u64, CommandSendError>;
}

impl ExactRetrySubmitter for ClientHandle {
    fn retry_exact(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
        self.retry_mutation(identity)
    }
}

/// Single current Exact Retry interaction; it never owns authoritative recovery state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ExactRetryWorkflow {
    pub(crate) state: ExactRetryState,
}

impl ExactRetryWorkflow {
    pub(crate) fn begin(
        &mut self,
        model: &WorkbenchModel,
        status: &RecoveryStatusTracker,
        identity: MutationIdentity,
    ) -> Result<(), ExactRetryError> {
        if !matches!(self.state, ExactRetryState::Idle) {
            return Err(ExactRetryError::WorkflowBusy);
        }
        self.state = ExactRetryState::AwaitingConfirmation(prepare(model, status, identity)?);
        Ok(())
    }

    pub(crate) fn confirm(
        &mut self,
        model: &WorkbenchModel,
        status: &RecoveryStatusTracker,
        submitter: &impl ExactRetrySubmitter,
    ) -> Result<u64, ExactRetryError> {
        let ExactRetryState::AwaitingConfirmation(prepared) = &self.state else {
            return Err(ExactRetryError::WorkflowBusy);
        };
        let prepared = prepared.clone();
        if !prepared.is_current(model, status) {
            self.state = ExactRetryState::DraftStale {
                prepared: prepared.clone(),
            };
            return Err(ExactRetryError::DraftStale);
        }
        let command_id = submitter
            .retry_exact(prepared.record.identity.clone())
            .map_err(ExactRetryError::LocalSubmission)?;
        self.state = ExactRetryState::Submitted {
            command_id,
            prepared,
        };
        Ok(command_id)
    }

    pub(crate) fn cancel_or_acknowledge(&mut self) {
        self.state = ExactRetryState::Idle;
    }

    /// Consumes ordered updates after the model and B1 status tracker consumed them.
    pub(crate) fn after_update(
        &mut self,
        update: &ClientUpdate,
        model: &WorkbenchModel,
        status: &RecoveryStatusTracker,
    ) {
        if let ExactRetryState::AwaitingConfirmation(prepared) = &self.state
            && !prepared.is_current(model, status)
        {
            self.state = ExactRetryState::DraftStale {
                prepared: prepared.clone(),
            };
            return;
        }

        let Some((command_id, prepared)) = submitted(&self.state) else {
            return;
        };
        match update {
            ClientUpdate::RecoveryProjection { active, .. } => {
                let Some(record) = active
                    .iter()
                    .find(|record| record.identity == prepared.record.identity)
                else {
                    self.state = ExactRetryState::Interrupted { prepared };
                    return;
                };
                if !same_payload(record, &prepared.record) {
                    self.state = ExactRetryState::Interrupted { prepared };
                    return;
                }
                self.state = match record.admission {
                    KnownAdmission::Accepted => ExactRetryState::Accepted {
                        command_id,
                        prepared,
                    },
                    KnownAdmission::Completed => ExactRetryState::Completed { prepared },
                    KnownAdmission::Failed => ExactRetryState::Failed { prepared },
                    KnownAdmission::Pending | KnownAdmission::Ambiguous => return,
                };
            }
            ClientUpdate::Reply {
                command_id: received,
                kind: ReplyKind::PublicError,
                envelope,
                ..
            } if *received == command_id => {
                let code = envelope
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown_application_error");
                self.state = if code == "outcome_unknown" {
                    ExactRetryState::OutcomeUnknown { prepared }
                } else {
                    ExactRetryState::ApplicationFailure {
                        prepared,
                        code: bounded_message(code),
                    }
                };
            }
            ClientUpdate::LocalRejected {
                command_id: received,
                reason,
            } if *received == command_id => {
                self.state = ExactRetryState::LocalFailure {
                    prepared,
                    reason: bounded_message(reason),
                };
            }
            ClientUpdate::TransportFailure { .. }
            | ClientUpdate::ResnapshotRequired {
                connection_lost: true,
                ..
            }
            | ClientUpdate::State(
                ConnectionState::Disconnected
                | ConnectionState::Stale
                | ConnectionState::Stopping
                | ConnectionState::Stopped,
            ) => self.state = ExactRetryState::Interrupted { prepared },
            _ => {}
        }
    }
}

fn prepare(
    model: &WorkbenchModel,
    status: &RecoveryStatusTracker,
    identity: MutationIdentity,
) -> Result<PreparedExactRetry, ExactRetryError> {
    if model.connection != ConnectionState::Ready {
        return Err(ExactRetryError::ClientNotReady);
    }
    let hello = model.hello.as_ref().ok_or(ExactRetryError::HelloMissing)?;
    if model.recovery_problem.is_some() {
        return Err(ExactRetryError::RecoveryJournalProblem);
    }
    if !model.recovery.quarantined.is_empty() {
        return Err(ExactRetryError::QuarantineUnresolved);
    }
    let record = model
        .recovery
        .mutations
        .iter()
        .find(|record| record.identity == identity)
        .cloned()
        .ok_or(ExactRetryError::RecordMissing)?;
    if record.boot_id != hello.boot_id || record.identity.scope != hello.scope {
        return Err(ExactRetryError::SessionMismatch);
    }
    if !unresolved(record.admission) {
        return Err(ExactRetryError::TerminalRecord);
    }
    if status.is_pending(&identity) {
        return Err(ExactRetryError::StatusPending);
    }
    Ok(PreparedExactRetry {
        record,
        boot_id: hello.boot_id.clone(),
        scope: hello.scope.clone(),
    })
}

fn submitted(state: &ExactRetryState) -> Option<(u64, PreparedExactRetry)> {
    match state {
        ExactRetryState::Submitted {
            command_id,
            prepared,
        }
        | ExactRetryState::Accepted {
            command_id,
            prepared,
        } => Some((*command_id, prepared.clone())),
        _ => None,
    }
}

fn unresolved(admission: KnownAdmission) -> bool {
    matches!(
        admission,
        KnownAdmission::Pending | KnownAdmission::Accepted | KnownAdmission::Ambiguous
    )
}

fn same_payload(left: &RecoveryRecord, right: &RecoveryRecord) -> bool {
    left.boot_id == right.boot_id
        && left.identity == right.identity
        && left.op == right.op
        && left.args == right.args
}

fn bounded_message(value: &str) -> String {
    if value.len() <= EXACT_RETRY_MESSAGE_BYTES {
        return value.to_owned();
    }
    let content_bytes = EXACT_RETRY_MESSAGE_BYTES - TRUNCATION_MARKER.len();
    let mut boundary = content_bytes.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}{}", &value[..boundary], TRUNCATION_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::types::{
            EventCursor, HelloState, QuarantinedRecoveryRecord, RecoveryQuarantineReason,
        },
        model::recovery_status::RecoveryStatusSubmitter,
        presentation::PresentationDocument,
    };
    use serde_json::{Value, json};
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeRetrySubmitter {
        sent: RefCell<Vec<MutationIdentity>>,
        result: RefCell<Option<Result<u64, CommandSendError>>>,
    }

    impl ExactRetrySubmitter for FakeRetrySubmitter {
        fn retry_exact(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.sent.borrow_mut().push(identity);
            self.result.borrow_mut().take().unwrap_or(Ok(41))
        }
    }

    #[derive(Default)]
    struct FakeStatusSubmitter {
        sent: RefCell<Vec<MutationIdentity>>,
    }

    impl RecoveryStatusSubmitter for FakeStatusSubmitter {
        fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.sent.borrow_mut().push(identity);
            Ok(17)
        }
    }

    fn identity(seq: u64) -> MutationIdentity {
        MutationIdentity {
            scope: "scope".into(),
            seq,
        }
    }

    fn record(admission: KnownAdmission) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "boot".into(),
            identity: identity(1),
            op: "reference_retune".into(),
            args: json!({"reference":"1","expected_revision":"1","target":2.0,"rate":1.0}),
            admission,
        }
    }

    fn hello(boot: &str, scope: &str) -> HelloState {
        HelloState {
            boot_id: boot.into(),
            scope: scope.into(),
            next_seq: 2,
            operations: vec!["operation_status".into(), "reference_retune".into()],
            capabilities: json!({}),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: boot.into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: boot.into(),
                seq: 0,
            },
        }
    }

    fn model(active: Vec<RecoveryRecord>) -> WorkbenchModel {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(hello("boot", "scope")));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active,
            quarantined: Vec::new(),
        });
        model
    }

    fn projection(record: RecoveryRecord) -> ClientUpdate {
        ClientUpdate::RecoveryProjection {
            active: vec![record],
            quarantined: Vec::new(),
        }
    }

    fn public_error(command_id: u64, code: &str) -> ClientUpdate {
        ClientUpdate::Reply {
            command_id,
            msg_id: "1".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::PublicError,
            envelope: json!({"type":"error","code":code}),
            recovery: None,
        }
    }

    #[test]
    fn prepare_captures_exact_record_and_confirm_submits_only_identity() {
        let retained = record(KnownAdmission::Ambiguous);
        let model = model(vec![retained.clone()]);
        let status = RecoveryStatusTracker::default();
        let submitter = FakeRetrySubmitter::default();
        let mut workflow = ExactRetryWorkflow::default();

        workflow
            .begin(&model, &status, retained.identity.clone())
            .unwrap();
        let ExactRetryState::AwaitingConfirmation(prepared) = &workflow.state else {
            panic!("expected confirmation")
        };
        assert_eq!(prepared.record, retained);
        assert_eq!(prepared.boot_id, "boot");
        assert_eq!(prepared.scope, "scope");
        assert!(EXACT_RETRY_WARNING.contains("same worker-retained payload"));
        assert!(!EXACT_RETRY_WARNING.contains("safe retry"));

        assert_eq!(workflow.confirm(&model, &status, &submitter), Ok(41));
        assert_eq!(submitter.sent.borrow().as_slice(), &[identity(1)]);
        assert!(model.actions.is_empty());
    }

    #[test]
    fn any_prepared_record_or_session_change_is_stale_and_sends_nothing() {
        for changed in [
            RecoveryRecord {
                admission: KnownAdmission::Accepted,
                ..record(KnownAdmission::Ambiguous)
            },
            RecoveryRecord {
                op: "recording_start".into(),
                ..record(KnownAdmission::Ambiguous)
            },
            RecoveryRecord {
                args: json!({"different":true}),
                ..record(KnownAdmission::Ambiguous)
            },
            RecoveryRecord {
                identity: identity(2),
                ..record(KnownAdmission::Ambiguous)
            },
        ] {
            let retained = record(KnownAdmission::Ambiguous);
            let mut model = model(vec![retained.clone()]);
            let status = RecoveryStatusTracker::default();
            let submitter = FakeRetrySubmitter::default();
            let mut workflow = ExactRetryWorkflow::default();
            workflow
                .begin(&model, &status, retained.identity.clone())
                .unwrap();
            model.apply_client_update(projection(changed));

            assert_eq!(
                workflow.confirm(&model, &status, &submitter),
                Err(ExactRetryError::DraftStale)
            );
            assert!(matches!(workflow.state, ExactRetryState::DraftStale { .. }));
            assert!(submitter.sent.borrow().is_empty());
        }

        for changed_hello in [hello("other-boot", "scope"), hello("boot", "other-scope")] {
            let retained = record(KnownAdmission::Ambiguous);
            let mut model = model(vec![retained.clone()]);
            let status = RecoveryStatusTracker::default();
            let submitter = FakeRetrySubmitter::default();
            let mut workflow = ExactRetryWorkflow::default();
            workflow
                .begin(&model, &status, retained.identity.clone())
                .unwrap();
            model.apply_client_update(ClientUpdate::Hello(changed_hello));

            assert_eq!(
                workflow.confirm(&model, &status, &submitter),
                Err(ExactRetryError::DraftStale)
            );
            assert!(submitter.sent.borrow().is_empty());
        }
    }

    #[test]
    fn terminal_quarantine_journal_and_status_pending_are_ineligible() {
        let status = RecoveryStatusTracker::default();
        for admission in [KnownAdmission::Completed, KnownAdmission::Failed] {
            let retained = record(admission);
            let model = model(vec![retained.clone()]);
            let mut workflow = ExactRetryWorkflow::default();
            assert_eq!(
                workflow.begin(&model, &status, retained.identity),
                Err(ExactRetryError::TerminalRecord)
            );
        }

        let retained = record(KnownAdmission::Ambiguous);
        let mut quarantined = model(Vec::new());
        quarantined.apply_client_update(ClientUpdate::RecoveryProjection {
            active: Vec::new(),
            quarantined: vec![QuarantinedRecoveryRecord {
                record: retained.clone(),
                reason: RecoveryQuarantineReason::AttachedBootMismatch,
            }],
        });
        assert_eq!(
            ExactRetryWorkflow::default().begin(&quarantined, &status, retained.identity.clone()),
            Err(ExactRetryError::QuarantineUnresolved)
        );

        let mut journal_problem = model(vec![retained.clone()]);
        journal_problem.apply_client_update(ClientUpdate::RecoveryJournalProblem {
            reason: "unwritable".into(),
        });
        assert_eq!(
            ExactRetryWorkflow::default().begin(
                &journal_problem,
                &status,
                retained.identity.clone()
            ),
            Err(ExactRetryError::RecoveryJournalProblem)
        );

        let attached = model(vec![retained.clone()]);
        let mut pending_status = RecoveryStatusTracker::default();
        let status_submitter = FakeStatusSubmitter::default();
        pending_status
            .check_status(&attached, &status_submitter, retained.identity.clone())
            .unwrap();
        assert_eq!(
            ExactRetryWorkflow::default().begin(&attached, &pending_status, retained.identity),
            Err(ExactRetryError::StatusPending)
        );
    }

    #[test]
    fn authoritative_projection_alone_advances_retry_admission_and_terminal_state() {
        for (terminal, expected_failed) in [
            (KnownAdmission::Completed, false),
            (KnownAdmission::Failed, true),
        ] {
            let retained = record(KnownAdmission::Ambiguous);
            let mut model = model(vec![retained.clone()]);
            let status = RecoveryStatusTracker::default();
            let submitter = FakeRetrySubmitter::default();
            let mut workflow = ExactRetryWorkflow::default();
            workflow
                .begin(&model, &status, retained.identity.clone())
                .unwrap();
            workflow.confirm(&model, &status, &submitter).unwrap();

            let accepted = projection(RecoveryRecord {
                admission: KnownAdmission::Accepted,
                ..retained.clone()
            });
            model.apply_client_update(accepted.clone());
            workflow.after_update(&accepted, &model, &status);
            assert!(matches!(workflow.state, ExactRetryState::Accepted { .. }));

            let terminal_update = projection(RecoveryRecord {
                admission: terminal,
                ..retained
            });
            model.apply_client_update(terminal_update.clone());
            workflow.after_update(&terminal_update, &model, &status);
            assert_eq!(
                matches!(workflow.state, ExactRetryState::Failed { .. }),
                expected_failed
            );
            assert_eq!(
                matches!(workflow.state, ExactRetryState::Completed { .. }),
                !expected_failed
            );
            assert!(model.actions.is_empty());
        }
    }

    #[test]
    fn outcome_unknown_and_other_failures_retain_evidence_without_history_growth() {
        let retained = record(KnownAdmission::Accepted);
        let mut model = model(vec![retained.clone()]);
        let status = RecoveryStatusTracker::default();
        let submitter = FakeRetrySubmitter::default();
        let mut workflow = ExactRetryWorkflow::default();
        workflow
            .begin(&model, &status, retained.identity.clone())
            .unwrap();
        let command_id = workflow.confirm(&model, &status, &submitter).unwrap();
        let unknown = public_error(command_id, "outcome_unknown");
        model.apply_client_update(unknown.clone());
        workflow.after_update(&unknown, &model, &status);

        assert!(matches!(
            workflow.state,
            ExactRetryState::OutcomeUnknown { .. }
        ));
        assert_eq!(model.recovery.mutations, vec![retained.clone()]);
        assert!(model.actions.is_empty());

        workflow.cancel_or_acknowledge();
        workflow
            .begin(&model, &status, retained.identity.clone())
            .unwrap();
        let command_id = workflow.confirm(&model, &status, &submitter).unwrap();
        let failure = public_error(command_id, "busy");
        workflow.after_update(&failure, &model, &status);
        assert!(matches!(
            workflow.state,
            ExactRetryState::ApplicationFailure { .. }
        ));
        assert_eq!(submitter.sent.borrow().len(), 2);
        assert_eq!(model.recovery.mutations, vec![retained]);
    }

    #[test]
    fn retry_failures_are_utf8_bounded_and_repeated_cycles_retain_no_history() {
        let retained = record(KnownAdmission::Accepted);
        let model = model(vec![retained.clone()]);
        let status = RecoveryStatusTracker::default();
        let submitter = FakeRetrySubmitter::default();
        let mut workflow = ExactRetryWorkflow::default();

        for attempt in 0..100 {
            workflow
                .begin(&model, &status, retained.identity.clone())
                .unwrap();
            let command_id = workflow.confirm(&model, &status, &submitter).unwrap();
            let code = format!("failure-{attempt}-{}", "é".repeat(400));
            let failure = public_error(command_id, &code);
            workflow.after_update(&failure, &model, &status);
            let ExactRetryState::ApplicationFailure { code, .. } = &workflow.state else {
                panic!("expected one current application failure")
            };
            assert!(code.len() <= EXACT_RETRY_MESSAGE_BYTES);
            assert!(code.ends_with(TRUNCATION_MARKER));
            assert!(std::str::from_utf8(code.as_bytes()).is_ok());
            workflow.cancel_or_acknowledge();
        }

        assert!(matches!(workflow.state, ExactRetryState::Idle));
        assert_eq!(submitter.sent.borrow().len(), 100);
        assert!(model.actions.is_empty());
    }
}
