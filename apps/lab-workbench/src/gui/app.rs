//! eframe main-thread owner and thin observational rendering.

use super::rebuild::RebuildCoordinator;
use crate::{
    client::ClientHandle,
    client::types::ConnectionState,
    model::{Freshness, UiCommand, WorkbenchModel},
    ownership::WorkspaceOwnership,
    presentation::{
        AxisOptions, Plot as PresentationPlot, PresentationDocument, RuntimeRef, Trace, TraceStyle,
    },
};
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot as EguiPlot, PlotPoints};
use serde_json::json;
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
    selected: Option<RuntimeRef>,
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
            selected: None,
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
        if ui
            .add_enabled(!connected, egui::Button::new("Connect"))
            .clicked()
            && let Some(client) = self.client.as_ref()
        {
            let scope = self.model.recovery.scope.clone();
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
                if ui.selectable_label(selected, label).clicked()
                    && matches!(
                        identity,
                        RuntimeRef::Signal { .. } | RuntimeRef::Reference { .. }
                    )
                {
                    self.selected = Some(identity);
                }
            }
        });
    }

    fn render_main(&self, ui: &mut egui::Ui) {
        let Some(selected) = self.selected.as_ref() else {
            ui.heading("Live signal");
            ui.label("No discovered signal is available.");
            return;
        };
        ui.heading(runtime_ref_label(selected));
        let unresolved = self.model.unresolved.contains(selected);
        let observation = self.model.observations.entities.get(selected);
        match (unresolved, observation) {
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
        if let Some(observation) = observation {
            ui.monospace(bounded_value_text(&observation.value));
        }
        if let Some(buffer) = self.model.observations.live.get(selected) {
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
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                smoke.phase = SmokePhase::AwaitMinimized;
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
                if let Some(client) = self.client.as_ref()
                    && client.connect(self.model.recovery.scope.clone()).is_ok()
                {
                    smoke.phase = SmokePhase::ReattachedFresh;
                }
            }
            SmokePhase::ReattachedFresh
                if self.model.observations.freshness == Freshness::Fresh
                    && has_signal_point(&self.model)
                    && has_fresh_reference(&self.model) =>
            {
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
            let _ = std::fs::write(
                &probe.result_path,
                json!({"status":"ready_for_forced_termination","renderer":"glow"}).to_string(),
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
        text.truncate(1024);
        text.push('…');
    }
    text
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

struct SmokeRun {
    result_path: PathBuf,
    started: Instant,
    phase: SmokePhase,
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
        }
    }

    fn finish(self, passed: bool, model: &WorkbenchModel, reason: &str, context: &egui::Context) {
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
        });
        let _ = std::fs::write(&self.result_path, result.to_string());
        context.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[derive(Clone, Copy)]
enum SmokePhase {
    InitialFresh,
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
            types::{KnownAdmission, MutationIdentity, RecoveryRecord},
        },
        presentation::PresentationDocument,
    };
    use std::sync::mpsc::sync_channel;

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
}
