//! Version-one client-owned presentation document and validation.

use super::reference::RuntimeRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Only supported presentation persistence version.
pub(crate) const PRESENTATION_FORMAT_VERSION: u32 = 1;
/// Maximum presentation file size in bytes.
pub(crate) const PRESENTATION_FILE_BYTES: usize = 1024 * 1024;
/// Maximum windows in one document.
pub(crate) const MAX_WINDOWS: usize = 64;
/// Maximum panels across all tabs in one document.
pub(crate) const MAX_PANELS: usize = 64;
/// Maximum plots in one document.
pub(crate) const MAX_PLOTS: usize = 32;
/// Maximum traces in one plot.
pub(crate) const MAX_TRACES_PER_PLOT: usize = 32;
/// Maximum UTF-8 bytes in any persisted string.
pub(crate) const MAX_STRING_BYTES: usize = 512;
/// Maximum controls in one deliberately modest v1 document.
pub(crate) const MAX_CONTROLS: usize = 64;
/// Maximum tabs per window.
pub(crate) const MAX_TABS_PER_WINDOW: usize = 64;
/// Largest accepted display time window.
pub(crate) const MAX_TIME_WINDOW_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;

/// A versioned, renderer-neutral, client-owned presentation document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationDocument {
    /// Persistence schema version; currently exactly one.
    pub(crate) format_version: u32,
    /// Stable client-owned document identity.
    pub(crate) document_id: String,
    /// Window/tab/panel arrangement.
    pub(crate) windows: Vec<PresentationWindow>,
    /// Plot configuration referenced by plot panels.
    pub(crate) plots: Vec<Plot>,
    /// Declarative operator controls referenced by control panels.
    pub(crate) controls: Vec<Control>,
}

impl PresentationDocument {
    /// Creates an empty valid v1 document.
    pub(crate) fn empty(document_id: impl Into<String>) -> Self {
        Self {
            format_version: PRESENTATION_FORMAT_VERSION,
            document_id: document_id.into(),
            windows: Vec::new(),
            plots: Vec::new(),
            controls: Vec::new(),
        }
    }

    /// Validates a complete candidate before it may replace active state or disk.
    pub(crate) fn validate(&self) -> Result<(), DocumentError> {
        if self.format_version != PRESENTATION_FORMAT_VERSION {
            return Err(DocumentError::UnsupportedVersion(self.format_version));
        }
        validate_string(&self.document_id)?;
        if self.windows.len() > MAX_WINDOWS {
            return Err(DocumentError::Limit("windows"));
        }
        if self.plots.len() > MAX_PLOTS {
            return Err(DocumentError::Limit("plots"));
        }
        if self.controls.len() > MAX_CONTROLS {
            return Err(DocumentError::Limit("controls"));
        }

        let mut ids = BTreeSet::new();
        insert_id(&mut ids, &self.document_id)?;
        let mut panels = 0usize;
        for window in &self.windows {
            validate_string(&window.id)?;
            validate_string(&window.title)?;
            insert_id(&mut ids, &window.id)?;
            if window.tabs.len() > MAX_TABS_PER_WINDOW {
                return Err(DocumentError::Limit("tabs_per_window"));
            }
            for tab in &window.tabs {
                validate_string(&tab.id)?;
                validate_string(&tab.title)?;
                insert_id(&mut ids, &tab.id)?;
                panels = panels
                    .checked_add(tab.panels.len())
                    .ok_or(DocumentError::Limit("panels"))?;
                if panels > MAX_PANELS {
                    return Err(DocumentError::Limit("panels"));
                }
                for panel in &tab.panels {
                    validate_string(&panel.id)?;
                    validate_string(&panel.title)?;
                    insert_id(&mut ids, &panel.id)?;
                    match &panel.kind {
                        PanelKind::Plot { plot_id } => validate_string(plot_id)?,
                        PanelKind::Controls { control_ids } => {
                            if control_ids.len() > MAX_CONTROLS {
                                return Err(DocumentError::Limit("panel_controls"));
                            }
                            for id in control_ids {
                                validate_string(id)?;
                            }
                        }
                        PanelKind::Status { source } => validate_runtime_ref(source)?,
                    }
                }
            }
        }

        for plot in &self.plots {
            validate_string(&plot.id)?;
            validate_string(&plot.title)?;
            insert_id(&mut ids, &plot.id)?;
            if !plot.time_window_seconds.is_finite()
                || plot.time_window_seconds <= 0.0
                || plot.time_window_seconds > MAX_TIME_WINDOW_SECONDS
            {
                return Err(DocumentError::InvalidNumber("time_window_seconds"));
            }
            plot.axes.validate()?;
            if plot.traces.len() > MAX_TRACES_PER_PLOT {
                return Err(DocumentError::Limit("traces_per_plot"));
            }
            for trace in &plot.traces {
                validate_string(&trace.id)?;
                validate_string(&trace.display_label)?;
                validate_optional_string(trace.display_unit.as_deref())?;
                validate_string(&trace.style.color)?;
                if !trace.style.width.is_finite()
                    || trace.style.width <= 0.0
                    || trace.style.width > 32.0
                {
                    return Err(DocumentError::InvalidNumber("trace_width"));
                }
                validate_runtime_ref(&trace.source)?;
                insert_id(&mut ids, &trace.id)?;
            }
        }

        for control in &self.controls {
            validate_string(&control.id)?;
            validate_string(&control.label)?;
            validate_runtime_ref(&control.target)?;
            insert_id(&mut ids, &control.id)?;
        }

        let plots = self
            .plots
            .iter()
            .map(|plot| plot.id.as_str())
            .collect::<BTreeSet<_>>();
        let controls = self
            .controls
            .iter()
            .map(|control| control.id.as_str())
            .collect::<BTreeSet<_>>();
        for panel in self
            .windows
            .iter()
            .flat_map(|window| &window.tabs)
            .flat_map(|tab| &tab.panels)
        {
            match &panel.kind {
                PanelKind::Plot { plot_id } if !plots.contains(plot_id.as_str()) => {
                    return Err(DocumentError::DanglingPresentationId(plot_id.clone()));
                }
                PanelKind::Controls { control_ids } => {
                    for id in control_ids {
                        if !controls.contains(id.as_str()) {
                            return Err(DocumentError::DanglingPresentationId(id.clone()));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Enforces the canonical pretty-JSON byte bound used by persistence and
    /// by the active presentation owner.
    pub(crate) fn validate_serialized_size(&self) -> Result<(), DocumentError> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| DocumentError::Serialization)?;
        if bytes.len() > PRESENTATION_FILE_BYTES {
            return Err(DocumentError::Limit("file_bytes"));
        }
        Ok(())
    }

    /// Iterates over every Runtime reference without resolving it.
    pub(crate) fn runtime_refs(&self) -> impl Iterator<Item = &RuntimeRef> {
        self.plots
            .iter()
            .flat_map(|plot| plot.traces.iter().map(|trace| &trace.source))
            .chain(self.controls.iter().map(|control| &control.target))
            .chain(
                self.windows
                    .iter()
                    .flat_map(|window| &window.tabs)
                    .flat_map(|tab| &tab.panels)
                    .filter_map(|panel| match &panel.kind {
                        PanelKind::Status { source } => Some(source),
                        _ => None,
                    }),
            )
    }
}

/// Client window configuration; it carries no Runtime authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationWindow {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Display title.
    pub(crate) title: String,
    /// Ordered tabs.
    pub(crate) tabs: Vec<PresentationTab>,
}

/// One tab of presentation panels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationTab {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Display title.
    pub(crate) title: String,
    /// Ordered panels.
    pub(crate) panels: Vec<Panel>,
}

/// One renderer-neutral panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Panel {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Display title.
    pub(crate) title: String,
    /// Presentation content selected for the panel.
    pub(crate) kind: PanelKind,
}

/// Supported modest v1 panel contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PanelKind {
    /// A panel referencing one plot configuration.
    Plot { plot_id: String },
    /// A panel referencing declarative controls.
    Controls { control_ids: Vec<String> },
    /// A simple status panel referencing one Runtime entity.
    Status { source: RuntimeRef },
}

/// Renderer-neutral plot configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plot {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Display title.
    pub(crate) title: String,
    /// Visible trailing time range in seconds.
    pub(crate) time_window_seconds: f64,
    /// Display-only axis configuration.
    pub(crate) axes: AxisOptions,
    /// Ordered trace configuration; samples are intentionally absent.
    pub(crate) traces: Vec<Trace>,
}

/// Display-only axis limits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AxisOptions {
    /// Optional lower Y bound.
    pub(crate) y_min: Option<f64>,
    /// Optional upper Y bound.
    pub(crate) y_max: Option<f64>,
}

impl AxisOptions {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.y_min.is_some_and(|value| !value.is_finite())
            || self.y_max.is_some_and(|value| !value.is_finite())
        {
            return Err(DocumentError::InvalidNumber("axis_range"));
        }
        if let (Some(min), Some(max)) = (self.y_min, self.y_max)
            && min >= max
        {
            return Err(DocumentError::InvalidNumber("axis_range"));
        }
        Ok(())
    }
}

/// One plotted Runtime source and its client-owned display choices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Trace {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Stable Runtime identity to observe.
    pub(crate) source: RuntimeRef,
    /// Client-owned label independent of Runtime naming.
    pub(crate) display_label: String,
    /// Whether the future renderer should display the trace.
    pub(crate) visible: bool,
    /// Display-only style.
    pub(crate) style: TraceStyle,
    /// Optional display unit; it does not alter Runtime values.
    pub(crate) display_unit: Option<String>,
}

/// Renderer-neutral trace style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceStyle {
    /// Client-owned color token.
    pub(crate) color: String,
    /// Future renderer line width.
    pub(crate) width: f64,
}

/// A declarative operator control mapped later to existing Application operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Control {
    /// Stable client-owned identity.
    pub(crate) id: String,
    /// Display label.
    pub(crate) label: String,
    /// Existing Runtime entity the action concerns.
    pub(crate) target: RuntimeRef,
    /// Existing Application action category, not executable script.
    pub(crate) kind: ControlKind,
    /// Whether the future UI should ask for confirmation before submission.
    pub(crate) confirm: bool,
}

/// Small declarative control vocabulary for v1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControlKind {
    /// Submit an existing Reference retune mutation.
    SetReference,
    /// Submit an existing controller lifecycle mutation.
    ControllerLifecycle,
    /// Submit an existing recording lifecycle mutation.
    RecordingLifecycle,
    /// Submit an existing configuration/property mutation.
    ApplyConfiguration,
    /// Submit an existing resource reconnect mutation.
    ReconnectResource,
}

/// Validation failure that leaves the active document untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DocumentError {
    /// The document is not schema version one.
    UnsupportedVersion(u32),
    /// A frozen cardinality or byte limit was exceeded.
    Limit(&'static str),
    /// A stable client-owned presentation identity was reused.
    DuplicateId(String),
    /// An internal presentation reference points to no defined item.
    DanglingPresentationId(String),
    /// A required string was empty or exceeded its byte bound.
    InvalidString,
    /// A numeric display option was non-finite or outside its accepted range.
    InvalidNumber(&'static str),
    /// A Runtime reference was structurally invalid.
    InvalidRuntimeRef,
    /// A structurally valid document could not be encoded canonically.
    Serialization,
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported presentation format version {version}"
                )
            }
            Self::Limit(name) => write!(formatter, "presentation limit exceeded: {name}"),
            Self::DuplicateId(id) => write!(formatter, "duplicate presentation ID: {id}"),
            Self::DanglingPresentationId(id) => {
                write!(formatter, "unknown presentation item ID: {id}")
            }
            Self::InvalidString => formatter.write_str("invalid presentation string"),
            Self::InvalidNumber(name) => write!(formatter, "invalid presentation number: {name}"),
            Self::InvalidRuntimeRef => formatter.write_str("invalid Runtime reference"),
            Self::Serialization => formatter.write_str("presentation serialization failed"),
        }
    }
}

impl std::error::Error for DocumentError {}

fn insert_id(ids: &mut BTreeSet<String>, id: &str) -> Result<(), DocumentError> {
    if !ids.insert(id.to_owned()) {
        return Err(DocumentError::DuplicateId(id.to_owned()));
    }
    Ok(())
}

fn validate_string(value: &str) -> Result<(), DocumentError> {
    if value.is_empty() || value.len() > MAX_STRING_BYTES {
        return Err(DocumentError::InvalidString);
    }
    Ok(())
}

fn validate_optional_string(value: Option<&str>) -> Result<(), DocumentError> {
    if let Some(value) = value {
        validate_string(value)?;
    }
    Ok(())
}

fn validate_runtime_ref(reference: &RuntimeRef) -> Result<(), DocumentError> {
    let mut valid = true;
    reference.strings(&mut |value| {
        valid &= !value.is_empty() && value.len() <= MAX_STRING_BYTES;
    });
    if !valid {
        return Err(DocumentError::InvalidRuntimeRef);
    }
    Ok(())
}
