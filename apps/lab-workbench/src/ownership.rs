//! OS-owned single-process guard for one resolved Workbench data workspace.

use std::{
    fmt, io,
    path::{Component, Path, PathBuf},
};

/// RAII ownership of one Workbench user-data workspace.
///
/// On Windows the underlying named mutex is released by the kernel even when the
/// process terminates abnormally. The guard must be acquired before either
/// presentation or recovery persistence is opened.
pub(crate) struct WorkspaceOwnership {
    workspace: PathBuf,
    #[cfg(windows)]
    _guard: win_desktop_utils::InstanceGuard,
}

impl WorkspaceOwnership {
    /// Resolves `workspace` without creating it and acquires its process guard.
    pub(crate) fn acquire(workspace: &Path) -> Result<Self, OwnershipError> {
        let workspace = resolve_without_creation(workspace).map_err(OwnershipError::Io)?;
        #[cfg(windows)]
        {
            let identity = guard_identity(&workspace);
            let Some(guard) = win_desktop_utils::single_instance_with_scope(
                &identity,
                win_desktop_utils::InstanceScope::Global,
            )
            .map_err(|error| OwnershipError::Platform(error.to_string()))?
            else {
                return Err(OwnershipError::AlreadyActive(workspace));
            };
            Ok(Self {
                workspace,
                _guard: guard,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = workspace;
            Err(OwnershipError::Unsupported)
        }
    }

    /// Canonical/lexically resolved workspace represented by this guard.
    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }
}

/// Visible startup failure before persistent Workbench state is touched.
#[derive(Debug)]
pub(crate) enum OwnershipError {
    /// Workspace path resolution failed.
    Io(io::Error),
    /// Another process already owns the same resolved workspace.
    AlreadyActive(PathBuf),
    /// The platform mutex API failed.
    Platform(String),
    /// The v0.1 Workbench process guard currently supports Windows only.
    #[cfg(not(windows))]
    Unsupported,
}

impl fmt::Display for OwnershipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "cannot resolve Workbench workspace: {error}"),
            Self::AlreadyActive(path) => write!(
                formatter,
                "another Workbench already owns workspace {}",
                path.display()
            ),
            Self::Platform(error) => write!(formatter, "Workbench ownership guard failed: {error}"),
            #[cfg(not(windows))]
            Self::Unsupported => {
                formatter.write_str("Workbench GUI is currently supported on Windows")
            }
        }
    }
}

impl std::error::Error for OwnershipError {}

fn resolve_without_creation(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let lexical = lexical_normalize(&absolute);
    let mut existing = lexical.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "workspace has no existing ancestor",
            )
        })?;
        suffix.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "workspace has no existing ancestor",
            )
        })?;
    }
    let mut resolved = existing.canonicalize()?;
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    Ok(lexical_normalize(&resolved))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(windows)]
fn guard_identity(workspace: &Path) -> String {
    // Stable FNV-1a over a case-folded Windows path keeps the named mutex short and
    // deterministic without making the hash an authority or security boundary.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for unit in workspace
        .as_os_str()
        .to_string_lossy()
        .to_lowercase()
        .encode_utf16()
    {
        for byte in unit.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("lab-runtime-workbench-v1-{hash:016x}")
}

#[cfg(all(test, windows))]
mod tests {
    use super::{OwnershipError, WorkspaceOwnership};
    use std::{env, fs, path::PathBuf, process::Command};

    const CHILD_MODE: &str = "LAB_WORKBENCH_OWNERSHIP_TEST_CHILD";
    const CHILD_PATH: &str = "LAB_WORKBENCH_OWNERSHIP_TEST_PATH";
    const TEST_NAME: &str = "ownership::tests::workspace_guard_is_process_owned_and_released";

    #[test]
    fn workspace_guard_is_process_owned_and_released() {
        if let Ok(mode) = env::var(CHILD_MODE) {
            let path = PathBuf::from(env::var_os(CHILD_PATH).expect("child workspace path"));
            let result = WorkspaceOwnership::acquire(&path);
            match mode.as_str() {
                "rejected" => assert!(matches!(result, Err(OwnershipError::AlreadyActive(_)))),
                "acquired" => assert!(result.is_ok()),
                _ => panic!("unknown child mode"),
            }
            return;
        }

        let workspace = env::temp_dir().join(format!(
            "lab-workbench-ownership-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = WorkspaceOwnership::acquire(&workspace).expect("first owns workspace");
        assert!(first.workspace().is_absolute());
        run_child("rejected", &workspace);
        assert_eq!(
            first.workspace(),
            super::resolve_without_creation(&workspace).unwrap()
        );
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("owner-remains-usable"), b"owned").unwrap();
        drop(first);
        run_child("acquired", &workspace);
        fs::remove_dir_all(&workspace).unwrap();
    }

    fn run_child(mode: &str, workspace: &std::path::Path) {
        let status = Command::new(env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_MODE, mode)
            .env(CHILD_PATH, workspace)
            .status()
            .expect("start ownership child");
        assert!(status.success(), "ownership child failed: {status}");
    }
}
