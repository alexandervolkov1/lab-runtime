//! Transport-independent Workbench call, presentation, and event dispatcher.
#![allow(
    dead_code,
    missing_docs,
    reason = "M17.2 freezes the private dispatcher surface before the M17.3 adapter"
)]

use crate::{
    client::{
        ClientHandle,
        types::{
            ClientUpdate, CommandSendError, ConnectionState, HelloState, KnownAdmission,
            MutationIdentity, QuarantinedRecoveryRecord, RecoveryQuarantineReason, RecoveryRecord,
            ReplyKind,
        },
    },
    model::{Freshness, UiCommand, UiCommandError, WorkbenchModel, apply_ui_command},
    presentation::{Plot, PresentationDocument, RuntimeRef, Trace},
    rebuild::RebuildCoordinator,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ops::Deref,
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const WORKBENCH_OPERATIONS: [&str; 15] = [
    "hello",
    "client_status",
    "recovery_get",
    "lab_query",
    "lab_mutation",
    "lab_operation_status",
    "presentation_get",
    "ui_add_plot",
    "ui_remove_plot",
    "ui_add_trace",
    "ui_remove_trace",
    "ui_set_trace_visibility",
    "ui_set_time_window",
    "ui_rename_item",
    "ui_set_trace_source",
];

const MAX_DISPATCH_LAB_CALLS: usize = 32;
const WORKBENCH_STRING_BYTES: usize = 512;
const WORKBENCH_NAME_BYTES: usize = 64;
const NOTICE_BYTES: usize = 256;
static WORKBENCH_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallerId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallOrigin {
    pub(crate) caller_id: CallerId,
    pub(crate) call_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PresentationExpectation {
    pub(crate) workbench_id: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryKind {
    Active,
    Quarantined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryExpectation {
    pub(crate) workbench_id: String,
    pub(crate) recovery_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryGetArgs {
    pub(crate) kind: RecoveryKind,
    pub(crate) index: u64,
    pub(crate) expected: Option<RecoveryExpectation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryStatusTarget {
    pub(crate) workbench_id: String,
    pub(crate) recovery_generation: u64,
    pub(crate) boot_id: String,
    pub(crate) request_id: MutationIdentity,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkbenchRequest {
    Hello,
    ClientStatus,
    RecoveryGet(RecoveryGetArgs),
    LabQuery {
        op: String,
        args: Value,
    },
    LabMutation {
        op: String,
        args: Value,
    },
    LabOperationStatus {
        target: RecoveryStatusTarget,
    },
    PresentationGet,
    UiAddPlot {
        expected: PresentationExpectation,
        plot: Plot,
    },
    UiRemovePlot {
        expected: PresentationExpectation,
        plot_id: String,
    },
    UiAddTrace {
        expected: PresentationExpectation,
        plot_id: String,
        trace: Trace,
    },
    UiRemoveTrace {
        expected: PresentationExpectation,
        plot_id: String,
        trace_id: String,
    },
    UiSetTraceVisibility {
        expected: PresentationExpectation,
        plot_id: String,
        trace_id: String,
        visible: bool,
    },
    UiSetTimeWindow {
        expected: PresentationExpectation,
        plot_id: String,
        seconds: f64,
    },
    UiRenameItem {
        expected: PresentationExpectation,
        item_id: String,
        label: String,
    },
    UiSetTraceSource {
        expected: PresentationExpectation,
        plot_id: String,
        trace_id: String,
        source: RuntimeRef,
    },
}

impl WorkbenchRequest {
    pub(crate) fn operation(&self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::ClientStatus => "client_status",
            Self::RecoveryGet(_) => "recovery_get",
            Self::LabQuery { .. } => "lab_query",
            Self::LabMutation { .. } => "lab_mutation",
            Self::LabOperationStatus { .. } => "lab_operation_status",
            Self::PresentationGet => "presentation_get",
            Self::UiAddPlot { .. } => "ui_add_plot",
            Self::UiRemovePlot { .. } => "ui_remove_plot",
            Self::UiAddTrace { .. } => "ui_add_trace",
            Self::UiRemoveTrace { .. } => "ui_remove_trace",
            Self::UiSetTraceVisibility { .. } => "ui_set_trace_visibility",
            Self::UiSetTimeWindow { .. } => "ui_set_time_window",
            Self::UiRenameItem { .. } => "ui_rename_item",
            Self::UiSetTraceSource { .. } => "ui_set_trace_source",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct WorkbenchLimits {
    json_body_bytes: usize,
    frame_bytes: usize,
    json_depth: usize,
    json_values: usize,
    json_string_bytes: usize,
    callers: usize,
    caller_input_messages: usize,
    caller_input_bytes: usize,
    owner_mailbox_messages: usize,
    owner_mailbox_bytes: usize,
    caller_output_messages: usize,
    caller_output_bytes: usize,
    reserved_call_ids_per_caller: usize,
    reserved_call_ids_total: usize,
    outstanding_lab_calls_per_caller: usize,
    outstanding_lab_calls_total: usize,
    client_deadline_ms: usize,
    presentation_document_bytes: usize,
    recovery_active_records: usize,
    recovery_quarantined_records: usize,
}

impl Default for WorkbenchLimits {
    fn default() -> Self {
        Self {
            json_body_bytes: 2_097_152,
            frame_bytes: 2_097_153,
            json_depth: 16,
            json_values: 16_384,
            json_string_bytes: 512,
            callers: 8,
            caller_input_messages: 4,
            caller_input_bytes: 4_194_304,
            owner_mailbox_messages: 32,
            owner_mailbox_bytes: 16_777_216,
            caller_output_messages: 4,
            caller_output_bytes: 4_194_304,
            reserved_call_ids_per_caller: 8,
            reserved_call_ids_total: 32,
            outstanding_lab_calls_per_caller: 8,
            outstanding_lab_calls_total: 32,
            client_deadline_ms: 2_000,
            presentation_document_bytes: 1_048_576,
            recovery_active_records: 8,
            recovery_quarantined_records: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ProtocolIdentity {
    id: &'static str,
    version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RuntimeClientStatus {
    connection: &'static str,
    freshness: &'static str,
    hello: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RecoverySummary {
    recovery_generation: String,
    active_count: String,
    quarantined_count: String,
    reconciliation_required_count: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PresentationStatus {
    presentation_revision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ClientStatusResult {
    workbench_id: String,
    runtime_client: RuntimeClientStatus,
    recovery: RecoverySummary,
    presentation: PresentationStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct HelloResult {
    protocol: ProtocolIdentity,
    workbench_id: String,
    operations: Vec<&'static str>,
    limits: WorkbenchLimits,
    runtime_client: RuntimeClientStatus,
    recovery: RecoverySummary,
    presentation: PresentationStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PresentationResult {
    workbench_id: String,
    presentation_revision: String,
    document: PresentationDocument,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PresentationMutationResult {
    workbench_id: String,
    presentation_revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct LabSubmissionResult {
    state: &'static str,
    command_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RecoveryCounts {
    active: String,
    quarantined: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct MutationIdentityResult {
    scope: String,
    seq: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionResult {
    Pending,
    Accepted,
    Ambiguous,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RecoveryRecordResult {
    boot_id: String,
    request_id: MutationIdentityResult,
    op: String,
    args: Value,
    admission: AdmissionResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QuarantineReasonResult {
    InstanceChanged,
    ScopeUnknown,
    AttachedBootMismatch,
    AttachedScopeMismatch,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct QuarantinedRecoveryRecordResult {
    record: RecoveryRecordResult,
    reason: QuarantineReasonResult,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum RecoveryRecordItem {
    Active(RecoveryRecordResult),
    Quarantined(QuarantinedRecoveryRecordResult),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryGetState {
    Record,
    End,
    Restart,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RecoveryGetResult {
    state: RecoveryGetState,
    workbench_id: String,
    recovery_generation: String,
    counts: RecoveryCounts,
    kind: RecoveryKind,
    index: String,
    record: Option<RecoveryRecordItem>,
    next_index: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum WorkbenchResult {
    Hello(HelloResult),
    ClientStatus(ClientStatusResult),
    RecoveryGet(RecoveryGetResult),
    LabSubmission(LabSubmissionResult),
    Presentation(PresentationResult),
    PresentationMutation(PresentationMutationResult),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkbenchErrorCode {
    InvalidArgs,
    Busy,
    ClientNotReady,
    RecoveryUnavailable,
    RevisionConflict,
    UnknownItem,
    InvalidPresentation,
    WorkerStopped,
    InternalError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkbenchDispatchError {
    pub(crate) code: WorkbenchErrorCode,
    pub(crate) message: &'static str,
}

impl WorkbenchDispatchError {
    fn new(code: WorkbenchErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for WorkbenchDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for WorkbenchDispatchError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EventRoute {
    Caller(CallerId),
    Broadcast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LabUpdateKind {
    Result,
    PublicError,
    MutationAccepted,
    MutationCompleted,
    MutationFailed,
    LocalRejected,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct LabUpdateEvent {
    call_id: String,
    command_id: String,
    op: String,
    kind: LabUpdateKind,
    runtime: Option<Value>,
    local_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ClientStateEvent {
    connection: &'static str,
    freshness: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PresentationChangedEvent {
    presentation_revision: String,
    operation: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RecoveryChangedEvent {
    recovery_generation: String,
    active_count: String,
    quarantined_count: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientNoticeKind {
    ResnapshotRequired,
    ReconciliationRequired,
    RecoveryJournalProblem,
    TransportFailure,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ClientNoticeEvent {
    kind: ClientNoticeKind,
    detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub(crate) enum WorkbenchEvent {
    LabUpdate(LabUpdateEvent),
    ClientState(ClientStateEvent),
    PresentationChanged(PresentationChangedEvent),
    RecoveryChanged(RecoveryChangedEvent),
    ClientNotice(ClientNoticeEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoutedWorkbenchEvent {
    pub(crate) workbench_id: String,
    pub(crate) route: EventRoute,
    pub(crate) event: WorkbenchEvent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DispatchOutcome {
    pub(crate) result: WorkbenchResult,
    pub(crate) events: Vec<RoutedWorkbenchEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingLabCall {
    origin: CallOrigin,
    op: String,
}

pub(crate) trait LabClient {
    fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError>;
    fn mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError>;
    fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError>;
}

impl LabClient for ClientHandle {
    fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        ClientHandle::query(self, op, args)
    }

    fn mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        ClientHandle::mutation(self, op, args)
    }

    fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
        ClientHandle::operation_status(self, identity)
    }
}

/// The single renderer-neutral Workbench model, presentation, and Runtime-client owner.
pub(crate) struct WorkbenchDispatcher<C> {
    model: WorkbenchModel,
    client: Option<C>,
    workbench_id: String,
    presentation_revision: u64,
    recovery_generation: u64,
    pending_lab: BTreeMap<u64, PendingLabCall>,
    boundary_failed: bool,
    last_client_connection: ConnectionState,
    last_client_freshness: Freshness,
}

impl<C> WorkbenchDispatcher<C> {
    pub(crate) fn new(model: WorkbenchModel, client: C) -> Self {
        Self::new_with_id(model, client, fresh_workbench_id())
    }

    fn new_with_id(model: WorkbenchModel, client: C, workbench_id: String) -> Self {
        debug_assert!(valid_workbench_id(&workbench_id));
        let last_client_connection = model.connection;
        let last_client_freshness = model.observations.freshness;
        Self {
            model,
            client: Some(client),
            workbench_id,
            presentation_revision: 1,
            recovery_generation: 1,
            pending_lab: BTreeMap::new(),
            boundary_failed: false,
            last_client_connection,
            last_client_freshness,
        }
    }

    pub(crate) fn client(&self) -> Option<&C> {
        self.client.as_ref()
    }

    pub(crate) fn take_client(&mut self) -> Option<C> {
        self.client.take()
    }

    /// Records one renderer/operator-local diagnostic without exposing mutable model state.
    pub(crate) fn set_client_error(&mut self, error: impl Into<String>) {
        self.model.client_error = Some(error.into());
    }

    /// Retains one existing GUI operator command in the model owner's bounded action table.
    pub(crate) fn track_operator_intent(&mut self, command_id: u64) -> Result<(), &'static str> {
        self.model.track_operator_intent(command_id)
    }

    /// Drops only one future adapter caller's local correlations. Admitted worker
    /// work and recovery evidence are deliberately untouched.
    pub(crate) fn detach_caller(&mut self, caller_id: CallerId) -> usize {
        let before = self.pending_lab.len();
        self.pending_lab
            .retain(|_, pending| pending.origin.caller_id != caller_id);
        before - self.pending_lab.len()
    }

    pub(crate) fn presentation_expectation(&self) -> PresentationExpectation {
        PresentationExpectation {
            workbench_id: self.workbench_id.clone(),
            revision: self.presentation_revision,
        }
    }

    pub(crate) fn presentation_revision(&self) -> u64 {
        self.presentation_revision
    }

    pub(crate) fn recovery_generation(&self) -> u64 {
        self.recovery_generation
    }

    pub(crate) fn apply_gui_ui(
        &mut self,
        command: UiCommand,
    ) -> Result<DispatchOutcome, WorkbenchDispatchError> {
        let expected = self.presentation_expectation();
        let operation = ui_command_operation(&command);
        let (result, event) = self.apply_presentation(expected, command, operation)?;
        Ok(DispatchOutcome {
            result: WorkbenchResult::PresentationMutation(result),
            events: vec![event],
        })
    }

    pub(crate) fn apply_client_update(
        &mut self,
        update: ClientUpdate,
    ) -> Vec<RoutedWorkbenchEvent> {
        if self.boundary_failed {
            return Vec::new();
        }

        let recovery_changed = match &update {
            ClientUpdate::RecoveryProjection {
                active,
                quarantined,
            } => {
                if let Err(reason) =
                    WorkbenchModel::validate_recovery_projection(active, quarantined)
                {
                    self.model.client_error = Some(reason.into());
                    return Vec::new();
                }
                self.model.recovery.mutations != *active
                    || self.model.recovery.quarantined != *quarantined
            }
            _ => false,
        };
        if recovery_changed && self.recovery_generation == u64::MAX {
            self.fail_boundary("Workbench recovery generation exhausted");
            return Vec::new();
        }

        let accepted = self.model.apply_client_update(update.clone());
        if !accepted {
            return Vec::new();
        }

        let mut events = Vec::with_capacity(2);
        if recovery_changed {
            self.recovery_generation += 1;
            events.push(self.recovery_changed_event());
        }
        match &update {
            ClientUpdate::State(_) | ClientUpdate::Hello(_) | ClientUpdate::WorkerStopped => {}
            ClientUpdate::Reply {
                command_id,
                kind,
                envelope,
                ..
            } => {
                if let Some(event) = self.lab_reply_event(*command_id, *kind, envelope.clone()) {
                    events.push(event);
                }
            }
            ClientUpdate::LocalRejected { command_id, reason } => {
                if let Some(event) = self.lab_local_rejected_event(*command_id, reason) {
                    events.push(event);
                }
            }
            ClientUpdate::ReconciliationRequired { records } => {
                events.push(self.client_notice(
                    ClientNoticeKind::ReconciliationRequired,
                    &format!(
                        "{} recovery record(s) require reconciliation",
                        records.len()
                    ),
                ));
            }
            ClientUpdate::ResnapshotRequired { reason, .. } => {
                events.push(self.client_notice(ClientNoticeKind::ResnapshotRequired, reason));
            }
            ClientUpdate::RecoveryJournalProblem { reason } => {
                events.push(self.client_notice(ClientNoticeKind::RecoveryJournalProblem, reason))
            }
            ClientUpdate::TransportFailure { reason } => {
                events.push(self.client_notice(ClientNoticeKind::TransportFailure, reason));
            }
            ClientUpdate::RecoveryProjection { .. }
            | ClientUpdate::Event { .. }
            | ClientUpdate::SubscriptionProgress(_)
            | ClientUpdate::ReferenceBootstrap { .. } => {}
        }
        if let Some(event) = self.client_state_change_event() {
            events.insert(0, event);
        }
        events
    }

    /// Reports a connection/freshness change made by another serialized owner step,
    /// such as completion of the existing GUI rebuild barrier.
    pub(crate) fn finish_model_turn(&mut self) -> Option<RoutedWorkbenchEvent> {
        self.client_state_change_event()
    }

    #[cfg(test)]
    fn complete_rebuild_for_test(&mut self) {
        self.model.complete_rebuild();
    }

    fn fail_boundary(&mut self, message: &'static str) {
        self.boundary_failed = true;
        self.model.client_error = Some(message.to_owned());
        self.pending_lab.clear();
    }

    fn ensure_open(&self) -> Result<(), WorkbenchDispatchError> {
        if self.boundary_failed {
            Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::InternalError,
                "Workbench client boundary is closed",
            ))
        } else {
            Ok(())
        }
    }

    fn status(&self) -> ClientStatusResult {
        ClientStatusResult {
            workbench_id: self.workbench_id.clone(),
            runtime_client: RuntimeClientStatus {
                connection: connection_name(self.model.connection),
                freshness: freshness_name(self.model.observations.freshness),
                hello: self.model.hello.as_ref().map(hello_value),
            },
            recovery: self.recovery_summary(),
            presentation: PresentationStatus {
                presentation_revision: self.presentation_revision.to_string(),
            },
        }
    }

    fn recovery_summary(&self) -> RecoverySummary {
        RecoverySummary {
            recovery_generation: self.recovery_generation.to_string(),
            active_count: self.model.recovery.mutations.len().to_string(),
            quarantined_count: self.model.recovery.quarantined.len().to_string(),
            reconciliation_required_count: self
                .model
                .recovery
                .reconciliation_required
                .len()
                .to_string(),
        }
    }

    fn hello(&self) -> HelloResult {
        let status = self.status();
        HelloResult {
            protocol: ProtocolIdentity {
                id: "lab-runtime.workbench",
                version: 1,
            },
            workbench_id: self.workbench_id.clone(),
            operations: WORKBENCH_OPERATIONS.to_vec(),
            limits: WorkbenchLimits::default(),
            runtime_client: status.runtime_client,
            recovery: status.recovery,
            presentation: status.presentation,
        }
    }

    fn recovery_get(
        &self,
        args: RecoveryGetArgs,
    ) -> Result<RecoveryGetResult, WorkbenchDispatchError> {
        if args.expected.is_none() && args.index != 0 {
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::InvalidArgs,
                "recovery enumeration without an expectation must start at index zero",
            ));
        }
        let counts = self.recovery_counts();
        if let Some(expected) = &args.expected
            && (expected.workbench_id != self.workbench_id
                || expected.recovery_generation != self.recovery_generation)
        {
            return Ok(RecoveryGetResult {
                state: RecoveryGetState::Restart,
                workbench_id: self.workbench_id.clone(),
                recovery_generation: self.recovery_generation.to_string(),
                counts,
                kind: args.kind,
                index: args.index.to_string(),
                record: None,
                next_index: None,
            });
        }

        let index = usize::try_from(args.index).map_err(|_| {
            WorkbenchDispatchError::new(
                WorkbenchErrorCode::InvalidArgs,
                "recovery index exceeds this process address space",
            )
        })?;
        let length = match args.kind {
            RecoveryKind::Active => self.model.recovery.mutations.len(),
            RecoveryKind::Quarantined => self.model.recovery.quarantined.len(),
        };
        if index > length {
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::InvalidArgs,
                "recovery index is beyond the selected projection",
            ));
        }
        let record = match args.kind {
            RecoveryKind::Active => self
                .model
                .recovery
                .mutations
                .get(index)
                .map(|record| RecoveryRecordItem::Active(recovery_record_result(record))),
            RecoveryKind::Quarantined => {
                self.model.recovery.quarantined.get(index).map(|record| {
                    RecoveryRecordItem::Quarantined(quarantined_record_result(record))
                })
            }
        };
        let has_record = record.is_some();
        Ok(RecoveryGetResult {
            state: if has_record {
                RecoveryGetState::Record
            } else {
                RecoveryGetState::End
            },
            workbench_id: self.workbench_id.clone(),
            recovery_generation: self.recovery_generation.to_string(),
            counts,
            kind: args.kind,
            index: args.index.to_string(),
            record,
            next_index: (has_record && index + 1 < length).then(|| (args.index + 1).to_string()),
        })
    }

    fn recovery_counts(&self) -> RecoveryCounts {
        RecoveryCounts {
            active: self.model.recovery.mutations.len().to_string(),
            quarantined: self.model.recovery.quarantined.len().to_string(),
        }
    }

    fn apply_presentation(
        &mut self,
        expected: PresentationExpectation,
        command: UiCommand,
        operation: &'static str,
    ) -> Result<(PresentationMutationResult, RoutedWorkbenchEvent), WorkbenchDispatchError> {
        self.ensure_open()?;
        if expected.workbench_id != self.workbench_id
            || expected.revision != self.presentation_revision
        {
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::RevisionConflict,
                "presentation expectation is stale",
            ));
        }
        let Some(next_revision) = self.presentation_revision.checked_add(1) else {
            self.fail_boundary("Workbench presentation revision exhausted");
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::InternalError,
                "Workbench presentation revision exhausted",
            ));
        };
        apply_ui_command(&mut self.model.presentation, command).map_err(map_ui_error)?;
        self.model.refresh_unresolved();
        self.presentation_revision = next_revision;
        let result = PresentationMutationResult {
            workbench_id: self.workbench_id.clone(),
            presentation_revision: next_revision.to_string(),
        };
        let event = RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Broadcast,
            event: WorkbenchEvent::PresentationChanged(PresentationChangedEvent {
                presentation_revision: next_revision.to_string(),
                operation,
            }),
        };
        Ok((result, event))
    }

    fn submit_lab(
        &mut self,
        origin: CallOrigin,
        op: String,
        submit: impl FnOnce(&C) -> Result<u64, CommandSendError>,
    ) -> Result<DispatchOutcome, WorkbenchDispatchError> {
        self.ensure_open()?;
        if self.model.connection != ConnectionState::Ready || self.model.hello.is_none() {
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::ClientNotReady,
                "Runtime client is not ready",
            ));
        }
        if self.pending_lab.len() == MAX_DISPATCH_LAB_CALLS {
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::Busy,
                "Workbench lab correlation capacity is full",
            ));
        }
        let client = self.client.as_ref().ok_or_else(|| {
            WorkbenchDispatchError::new(
                WorkbenchErrorCode::WorkerStopped,
                "Runtime client worker is unavailable",
            )
        })?;
        let command_id = submit(client).map_err(map_send_error)?;
        if self
            .pending_lab
            .insert(command_id, PendingLabCall { origin, op })
            .is_some()
        {
            self.fail_boundary("Workbench command identity was reused");
            return Err(WorkbenchDispatchError::new(
                WorkbenchErrorCode::InternalError,
                "Workbench command identity was reused",
            ));
        }
        Ok(DispatchOutcome {
            result: WorkbenchResult::LabSubmission(LabSubmissionResult {
                state: "submitted",
                command_id: command_id.to_string(),
            }),
            events: Vec::new(),
        })
    }

    fn submit_status(
        &mut self,
        origin: CallOrigin,
        target: RecoveryStatusTarget,
    ) -> Result<DispatchOutcome, WorkbenchDispatchError>
    where
        C: LabClient,
    {
        self.ensure_open()?;
        let hello = self.model.hello.as_ref().ok_or_else(recovery_unavailable)?;
        if self.model.connection != ConnectionState::Ready
            || self.model.recovery_problem.is_some()
            || target.workbench_id != self.workbench_id
            || target.recovery_generation != self.recovery_generation
            || target.boot_id != hello.boot_id
            || target.request_id.scope != hello.scope
        {
            return Err(recovery_unavailable());
        }
        let Some(record) = self.model.recovery.mutations.iter().find(|record| {
            record.boot_id == target.boot_id && record.identity == target.request_id
        }) else {
            return Err(recovery_unavailable());
        };
        let identity = record.identity.clone();
        self.submit_lab(origin, "operation_status".to_owned(), |client| {
            client.operation_status(identity)
        })
    }

    fn client_state_event(&self) -> RoutedWorkbenchEvent {
        RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Broadcast,
            event: WorkbenchEvent::ClientState(ClientStateEvent {
                connection: connection_name(self.model.connection),
                freshness: freshness_name(self.model.observations.freshness),
            }),
        }
    }

    fn client_state_change_event(&mut self) -> Option<RoutedWorkbenchEvent> {
        let connection = self.model.connection;
        let freshness = self.model.observations.freshness;
        if connection == self.last_client_connection && freshness == self.last_client_freshness {
            return None;
        }
        self.last_client_connection = connection;
        self.last_client_freshness = freshness;
        Some(self.client_state_event())
    }

    fn recovery_changed_event(&self) -> RoutedWorkbenchEvent {
        RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Broadcast,
            event: WorkbenchEvent::RecoveryChanged(RecoveryChangedEvent {
                recovery_generation: self.recovery_generation.to_string(),
                active_count: self.model.recovery.mutations.len().to_string(),
                quarantined_count: self.model.recovery.quarantined.len().to_string(),
            }),
        }
    }

    fn client_notice(&self, kind: ClientNoticeKind, detail: &str) -> RoutedWorkbenchEvent {
        RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Broadcast,
            event: WorkbenchEvent::ClientNotice(ClientNoticeEvent {
                kind,
                detail: bounded_text(detail, NOTICE_BYTES),
            }),
        }
    }

    fn lab_reply_event(
        &mut self,
        command_id: u64,
        kind: ReplyKind,
        envelope: Value,
    ) -> Option<RoutedWorkbenchEvent> {
        let terminal = kind != ReplyKind::MutationAccepted;
        let pending = if terminal {
            self.pending_lab.remove(&command_id)
        } else {
            self.pending_lab.get(&command_id).cloned()
        }?;
        Some(RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Caller(pending.origin.caller_id),
            event: WorkbenchEvent::LabUpdate(LabUpdateEvent {
                call_id: pending.origin.call_id,
                command_id: command_id.to_string(),
                op: pending.op,
                kind: lab_update_kind(kind),
                runtime: Some(envelope),
                local_reason: None,
            }),
        })
    }

    fn lab_local_rejected_event(
        &mut self,
        command_id: u64,
        reason: &str,
    ) -> Option<RoutedWorkbenchEvent> {
        let pending = self.pending_lab.remove(&command_id)?;
        Some(RoutedWorkbenchEvent {
            workbench_id: self.workbench_id.clone(),
            route: EventRoute::Caller(pending.origin.caller_id),
            event: WorkbenchEvent::LabUpdate(LabUpdateEvent {
                call_id: pending.origin.call_id,
                command_id: command_id.to_string(),
                op: pending.op,
                kind: LabUpdateKind::LocalRejected,
                runtime: None,
                local_reason: Some(bounded_text(reason, NOTICE_BYTES)),
            }),
        })
    }

    #[cfg(test)]
    fn set_presentation_revision_for_test(&mut self, revision: u64) {
        self.presentation_revision = revision;
    }

    #[cfg(test)]
    fn set_recovery_generation_for_test(&mut self, generation: u64) {
        self.recovery_generation = generation;
    }
}

impl WorkbenchDispatcher<ClientHandle> {
    /// Advances the accepted bounded rebuild coordinator inside the single model owner.
    pub(crate) fn advance_rebuild(
        &mut self,
        rebuild: &mut RebuildCoordinator,
        update: &ClientUpdate,
    ) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        rebuild.after_update(update, &mut self.model, client);
    }
}

impl<C: LabClient> WorkbenchDispatcher<C> {
    pub(crate) fn dispatch(
        &mut self,
        origin: CallOrigin,
        request: WorkbenchRequest,
    ) -> Result<DispatchOutcome, WorkbenchDispatchError> {
        self.ensure_open()?;
        match request {
            WorkbenchRequest::Hello => Ok(local_outcome(WorkbenchResult::Hello(self.hello()))),
            WorkbenchRequest::ClientStatus => {
                Ok(local_outcome(WorkbenchResult::ClientStatus(self.status())))
            }
            WorkbenchRequest::RecoveryGet(args) => Ok(local_outcome(WorkbenchResult::RecoveryGet(
                self.recovery_get(args)?,
            ))),
            WorkbenchRequest::LabQuery { op, args } => {
                validate_lab_args(&op, &args, false)?;
                let submitted_op = op.clone();
                self.submit_lab(origin, op, |client| client.query(&submitted_op, args))
            }
            WorkbenchRequest::LabMutation { op, args } => {
                validate_lab_args(&op, &args, true)?;
                let submitted_op = op.clone();
                self.submit_lab(origin, op, |client| client.mutation(&submitted_op, args))
            }
            WorkbenchRequest::LabOperationStatus { target } => self.submit_status(origin, target),
            WorkbenchRequest::PresentationGet => Ok(local_outcome(WorkbenchResult::Presentation(
                PresentationResult {
                    workbench_id: self.workbench_id.clone(),
                    presentation_revision: self.presentation_revision.to_string(),
                    document: self.model.presentation.clone(),
                },
            ))),
            WorkbenchRequest::UiAddPlot { expected, plot } => {
                self.ui_outcome(expected, UiCommand::AddPlot { plot }, "ui_add_plot")
            }
            WorkbenchRequest::UiRemovePlot { expected, plot_id } => self.ui_outcome(
                expected,
                UiCommand::RemovePlot { plot_id },
                "ui_remove_plot",
            ),
            WorkbenchRequest::UiAddTrace {
                expected,
                plot_id,
                trace,
            } => self.ui_outcome(
                expected,
                UiCommand::AddTrace { plot_id, trace },
                "ui_add_trace",
            ),
            WorkbenchRequest::UiRemoveTrace {
                expected,
                plot_id,
                trace_id,
            } => self.ui_outcome(
                expected,
                UiCommand::RemoveTrace { plot_id, trace_id },
                "ui_remove_trace",
            ),
            WorkbenchRequest::UiSetTraceVisibility {
                expected,
                plot_id,
                trace_id,
                visible,
            } => self.ui_outcome(
                expected,
                UiCommand::SetTraceVisibility {
                    plot_id,
                    trace_id,
                    visible,
                },
                "ui_set_trace_visibility",
            ),
            WorkbenchRequest::UiSetTimeWindow {
                expected,
                plot_id,
                seconds,
            } => self.ui_outcome(
                expected,
                UiCommand::SetTimeWindow { plot_id, seconds },
                "ui_set_time_window",
            ),
            WorkbenchRequest::UiRenameItem {
                expected,
                item_id,
                label,
            } => self.ui_outcome(
                expected,
                UiCommand::RenamePresentationItem { item_id, label },
                "ui_rename_item",
            ),
            WorkbenchRequest::UiSetTraceSource {
                expected,
                plot_id,
                trace_id,
                source,
            } => self.ui_outcome(
                expected,
                UiCommand::SetTraceSource {
                    plot_id,
                    trace_id,
                    source,
                },
                "ui_set_trace_source",
            ),
        }
    }

    fn ui_outcome(
        &mut self,
        expected: PresentationExpectation,
        command: UiCommand,
        operation: &'static str,
    ) -> Result<DispatchOutcome, WorkbenchDispatchError> {
        let (result, event) = self.apply_presentation(expected, command, operation)?;
        Ok(DispatchOutcome {
            result: WorkbenchResult::PresentationMutation(result),
            events: vec![event],
        })
    }
}

impl<C> Deref for WorkbenchDispatcher<C> {
    type Target = WorkbenchModel;

    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

fn local_outcome(result: WorkbenchResult) -> DispatchOutcome {
    DispatchOutcome {
        result,
        events: Vec::new(),
    }
}

fn validate_lab_args(op: &str, args: &Value, mutation: bool) -> Result<(), WorkbenchDispatchError> {
    if op.is_empty() || op.len() > WORKBENCH_NAME_BYTES || !args.is_object() {
        return Err(WorkbenchDispatchError::new(
            WorkbenchErrorCode::InvalidArgs,
            "lab operation and arguments are invalid",
        ));
    }
    if mutation && args.get("request_id").is_some() {
        return Err(WorkbenchDispatchError::new(
            WorkbenchErrorCode::InvalidArgs,
            "Runtime mutation identity is worker-owned",
        ));
    }
    Ok(())
}

fn map_ui_error(error: UiCommandError) -> WorkbenchDispatchError {
    match error {
        UiCommandError::UnknownItem(_) => WorkbenchDispatchError::new(
            WorkbenchErrorCode::UnknownItem,
            "presentation target does not exist",
        ),
        UiCommandError::InvalidCandidate(_) => WorkbenchDispatchError::new(
            WorkbenchErrorCode::InvalidPresentation,
            "presentation candidate is invalid",
        ),
    }
}

fn map_send_error(error: CommandSendError) -> WorkbenchDispatchError {
    match error {
        CommandSendError::Busy => WorkbenchDispatchError::new(
            WorkbenchErrorCode::Busy,
            "Runtime client command admission is full",
        ),
        CommandSendError::WorkerStopped => WorkbenchDispatchError::new(
            WorkbenchErrorCode::WorkerStopped,
            "Runtime client worker has stopped",
        ),
        CommandSendError::IdExhausted => WorkbenchDispatchError::new(
            WorkbenchErrorCode::InternalError,
            "Workbench command identity exhausted",
        ),
    }
}

fn recovery_unavailable() -> WorkbenchDispatchError {
    WorkbenchDispatchError::new(
        WorkbenchErrorCode::RecoveryUnavailable,
        "recovery target is not active in the current Runtime session",
    )
}

fn connection_name(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::AwaitingHello => "awaiting_hello",
        ConnectionState::Reattaching => "reattaching",
        ConnectionState::Ready => "ready",
        ConnectionState::Stale => "stale",
        ConnectionState::Stopping => "stopping",
        ConnectionState::Stopped => "stopped",
    }
}

fn freshness_name(state: Freshness) -> &'static str {
    match state {
        Freshness::Unknown => "unknown",
        Freshness::Rebuilding => "rebuilding",
        Freshness::Fresh => "fresh",
        Freshness::Stale => "stale",
    }
}

fn hello_value(hello: &HelloState) -> Value {
    json!({
        "boot_id": hello.boot_id,
        "scope": hello.scope,
        "next_seq": hello.next_seq.to_string(),
        "operations": hello.operations,
        "capabilities": hello.capabilities,
        "limits": hello.limits,
        "event_oldest": hello.event_oldest.to_json(),
        "event_latest": hello.event_latest.to_json(),
    })
}

fn recovery_record_result(record: &RecoveryRecord) -> RecoveryRecordResult {
    RecoveryRecordResult {
        boot_id: record.boot_id.clone(),
        request_id: MutationIdentityResult {
            scope: record.identity.scope.clone(),
            seq: record.identity.seq.to_string(),
        },
        op: record.op.clone(),
        args: record.args.clone(),
        admission: match record.admission {
            KnownAdmission::Pending => AdmissionResult::Pending,
            KnownAdmission::Accepted => AdmissionResult::Accepted,
            KnownAdmission::Ambiguous => AdmissionResult::Ambiguous,
            KnownAdmission::Completed => AdmissionResult::Completed,
            KnownAdmission::Failed => AdmissionResult::Failed,
        },
    }
}

fn quarantined_record_result(
    record: &QuarantinedRecoveryRecord,
) -> QuarantinedRecoveryRecordResult {
    QuarantinedRecoveryRecordResult {
        record: recovery_record_result(&record.record),
        reason: match record.reason {
            RecoveryQuarantineReason::InstanceChanged => QuarantineReasonResult::InstanceChanged,
            RecoveryQuarantineReason::ScopeUnknown => QuarantineReasonResult::ScopeUnknown,
            RecoveryQuarantineReason::AttachedBootMismatch => {
                QuarantineReasonResult::AttachedBootMismatch
            }
            RecoveryQuarantineReason::AttachedScopeMismatch => {
                QuarantineReasonResult::AttachedScopeMismatch
            }
        },
    }
}

fn lab_update_kind(kind: ReplyKind) -> LabUpdateKind {
    match kind {
        ReplyKind::Result => LabUpdateKind::Result,
        ReplyKind::PublicError => LabUpdateKind::PublicError,
        ReplyKind::MutationAccepted => LabUpdateKind::MutationAccepted,
        ReplyKind::MutationCompleted => LabUpdateKind::MutationCompleted,
        ReplyKind::MutationFailed => LabUpdateKind::MutationFailed,
    }
}

fn ui_command_operation(command: &UiCommand) -> &'static str {
    match command {
        UiCommand::AddPlot { .. } => "ui_add_plot",
        UiCommand::RemovePlot { .. } => "ui_remove_plot",
        UiCommand::AddTrace { .. } => "ui_add_trace",
        UiCommand::RemoveTrace { .. } => "ui_remove_trace",
        UiCommand::SetTraceVisibility { .. } => "ui_set_trace_visibility",
        UiCommand::SetTimeWindow { .. } => "ui_set_time_window",
        UiCommand::RenamePresentationItem { .. } => "ui_rename_item",
        UiCommand::SetTraceSource { .. } => "ui_set_trace_source",
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn fresh_workbench_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = u128::from(WORKBENCH_ID_COUNTER.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(process::id());
    format!("{:032x}", time ^ (process << 64) ^ counter)
}

fn valid_workbench_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.len() <= WORKBENCH_STRING_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::types::EventCursor,
        presentation::{AxisOptions, TraceStyle},
    };
    use std::cell::{Cell, RefCell};

    #[derive(Clone, Debug, PartialEq)]
    enum Submitted {
        Query(String, Value),
        Mutation(String, Value),
        Status(MutationIdentity),
    }

    #[derive(Default)]
    struct FakeClient {
        next: Cell<u64>,
        submitted: RefCell<Vec<Submitted>>,
    }

    impl FakeClient {
        fn command_id(&self) -> u64 {
            let command_id = self.next.get().max(1);
            self.next.set(command_id + 1);
            command_id
        }
    }

    impl LabClient for FakeClient {
        fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
            self.submitted
                .borrow_mut()
                .push(Submitted::Query(op.to_owned(), args));
            Ok(self.command_id())
        }

        fn mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
            self.submitted
                .borrow_mut()
                .push(Submitted::Mutation(op.to_owned(), args));
            Ok(self.command_id())
        }

        fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.submitted
                .borrow_mut()
                .push(Submitted::Status(identity));
            Ok(self.command_id())
        }
    }

    fn dispatcher() -> WorkbenchDispatcher<FakeClient> {
        WorkbenchDispatcher::new_with_id(
            WorkbenchModel::new(PresentationDocument::empty("main")),
            FakeClient::default(),
            "0123456789abcdef0123456789abcdef".to_owned(),
        )
    }

    fn origin(call_id: &str) -> CallOrigin {
        CallOrigin {
            caller_id: CallerId(7),
            call_id: call_id.to_owned(),
        }
    }

    fn hello() -> HelloState {
        HelloState {
            boot_id: "boot".to_owned(),
            scope: "scope".to_owned(),
            next_seq: 2,
            operations: vec!["reference".to_owned(), "reference_retune".to_owned()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot".to_owned(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot".to_owned(),
                seq: 1,
            },
        }
    }

    fn ready(dispatcher: &mut WorkbenchDispatcher<FakeClient>) {
        dispatcher.apply_client_update(ClientUpdate::Hello(hello()));
    }

    fn plot() -> Plot {
        Plot {
            id: "plot".to_owned(),
            title: "Plot".to_owned(),
            time_window_seconds: 60.0,
            axes: AxisOptions::default(),
            traces: Vec::new(),
        }
    }

    fn trace() -> Trace {
        Trace {
            id: "trace".to_owned(),
            source: RuntimeRef::Reference {
                reference: "1".to_owned(),
            },
            display_label: "Trace".to_owned(),
            visible: true,
            style: TraceStyle {
                color: "white".to_owned(),
                width: 1.0,
            },
            display_unit: None,
        }
    }

    fn record(seq: u64) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "boot".to_owned(),
            identity: MutationIdentity {
                scope: "scope".to_owned(),
                seq,
            },
            op: "reference_retune".to_owned(),
            args: json!({"reference":"1","target":2.0,"rate":1.0}),
            admission: KnownAdmission::Ambiguous,
        }
    }

    #[test]
    fn dispatcher_exposes_exactly_the_frozen_operations() {
        let requests = vec![
            WorkbenchRequest::Hello,
            WorkbenchRequest::ClientStatus,
            WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                kind: RecoveryKind::Active,
                index: 0,
                expected: None,
            }),
            WorkbenchRequest::LabQuery {
                op: "reference".to_owned(),
                args: json!({}),
            },
            WorkbenchRequest::LabMutation {
                op: "reference_retune".to_owned(),
                args: json!({}),
            },
            WorkbenchRequest::LabOperationStatus {
                target: RecoveryStatusTarget {
                    workbench_id: "w".to_owned(),
                    recovery_generation: 1,
                    boot_id: "b".to_owned(),
                    request_id: MutationIdentity {
                        scope: "s".to_owned(),
                        seq: 1,
                    },
                },
            },
            WorkbenchRequest::PresentationGet,
            WorkbenchRequest::UiAddPlot {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot: plot(),
            },
            WorkbenchRequest::UiRemovePlot {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
            },
            WorkbenchRequest::UiAddTrace {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
                trace: trace(),
            },
            WorkbenchRequest::UiRemoveTrace {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
                trace_id: "t".to_owned(),
            },
            WorkbenchRequest::UiSetTraceVisibility {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
                trace_id: "t".to_owned(),
                visible: false,
            },
            WorkbenchRequest::UiSetTimeWindow {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
                seconds: 1.0,
            },
            WorkbenchRequest::UiRenameItem {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                item_id: "p".to_owned(),
                label: "x".to_owned(),
            },
            WorkbenchRequest::UiSetTraceSource {
                expected: PresentationExpectation {
                    workbench_id: "w".to_owned(),
                    revision: 1,
                },
                plot_id: "p".to_owned(),
                trace_id: "t".to_owned(),
                source: RuntimeRef::Recorder,
            },
        ];
        assert_eq!(
            requests
                .iter()
                .map(WorkbenchRequest::operation)
                .collect::<Vec<_>>(),
            WORKBENCH_OPERATIONS
        );
    }

    #[test]
    fn local_projection_results_use_the_frozen_counter_and_dto_shapes() {
        let mut dispatcher = dispatcher();
        ready(&mut dispatcher);

        let hello = dispatcher
            .dispatch(origin("hello"), WorkbenchRequest::Hello)
            .unwrap();
        let hello = serde_json::to_value(hello.result).unwrap();
        assert_eq!(
            hello["protocol"],
            json!({"id":"lab-runtime.workbench","version":1})
        );
        assert_eq!(hello["operations"], json!(WORKBENCH_OPERATIONS));
        assert_eq!(hello["runtime_client"]["hello"]["next_seq"], "2");
        assert_eq!(hello["recovery"]["recovery_generation"], "1");
        assert_eq!(hello["presentation"]["presentation_revision"], "1");
        assert_eq!(hello["limits"]["owner_mailbox_messages"], 32);

        let status = dispatcher
            .dispatch(origin("status"), WorkbenchRequest::ClientStatus)
            .unwrap();
        let status = serde_json::to_value(status.result).unwrap();
        assert_eq!(status["runtime_client"]["connection"], "ready");
        assert_eq!(status["runtime_client"]["freshness"], "rebuilding");
        assert_eq!(status["recovery"]["active_count"], "0");

        let presentation = dispatcher
            .dispatch(origin("presentation"), WorkbenchRequest::PresentationGet)
            .unwrap();
        let presentation = serde_json::to_value(presentation.result).unwrap();
        assert_eq!(presentation["presentation_revision"], "1");
        assert_eq!(
            presentation["document"],
            json!({"format_version":1,"document_id":"main","windows":[],"plots":[],"controls":[]})
        );
    }

    #[test]
    fn typed_event_seam_routes_only_current_process_notifications() {
        let mut dispatcher = dispatcher();
        let state =
            dispatcher.apply_client_update(ClientUpdate::State(ConnectionState::Connecting));
        assert_eq!(state.len(), 1);
        assert_eq!(state[0].route, EventRoute::Broadcast);
        assert!(matches!(state[0].event, WorkbenchEvent::ClientState(_)));

        let notices = dispatcher.apply_client_update(ClientUpdate::ResnapshotRequired {
            reason: "gap".to_owned(),
            envelope: None,
            connection_lost: false,
        });
        assert_eq!(notices.len(), 1);
        assert!(matches!(
            notices[0].event,
            WorkbenchEvent::ClientNotice(ClientNoticeEvent {
                kind: ClientNoticeKind::ResnapshotRequired,
                ..
            })
        ));
        dispatcher.complete_rebuild_for_test();
        let rebuilt = dispatcher
            .finish_model_turn()
            .expect("freshness change produces one client-state event");
        assert!(matches!(
            rebuilt.event,
            WorkbenchEvent::ClientState(ClientStateEvent {
                freshness: "fresh",
                ..
            })
        ));
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());
    }

    #[test]
    fn all_ui_variants_share_revision_owner_and_advance_once() {
        let mut dispatcher = dispatcher();
        let requests = [
            WorkbenchRequest::UiAddPlot {
                expected: dispatcher.presentation_expectation(),
                plot: plot(),
            },
            WorkbenchRequest::UiAddTrace {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 2,
                },
                plot_id: "plot".to_owned(),
                trace: trace(),
            },
            WorkbenchRequest::UiSetTraceVisibility {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 3,
                },
                plot_id: "plot".to_owned(),
                trace_id: "trace".to_owned(),
                visible: false,
            },
            WorkbenchRequest::UiSetTimeWindow {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 4,
                },
                plot_id: "plot".to_owned(),
                seconds: 120.0,
            },
            WorkbenchRequest::UiRenameItem {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 5,
                },
                item_id: "trace".to_owned(),
                label: "Renamed".to_owned(),
            },
            WorkbenchRequest::UiSetTraceSource {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 6,
                },
                plot_id: "plot".to_owned(),
                trace_id: "trace".to_owned(),
                source: RuntimeRef::Recorder,
            },
            WorkbenchRequest::UiRemoveTrace {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 7,
                },
                plot_id: "plot".to_owned(),
                trace_id: "trace".to_owned(),
            },
            WorkbenchRequest::UiRemovePlot {
                expected: PresentationExpectation {
                    workbench_id: dispatcher.workbench_id.clone(),
                    revision: 8,
                },
                plot_id: "plot".to_owned(),
            },
        ];
        for (index, request) in requests.into_iter().enumerate() {
            let outcome = dispatcher.dispatch(origin("ui"), request).unwrap();
            assert_eq!(outcome.events.len(), 1);
            assert_eq!(dispatcher.presentation_revision(), index as u64 + 2);
        }
    }

    #[test]
    fn revision_precheck_invalid_candidate_and_overflow_fail_closed() {
        let mut dispatcher = dispatcher();
        let original = dispatcher.presentation.clone();
        let conflict = dispatcher.dispatch(
            origin("conflict"),
            WorkbenchRequest::UiAddPlot {
                expected: PresentationExpectation {
                    workbench_id: "wrong".to_owned(),
                    revision: 1,
                },
                plot: Plot {
                    time_window_seconds: f64::NAN,
                    ..plot()
                },
            },
        );
        assert_eq!(
            conflict.unwrap_err().code,
            WorkbenchErrorCode::RevisionConflict
        );
        assert_eq!(dispatcher.presentation, original);
        assert_eq!(dispatcher.presentation_revision(), 1);

        let invalid = dispatcher.dispatch(
            origin("invalid"),
            WorkbenchRequest::UiAddPlot {
                expected: dispatcher.presentation_expectation(),
                plot: Plot {
                    time_window_seconds: f64::NAN,
                    ..plot()
                },
            },
        );
        assert_eq!(
            invalid.unwrap_err().code,
            WorkbenchErrorCode::InvalidPresentation
        );
        assert_eq!(dispatcher.presentation, original);
        assert_eq!(dispatcher.presentation_revision(), 1);

        dispatcher.set_presentation_revision_for_test(u64::MAX);
        let overflow = dispatcher.apply_gui_ui(UiCommand::AddPlot { plot: plot() });
        assert_eq!(
            overflow.unwrap_err().code,
            WorkbenchErrorCode::InternalError
        );
        assert_eq!(dispatcher.presentation, original);
        assert!(dispatcher.boundary_failed);
    }

    #[test]
    fn gui_and_typed_calls_mutate_the_same_presentation_owner() {
        let mut dispatcher = dispatcher();
        let gui = dispatcher
            .apply_gui_ui(UiCommand::AddPlot { plot: plot() })
            .unwrap();
        assert!(matches!(
            gui.events[0].event,
            WorkbenchEvent::PresentationChanged(_)
        ));
        assert_eq!(dispatcher.presentation_revision(), 2);
        let outcome = dispatcher
            .dispatch(
                origin("external"),
                WorkbenchRequest::UiRenameItem {
                    expected: dispatcher.presentation_expectation(),
                    item_id: "plot".to_owned(),
                    label: "External".to_owned(),
                },
            )
            .unwrap();
        assert!(matches!(
            outcome.result,
            WorkbenchResult::PresentationMutation(_)
        ));
        assert_eq!(dispatcher.presentation.plots[0].title, "External");
        assert_eq!(dispatcher.presentation_revision(), 3);
    }

    #[test]
    fn lab_calls_use_only_the_existing_client_paths_and_preserve_correlation() {
        let mut dispatcher = dispatcher();
        ready(&mut dispatcher);
        let query = dispatcher
            .dispatch(
                origin("query-call"),
                WorkbenchRequest::LabQuery {
                    op: "reference".to_owned(),
                    args: json!({"reference":"1"}),
                },
            )
            .unwrap();
        let mutation = dispatcher
            .dispatch(
                origin("mutation-call"),
                WorkbenchRequest::LabMutation {
                    op: "reference_retune".to_owned(),
                    args: json!({"reference":"1","target":2.0,"rate":1.0}),
                },
            )
            .unwrap();
        assert!(matches!(query.result, WorkbenchResult::LabSubmission(_)));
        assert!(matches!(mutation.result, WorkbenchResult::LabSubmission(_)));
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            &[
                Submitted::Query("reference".to_owned(), json!({"reference":"1"})),
                Submitted::Mutation(
                    "reference_retune".to_owned(),
                    json!({"reference":"1","target":2.0,"rate":1.0})
                ),
            ]
        );

        let events = dispatcher.apply_client_update(ClientUpdate::Reply {
            command_id: 2,
            msg_id: "runtime-msg".to_owned(),
            op: "reference_retune".to_owned(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"type":"operation_completed","request_id":{"scope":"scope","seq":"9"}}),
            recovery: None,
        });
        let WorkbenchEvent::LabUpdate(update) = &events[0].event else {
            panic!("expected lab update");
        };
        assert_eq!(events[0].route, EventRoute::Caller(CallerId(7)));
        assert_eq!(update.call_id, "mutation-call");
        assert_eq!(
            update.runtime,
            Some(json!({"type":"operation_completed","request_id":{"scope":"scope","seq":"9"}}))
        );

        let caller_identity = dispatcher.dispatch(
            origin("bad"),
            WorkbenchRequest::LabMutation {
                op: "reference_retune".to_owned(),
                args: json!({"request_id":{"scope":"caller","seq":"99"}}),
            },
        );
        assert_eq!(
            caller_identity.unwrap_err().code,
            WorkbenchErrorCode::InvalidArgs
        );
    }

    #[test]
    fn connection_loss_never_synthesizes_local_rejection_or_worker_work() {
        let mut dispatcher = dispatcher();
        ready(&mut dispatcher);
        dispatcher
            .dispatch(
                origin("pending-mutation"),
                WorkbenchRequest::LabMutation {
                    op: "reference_retune".to_owned(),
                    args: json!({"reference":"1","target":2.0,"rate":1.0}),
                },
            )
            .unwrap();
        let submitted = dispatcher.client().unwrap().submitted.borrow().clone();

        let continuity = dispatcher.apply_client_update(ClientUpdate::ResnapshotRequired {
            reason: "Runtime connection lost".to_owned(),
            envelope: None,
            connection_lost: true,
        });

        assert!(continuity.iter().any(|event| matches!(
            event.event,
            WorkbenchEvent::ClientNotice(ClientNoticeEvent {
                kind: ClientNoticeKind::ResnapshotRequired,
                ..
            })
        )));
        assert!(
            continuity
                .iter()
                .any(|event| matches!(event.event, WorkbenchEvent::ClientState(_)))
        );
        assert!(
            !continuity
                .iter()
                .any(|event| matches!(event.event, WorkbenchEvent::LabUpdate(_)))
        );
        assert_eq!(dispatcher.pending_lab.len(), 1);
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            submitted.as_slice()
        );

        let rejected = dispatcher.apply_client_update(ClientUpdate::LocalRejected {
            command_id: 1,
            reason: "worker rejected before admission".to_owned(),
        });
        assert_eq!(dispatcher.pending_lab.len(), 0);
        assert!(rejected.iter().any(|event| matches!(
            &event.event,
            WorkbenchEvent::LabUpdate(LabUpdateEvent {
                call_id,
                kind: LabUpdateKind::LocalRejected,
                runtime: None,
                ..
            }) if call_id == "pending-mutation"
        )));
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            submitted.as_slice()
        );
    }

    #[test]
    fn caller_detach_discards_only_local_correlation_and_never_cancels_work() {
        let mut dispatcher = dispatcher();
        ready(&mut dispatcher);
        dispatcher
            .dispatch(
                origin("lost-caller"),
                WorkbenchRequest::LabMutation {
                    op: "reference_retune".to_owned(),
                    args: json!({"reference":"1","target":2.0,"rate":1.0}),
                },
            )
            .unwrap();
        let submitted = dispatcher.client().unwrap().submitted.borrow().clone();
        assert_eq!(dispatcher.detach_caller(CallerId(7)), 1);
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            submitted.as_slice()
        );
        assert!(
            dispatcher
                .apply_client_update(ClientUpdate::Reply {
                    command_id: 1,
                    msg_id: "runtime-msg".to_owned(),
                    op: "reference_retune".to_owned(),
                    kind: ReplyKind::MutationAccepted,
                    envelope: json!({"type":"operation_accepted"}),
                    recovery: Some(record(1)),
                })
                .is_empty()
        );
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().as_slice(),
            submitted.as_slice()
        );
    }

    #[test]
    fn status_accepts_only_exact_current_active_recovery_identity() {
        let mut dispatcher = dispatcher();
        ready(&mut dispatcher);
        dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record(4)],
            quarantined: vec![QuarantinedRecoveryRecord {
                record: record(5),
                reason: RecoveryQuarantineReason::InstanceChanged,
            }],
        });
        let target = RecoveryStatusTarget {
            workbench_id: dispatcher.workbench_id.clone(),
            recovery_generation: dispatcher.recovery_generation(),
            boot_id: "boot".to_owned(),
            request_id: MutationIdentity {
                scope: "scope".to_owned(),
                seq: 4,
            },
        };
        dispatcher
            .dispatch(
                origin("status"),
                WorkbenchRequest::LabOperationStatus {
                    target: target.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            dispatcher.client().unwrap().submitted.borrow().last(),
            Some(&Submitted::Status(target.request_id.clone()))
        );

        let cases = [
            RecoveryStatusTarget {
                recovery_generation: target.recovery_generation - 1,
                ..target.clone()
            },
            RecoveryStatusTarget {
                request_id: MutationIdentity {
                    scope: "scope".to_owned(),
                    seq: 5,
                },
                ..target.clone()
            },
            RecoveryStatusTarget {
                boot_id: "old-boot".to_owned(),
                ..target.clone()
            },
            RecoveryStatusTarget {
                workbench_id: "fedcba9876543210fedcba9876543210".to_owned(),
                ..target
            },
        ];
        for target in cases {
            let error = dispatcher
                .dispatch(
                    origin("rejected"),
                    WorkbenchRequest::LabOperationStatus { target },
                )
                .unwrap_err();
            assert_eq!(error.code, WorkbenchErrorCode::RecoveryUnavailable);
        }

        dispatcher.apply_client_update(ClientUpdate::State(ConnectionState::Stale));
        let stale = dispatcher
            .dispatch(
                origin("stale"),
                WorkbenchRequest::LabOperationStatus {
                    target: RecoveryStatusTarget {
                        workbench_id: dispatcher.workbench_id.clone(),
                        recovery_generation: dispatcher.recovery_generation(),
                        boot_id: "boot".to_owned(),
                        request_id: MutationIdentity {
                            scope: "scope".to_owned(),
                            seq: 4,
                        },
                    },
                },
            )
            .unwrap_err();
        assert_eq!(stale.code, WorkbenchErrorCode::RecoveryUnavailable);
    }

    #[test]
    fn recovery_generation_and_enumeration_are_snapshot_safe_and_passive() {
        let mut dispatcher = dispatcher();
        assert_eq!(dispatcher.recovery_generation(), 1);
        let changed = ClientUpdate::RecoveryProjection {
            active: vec![record(1), record(2)],
            quarantined: Vec::new(),
        };
        let events = dispatcher.apply_client_update(changed.clone());
        assert_eq!(dispatcher.recovery_generation(), 2);
        assert!(matches!(
            events[0].event,
            WorkbenchEvent::RecoveryChanged(_)
        ));
        assert!(dispatcher.apply_client_update(changed).is_empty());
        assert_eq!(dispatcher.recovery_generation(), 2);

        let first = dispatcher
            .dispatch(
                origin("first"),
                WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                    kind: RecoveryKind::Active,
                    index: 0,
                    expected: None,
                }),
            )
            .unwrap();
        let WorkbenchResult::RecoveryGet(first) = first.result else {
            panic!("expected recovery result");
        };
        assert_eq!(first.state, RecoveryGetState::Record);
        assert_eq!(first.next_index.as_deref(), Some("1"));
        let expected = RecoveryExpectation {
            workbench_id: first.workbench_id.clone(),
            recovery_generation: 2,
        };
        let second = dispatcher
            .dispatch(
                origin("second"),
                WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                    kind: RecoveryKind::Active,
                    index: 1,
                    expected: Some(expected.clone()),
                }),
            )
            .unwrap();
        let WorkbenchResult::RecoveryGet(second) = second.result else {
            panic!("expected recovery result");
        };
        assert_eq!(second.state, RecoveryGetState::Record);
        assert_eq!(second.next_index, None);
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());

        dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record(2)],
            quarantined: Vec::new(),
        });
        let restart = dispatcher
            .dispatch(
                origin("restart"),
                WorkbenchRequest::RecoveryGet(RecoveryGetArgs {
                    kind: RecoveryKind::Active,
                    index: 1,
                    expected: Some(expected),
                }),
            )
            .unwrap();
        let WorkbenchResult::RecoveryGet(restart) = restart.result else {
            panic!("expected recovery result");
        };
        assert_eq!(restart.state, RecoveryGetState::Restart);
        assert!(restart.record.is_none());
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());
    }

    #[test]
    fn recovery_generation_max_identical_projection_remains_open() {
        let mut dispatcher = dispatcher();
        dispatcher.set_recovery_generation_for_test(u64::MAX);
        let events = dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: Vec::new(),
            quarantined: Vec::new(),
        });
        assert!(events.is_empty());
        assert!(!dispatcher.boundary_failed);
        assert_eq!(dispatcher.recovery_generation(), u64::MAX);
        assert!(dispatcher.recovery.mutations.is_empty());
    }

    #[test]
    fn recovery_generation_max_rejected_projection_remains_open_and_unpublished() {
        let mut dispatcher = dispatcher();
        dispatcher.set_recovery_generation_for_test(u64::MAX);
        let duplicate = record(1);
        let events = dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![duplicate.clone(), duplicate],
            quarantined: Vec::new(),
        });
        assert!(events.is_empty());
        assert!(!dispatcher.boundary_failed);
        assert_eq!(dispatcher.recovery_generation(), u64::MAX);
        assert!(dispatcher.recovery.mutations.is_empty());
        assert_eq!(
            dispatcher.client_error.as_deref(),
            Some("worker recovery projection contained duplicates")
        );
    }

    #[test]
    fn recovery_generation_max_valid_change_closes_without_publication() {
        let mut dispatcher = dispatcher();
        dispatcher.set_recovery_generation_for_test(u64::MAX);
        dispatcher.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record(1)],
            quarantined: Vec::new(),
        });
        assert!(dispatcher.boundary_failed);
        assert_eq!(dispatcher.recovery_generation(), u64::MAX);
        assert!(dispatcher.recovery.mutations.is_empty());
        let error = dispatcher
            .dispatch(origin("closed"), WorkbenchRequest::ClientStatus)
            .unwrap_err();
        assert_eq!(error.code, WorkbenchErrorCode::InternalError);
        assert!(dispatcher.client().unwrap().submitted.borrow().is_empty());
    }
}
