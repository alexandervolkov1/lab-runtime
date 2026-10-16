//! Shared unit-test support. This module is never compiled into Runtime builds.

use std::path::PathBuf;

pub(crate) fn temporary_database(label: &str) -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).expect("test entropy");
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-test-{label}-{suffix}.sqlite"))
}

pub(crate) fn remove_database(path: PathBuf) {
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}
