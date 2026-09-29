//! Bounded renderer-neutral correlation for manual recovery status queries.

use crate::{
    client::{
        ClientHandle,
        types::{
            ClientUpdate, CommandSendError, ConnectionState, KnownAdmission, MAX_IN_FLIGHT,
            MutationIdentity, RecoveryRecord, ReplyKind,
        },
    },
    model::WorkbenchModel,
};
use serde_json::Value;

/// Maximum bytes retained for one local recovery-status diagnostic.
pub(crate) const STATUS_MESSAGE_BYTES: usize = 512;
const TRUNCATION_MARKER: &str = "… [truncated]";

/// Relationship between a recovery record and the currently attached session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryAttachment {
    /// Record boot and scope match the current authoritative hello.
    Attached,
    /// No authoritative hello is currently available.
    NoHello,
    /// Record belongs to another Runtime boot.
    BootMismatch,
    /// Record belongs to another retained scope.
    ScopeMismatch,
}

/// Local reason why Check Status cannot currently be submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusEligibility {
    Eligible,
    ClientNotReady,
    HelloMissing,
    OperationUnavailable,
    RecoveryJournalProblem,
    RecordMissing,
    BootMismatch,
    ScopeMismatch,
    TerminalRecord,
}

/// Bounded UI-only lifecycle of one manual status request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum RecoveryStatusState {
    /// No status request is outstanding and no last-query notice is retained.
    #[default]
    Idle,
    /// One caller-local command is outstanding for this exact identity.
    Pending { command_id: u64 },
    /// Runtime could not provide a retained authoritative outcome.
    OutcomeUnknown,
    /// The worker rejected the status command locally.
    LocalFailure { message: String },
    /// Application returned a public error for the status request.
    ApplicationFailure { code: String },
    /// Connection continuity was lost while the status request was outstanding.
    Interrupted,
}

/// Read-only bounded presentation of one worker-owned recovery record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryRecordPresentation {
    pub(crate) identity: MutationIdentity,
    pub(crate) op: String,
    pub(crate) admission: KnownAdmission,
    pub(crate) attachment: RecoveryAttachment,
    pub(crate) status: RecoveryStatusState,
    pub(crate) eligibility: StatusEligibility,
}

/// Local submission/correlation error. It never changes worker recovery state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryStatusError {
    Ineligible(StatusEligibility),
    AlreadyPending,
    TrackerCapacity,
    Submission(CommandSendError),
}

impl std::fmt::Display for RecoveryStatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ineligible(reason) => {
                write!(formatter, "Check Status is unavailable: {reason:?}")
            }
            Self::AlreadyPending => formatter.write_str("Check Status is already pending"),
            Self::TrackerCapacity => formatter.write_str("recovery status tracker is full"),
            Self::Submission(error) => write!(formatter, "could not submit Check Status: {error}"),
        }
    }
}

impl std::error::Error for RecoveryStatusError {}

/// Narrow submitter used by the GUI and deterministic tests.
pub(crate) trait RecoveryStatusSubmitter {
    fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError>;
}

impl RecoveryStatusSubmitter for ClientHandle {
    fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
        ClientHandle::operation_status(self, identity)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackedStatus {
    identity: MutationIdentity,
    state: RecoveryStatusState,
}

/// UI/client correlation only; authoritative recovery remains worker-owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecoveryStatusTracker {
    tracked: Vec<TrackedStatus>,
}

impl RecoveryStatusTracker {
    /// Whether one manual status request is outstanding for this exact identity.
    pub(crate) fn is_pending(&self, identity: &MutationIdentity) -> bool {
        matches!(self.state(identity), RecoveryStatusState::Pending { .. })
    }

    /// Builds bounded read-only presentation rows from the authoritative model projection.
    pub(crate) fn presentations(&self, model: &WorkbenchModel) -> Vec<RecoveryRecordPresentation> {
        model
            .recovery
            .mutations
            .iter()
            .take(MAX_IN_FLIGHT)
            .map(|record| RecoveryRecordPresentation {
                identity: record.identity.clone(),
                op: record.op.clone(),
                admission: record.admission,
                attachment: attachment(model, record),
                status: self.state(&record.identity),
                eligibility: eligibility(model, record),
            })
            .collect()
    }

    /// Submits exactly one worker-owned `operation_status` request for an exact identity.
    pub(crate) fn check_status(
        &mut self,
        model: &WorkbenchModel,
        submitter: &impl RecoveryStatusSubmitter,
        identity: MutationIdentity,
    ) -> Result<u64, RecoveryStatusError> {
        self.retain_current(&model.recovery.mutations);
        let record = model
            .recovery
            .mutations
            .iter()
            .find(|record| record.identity == identity)
            .ok_or(RecoveryStatusError::Ineligible(
                StatusEligibility::RecordMissing,
            ))?;
        let eligible = eligibility(model, record);
        if eligible != StatusEligibility::Eligible {
            return Err(RecoveryStatusError::Ineligible(eligible));
        }
        if matches!(self.state(&identity), RecoveryStatusState::Pending { .. }) {
            return Err(RecoveryStatusError::AlreadyPending);
        }
        if !self
            .tracked
            .iter()
            .any(|tracked| tracked.identity == identity)
            && self.tracked.len() == MAX_IN_FLIGHT
        {
            return Err(RecoveryStatusError::TrackerCapacity);
        }
        let command_id = match submitter.operation_status(identity.clone()) {
            Ok(command_id) => command_id,
            Err(error) => {
                self.set_state(
                    identity,
                    RecoveryStatusState::LocalFailure {
                        message: bounded_message(&error.to_string()),
                    },
                )?;
                return Err(RecoveryStatusError::Submission(error));
            }
        };
        self.set_state(identity, RecoveryStatusState::Pending { command_id })?;
        Ok(command_id)
    }

    /// Consumes ordered worker updates without changing authoritative recovery records.
    pub(crate) fn after_update(&mut self, update: &ClientUpdate) {
        match update {
            ClientUpdate::RecoveryProjection { active, .. } => self.retain_current(active),
            ClientUpdate::Reply {
                command_id,
                op,
                kind,
                envelope,
                ..
            } if op == "operation_status" => {
                let Some(index) = self.pending_index(*command_id) else {
                    return;
                };
                self.tracked[index].state = match kind {
                    ReplyKind::Result => status_result_state(envelope),
                    ReplyKind::PublicError => RecoveryStatusState::ApplicationFailure {
                        code: bounded_message(
                            envelope
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown_application_error"),
                        ),
                    },
                    ReplyKind::MutationAccepted
                    | ReplyKind::MutationCompleted
                    | ReplyKind::MutationFailed => RecoveryStatusState::ApplicationFailure {
                        code: "invalid_operation_status_reply_kind".into(),
                    },
                };
            }
            ClientUpdate::LocalRejected { command_id, reason } => {
                if let Some(index) = self.pending_index(*command_id) {
                    self.tracked[index].state = RecoveryStatusState::LocalFailure {
                        message: bounded_message(reason),
                    };
                }
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
            ) => self.interrupt_pending(),
            _ => {}
        }
    }

    fn state(&self, identity: &MutationIdentity) -> RecoveryStatusState {
        self.tracked
            .iter()
            .find(|tracked| &tracked.identity == identity)
            .map(|tracked| tracked.state.clone())
            .unwrap_or_default()
    }

    fn set_state(
        &mut self,
        identity: MutationIdentity,
        state: RecoveryStatusState,
    ) -> Result<(), RecoveryStatusError> {
        if let Some(existing) = self
            .tracked
            .iter_mut()
            .find(|tracked| tracked.identity == identity)
        {
            existing.state = state;
            return Ok(());
        }
        if self.tracked.len() == MAX_IN_FLIGHT {
            return Err(RecoveryStatusError::TrackerCapacity);
        }
        self.tracked.push(TrackedStatus { identity, state });
        Ok(())
    }

    fn retain_current(&mut self, records: &[RecoveryRecord]) {
        self.tracked.retain(|tracked| {
            records
                .iter()
                .any(|record| record.identity == tracked.identity)
        });
        debug_assert!(self.tracked.len() <= MAX_IN_FLIGHT);
    }

    fn pending_index(&self, command_id: u64) -> Option<usize> {
        self.tracked.iter().position(|tracked| {
            matches!(
                tracked.state,
                RecoveryStatusState::Pending {
                    command_id: pending
                } if pending == command_id
            )
        })
    }

    fn interrupt_pending(&mut self) {
        for tracked in &mut self.tracked {
            if matches!(tracked.state, RecoveryStatusState::Pending { .. }) {
                tracked.state = RecoveryStatusState::Interrupted;
            }
        }
    }

    #[cfg(test)]
    fn tracked_len(&self) -> usize {
        self.tracked.len()
    }
}

fn attachment(model: &WorkbenchModel, record: &RecoveryRecord) -> RecoveryAttachment {
    let Some(hello) = model.hello.as_ref() else {
        return RecoveryAttachment::NoHello;
    };
    if record.boot_id != hello.boot_id {
        RecoveryAttachment::BootMismatch
    } else if record.identity.scope != hello.scope {
        RecoveryAttachment::ScopeMismatch
    } else {
        RecoveryAttachment::Attached
    }
}

fn eligibility(model: &WorkbenchModel, record: &RecoveryRecord) -> StatusEligibility {
    if model.connection != ConnectionState::Ready {
        return StatusEligibility::ClientNotReady;
    }
    let Some(hello) = model.hello.as_ref() else {
        return StatusEligibility::HelloMissing;
    };
    if !hello.operations.iter().any(|op| op == "operation_status") {
        return StatusEligibility::OperationUnavailable;
    }
    if model.recovery_problem.is_some() {
        return StatusEligibility::RecoveryJournalProblem;
    }
    if record.boot_id != hello.boot_id {
        return StatusEligibility::BootMismatch;
    }
    if record.identity.scope != hello.scope {
        return StatusEligibility::ScopeMismatch;
    }
    if !matches!(
        record.admission,
        KnownAdmission::Pending | KnownAdmission::Accepted | KnownAdmission::Ambiguous
    ) {
        return StatusEligibility::TerminalRecord;
    }
    StatusEligibility::Eligible
}

fn status_result_state(envelope: &Value) -> RecoveryStatusState {
    match envelope.pointer("/result/state").and_then(Value::as_str) {
        Some("outcome_unknown") => RecoveryStatusState::OutcomeUnknown,
        Some("accepted" | "completed" | "failed") => RecoveryStatusState::Idle,
        _ => RecoveryStatusState::ApplicationFailure {
            code: "malformed_operation_status_result".into(),
        },
    }
}

fn bounded_message(value: &str) -> String {
    if value.len() <= STATUS_MESSAGE_BYTES {
        return value.to_owned();
    }
    let mut keep = STATUS_MESSAGE_BYTES.saturating_sub(TRUNCATION_MARKER.len());
    while !value.is_char_boundary(keep) {
        keep -= 1;
    }
    let mut bounded = value[..keep].to_owned();
    bounded.push_str(TRUNCATION_MARKER);
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::types::{
            EventCursor, HelloState, QuarantinedRecoveryRecord, RecoveryQuarantineReason,
        },
        presentation::PresentationDocument,
    };
    use serde_json::json;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct FakeSubmitter {
        next: Cell<u64>,
        sent: RefCell<Vec<MutationIdentity>>,
        failure: Cell<Option<CommandSendError>>,
    }

    impl RecoveryStatusSubmitter for FakeSubmitter {
        fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            if let Some(error) = self.failure.take() {
                return Err(error);
            }
            self.sent.borrow_mut().push(identity);
            let id = self.next.get().max(1);
            self.next.set(id + 1);
            Ok(id)
        }
    }

    fn identity(seq: u64) -> MutationIdentity {
        MutationIdentity {
            scope: "scope-a".into(),
            seq,
        }
    }

    fn record(seq: u64, admission: KnownAdmission) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "boot-a".into(),
            identity: identity(seq),
            op: "reference_retune".into(),
            args: json!({"reference":"1","target":2.0,"rate":1.0}),
            admission,
        }
    }

    fn model(records: Vec<RecoveryRecord>) -> WorkbenchModel {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("status-test"));
        model.connection = ConnectionState::Ready;
        model.hello = Some(HelloState {
            boot_id: "boot-a".into(),
            scope: "scope-a".into(),
            next_seq: 9,
            operations: vec!["operation_status".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot-a".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot-a".into(),
                seq: 0,
            },
        });
        model.recovery.mutations = records;
        model
    }

    fn result(command_id: u64, state: &str) -> ClientUpdate {
        ClientUpdate::Reply {
            command_id,
            msg_id: command_id.to_string(),
            op: "operation_status".into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":{"state":state}}),
            recovery: None,
        }
    }

    #[test]
    fn recovery_projection_renders_without_fabricating_command_identity() {
        let model = model(vec![record(1, KnownAdmission::Ambiguous)]);
        let tracker = RecoveryStatusTracker::default();
        let rows = tracker.presentations(&model);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].identity, identity(1));
        assert_eq!(rows[0].op, "reference_retune");
        assert_eq!(rows[0].admission, KnownAdmission::Ambiguous);
        assert_eq!(rows[0].attachment, RecoveryAttachment::Attached);
        assert_eq!(rows[0].status, RecoveryStatusState::Idle);
        assert_eq!(tracker.tracked_len(), 0);
    }

    #[test]
    fn exact_identity_is_sent_once_and_duplicate_is_rejected() {
        let model = model(vec![record(7, KnownAdmission::Pending)]);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        assert_eq!(tracker.check_status(&model, &sender, identity(7)), Ok(1));
        assert_eq!(
            tracker.check_status(&model, &sender, identity(7)),
            Err(RecoveryStatusError::AlreadyPending)
        );
        assert_eq!(&*sender.sent.borrow(), &[identity(7)]);
    }

    #[test]
    fn different_identities_are_bounded_by_recovery_capacity() {
        let records = (1..=MAX_IN_FLIGHT as u64)
            .map(|seq| record(seq, KnownAdmission::Ambiguous))
            .collect::<Vec<_>>();
        let model = model(records);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        for seq in 1..=MAX_IN_FLIGHT as u64 {
            tracker
                .check_status(&model, &sender, identity(seq))
                .unwrap();
        }
        assert_eq!(tracker.tracked_len(), MAX_IN_FLIGHT);
        assert_eq!(sender.sent.borrow().len(), MAX_IN_FLIGHT);
    }

    #[test]
    fn eligibility_rejects_missing_operation_connection_and_identity_mismatch() {
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();

        let mut missing_op = model(vec![record(1, KnownAdmission::Ambiguous)]);
        missing_op.hello.as_mut().unwrap().operations.clear();
        assert_eq!(
            tracker.check_status(&missing_op, &sender, identity(1)),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::OperationUnavailable
            ))
        );

        let mut disconnected = model(vec![record(1, KnownAdmission::Ambiguous)]);
        disconnected.connection = ConnectionState::Stale;
        assert_eq!(
            tracker.check_status(&disconnected, &sender, identity(1)),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::ClientNotReady
            ))
        );

        let mut boot = record(1, KnownAdmission::Ambiguous);
        boot.boot_id = "boot-b".into();
        let boot_model = model(vec![boot]);
        assert_eq!(
            tracker.check_status(&boot_model, &sender, identity(1)),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::BootMismatch
            ))
        );

        let mut scope = record(1, KnownAdmission::Ambiguous);
        scope.identity.scope = "scope-b".into();
        let scope_identity = scope.identity.clone();
        let scope_model = model(vec![scope]);
        assert_eq!(
            tracker.check_status(&scope_model, &sender, scope_identity),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::ScopeMismatch
            ))
        );
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn terminal_and_journal_problem_records_are_ineligible() {
        let sender = FakeSubmitter::default();
        for admission in [KnownAdmission::Completed, KnownAdmission::Failed] {
            let model = model(vec![record(1, admission)]);
            let mut tracker = RecoveryStatusTracker::default();
            assert_eq!(
                tracker.check_status(&model, &sender, identity(1)),
                Err(RecoveryStatusError::Ineligible(
                    StatusEligibility::TerminalRecord
                ))
            );
        }
        let mut blocked = model(vec![record(1, KnownAdmission::Ambiguous)]);
        blocked.recovery_problem = Some("journal unavailable".into());
        let mut tracker = RecoveryStatusTracker::default();
        assert_eq!(
            tracker.check_status(&blocked, &sender, identity(1)),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::RecoveryJournalProblem
            ))
        );
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn quarantined_identity_is_not_a_check_status_candidate() {
        let quarantined = record(1, KnownAdmission::Ambiguous);
        let mut model = model(vec![quarantined.clone()]);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        assert_eq!(tracker.presentations(&model).len(), 1);

        let classified = ClientUpdate::RecoveryProjection {
            active: Vec::new(),
            quarantined: vec![QuarantinedRecoveryRecord {
                record: quarantined.clone(),
                reason: RecoveryQuarantineReason::AttachedBootMismatch,
            }],
        };
        model.apply_client_update(classified.clone());
        tracker.after_update(&classified);

        assert!(tracker.presentations(&model).is_empty());
        assert_eq!(model.recovery.quarantined.len(), 1);
        assert_eq!(
            tracker.check_status(&model, &sender, quarantined.identity),
            Err(RecoveryStatusError::Ineligible(
                StatusEligibility::RecordMissing
            ))
        );
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn status_reply_never_mutates_admission_without_recovery_state() {
        for (wire_state, authoritative) in [
            ("accepted", KnownAdmission::Accepted),
            ("completed", KnownAdmission::Completed),
            ("failed", KnownAdmission::Failed),
        ] {
            let mut model = model(vec![record(1, KnownAdmission::Ambiguous)]);
            let sender = FakeSubmitter::default();
            let mut tracker = RecoveryStatusTracker::default();
            let command_id = tracker.check_status(&model, &sender, identity(1)).unwrap();
            let reply = result(command_id, wire_state);
            model.apply_client_update(reply.clone());
            tracker.after_update(&reply);
            assert_eq!(
                model.recovery.mutations[0].admission,
                KnownAdmission::Ambiguous
            );
            let projection = ClientUpdate::RecoveryProjection {
                active: vec![record(1, authoritative)],
                quarantined: Vec::new(),
            };
            model.apply_client_update(projection.clone());
            tracker.after_update(&projection);
            assert_eq!(model.recovery.mutations[0].admission, authoritative);
            assert_eq!(
                tracker.presentations(&model)[0].status,
                RecoveryStatusState::Idle
            );
        }
    }

    #[test]
    fn outcome_unknown_is_visible_and_admission_is_unchanged() {
        let mut model = model(vec![record(1, KnownAdmission::Ambiguous)]);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        let command_id = tracker.check_status(&model, &sender, identity(1)).unwrap();
        let reply = result(command_id, "outcome_unknown");
        model.apply_client_update(reply.clone());
        tracker.after_update(&reply);
        assert_eq!(
            model.recovery.mutations[0].admission,
            KnownAdmission::Ambiguous
        );
        assert_eq!(
            tracker.presentations(&model)[0].status,
            RecoveryStatusState::OutcomeUnknown
        );
    }

    #[test]
    fn local_rejection_and_transport_loss_clear_pending_without_resubmission() {
        let model = model(vec![record(1, KnownAdmission::Ambiguous)]);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        let command_id = tracker.check_status(&model, &sender, identity(1)).unwrap();
        tracker.after_update(&ClientUpdate::LocalRejected {
            command_id,
            reason: "client_not_ready".into(),
        });
        assert!(matches!(
            tracker.presentations(&model)[0].status,
            RecoveryStatusState::LocalFailure { .. }
        ));

        tracker.check_status(&model, &sender, identity(1)).unwrap();
        tracker.after_update(&ClientUpdate::TransportFailure {
            reason: "lost".into(),
        });
        assert_eq!(
            tracker.presentations(&model)[0].status,
            RecoveryStatusState::Interrupted
        );
        assert_eq!(sender.sent.borrow().len(), 2);
    }

    #[test]
    fn status_cycles_do_not_create_action_or_request_history() {
        let mut model = model(vec![record(1, KnownAdmission::Ambiguous)]);
        let sender = FakeSubmitter::default();
        let mut tracker = RecoveryStatusTracker::default();
        for _ in 0..128 {
            let command_id = tracker.check_status(&model, &sender, identity(1)).unwrap();
            let reply = result(command_id, "outcome_unknown");
            model.apply_client_update(reply.clone());
            tracker.after_update(&reply);
            assert_eq!(tracker.tracked_len(), 1);
            assert!(model.actions.is_empty());
        }
        assert_eq!(model.recovery.mutations.len(), 1);
        assert_eq!(sender.sent.borrow().len(), 128);
    }

    #[test]
    fn failed_local_message_is_utf8_bounded_and_visibly_truncated() {
        let model = model(vec![record(1, KnownAdmission::Ambiguous)]);
        let sender = FakeSubmitter::default();
        sender.failure.set(Some(CommandSendError::Busy));
        let mut tracker = RecoveryStatusTracker::default();
        assert_eq!(
            tracker.check_status(&model, &sender, identity(1)),
            Err(RecoveryStatusError::Submission(CommandSendError::Busy))
        );
        assert!(matches!(
            tracker.presentations(&model)[0].status,
            RecoveryStatusState::LocalFailure { .. }
        ));

        let bounded = bounded_message(&"é".repeat(400));
        assert!(bounded.len() <= STATUS_MESSAGE_BYTES);
        assert!(bounded.ends_with(TRUNCATION_MARKER));
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
    }
}
