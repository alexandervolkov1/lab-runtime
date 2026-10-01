//! Shared, deterministic support for Runtime integration tests.

use std::{
    path::{Path, PathBuf},
    thread,
    time::Instant,
};

/// Allocate a process-safe temporary SQLite path with the requested test label.
pub fn temporary_database(label: &str) -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).expect("test entropy");
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-test-{label}-{suffix}.sqlite"))
}

/// Remove a SQLite database and its WAL/SHM sidecars after worker shutdown.
#[allow(dead_code)]
pub fn remove_database(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

/// Poll a bounded condition without changing the caller's timeout semantics.
#[allow(dead_code)]
pub fn wait_until<T>(
    deadline: Instant,
    description: &str,
    mut poll: impl FnMut() -> Option<T>,
) -> T {
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "{description}");
        thread::yield_now();
    }
}
