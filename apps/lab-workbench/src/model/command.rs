//! Validated Workbench command split shared by future GUI and scripting adapters.

use crate::presentation::{Plot, PresentationDocument, RuntimeRef, Trace};
use serde_json::Value;

/// One future Workbench entry point, explicitly separating lab and UI authority.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkbenchCommand {
    /// Intent that must be sent through the frozen M14.2 Application client owner.
    Lab(LabCommand),
    /// Client-only presentation mutation.
    Ui(UiCommand),
}

/// Small Workbench-level lab intent surface; it is not a copied operation registry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LabCommand {
    /// Submit one ordinary Application query.
    Query { op: String, args: Value },
    /// Submit one Application mutation through worker-owned request sequencing.
    Mutation { op: String, args: Value },
    /// Reconcile a worker-owned retained mutation.
    OperationStatus { scope: String, seq: u64 },
}

/// Renderer-neutral presentation mutation used by future GUI and `ui/*` bindings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UiCommand {
    /// Add a fully specified plot after document-wide validation.
    AddPlot { plot: Plot },
    /// Remove an unreferenced plot.
    RemovePlot { plot_id: String },
    /// Add a trace to an existing plot.
    AddTrace { plot_id: String, trace: Trace },
    /// Remove a trace from an existing plot.
    RemoveTrace { plot_id: String, trace_id: String },
    /// Change trace visibility.
    SetTraceVisibility {
        plot_id: String,
        trace_id: String,
        visible: bool,
    },
    /// Change a plot's display-only time window.
    SetTimeWindow { plot_id: String, seconds: f64 },
    /// Rename a plot, trace, control, panel, tab, or window by stable ID.
    RenamePresentationItem { item_id: String, label: String },
    /// Retarget a trace while preserving its client-owned identity and styling.
    SetTraceSource {
        plot_id: String,
        trace_id: String,
        source: RuntimeRef,
    },
}

/// Applies one UI command transactionally: invalid candidates never replace active state.
pub(crate) fn apply_ui_command(
    document: &mut PresentationDocument,
    command: UiCommand,
) -> Result<(), UiCommandError> {
    let mut candidate = document.clone();
    apply_unvalidated(&mut candidate, command)?;
    candidate
        .validate()
        .map_err(UiCommandError::InvalidCandidate)?;
    *document = candidate;
    Ok(())
}

fn apply_unvalidated(
    document: &mut PresentationDocument,
    command: UiCommand,
) -> Result<(), UiCommandError> {
    match command {
        UiCommand::AddPlot { plot } => document.plots.push(plot),
        UiCommand::RemovePlot { plot_id } => {
            let before = document.plots.len();
            document.plots.retain(|plot| plot.id != plot_id);
            if document.plots.len() == before {
                return Err(UiCommandError::UnknownItem(plot_id));
            }
        }
        UiCommand::AddTrace { plot_id, trace } => {
            find_plot_mut(document, &plot_id)?.traces.push(trace);
        }
        UiCommand::RemoveTrace { plot_id, trace_id } => {
            let plot = find_plot_mut(document, &plot_id)?;
            let before = plot.traces.len();
            plot.traces.retain(|trace| trace.id != trace_id);
            if plot.traces.len() == before {
                return Err(UiCommandError::UnknownItem(trace_id));
            }
        }
        UiCommand::SetTraceVisibility {
            plot_id,
            trace_id,
            visible,
        } => find_trace_mut(document, &plot_id, &trace_id)?.visible = visible,
        UiCommand::SetTimeWindow { plot_id, seconds } => {
            find_plot_mut(document, &plot_id)?.time_window_seconds = seconds;
        }
        UiCommand::RenamePresentationItem { item_id, label } => {
            if let Some(plot) = document.plots.iter_mut().find(|plot| plot.id == item_id) {
                plot.title = label;
            } else if let Some(trace) = document
                .plots
                .iter_mut()
                .flat_map(|plot| &mut plot.traces)
                .find(|trace| trace.id == item_id)
            {
                trace.display_label = label;
            } else if let Some(control) =
                document.controls.iter_mut().find(|item| item.id == item_id)
            {
                control.label = label;
            } else if let Some(window) = document.windows.iter_mut().find(|item| item.id == item_id)
            {
                window.title = label;
            } else if let Some(tab) = document
                .windows
                .iter_mut()
                .flat_map(|window| &mut window.tabs)
                .find(|item| item.id == item_id)
            {
                tab.title = label;
            } else if let Some(panel) = document
                .windows
                .iter_mut()
                .flat_map(|window| &mut window.tabs)
                .flat_map(|tab| &mut tab.panels)
                .find(|item| item.id == item_id)
            {
                panel.title = label;
            } else {
                return Err(UiCommandError::UnknownItem(item_id));
            }
        }
        UiCommand::SetTraceSource {
            plot_id,
            trace_id,
            source,
        } => find_trace_mut(document, &plot_id, &trace_id)?.source = source,
    }
    Ok(())
}

fn find_plot_mut<'a>(
    document: &'a mut PresentationDocument,
    plot_id: &str,
) -> Result<&'a mut Plot, UiCommandError> {
    document
        .plots
        .iter_mut()
        .find(|plot| plot.id == plot_id)
        .ok_or_else(|| UiCommandError::UnknownItem(plot_id.to_owned()))
}

fn find_trace_mut<'a>(
    document: &'a mut PresentationDocument,
    plot_id: &str,
    trace_id: &str,
) -> Result<&'a mut Trace, UiCommandError> {
    find_plot_mut(document, plot_id)?
        .traces
        .iter_mut()
        .find(|trace| trace.id == trace_id)
        .ok_or_else(|| UiCommandError::UnknownItem(trace_id.to_owned()))
}

/// UI command failure; active presentation remains unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UiCommandError {
    /// The command named no existing presentation item.
    UnknownItem(String),
    /// The resulting document failed complete validation.
    InvalidCandidate(crate::presentation::DocumentError),
}

impl std::fmt::Display for UiCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownItem(id) => write!(formatter, "unknown presentation item: {id}"),
            Self::InvalidCandidate(error) => write!(formatter, "invalid UI command: {error}"),
        }
    }
}

impl std::error::Error for UiCommandError {}
