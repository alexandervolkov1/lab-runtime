//! Bounded JSON loading and replace-after-validation presentation persistence.

use super::document::{PRESENTATION_FILE_BYTES, PresentationDocument};
use crate::storage::{read_bounded, write_replace};
use std::{
    env, io,
    path::{Path, PathBuf},
};

/// Returns the Workbench-owned per-user presentation path.
pub(crate) fn default_presentation_path() -> io::Result<PathBuf> {
    let root = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Windows user-data directory unavailable",
            )
        })?;
    Ok(PathBuf::from(root)
        .join("lab-runtime")
        .join("workbench")
        .join("presentation-v1.json"))
}

/// Loads and fully validates a candidate without touching active model state.
pub(crate) fn load_presentation(
    path: &Path,
) -> Result<PresentationDocument, PresentationLoadError> {
    let bytes = read_bounded(path, PRESENTATION_FILE_BYTES).map_err(PresentationLoadError::Io)?;
    if bytes.is_empty() {
        return Err(PresentationLoadError::Json(
            "empty presentation file".into(),
        ));
    }
    let candidate: PresentationDocument = serde_json::from_slice(&bytes)
        .map_err(|error| PresentationLoadError::Json(error.to_string()))?;
    candidate
        .validate()
        .map_err(PresentationLoadError::Validation)?;
    Ok(candidate)
}

/// Validates and writes a bounded candidate before replacing the prior file.
pub(crate) fn save_presentation(
    path: &Path,
    candidate: &PresentationDocument,
) -> Result<(), PresentationLoadError> {
    candidate
        .validate()
        .map_err(PresentationLoadError::Validation)?;
    let bytes = serde_json::to_vec_pretty(candidate)
        .map_err(|error| PresentationLoadError::Json(error.to_string()))?;
    if bytes.len() > PRESENTATION_FILE_BYTES {
        return Err(PresentationLoadError::Validation(
            super::document::DocumentError::Limit("file_bytes"),
        ));
    }
    write_replace(path, &bytes).map_err(PresentationLoadError::Io)
}

/// Replaces active state only after the persisted candidate has fully validated.
pub(crate) fn replace_presentation(
    active: &mut PresentationDocument,
    path: &Path,
) -> Result<(), PresentationLoadError> {
    let candidate = load_presentation(path)?;
    *active = candidate;
    Ok(())
}

/// Presentation persistence failure; active state is not changed on this error.
#[derive(Debug)]
pub(crate) enum PresentationLoadError {
    /// Filesystem or bounded-read/write failure.
    Io(io::Error),
    /// JSON/UTF-8/root/schema decoding failure.
    Json(String),
    /// Fully decoded candidate failed semantic validation.
    Validation(super::document::DocumentError),
}

impl std::fmt::Display for PresentationLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "presentation I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "presentation JSON failed: {error}"),
            Self::Validation(error) => write!(formatter, "presentation validation failed: {error}"),
        }
    }
}

impl std::error::Error for PresentationLoadError {}
