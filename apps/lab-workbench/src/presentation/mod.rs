//! Client-owned, renderer-neutral presentation state.
#![allow(
    dead_code,
    unused_imports,
    reason = "M14.3 presentation API precedes the M14.4 GUI consumer"
)]

mod document;
mod persistence;
mod reference;

pub(crate) use document::{
    AxisOptions, Control, ControlKind, DocumentError, Panel, PanelKind, Plot, PresentationDocument,
    PresentationTab, PresentationWindow, Trace, TraceStyle,
};
pub(crate) use persistence::{
    PresentationLoadError, default_presentation_path, load_presentation, replace_presentation,
    save_presentation,
};
pub(crate) use reference::{ConfigurationOwner, RuntimeRef};

#[cfg(test)]
mod tests;
