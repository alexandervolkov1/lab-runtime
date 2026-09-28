//! Small bounded file primitives shared by client-owned persistence formats.
#![allow(dead_code, reason = "bounded helpers include M14.3 test inspection")]

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "persisted file exceeds its byte limit",
        ));
    }
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "persisted file exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

pub(crate) fn write_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "persistence path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let id = TEMP_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    });
    let id = id.map_err(|_| io::Error::other("temporary file identity exhausted"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid file name"))?;
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), id));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        // The temporary is a sibling, so the rename stays on one volume. Rust's
        // Windows implementation uses replace-existing file semantics; on the
        // supported platform this is one directory-entry replacement operation.
        fs::rename(&temporary, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn sibling_files(path: &Path) -> Vec<PathBuf> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|candidate| candidate.to_str())
                .is_some_and(|candidate| candidate.starts_with(&format!(".{name}.")))
        })
        .collect()
}
