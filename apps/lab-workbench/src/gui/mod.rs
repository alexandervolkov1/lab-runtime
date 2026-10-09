//! Minimal native Workbench GUI over the one bounded Application client owner.

mod app;
pub(crate) use crate::rebuild;

use crate::{
    client::endpoint::RuntimeEndpoint,
    external::PreparedEndpoint,
    ownership::WorkspaceOwnership,
    presentation::{PresentationDocument, load_presentation},
    recovery::load_journal,
};
use std::{net::SocketAddrV4, path::PathBuf};

pub(crate) struct GuiLaunch {
    pub(crate) address: RuntimeEndpoint,
    pub(crate) observation_only: bool,
    pub(crate) scope: Option<String>,
    pub(crate) workspace: PathBuf,
    pub(crate) workbench_listen: Option<SocketAddrV4>,
}

pub(crate) fn run(launch: GuiLaunch) -> Result<(), String> {
    let ownership =
        WorkspaceOwnership::acquire(&launch.workspace).map_err(|error| error.to_string())?;
    let workspace = ownership.workspace().to_owned();
    let presentation_path = workspace.join("presentation-v1.json");
    let journal_path = workspace.join("recovery-v1.json");

    let (presentation, presentation_problem) = if presentation_path.exists() {
        match load_presentation(&presentation_path) {
            Ok(document) => (document, None),
            Err(error) => (
                PresentationDocument::empty("default"),
                Some(format!("presentation was not replaced: {error}")),
            ),
        }
    } else {
        (PresentationDocument::empty("default"), None)
    };
    let journal_scope = if journal_path.exists() {
        load_journal(&journal_path)
            .ok()
            .map(|journal| journal.scope)
    } else {
        None
    };
    let desired_scope = launch.scope.or(journal_scope);
    let endpoint = launch
        .workbench_listen
        .map(PreparedEndpoint::bind)
        .transpose()
        .map_err(|error| error.to_string())?;
    let address = launch.address;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("lab-runtime Workbench")
            .with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lab-runtime Workbench",
        options,
        Box::new(move |context| {
            Ok(Box::new(app::WorkbenchApp::new(
                context,
                address,
                launch.observation_only,
                desired_scope,
                journal_path,
                presentation,
                presentation_problem,
                ownership,
                endpoint,
            )?))
        }),
    )
    .map_err(|error| error.to_string())
}
