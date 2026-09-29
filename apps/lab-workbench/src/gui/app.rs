//! eframe main-thread owner and thin observational rendering.

use super::rebuild::RebuildCoordinator;
use crate::{
    client::ClientHandle,
    client::types::{ConnectionState, KnownAdmission, RecoveryQuarantineReason},
    model::{
        ControllerLifecycleIntent, EXACT_RETRY_WARNING, ExactRetryState, ExactRetryWorkflow,
        Freshness, OperatorIntent, OperatorWarning, OperatorWorkflow, OperatorWorkflowState,
        PROPERTY_TEXT_BYTES, PidCandidate, PreparedExactRetry, PropertyMutationCandidate,
        RECORDING_LABEL_BYTES, RecoveryAttachment, RecoveryRecordPresentation, RecoveryStatusState,
        RecoveryStatusTracker, StatusEligibility, UiCommand, WorkbenchModel,
    },
    ownership::WorkspaceOwnership,
    presentation::{
        AxisOptions, Plot as PresentationPlot, PresentationDocument, RuntimeRef, Trace, TraceStyle,
    },
};
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot as EguiPlot, PlotPoints};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};

pub(crate) const UPDATES_PER_FRAME: usize = 64;
const SMOKE_DEADLINE: Duration = Duration::from_secs(30);

pub(crate) struct WorkbenchApp {
    model: WorkbenchModel,
    client: Option<ClientHandle>,
    rebuild: RebuildCoordinator,
    operator: OperatorWorkflow,
    recovery_status: RecoveryStatusTracker,
    exact_retry: ExactRetryWorkflow,
    selected: Option<RuntimeRef>,
    editor_target: Option<RuntimeRef>,
    controller_editor_revision: Option<Value>,
    fixed_value: String,
    ramp_target: String,
    ramp_rate: String,
    pid_fields: [String; 5],
    property_text: String,
    property_bool: bool,
    recording_label: String,
    operator_problem: Option<String>,
    presentation_problem: Option<String>,
    _ownership: WorkspaceOwnership,
    smoke: Option<SmokeRun>,
    kill_probe: Option<KillProbe>,
}

impl WorkbenchApp {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        address: SocketAddr,
        desired_scope: Option<String>,
        journal_path: PathBuf,
        presentation: PresentationDocument,
        presentation_problem: Option<String>,
        ownership: WorkspaceOwnership,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let repaint = context.egui_ctx.clone();
        let wake = Arc::new(move || repaint.request_repaint());
        let client = ClientHandle::spawn_with_recovery_journal_and_wake(
            address,
            Some(journal_path),
            Some(wake),
        )?;
        client.connect(desired_scope)?;
        let smoke = std::env::var_os("LAB_WORKBENCH_GUI_SMOKE_RESULT")
            .map(PathBuf::from)
            .map(SmokeRun::new);
        let kill_probe = std::env::var_os("LAB_WORKBENCH_GUI_KILL_READY")
            .map(PathBuf::from)
            .map(KillProbe::new);
        Ok(Self {
            model: WorkbenchModel::new(presentation),
            client: Some(client),
            rebuild: RebuildCoordinator::default(),
            operator: OperatorWorkflow::default(),
            recovery_status: RecoveryStatusTracker::default(),
            exact_retry: ExactRetryWorkflow::default(),
            selected: None,
            editor_target: None,
            controller_editor_revision: None,
            fixed_value: String::new(),
            ramp_target: String::new(),
            ramp_rate: String::new(),
            pid_fields: Default::default(),
            property_text: String::new(),
            property_bool: false,
            recording_label: "Workbench run".into(),
            operator_problem: None,
            presentation_problem,
            _ownership: ownership,
            smoke,
            kill_probe,
        })
    }

    fn drain_updates(&mut self) -> usize {
        let Some(client) = self.client.as_ref() else {
            return 0;
        };
        let mut updates = Vec::with_capacity(UPDATES_PER_FRAME);
        let count = drain_bounded(
            || client.try_recv(),
            |update| updates.push(update),
            UPDATES_PER_FRAME,
        );
        for update in updates {
            self.model.apply_client_update(update.clone());
            self.recovery_status.after_update(&update);
            self.exact_retry
                .after_update(&update, &self.model, &self.recovery_status);
            self.operator.after_update(&update);
            self.rebuild.after_update(&update, &mut self.model, client);
        }
        self.ensure_selection_and_default_plot();
        count
    }

    fn ensure_selection_and_default_plot(&mut self) {
        if self.selected.is_none() {
            self.selected = self
                .model
                .observations
                .entities
                .keys()
                .find(|identity| matches!(identity, RuntimeRef::Signal { .. }))
                .cloned();
        }
        let Some(signal) = self.selected.clone() else {
            return;
        };
        if self.model.presentation.plots.is_empty() {
            let plot = PresentationPlot {
                id: "default-live-plot".into(),
                title: "Live signal".into(),
                time_window_seconds: 60.0,
                axes: AxisOptions::default(),
                traces: vec![Trace {
                    id: "default-live-trace".into(),
                    source: signal,
                    display_label: "Selected signal".into(),
                    visible: true,
                    style: TraceStyle {
                        color: "cyan".into(),
                        width: 1.5,
                    },
                    display_unit: None,
                }],
            };
            if let Err(error) = self.model.apply_ui_command(UiCommand::AddPlot { plot }) {
                self.model.client_error = Some(error.to_string());
            }
        }
    }

    fn connection_controls(&mut self, ui: &mut egui::Ui) {
        let connected = matches!(
            self.model.connection,
            ConnectionState::Connecting
                | ConnectionState::AwaitingHello
                | ConnectionState::Reattaching
                | ConnectionState::Ready
        );
        let quarantined = !self.model.recovery.quarantined.is_empty();
        let connect_label = if quarantined {
            "Connect new scope"
        } else {
            "Connect"
        };
        if ui
            .add_enabled(!connected, egui::Button::new(connect_label))
            .clicked()
            && let Some(client) = self.client.as_ref()
        {
            let scope = (!quarantined)
                .then(|| self.model.recovery.scope.clone())
                .flatten();
            if let Err(error) = client.connect(scope) {
                self.model.client_error = Some(error.to_string());
            }
        }
        if ui
            .add_enabled(connected, egui::Button::new("Disconnect"))
            .clicked()
            && let Some(client) = self.client.as_ref()
            && let Err(error) = client.disconnect()
        {
            self.model.client_error = Some(error.to_string());
        }
    }

    fn render_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Connection:");
            ui.label(format!("{:?}", self.model.connection));
            ui.separator();
            freshness_label(ui, self.model.observations.freshness);
            if let Some(hello) = &self.model.hello {
                ui.separator();
                ui.label(format!("boot {}", hello.boot_id));
                ui.label(format!("scope {}", hello.scope));
            }
            ui.separator();
            self.connection_controls(ui);
        });
        if let Some(problem) = &self.presentation_problem {
            ui.colored_label(Color32::YELLOW, problem);
        }
        if let Some(error) = &self.model.client_error {
            ui.colored_label(Color32::YELLOW, error);
        }
        let (records, unresolved) = recovery_summary(&self.model);
        if records > 0 {
            ui.colored_label(
                Color32::YELLOW,
                format!("Recovery records: {records}; unresolved: {unresolved}"),
            );
        }
        if !self.model.recovery.reconciliation_required.is_empty() {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!(
                    "Reconciliation required for {} mutation(s)",
                    self.model.recovery.reconciliation_required.len()
                ),
            );
        }
        if self.model.quarantine_blocks_mutations() {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!(
                    "Quarantined recovery evidence: {}; new mutations are blocked",
                    self.model.recovery.quarantined.len()
                ),
            );
        }
        self.render_recovery(ui);
    }

    fn render_recovery(&mut self, ui: &mut egui::Ui) {
        let records = self.recovery_status.presentations(&self.model);
        egui::CollapsingHeader::new("Recovery / reconciliation")
            .default_open(
                !records.is_empty()
                    || !self.model.recovery.quarantined.is_empty()
                    || self.model.recovery_problem.is_some(),
            )
            .show(ui, |ui| {
                if let Some(problem) = &self.model.recovery_problem {
                    ui.colored_label(Color32::LIGHT_RED, problem);
                }
                if records.is_empty() && self.model.recovery.quarantined.is_empty() {
                    ui.label("No retained recovery records");
                }
                for record in records {
                    self.render_recovery_record(ui, record);
                    ui.separator();
                }
                if !self.model.recovery.quarantined.is_empty() {
                    ui.strong("Quarantined recovery evidence");
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "Not attached to the current Runtime session. Status/retry unavailable. New experiment mutations remain blocked.",
                    );
                    for entry in &self.model.recovery.quarantined {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(format!(
                                "boot={} scope={} seq={}",
                                entry.record.boot_id,
                                entry.record.identity.scope,
                                entry.record.identity.seq
                            ));
                            ui.label(format!("op={}", entry.record.op));
                            ui.label(format!("admission={:?}", entry.record.admission));
                            ui.label(format!(
                                "reason={}",
                                quarantine_reason_text(entry.reason)
                            ));
                        });
                    }
                }
                self.render_exact_retry(ui);
            });
    }

    fn render_recovery_record(&mut self, ui: &mut egui::Ui, record: RecoveryRecordPresentation) {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "scope={} seq={}",
                record.identity.scope, record.identity.seq
            ));
            ui.label(format!("op={}", record.op));
            ui.label(format!("admission={:?}", record.admission));
            ui.label(recovery_attachment_text(record.attachment));
        });
        match &record.status {
            RecoveryStatusState::Idle => {}
            RecoveryStatusState::Pending { .. } => {
                ui.colored_label(Color32::LIGHT_BLUE, "Check Status pending");
            }
            RecoveryStatusState::OutcomeUnknown => {
                ui.colored_label(
                    Color32::YELLOW,
                    "Runtime reported outcome_unknown; admission remains unresolved",
                );
            }
            RecoveryStatusState::LocalFailure { message } => {
                ui.colored_label(Color32::YELLOW, format!("Local status failure: {message}"));
            }
            RecoveryStatusState::ApplicationFailure { code } => {
                ui.colored_label(Color32::YELLOW, format!("Application status error: {code}"));
            }
            RecoveryStatusState::Interrupted => {
                ui.colored_label(
                    Color32::YELLOW,
                    "Status request interrupted; it was not resubmitted",
                );
            }
        }
        let availability = recovery_action_availability(
            &self.model,
            &self.recovery_status,
            &self.exact_retry,
            &self.operator,
            &record,
        );
        let pending = matches!(record.status, RecoveryStatusState::Pending { .. });
        let response = ui.add_enabled(availability.check_status, egui::Button::new("Check Status"));
        if response.clicked()
            && let Some(client) = self.client.as_ref()
            && let Err(error) =
                self.recovery_status
                    .check_status(&self.model, client, record.identity.clone())
        {
            self.model.client_error = Some(error.to_string());
        }
        if !availability.check_status && !pending {
            ui.small(format!(
                "Check Status unavailable: {}",
                status_eligibility_text(record.eligibility)
            ));
        }
        if ui
            .add_enabled(availability.exact_retry, egui::Button::new("Exact Retry…"))
            .clicked()
            && let Err(error) =
                self.exact_retry
                    .begin(&self.model, &self.recovery_status, record.identity)
        {
            self.model.client_error = Some(error.to_string());
        }
    }

    fn render_exact_retry(&mut self, ui: &mut egui::Ui) {
        let state = self.exact_retry.state.clone();
        let Some(message) = exact_retry_state_message(&state) else {
            return;
        };
        ui.separator();
        ui.strong("Exact Retry");
        match &state {
            ExactRetryState::AwaitingConfirmation(prepared) => {
                let view = exact_retry_confirmation_view(prepared);
                ui.colored_label(Color32::YELLOW, message);
                ui.monospace(format!(
                    "scope={} seq={} operation={} admission={:?}",
                    view.scope, view.seq, view.operation, view.admission
                ));
                ui.label("Exact worker-retained arguments (read-only):");
                ui.monospace(view.args);
                ui.colored_label(Color32::YELLOW, view.warning);
                let current = self
                    .exact_retry
                    .confirmation_is_current(&self.model, &self.recovery_status);
                if !current {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "Recovery evidence/session changed before confirmation. Review the current record again.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(current, egui::Button::new("Confirm Exact Retry"))
                        .clicked()
                        && let Some(client) = self.client.as_ref()
                    {
                        let _ =
                            self.exact_retry
                                .confirm(&self.model, &self.recovery_status, client);
                    }
                    if ui.button("Cancel").clicked() {
                        self.exact_retry.cancel_or_acknowledge();
                    }
                });
            }
            ExactRetryState::Idle => {}
            ExactRetryState::Submitted { .. } | ExactRetryState::Accepted { .. } => {
                ui.colored_label(Color32::LIGHT_BLUE, message);
            }
            ExactRetryState::Completed { .. } => {
                ui.colored_label(Color32::LIGHT_GREEN, message);
                ui.label(
                    "Displayed device and experiment state remains authoritative through normal Runtime observations.",
                );
                if ui.button("Acknowledge").clicked() {
                    self.exact_retry.cancel_or_acknowledge();
                }
            }
            ExactRetryState::Failed { .. } => {
                ui.colored_label(Color32::LIGHT_RED, message);
                ui.label("This operation outcome is not a physical-safety claim.");
                if ui.button("Acknowledge").clicked() {
                    self.exact_retry.cancel_or_acknowledge();
                }
            }
            ExactRetryState::OutcomeUnknown { .. }
            | ExactRetryState::Interrupted { .. }
            | ExactRetryState::DraftStale { .. } => {
                ui.colored_label(Color32::YELLOW, message);
                if ui.button("Acknowledge").clicked() {
                    self.exact_retry.cancel_or_acknowledge();
                }
            }
            ExactRetryState::LocalFailure { reason, .. } => {
                ui.colored_label(Color32::YELLOW, message);
                ui.monospace(reason);
                if ui.button("Acknowledge").clicked() {
                    self.exact_retry.cancel_or_acknowledge();
                }
            }
            ExactRetryState::ApplicationFailure { code, .. } => {
                ui.colored_label(Color32::YELLOW, message);
                ui.monospace(code);
                if ui.button("Acknowledge").clicked() {
                    self.exact_retry.cancel_or_acknowledge();
                }
            }
        }
    }

    fn render_discovery(&mut self, ui: &mut egui::Ui) {
        ui.heading("Discovery");
        let identities = self
            .model
            .observations
            .entities
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for identity in identities {
                let label = runtime_ref_label(&identity);
                let selected = self.selected.as_ref() == Some(&identity);
                if ui.selectable_label(selected, label).clicked() {
                    self.selected = Some(identity);
                }
            }
        });
    }

    fn render_main(&mut self, ui: &mut egui::Ui) {
        let Some(selected) = self.selected.clone() else {
            ui.heading("Live signal");
            ui.label("No discovered signal is available.");
            self.render_recorder_controls(ui);
            return;
        };
        self.ensure_editor_for(&selected);
        ui.heading(runtime_ref_label(&selected));
        let unresolved = self.model.unresolved.contains(&selected);
        let observation = self.model.observations.entities.get(&selected).cloned();
        match (unresolved, observation.as_ref()) {
            (true, _) => {
                ui.colored_label(Color32::LIGHT_RED, "Unresolved in the current Runtime view");
            }
            (false, Some(observation)) if observation.freshness == Freshness::Fresh => {
                ui.colored_label(Color32::LIGHT_GREEN, "Fresh observation");
            }
            (_, Some(_)) => {
                ui.colored_label(Color32::YELLOW, "Cached last observation (stale)");
            }
            _ => {
                ui.colored_label(Color32::GRAY, "Unknown");
            }
        }
        if let Some(observation) = &observation {
            ui.monospace(bounded_value_text(&observation.value));
        }
        if let Some(buffer) = self.model.observations.live.get(&selected) {
            let points = buffer
                .points()
                .iter()
                .map(|point| [point.time_seconds, point.value])
                .collect::<Vec<_>>();
            EguiPlot::new("selected-live-signal")
                .height(360.0)
                .show(ui, |plot_ui| {
                    plot_ui.line(Line::new("observed", PlotPoints::from(points)));
                });
            if buffer.dropped() > 0 {
                ui.colored_label(
                    Color32::YELLOW,
                    format!("Display window truncated {} old point(s)", buffer.dropped()),
                );
            }
        } else if matches!(selected, RuntimeRef::Signal { .. }) {
            ui.label("Waiting for good finite signal events.");
        }
        ui.separator();
        self.render_operator_workflow(ui);
        if matches!(self.operator.state, OperatorWorkflowState::Idle) {
            self.render_target_controls(
                ui,
                &selected,
                observation.as_ref().map(|value| &value.value),
            );
            self.render_recorder_controls(ui);
        }
    }

    fn ensure_editor_for(&mut self, selected: &RuntimeRef) {
        let value = self
            .model
            .observations
            .entities
            .get(selected)
            .map(|observation| &observation.value);
        let controller_snapshot = match selected {
            RuntimeRef::Controller { controller } => {
                controller_pid_editor_snapshot(&self.model, controller)
            }
            _ => None,
        };
        let controller_revision = controller_snapshot
            .as_ref()
            .map(|(revision, _)| revision.clone());
        let controller_changed = matches!(selected, RuntimeRef::Controller { .. })
            && controller_revision != self.controller_editor_revision;
        if self.editor_target.as_ref() == Some(selected) && !controller_changed {
            return;
        }
        self.editor_target = Some(selected.clone());
        self.controller_editor_revision = controller_revision;
        if matches!(selected, RuntimeRef::Reference { .. }) {
            self.fixed_value = numeric_text(value.and_then(|item| item.get("value")));
            self.ramp_target = numeric_text(
                value
                    .and_then(|item| item.get("target"))
                    .or_else(|| value.and_then(|item| item.get("value"))),
            );
            self.ramp_rate = numeric_text(value.and_then(|item| item.get("rate")));
            if self.ramp_rate.is_empty() {
                self.ramp_rate = "1".into();
            }
        }
        if matches!(selected, RuntimeRef::Controller { .. })
            && let Some((_, fields)) = controller_snapshot
        {
            self.pid_fields = fields;
        }
        if matches!(selected, RuntimeRef::ConfigurationProperty { .. }) {
            let current = value.and_then(|item| item.get("current"));
            self.property_text = scalar_text(current);
            self.property_bool = current.and_then(Value::as_bool).unwrap_or(false);
        }
    }

    fn render_operator_workflow(&mut self, ui: &mut egui::Ui) {
        ui.heading("Operator action");
        if let Some(problem) = &self.operator_problem {
            ui.colored_label(Color32::LIGHT_RED, problem);
        }
        let state = self.operator.state.clone();
        let confirmation_current = awaiting_confirmation_current(&state, &self.model);
        match state {
            OperatorWorkflowState::Idle => {
                ui.label("No operator mutation is active.");
            }
            OperatorWorkflowState::AwaitingConfirmation(prepared) => {
                ui.colored_label(Color32::YELLOW, "Confirmation required");
                ui.label(&prepared.confirmation);
                ui.monospace(format!("{} {}", prepared.operation, prepared.args));
                if let Some(warning) = &prepared.warning {
                    render_operator_warning(ui, warning);
                }
                ui.label("Confirm expresses intent only; it is not evidence of completion.");
                let current = confirmation_current.unwrap_or(false);
                if !current {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "Draft stale / authoritative state changed. Cancel and review again.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(current, egui::Button::new("Confirm"))
                        .clicked()
                        && let Some(client) = self.client.as_ref()
                    {
                        match self.operator.confirm(&self.model, client) {
                            Ok(command_id) => {
                                self.operator_problem = self
                                    .model
                                    .track_operator_intent(command_id)
                                    .err()
                                    .map(str::to_owned);
                            }
                            Err(error) => self.operator_problem = Some(error.to_string()),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.operator.cancel_or_acknowledge();
                        self.operator_problem = None;
                    }
                });
            }
            OperatorWorkflowState::Submitted { prepared, .. } => {
                ui.colored_label(Color32::LIGHT_BLUE, "Submitted; admission not yet known");
                ui.label(prepared.confirmation);
            }
            OperatorWorkflowState::Accepted { prepared, .. } => {
                ui.colored_label(Color32::LIGHT_BLUE, "Accepted / in progress");
                ui.label(prepared.confirmation);
                ui.label("Displayed Runtime state remains authoritative and may not yet change.");
            }
            OperatorWorkflowState::Completed { prepared, envelope } => {
                ui.colored_label(Color32::LIGHT_GREEN, "Operation completed");
                ui.label(prepared.confirmation);
                ui.label("Await authoritative projection/event for displayed Runtime state.");
                ui.monospace(bounded_value_text(&envelope));
                if ui.button("Acknowledge").clicked() {
                    self.operator.cancel_or_acknowledge();
                }
            }
            OperatorWorkflowState::Failed {
                prepared,
                envelope,
                conflict,
            } => {
                ui.colored_label(Color32::LIGHT_RED, "Operation failed");
                ui.label(prepared.confirmation);
                if conflict {
                    ui.label("Revision/generation conflict: reload and review a fresh draft.");
                }
                ui.monospace(bounded_value_text(&envelope));
                if ui.button("Acknowledge").clicked() {
                    self.operator.cancel_or_acknowledge();
                }
            }
            OperatorWorkflowState::Ambiguous { prepared, identity } => {
                ui.colored_label(Color32::LIGHT_RED, "Unknown / reconciliation required");
                ui.label(prepared.confirmation);
                if let Some(identity) = identity {
                    ui.label(format!(
                        "retained request {}:{}",
                        identity.scope, identity.seq
                    ));
                }
                ui.label("No automatic retry is performed; the worker-owned payload is immutable.");
            }
            OperatorWorkflowState::ReconciledTerminal {
                prepared,
                identity,
                admission,
            } => {
                ui.colored_label(
                    if admission == KnownAdmission::Completed {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::LIGHT_RED
                    },
                    reconciled_operator_message(&admission),
                );
                ui.label(prepared.confirmation);
                ui.label(format!(
                    "retained request {}:{}",
                    identity.scope, identity.seq
                ));
                ui.label("Resolved from authoritative worker recovery evidence.");
                if ui.button("Acknowledge").clicked() {
                    self.operator.cancel_or_acknowledge();
                }
            }
        }
    }

    fn render_target_controls(
        &mut self,
        ui: &mut egui::Ui,
        selected: &RuntimeRef,
        observation: Option<&Value>,
    ) {
        match selected {
            RuntimeRef::Reference { reference } => {
                ui.heading("Reference controls");
                ui.horizontal(|ui| {
                    ui.label("Fixed value");
                    ui.text_edit_singleline(&mut self.fixed_value);
                    truncate_utf8(&mut self.fixed_value, 64);
                    let enabled = self.control_enabled(selected, "reference_configure");
                    if ui
                        .add_enabled(enabled, egui::Button::new("Configure fixed"))
                        .clicked()
                    {
                        match parse_finite(&self.fixed_value) {
                            Ok(value) => {
                                self.begin_operator(OperatorIntent::ConfigureReferenceFixed {
                                    reference: reference.clone(),
                                    value,
                                })
                            }
                            Err(error) => self.operator_problem = Some(error.into()),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Target");
                    ui.text_edit_singleline(&mut self.ramp_target);
                    truncate_utf8(&mut self.ramp_target, 64);
                    ui.label("Rate");
                    ui.text_edit_singleline(&mut self.ramp_rate);
                    truncate_utf8(&mut self.ramp_rate, 64);
                    let configure = self.control_enabled(selected, "reference_configure");
                    if ui
                        .add_enabled(configure, egui::Button::new("Configure ramp"))
                        .clicked()
                    {
                        self.begin_reference_ramp(reference, false);
                    }
                    let retune = observation
                        .and_then(|value| value.get("kind"))
                        .and_then(Value::as_str)
                        == Some("ramp")
                        && self.control_enabled(selected, "reference_retune");
                    if ui
                        .add_enabled(retune, egui::Button::new("Retune"))
                        .clicked()
                    {
                        self.begin_reference_ramp(reference, true);
                    }
                });
            }
            RuntimeRef::Controller { controller } => {
                ui.heading("Controller controls");
                let state = observation
                    .and_then(|value| value.get("state"))
                    .and_then(Value::as_str);
                ui.horizontal(|ui| {
                    for (label, action, required_state, operation) in [
                        (
                            "Start",
                            ControllerLifecycleIntent::Start,
                            "ready",
                            "controller_start",
                        ),
                        (
                            "Pause",
                            ControllerLifecycleIntent::Pause,
                            "running",
                            "controller_pause",
                        ),
                        (
                            "Resume",
                            ControllerLifecycleIntent::Resume,
                            "paused",
                            "controller_resume",
                        ),
                        (
                            "Reset failed",
                            ControllerLifecycleIntent::ResetFailed,
                            "failed",
                            "controller_reset_failed",
                        ),
                    ] {
                        let state_ok = if action == ControllerLifecycleIntent::Pause {
                            matches!(state, Some("running" | "warming"))
                        } else {
                            state == Some(required_state)
                        };
                        if ui
                            .add_enabled(
                                state_ok && self.control_enabled(selected, operation),
                                egui::Button::new(label),
                            )
                            .clicked()
                        {
                            self.begin_operator(OperatorIntent::ControllerLifecycle {
                                controller: controller.clone(),
                                action,
                            });
                        }
                    }
                });
                ui.label("PID (kp, ki, kd, output min, output max)");
                ui.horizontal(|ui| {
                    for field in &mut self.pid_fields {
                        ui.add(egui::TextEdit::singleline(field).desired_width(72.0));
                        truncate_utf8(field, 64);
                    }
                    if ui
                        .add_enabled(
                            self.model.controller_detail_is_fresh(controller)
                                && self.control_enabled(selected, "controller_configure_pid"),
                            egui::Button::new("Configure PID"),
                        )
                        .clicked()
                    {
                        let parsed = self.pid_fields.each_ref().map(|value| parse_finite(value));
                        match parsed {
                            [Ok(kp), Ok(ki), Ok(kd), Ok(output_min), Ok(output_max)] => {
                                self.begin_operator(OperatorIntent::ConfigureControllerPid {
                                    controller: controller.clone(),
                                    pid: PidCandidate {
                                        kp,
                                        ki,
                                        kd,
                                        output_min,
                                        output_max,
                                    },
                                });
                            }
                            _ => {
                                self.operator_problem =
                                    Some("PID fields require finite numbers".into())
                            }
                        }
                    }
                });
            }
            RuntimeRef::ConfigurationProperty { .. } => {
                ui.heading("Configuration property");
                let value_type = observation
                    .and_then(|value| value.get("value_type"))
                    .and_then(Value::as_str);
                let access = observation
                    .and_then(|value| value.get("access"))
                    .and_then(Value::as_str);
                ui.label(format!(
                    "type: {}; access: {}",
                    value_type.unwrap_or("unknown"),
                    access.unwrap_or("unknown")
                ));
                match value_type {
                    Some("boolean") => {
                        ui.add_enabled(
                            false,
                            egui::Checkbox::new(&mut self.property_bool, "current value"),
                        );
                    }
                    _ => {
                        ui.text_edit_singleline(&mut self.property_text);
                        truncate_utf8(&mut self.property_text, PROPERTY_TEXT_BYTES);
                    }
                }
                let mutation_type = matches!(value_type, Some("integer" | "text"));
                let enabled = access == Some("read_write")
                    && mutation_type
                    && self.control_enabled(selected, "property_configure");
                if ui
                    .add_enabled(enabled, egui::Button::new("Configure property"))
                    .clicked()
                {
                    match property_candidate(value_type, &self.property_text) {
                        Ok(value) => self.begin_operator(OperatorIntent::ConfigureProperty {
                            target: selected.clone(),
                            value,
                        }),
                        Err(error) => self.operator_problem = Some(error.into()),
                    }
                }
            }
            RuntimeRef::Resource { resource } => {
                ui.heading("Resource control");
                let capable = observation
                    .and_then(|value| value.pointer("/capabilities/reconnect"))
                    == Some(&Value::Bool(true));
                if ui
                    .add_enabled(
                        resource_reconnect_enabled(
                            &self.model,
                            resource,
                            capable,
                            self.control_enabled(selected, "reconnect_resource"),
                        ),
                        egui::Button::new("Reconnect"),
                    )
                    .clicked()
                {
                    self.begin_operator(OperatorIntent::ReconnectResource {
                        resource: resource.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    fn render_recorder_controls(&mut self, ui: &mut egui::Ui) {
        let target = RuntimeRef::Recorder;
        let Some(observation) = self.model.observations.entities.get(&target).cloned() else {
            return;
        };
        ui.separator();
        ui.heading("Recorder");
        ui.label(format!(
            "authoritative state: {}",
            observation
                .value
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.recording_label);
            truncate_utf8(&mut self.recording_label, RECORDING_LABEL_BYTES);
            let idle = observation.value.get("state").and_then(Value::as_str) == Some("idle");
            if ui
                .add_enabled(
                    idle && self.control_enabled(&target, "recording_start"),
                    egui::Button::new("Start recording"),
                )
                .clicked()
            {
                self.begin_operator(OperatorIntent::StartRecording {
                    label: self.recording_label.clone(),
                });
            }
            let has_run = observation
                .value
                .get("active_run")
                .is_some_and(|run| !run.is_null());
            if ui
                .add_enabled(
                    has_run && self.control_enabled(&target, "recording_stop"),
                    egui::Button::new("Stop recording"),
                )
                .clicked()
            {
                self.begin_operator(OperatorIntent::StopRecording);
            }
        });
    }

    fn control_enabled(&self, target: &RuntimeRef, operation: &str) -> bool {
        self.model.connection == ConnectionState::Ready
            && self.model.observations.freshness == Freshness::Fresh
            && self
                .model
                .observations
                .entities
                .get(target)
                .is_some_and(|observation| observation.freshness == Freshness::Fresh)
            && self
                .model
                .hello
                .as_ref()
                .is_some_and(|hello| hello.operations.iter().any(|item| item == operation))
            && self.model.recovery_problem.is_none()
            && !self.model.quarantine_blocks_mutations()
            && self.model.recovery.reconciliation_required.is_empty()
            && !self.model.recovery.mutations.iter().any(|record| {
                matches!(
                    record.admission,
                    KnownAdmission::Pending | KnownAdmission::Accepted | KnownAdmission::Ambiguous
                )
            })
            && matches!(self.operator.state, OperatorWorkflowState::Idle)
    }

    fn begin_operator(&mut self, intent: OperatorIntent) {
        match self.operator.begin(&self.model, intent) {
            Ok(()) => self.operator_problem = None,
            Err(error) => self.operator_problem = Some(error.to_string()),
        }
    }

    fn begin_reference_ramp(&mut self, reference: &str, retune: bool) {
        match (
            parse_finite(&self.ramp_target),
            parse_finite(&self.ramp_rate),
        ) {
            (Ok(target), Ok(rate)) if rate > 0.0 => {
                let intent = if retune {
                    OperatorIntent::RetuneReference {
                        reference: reference.into(),
                        target,
                        rate,
                    }
                } else {
                    OperatorIntent::ConfigureReferenceRamp {
                        reference: reference.into(),
                        target,
                        rate,
                    }
                };
                self.begin_operator(intent);
            }
            _ => {
                self.operator_problem =
                    Some("Reference target/rate require finite values and positive rate".into())
            }
        }
    }

    fn drive_smoke(&mut self, context: &egui::Context) {
        let Some(mut smoke) = self.smoke.take() else {
            return;
        };
        if smoke.started.elapsed() >= SMOKE_DEADLINE {
            smoke.finish(false, &self.model, "deadline", context);
            return;
        }
        match smoke.phase {
            SmokePhase::InitialFresh
                if self.model.observations.freshness == Freshness::Fresh
                    && has_signal_point(&self.model)
                    && has_fresh_reference(&self.model) =>
            {
                let reference =
                    self.model
                        .observations
                        .entities
                        .iter()
                        .find_map(|(identity, observation)| match identity {
                            RuntimeRef::Reference { reference }
                                if observation.freshness == Freshness::Fresh
                                    && observation.value.get("kind").and_then(Value::as_str)
                                        == Some("ramp") =>
                            {
                                Some((reference.clone(), observation.value.clone()))
                            }
                            _ => None,
                        });
                let Some((reference, value)) = reference else {
                    smoke.finish(false, &self.model, "no_ramp_reference", context);
                    return;
                };
                let Some(revision) = value.get("revision").and_then(Value::as_str) else {
                    smoke.finish(false, &self.model, "reference_revision_missing", context);
                    return;
                };
                let Some(target) = value.get("target").and_then(Value::as_f64) else {
                    smoke.finish(false, &self.model, "reference_target_missing", context);
                    return;
                };
                let desired = target + 0.25;
                if let Err(error) = self.operator.begin(
                    &self.model,
                    OperatorIntent::RetuneReference {
                        reference: reference.clone(),
                        target: desired,
                        rate: 2.0,
                    },
                ) {
                    smoke.finish(
                        false,
                        &self.model,
                        &format!("cannot_open_confirmation:{error}"),
                        context,
                    );
                    return;
                }
                smoke.reference = Some(reference);
                smoke.original_revision = Some(revision.to_owned());
                smoke.desired_target = Some(desired);
                smoke.confirmation_observed = matches!(
                    self.operator.state,
                    OperatorWorkflowState::AwaitingConfirmation(_)
                );
                smoke.phase = SmokePhase::ConfirmMutation;
            }
            SmokePhase::ConfirmMutation => {
                let Some(client) = self.client.as_ref() else {
                    smoke.finish(false, &self.model, "client_missing", context);
                    return;
                };
                match self.operator.confirm(&self.model, client) {
                    Ok(command_id) => {
                        if let Err(error) = self.model.track_operator_intent(command_id) {
                            smoke.finish(false, &self.model, error, context);
                            return;
                        }
                        smoke.phase = SmokePhase::AwaitMutation;
                    }
                    Err(error) => {
                        smoke.finish(
                            false,
                            &self.model,
                            &format!("confirm_failed:{error}"),
                            context,
                        );
                        return;
                    }
                }
            }
            SmokePhase::AwaitMutation => {
                if self.operator.accepted_observed {
                    smoke.accepted_observed = true;
                }
                if matches!(self.operator.state, OperatorWorkflowState::Completed { .. }) {
                    smoke.completed_observed = true;
                }
                let updated = smoke.reference.as_ref().is_some_and(|reference| {
                    self.model
                        .observations
                        .entities
                        .get(&RuntimeRef::Reference {
                            reference: reference.clone(),
                        })
                        .is_some_and(|observation| {
                            observation.freshness == Freshness::Fresh
                                && observation.value.get("revision").and_then(Value::as_str)
                                    != smoke.original_revision.as_deref()
                                && observation.value.get("target").and_then(Value::as_f64)
                                    == smoke.desired_target
                        })
                });
                if smoke.completed_observed && updated {
                    smoke.authoritative_refresh_observed = true;
                    self.operator.cancel_or_acknowledge();
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    smoke.phase = SmokePhase::AwaitMinimized;
                }
            }
            SmokePhase::AwaitMinimized
                if context.input(|input| input.viewport().minimized) == Some(true) =>
            {
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                smoke.phase = SmokePhase::AwaitRestored;
            }
            SmokePhase::AwaitRestored
                if context.input(|input| input.viewport().minimized) == Some(false) =>
            {
                if let Some(client) = self.client.as_ref()
                    && client.disconnect().is_ok()
                {
                    smoke.phase = SmokePhase::AwaitStale;
                }
            }
            SmokePhase::AwaitStale
                if matches!(
                    self.model.connection,
                    ConnectionState::Disconnected | ConnectionState::Stale
                ) && self.model.observations.freshness == Freshness::Stale =>
            {
                smoke.stale_controls_disabled = smoke.reference.as_ref().is_some_and(|reference| {
                    !self.control_enabled(
                        &RuntimeRef::Reference {
                            reference: reference.clone(),
                        },
                        "reference_retune",
                    )
                });
                if let Some(client) = self.client.as_ref()
                    && client.connect(self.model.recovery.scope.clone()).is_ok()
                {
                    smoke.phase = SmokePhase::ReattachedFresh;
                }
            }
            SmokePhase::ReattachedFresh
                if self.model.observations.freshness == Freshness::Fresh
                    && has_signal_point(&self.model)
                    && has_fresh_reference(&self.model)
                    && self.model.observations.entities.keys().any(|target| {
                        matches!(target, RuntimeRef::Reference { .. })
                            && self.control_enabled(target, "reference_retune")
                    }) =>
            {
                smoke.controls_reenabled_after_fresh = true;
                smoke.finish(true, &self.model, "pass", context);
                return;
            }
            _ => {}
        }
        context.request_repaint_after(Duration::from_millis(100));
        self.smoke = Some(smoke);
    }

    fn drive_kill_probe(&mut self, context: &egui::Context) {
        let Some(probe) = self.kill_probe.as_mut() else {
            return;
        };
        if !probe.minimize_requested
            && self.model.observations.freshness == Freshness::Fresh
            && has_signal_point(&self.model)
            && has_fresh_reference(&self.model)
        {
            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            probe.minimize_requested = true;
            context.request_repaint_after(Duration::from_millis(100));
        } else if probe.minimize_requested
            && context.input(|input| input.viewport().minimized) == Some(true)
            && !probe.result_path.exists()
        {
            let controller = authoritative_controller_snapshot(&self.model);
            let recorder = authoritative_recorder_snapshot(&self.model);
            let _ = std::fs::write(
                &probe.result_path,
                json!({
                    "status": if controller.is_some() && recorder.is_some() {
                        "ready_for_forced_termination"
                    } else {
                        "fail"
                    },
                    "reason": if controller.is_none() {
                        "authoritative_controller_missing"
                    } else if recorder.is_none() {
                        "authoritative_recorder_missing"
                    } else {
                        "ready"
                    },
                    "renderer":"glow",
                    "controller": controller,
                    "recorder": recorder,
                })
                .to_string(),
            );
        }
    }

    fn shutdown_client(&mut self) {
        if let Some(client) = self.client.take()
            && let Err(error) = client.shutdown()
        {
            self.model.client_error = Some(error.to_owned());
        }
    }
}

impl eframe::App for WorkbenchApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_updates();
        self.drive_smoke(context);
        self.drive_kill_probe(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("status").show(ui, |ui| self.render_status(ui));
        egui::Panel::left("discovery")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| self.render_discovery(ui));
        egui::CentralPanel::default().show(ui, |ui| self.render_main(ui));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown_client();
    }
}

impl Drop for WorkbenchApp {
    fn drop(&mut self) {
        self.shutdown_client();
    }
}

fn drain_bounded<T>(
    mut receive: impl FnMut() -> Result<T, TryRecvError>,
    mut apply: impl FnMut(T),
    limit: usize,
) -> usize {
    let mut drained = 0;
    while drained < limit {
        match receive() {
            Ok(update) => {
                apply(update);
                drained += 1;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    drained
}

fn freshness_label(ui: &mut egui::Ui, freshness: Freshness) {
    let (text, color) = match freshness {
        Freshness::Fresh => ("Fresh", Color32::LIGHT_GREEN),
        Freshness::Rebuilding => ("Rebuilding", Color32::LIGHT_BLUE),
        Freshness::Stale => ("Stale", Color32::YELLOW),
        Freshness::Unknown => ("Unknown", Color32::GRAY),
    };
    ui.label(RichText::new(text).color(color).strong());
}

fn recovery_attachment_text(attachment: RecoveryAttachment) -> &'static str {
    match attachment {
        RecoveryAttachment::Attached => "attached to current boot/scope",
        RecoveryAttachment::NoHello => "no authoritative hello",
        RecoveryAttachment::BootMismatch => "different Runtime boot",
        RecoveryAttachment::ScopeMismatch => "different retained scope",
    }
}

fn status_eligibility_text(eligibility: StatusEligibility) -> &'static str {
    match eligibility {
        StatusEligibility::Eligible => "eligible",
        StatusEligibility::ClientNotReady => "client is not Ready",
        StatusEligibility::HelloMissing => "hello is unavailable",
        StatusEligibility::OperationUnavailable => "operation_status is not advertised",
        StatusEligibility::RecoveryJournalProblem => "recovery journal is unavailable",
        StatusEligibility::RecordMissing => "recovery record is unavailable",
        StatusEligibility::BootMismatch => "record belongs to another Runtime boot",
        StatusEligibility::ScopeMismatch => "record belongs to another scope",
        StatusEligibility::TerminalRecord => "record is already terminal",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecoveryActionAvailability {
    check_status: bool,
    exact_retry: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct ExactRetryConfirmationView {
    scope: String,
    seq: u64,
    operation: String,
    admission: KnownAdmission,
    args: String,
    warning: &'static str,
}

fn exact_retry_confirmation_view(prepared: &PreparedExactRetry) -> ExactRetryConfirmationView {
    ExactRetryConfirmationView {
        scope: prepared.record.identity.scope.clone(),
        seq: prepared.record.identity.seq,
        operation: prepared.record.op.clone(),
        admission: prepared.record.admission,
        args: bounded_value_text(&prepared.record.args),
        warning: EXACT_RETRY_WARNING,
    }
}

fn recovery_action_availability(
    model: &WorkbenchModel,
    status: &RecoveryStatusTracker,
    exact_retry: &ExactRetryWorkflow,
    operator: &OperatorWorkflow,
    record: &RecoveryRecordPresentation,
) -> RecoveryActionAvailability {
    let status_pending = matches!(record.status, RecoveryStatusState::Pending { .. });
    RecoveryActionAvailability {
        check_status: record.eligibility == StatusEligibility::Eligible
            && !status_pending
            && !exact_retry.blocks_status(&record.identity),
        exact_retry: operator.allows_exact_retry(&record.identity)
            && exact_retry.can_begin(model, status, &record.identity),
    }
}

fn exact_retry_state_message(state: &ExactRetryState) -> Option<&'static str> {
    match state {
        ExactRetryState::Idle => None,
        ExactRetryState::AwaitingConfirmation(_) => {
            Some("Review the exact retained identity and payload before confirming Exact Retry.")
        }
        ExactRetryState::Submitted { .. } => {
            Some("Exact Retry submitted. Admission/outcome not yet known.")
        }
        ExactRetryState::Accepted { .. } => {
            Some("Runtime reports this exact request identity as accepted/in progress.")
        }
        ExactRetryState::Completed { .. } => {
            Some("Runtime reports the retained operation completed.")
        }
        ExactRetryState::Failed { .. } => Some("Runtime reports the retained operation failed."),
        ExactRetryState::OutcomeUnknown { .. } => Some(
            "Runtime no longer knows the retained outcome for this request identity. The original recovery evidence remains unresolved.",
        ),
        ExactRetryState::LocalFailure { .. } => {
            Some("Exact Retry was not submitted by the client. Recovery evidence is unchanged.")
        }
        ExactRetryState::ApplicationFailure { .. } => Some(
            "The Exact Retry request was rejected or failed to reconcile. The underlying original mutation is not classified as Failed by this response.",
        ),
        ExactRetryState::Interrupted { .. } => {
            Some("Exact Retry exchange was interrupted. No automatic retry was performed.")
        }
        ExactRetryState::DraftStale { .. } => Some(
            "Recovery evidence/session changed before confirmation. Review the current record again.",
        ),
    }
}

fn reconciled_operator_message(admission: &KnownAdmission) -> String {
    format!("Previously ambiguous operator action reconciled: {admission:?}")
}

fn quarantine_reason_text(reason: RecoveryQuarantineReason) -> &'static str {
    match reason {
        RecoveryQuarantineReason::InstanceChanged => "Runtime instance changed",
        RecoveryQuarantineReason::ScopeUnknown => "retained scope is unknown",
        RecoveryQuarantineReason::AttachedBootMismatch => "attached Runtime boot differs",
        RecoveryQuarantineReason::AttachedScopeMismatch => "attached scope differs",
    }
}

fn awaiting_confirmation_current(
    state: &OperatorWorkflowState,
    model: &WorkbenchModel,
) -> Option<bool> {
    match state {
        OperatorWorkflowState::AwaitingConfirmation(prepared) => Some(prepared.is_current(model)),
        _ => None,
    }
}

fn resource_reconnect_enabled(
    model: &WorkbenchModel,
    resource: &str,
    capable: bool,
    ordinary_ready: bool,
) -> bool {
    capable && ordinary_ready && model.resource_detail_is_fresh(resource)
}

fn render_operator_warning(ui: &mut egui::Ui, warning: &OperatorWarning) {
    let text = match warning {
        OperatorWarning::ControllerPolicyChange => {
            "Controller policy/configuration will be revalidated by Runtime."
        }
        OperatorWarning::ResourceReconnect => {
            "Resource reconnect may interrupt availability; Runtime remains authoritative."
        }
        OperatorWarning::RecorderStop => {
            "Recorder stop requests terminalization; completion is not durability evidence."
        }
        OperatorWarning::PropertyMutationClass(class) => {
            ui.colored_label(
                Color32::YELLOW,
                format!("Runtime property mutation class: {class}."),
            );
            return;
        }
    };
    ui.colored_label(Color32::YELLOW, text);
}

fn runtime_ref_label(reference: &RuntimeRef) -> String {
    match reference {
        RuntimeRef::Instrument { instrument } => format!("Instrument {instrument}"),
        RuntimeRef::Signal {
            instrument,
            parameter,
        } => {
            format!("Signal {instrument}/{parameter}")
        }
        RuntimeRef::Reference { reference } => format!("Reference {reference}"),
        RuntimeRef::Controller { controller } => format!("Controller {controller}"),
        RuntimeRef::Resource { resource } => format!("Resource {resource}"),
        RuntimeRef::Component { component } => format!("Component {component}"),
        RuntimeRef::Recorder => "Recorder".into(),
        RuntimeRef::ConfigurationProperty { owner, property } => {
            format!("Property {owner:?}/{property}")
        }
    }
}

fn bounded_value_text(value: &serde_json::Value) -> String {
    let mut text = value.to_string();
    if text.len() > 1024 {
        truncate_utf8(&mut text, 1024);
        text.push('…');
    }
    text
}

fn numeric_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_f64)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn controller_pid_editor_snapshot(
    model: &WorkbenchModel,
    controller: &str,
) -> Option<(Value, [String; 5])> {
    if !model.controller_detail_is_fresh(controller) {
        return None;
    }
    let value = &model
        .observations
        .entities
        .get(&RuntimeRef::Controller {
            controller: controller.to_owned(),
        })?
        .value;
    let revision = value.get("revision")?.clone();
    let fields = ["kp", "ki", "kd", "output_min", "output_max"]
        .map(|field| numeric_text(value.pointer(&format!("/config/pid/{field}"))));
    Some((revision, fields))
}

fn scalar_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn parse_finite(value: &str) -> Result<f64, &'static str> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or("numeric field requires a finite value")
}

fn property_candidate(
    value_type: Option<&str>,
    text: &str,
) -> Result<PropertyMutationCandidate, &'static str> {
    match value_type {
        Some("integer") => text
            .trim()
            .parse::<i64>()
            .map(PropertyMutationCandidate::Integer)
            .map_err(|_| "integer property requires a signed integer"),
        Some("text") => Ok(PropertyMutationCandidate::Text(text.to_owned())),
        _ => Err("v1 property mutation supports only integer or text values"),
    }
}

fn truncate_utf8(value: &mut String, maximum_bytes: usize) {
    if value.len() <= maximum_bytes {
        return;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

fn recovery_summary(model: &WorkbenchModel) -> (usize, usize) {
    (
        model.recovery.mutations.len(),
        model.recovery.reconciliation_required.len(),
    )
}

fn has_signal_point(model: &WorkbenchModel) -> bool {
    model
        .observations
        .live
        .values()
        .any(|buffer| !buffer.points().is_empty())
}

fn has_fresh_reference(model: &WorkbenchModel) -> bool {
    model
        .observations
        .entities
        .iter()
        .any(|(identity, observation)| {
            matches!(identity, RuntimeRef::Reference { .. })
                && observation.freshness == Freshness::Fresh
        })
}

fn authoritative_controller_snapshot(model: &WorkbenchModel) -> Option<Value> {
    model
        .observations
        .entities
        .iter()
        .find_map(|(identity, observation)| match identity {
            RuntimeRef::Controller { controller } if observation.freshness == Freshness::Fresh => {
                Some(json!({
                    "controller": controller,
                    "state": observation.value.get("state")?,
                    "revision": observation.value.get("revision")?,
                }))
            }
            _ => None,
        })
}

fn authoritative_recorder_snapshot(model: &WorkbenchModel) -> Option<Value> {
    model
        .observations
        .entities
        .get(&RuntimeRef::Recorder)
        .filter(|observation| observation.freshness == Freshness::Fresh)
        .and_then(|observation| {
            Some(json!({
                "state": observation.value.get("state")?,
                "active_run": observation.value.get("active_run"),
                "run_id": observation.value.get("run_id"),
            }))
        })
}

struct SmokeRun {
    result_path: PathBuf,
    started: Instant,
    phase: SmokePhase,
    reference: Option<String>,
    original_revision: Option<String>,
    desired_target: Option<f64>,
    confirmation_observed: bool,
    accepted_observed: bool,
    completed_observed: bool,
    authoritative_refresh_observed: bool,
    stale_controls_disabled: bool,
    controls_reenabled_after_fresh: bool,
}

struct KillProbe {
    result_path: PathBuf,
    minimize_requested: bool,
}

impl KillProbe {
    fn new(result_path: PathBuf) -> Self {
        Self {
            result_path,
            minimize_requested: false,
        }
    }
}

impl SmokeRun {
    fn new(result_path: PathBuf) -> Self {
        Self {
            result_path,
            started: Instant::now(),
            phase: SmokePhase::InitialFresh,
            reference: None,
            original_revision: None,
            desired_target: None,
            confirmation_observed: false,
            accepted_observed: false,
            completed_observed: false,
            authoritative_refresh_observed: false,
            stale_controls_disabled: false,
            controls_reenabled_after_fresh: false,
        }
    }

    fn finish(self, passed: bool, model: &WorkbenchModel, reason: &str, context: &egui::Context) {
        let final_reference = self.reference.as_ref().and_then(|reference| {
            model.observations.entities.get(&RuntimeRef::Reference {
                reference: reference.clone(),
            })
        });
        let result = json!({
            "status": if passed { "pass" } else { "fail" },
            "reason": reason,
            "renderer": "glow",
            "connection": format!("{:?}", model.connection),
            "freshness": format!("{:?}", model.observations.freshness),
            "boot_id": model.recovery.boot_id,
            "scope": model.recovery.scope,
            "entities": model.observations.entities.len(),
            "live_signal_points": model.observations.live.values().map(|buffer| buffer.points().len()).sum::<usize>(),
            "presentation_plots": model.presentation.plots.len(),
            "minimize_restore_observed": passed,
            "confirmation_observed": self.confirmation_observed,
            "mutation_accepted_observed": self.accepted_observed,
            "mutation_completed_observed": self.completed_observed,
            "authoritative_refresh_observed": self.authoritative_refresh_observed,
            "stale_controls_disabled": self.stale_controls_disabled,
            "controls_reenabled_after_fresh": self.controls_reenabled_after_fresh,
            "original_reference_revision": self.original_revision,
            "desired_reference_target": self.desired_target,
            "final_reference_revision": final_reference
                .and_then(|observation| observation.value.get("revision")),
            "final_reference_target": final_reference
                .and_then(|observation| observation.value.get("target")),
            "controller": authoritative_controller_snapshot(model),
            "recorder": authoritative_recorder_snapshot(model),
        });
        let _ = std::fs::write(&self.result_path, result.to_string());
        context.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[derive(Clone, Copy)]
enum SmokePhase {
    InitialFresh,
    ConfirmMutation,
    AwaitMutation,
    AwaitMinimized,
    AwaitRestored,
    AwaitStale,
    ReattachedFresh,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::{
            ClientUpdate,
            types::{
                CommandSendError, EventCursor, HelloState, KnownAdmission, MutationIdentity,
                QuarantinedRecoveryRecord, RecoveryQuarantineReason, RecoveryRecord,
            },
        },
        model::{ExactRetryError, ExactRetrySubmitter, RecoveryStatusSubmitter},
        presentation::PresentationDocument,
    };
    use std::cell::RefCell;
    use std::sync::mpsc::sync_channel;

    #[derive(Default)]
    struct FakeStatusSubmitter {
        sent: RefCell<Vec<MutationIdentity>>,
    }

    impl RecoveryStatusSubmitter for FakeStatusSubmitter {
        fn operation_status(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.sent.borrow_mut().push(identity);
            Ok(71)
        }
    }

    #[derive(Default)]
    struct FakeRetrySubmitter {
        sent: RefCell<Vec<MutationIdentity>>,
    }

    impl ExactRetrySubmitter for FakeRetrySubmitter {
        fn retry_exact(&self, identity: MutationIdentity) -> Result<u64, CommandSendError> {
            self.sent.borrow_mut().push(identity);
            Ok(81)
        }
    }

    fn recovery_record(seq: u64, admission: KnownAdmission) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "boot".into(),
            identity: MutationIdentity {
                scope: "scope".into(),
                seq,
            },
            op: "reference_retune".into(),
            args: json!({"reference":"1","target":2.0,"rate":1.0}),
            admission,
        }
    }

    fn recovery_model(records: Vec<RecoveryRecord>) -> WorkbenchModel {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("recovery-gui"));
        model.apply_client_update(ClientUpdate::Hello(HelloState {
            boot_id: "boot".into(),
            scope: "scope".into(),
            next_seq: 9,
            operations: vec!["operation_status".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
        }));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: records,
            quarantined: Vec::new(),
        });
        model
    }

    fn apply_recovery_update(
        model: &mut WorkbenchModel,
        status: &mut RecoveryStatusTracker,
        retry: &mut ExactRetryWorkflow,
        records: Vec<RecoveryRecord>,
    ) {
        let update = ClientUpdate::RecoveryProjection {
            active: records,
            quarantined: Vec::new(),
        };
        model.apply_client_update(update.clone());
        status.after_update(&update);
        retry.after_update(&update, model, status);
    }

    #[test]
    fn gui_drain_is_bounded_to_the_worker_queue_capacity() {
        let (sender, receiver) = sync_channel(UPDATES_PER_FRAME + 1);
        for value in 0..=UPDATES_PER_FRAME {
            sender.send(value).unwrap();
        }
        let mut applied = Vec::new();
        let drained = drain_bounded(
            || receiver.try_recv(),
            |value| applied.push(value),
            UPDATES_PER_FRAME,
        );
        assert_eq!(drained, UPDATES_PER_FRAME);
        assert_eq!(applied.len(), UPDATES_PER_FRAME);
        assert_eq!(receiver.try_recv(), Ok(UPDATES_PER_FRAME));
    }

    #[test]
    fn recovery_records_and_reconciliation_warning_are_distinct() {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.recovery.mutations.push(RecoveryRecord {
            boot_id: "boot".into(),
            identity: MutationIdentity {
                scope: "scope".into(),
                seq: 1,
            },
            op: "reference_retune".into(),
            args: json!({}),
            admission: KnownAdmission::Completed,
        });
        assert_eq!(recovery_summary(&model), (1, 0));
        model
            .recovery
            .reconciliation_required
            .push(MutationIdentity {
                scope: "scope".into(),
                seq: 2,
            });
        assert_eq!(recovery_summary(&model), (1, 1));
    }

    #[test]
    fn presentation_is_not_changed_by_stale_and_reconnect_updates() {
        let presentation = PresentationDocument::empty("stable-presentation");
        let mut model = WorkbenchModel::new(presentation.clone());
        model.apply_client_update(ClientUpdate::State(ConnectionState::Disconnected));
        model.apply_client_update(ClientUpdate::State(ConnectionState::Connecting));
        assert_eq!(model.presentation, presentation);
    }

    #[test]
    fn stale_confirmation_is_visible_to_the_gui_before_submission() {
        let hello = |boot: &str| HelloState {
            boot_id: boot.into(),
            scope: "scope".into(),
            next_seq: 1,
            operations: vec!["reference_retune".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: boot.into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: boot.into(),
                seq: 0,
            },
        };
        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(hello("boot-a")));
        model.observations.observe(
            target.clone(),
            json!({"reference":"1","kind":"ramp","revision":"3","target":1.0,"rate":1.0}),
            None,
        );
        model.complete_rebuild();
        let mut workflow = OperatorWorkflow::default();
        workflow
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: 2.0,
                    rate: 1.0,
                },
            )
            .unwrap();
        assert_eq!(
            awaiting_confirmation_current(&workflow.state, &model),
            Some(true)
        );

        model.apply_client_update(ClientUpdate::Hello(hello("boot-b")));
        model.observations.observe(
            target,
            json!({"reference":"1","kind":"ramp","revision":"3","target":1.0,"rate":1.0}),
            None,
        );
        model.complete_rebuild();
        assert_eq!(
            awaiting_confirmation_current(&workflow.state, &model),
            Some(false)
        );
    }

    #[test]
    fn pid_editor_never_pairs_old_fields_with_a_new_event_revision() {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "1".into(),
            op: "controller".into(),
            kind: crate::client::types::ReplyKind::Result,
            envelope: json!({"type":"result","result":{"controller":"2","state":"paused",
                "revision":"7","config":{"pid":{"kp":1.0,"ki":0.0,"kd":0.0,
                "output_min":0.0,"output_max":10.0}}}}),
            recovery: None,
        });
        let (_, fields) = controller_pid_editor_snapshot(&model, "2").unwrap();
        assert_eq!(fields[0], "1");

        model.apply_client_update(ClientUpdate::Event {
            cursor: EventCursor {
                boot_id: "boot".into(),
                seq: 2,
            },
            envelope: json!({"kind":"controller","target":{"id":"2"},
                "data":{"state":"paused","status":"valid","failure":null,"active":false,
                    "paused":true,"revision":"8","last_tick":"1","latest_output":0.0}}),
        });
        assert!(controller_pid_editor_snapshot(&model, "2").is_none());

        model.apply_client_update(ClientUpdate::Reply {
            command_id: 2,
            msg_id: "2".into(),
            op: "controller".into(),
            kind: crate::client::types::ReplyKind::Result,
            envelope: json!({"type":"result","result":{"controller":"2","state":"paused",
                "revision":"8","config":{"pid":{"kp":2.0,"ki":0.0,"kd":0.0,
                "output_min":0.0,"output_max":10.0}}}}),
            recovery: None,
        });
        let (revision, fields) = controller_pid_editor_snapshot(&model, "2").unwrap();
        assert_eq!(revision, "8");
        assert_eq!(fields[0], "2");
    }

    #[test]
    fn reconnect_control_is_disabled_while_full_resource_detail_is_stale() {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.observations.observe_resource_full(
            "3".into(),
            json!({"resource":"3","binding_generation":"4","transport_generation":"9",
                "capabilities":{"reconnect":true}}),
        );
        assert!(resource_reconnect_enabled(&model, "3", true, true));

        model.observations.observe_resource_event(
            "3".into(),
            json!({"state":"recovering","queue_len":0,"generation":"10",
                "active":null,"latest":null}),
            1,
        );
        assert_eq!(
            model.observations.entities[&RuntimeRef::Resource {
                resource: "3".into()
            }]
                .freshness,
            Freshness::Fresh
        );
        assert!(!resource_reconnect_enabled(&model, "3", true, true));

        model.observations.observe_resource_full(
            "3".into(),
            json!({"resource":"3","binding_generation":"5","transport_generation":"10",
                "capabilities":{"reconnect":true}}),
        );
        assert!(resource_reconnect_enabled(&model, "3", true, true));
    }

    #[test]
    fn recovery_row_actions_share_renderer_neutral_retry_and_status_eligibility() {
        let first = recovery_record(1, KnownAdmission::Ambiguous);
        let second = recovery_record(2, KnownAdmission::Accepted);
        let mut model = recovery_model(vec![first.clone(), second.clone()]);
        let mut status = RecoveryStatusTracker::default();
        let mut retry = ExactRetryWorkflow::default();
        let operator = OperatorWorkflow::default();
        let rows = status.presentations(&model);

        assert_eq!(
            recovery_action_availability(&model, &status, &retry, &operator, &rows[0]),
            RecoveryActionAvailability {
                check_status: true,
                exact_retry: true,
            }
        );

        retry
            .begin(&model, &status, first.identity.clone())
            .unwrap();
        assert_eq!(
            recovery_action_availability(&model, &status, &retry, &operator, &rows[0]),
            RecoveryActionAvailability {
                check_status: false,
                exact_retry: false,
            }
        );
        retry.cancel_or_acknowledge();

        let status_submitter = FakeStatusSubmitter::default();
        status
            .check_status(&model, &status_submitter, first.identity.clone())
            .unwrap();
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(&model, &status, &retry, &operator, &rows[0]).exact_retry
        );
        assert_eq!(
            recovery_action_availability(&model, &status, &retry, &operator, &rows[1]),
            RecoveryActionAvailability {
                check_status: true,
                exact_retry: true,
            }
        );

        model.recovery_problem = Some("journal unavailable".into());
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(&model, &status, &retry, &operator, &rows[1]).exact_retry
        );
        model.recovery_problem = None;
        model.recovery.quarantined.push(QuarantinedRecoveryRecord {
            record: recovery_record(3, KnownAdmission::Ambiguous),
            reason: RecoveryQuarantineReason::InstanceChanged,
        });
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(&model, &status, &retry, &operator, &rows[1]).exact_retry
        );
        model.recovery.quarantined.clear();
        model.connection = ConnectionState::Stale;
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(&model, &status, &retry, &operator, &rows[1]).exact_retry
        );

        model.connection = ConnectionState::Ready;
        model.recovery.mutations = vec![recovery_record(4, KnownAdmission::Completed)];
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(&model, &status, &retry, &operator, &rows[0]).exact_retry
        );
    }

    #[test]
    fn exact_retry_confirmation_is_read_only_and_submits_one_identity() {
        let record = recovery_record(5, KnownAdmission::Ambiguous);
        let mut model = recovery_model(vec![record.clone()]);
        let mut status = RecoveryStatusTracker::default();
        let mut retry = ExactRetryWorkflow::default();
        let submitter = FakeRetrySubmitter::default();

        retry
            .begin(&model, &status, record.identity.clone())
            .unwrap();
        assert!(submitter.sent.borrow().is_empty());
        let ExactRetryState::AwaitingConfirmation(prepared) = &retry.state else {
            panic!("confirmation was not opened");
        };
        let view = exact_retry_confirmation_view(prepared);
        assert_eq!(view.scope, "scope");
        assert_eq!(view.seq, 5);
        assert_eq!(view.operation, "reference_retune");
        assert_eq!(view.admission, KnownAdmission::Ambiguous);
        assert!(view.args.contains("\"target\":2.0"));
        assert_eq!(view.warning, EXACT_RETRY_WARNING);
        let mut unicode = prepared.clone();
        unicode.record.args = json!({"text":"я".repeat(2_000)});
        let bounded = exact_retry_confirmation_view(&unicode).args;
        assert!(bounded.ends_with('…'));
        assert!(bounded.len() <= 1024 + '…'.len_utf8());

        retry.confirm(&model, &status, &submitter).unwrap();
        assert_eq!(
            submitter.sent.borrow().as_slice(),
            std::slice::from_ref(&record.identity)
        );
        assert!(retry.confirm(&model, &status, &submitter).is_err());
        assert_eq!(submitter.sent.borrow().len(), 1);
        assert_eq!(
            exact_retry_state_message(&retry.state),
            Some("Exact Retry submitted. Admission/outcome not yet known.")
        );
        assert!(model.actions.is_empty());

        let mut accepted = record.clone();
        accepted.admission = KnownAdmission::Accepted;
        apply_recovery_update(&mut model, &mut status, &mut retry, vec![accepted.clone()]);
        assert!(matches!(retry.state, ExactRetryState::Accepted { .. }));
        let rows = status.presentations(&model);
        assert!(
            !recovery_action_availability(
                &model,
                &status,
                &retry,
                &OperatorWorkflow::default(),
                &rows[0]
            )
            .check_status
        );

        accepted.admission = KnownAdmission::Completed;
        apply_recovery_update(&mut model, &mut status, &mut retry, vec![accepted]);
        assert_eq!(
            exact_retry_state_message(&retry.state),
            Some("Runtime reports the retained operation completed.")
        );
        retry.cancel_or_acknowledge();
        assert_eq!(retry.state, ExactRetryState::Idle);
        assert!(model.actions.is_empty());
    }

    #[test]
    fn changed_retry_evidence_stales_confirmation_without_a_send() {
        let record = recovery_record(6, KnownAdmission::Ambiguous);
        let mut model = recovery_model(vec![record.clone()]);
        let status = RecoveryStatusTracker::default();
        let mut retry = ExactRetryWorkflow::default();
        let submitter = FakeRetrySubmitter::default();
        retry
            .begin(&model, &status, record.identity.clone())
            .unwrap();

        let mut changed = record;
        changed.args = json!({"reference":"1","target":99.0,"rate":1.0});
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![changed],
            quarantined: Vec::new(),
        });
        assert_eq!(
            retry.confirm(&model, &status, &submitter),
            Err(ExactRetryError::DraftStale)
        );
        assert!(matches!(retry.state, ExactRetryState::DraftStale { .. }));
        assert!(submitter.sent.borrow().is_empty());
        assert_eq!(
            exact_retry_state_message(&retry.state),
            Some(
                "Recovery evidence/session changed before confirmation. Review the current record again."
            )
        );
    }

    #[test]
    fn retry_and_reconciled_terminal_wording_preserves_authority_boundaries() {
        let prepared = PreparedExactRetry {
            record: recovery_record(7, KnownAdmission::Ambiguous),
            boot_id: "boot".into(),
            scope: "scope".into(),
        };
        let cases = [
            (
                ExactRetryState::Accepted {
                    command_id: 1,
                    prepared: prepared.clone(),
                },
                "Runtime reports this exact request identity as accepted/in progress.",
            ),
            (
                ExactRetryState::Completed {
                    prepared: prepared.clone(),
                },
                "Runtime reports the retained operation completed.",
            ),
            (
                ExactRetryState::Failed {
                    prepared: prepared.clone(),
                },
                "Runtime reports the retained operation failed.",
            ),
            (
                ExactRetryState::OutcomeUnknown {
                    prepared: prepared.clone(),
                },
                "Runtime no longer knows the retained outcome for this request identity. The original recovery evidence remains unresolved.",
            ),
            (
                ExactRetryState::ApplicationFailure {
                    prepared: prepared.clone(),
                    code: "outcome_conflict".into(),
                },
                "The Exact Retry request was rejected or failed to reconcile. The underlying original mutation is not classified as Failed by this response.",
            ),
            (
                ExactRetryState::LocalFailure {
                    prepared: prepared.clone(),
                    reason: "client busy".into(),
                },
                "Exact Retry was not submitted by the client. Recovery evidence is unchanged.",
            ),
            (
                ExactRetryState::Interrupted {
                    prepared: prepared.clone(),
                },
                "Exact Retry exchange was interrupted. No automatic retry was performed.",
            ),
        ];
        for (state, expected) in cases {
            assert_eq!(exact_retry_state_message(&state), Some(expected));
        }
        assert_eq!(
            reconciled_operator_message(&KnownAdmission::Completed),
            "Previously ambiguous operator action reconciled: Completed"
        );
        assert_eq!(
            reconciled_operator_message(&KnownAdmission::Failed),
            "Previously ambiguous operator action reconciled: Failed"
        );
    }
}
