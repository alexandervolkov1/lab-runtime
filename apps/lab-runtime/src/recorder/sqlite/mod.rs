//! SQLite schema-version-one implementation owned exclusively by the Recorder worker.
//!
//! `open` owns connection/recovery settings; `write` owns lifecycle and fact
//! transactions; `history` owns bounded archive reads; `provenance` owns immutable
//! activation content; `schema` owns the fixed DDL; `encoding` owns row codecs.
//! No module here owns Runtime or Application semantics.

use super::*;
use lab_core::{
    InstrumentId, ParameterId, SampleQuality, SignalId, Value,
    managed::CapturedInput,
    recording::{OutputStage, RecordingFact},
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, limits::Limit, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

mod encoding;
mod history;
mod open;
mod provenance;
mod schema;
mod write;

use encoding::*;
use schema::*;

const APPLICATION_ID: i32 = 0x4c41_4252; // "LABR"; unrelated SQLite files are rejected.
const SCHEMA_VERSION: i64 = 1;
const MAX_RAW_PAGE: usize = 128;
const MAIN_FILE_QUOTA_BYTES: u64 = 1024 * 1024 * 1024;
const WAL_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;

/// Stable, nonrecursive storage failure surfaced to the host policy.
#[derive(Debug)]
pub struct StorageError(pub(super) String);
impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for StorageError {}
impl StorageError {
    pub(crate) fn disabled() -> Self {
        Self("recorder disabled".into())
    }
}
impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        if matches!(&error, rusqlite::Error::SqliteFailure(failure,_)
            if failure.code == ErrorCode::OperationInterrupted)
        {
            return Self("history_budget_exceeded".into());
        }
        Self(format!("sqlite: {error}"))
    }
}

/// Worker-observed storage footprint; status reads this receipt without SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageHealth {
    /// Logical main database page bytes, including committed WAL-only changes.
    pub main_logical_bytes: u64,
    /// Checked hard main-file page quota in bytes.
    pub main_quota_bytes: u64,
    /// Actual sidecar size at the worker observation point.
    pub wal_bytes: u64,
    /// Current checked WAL checkpoint threshold.
    pub wal_threshold_bytes: u64,
    /// Successful explicit threshold checkpoints in this boot.
    pub wal_checkpoints: u64,
}

/// Actual connection durability settings, inspected only on the storage worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqliteSettings {
    /// `wal` for the reviewed version-one adapter.
    pub journal_mode: String,
    /// SQLite numeric synchronous level; 2 is FULL.
    pub synchronous: i64,
    /// Whether foreign-key constraints are enabled on this connection.
    pub foreign_keys: bool,
    /// Schema version stored by the application.
    pub user_version: i64,
}

/// Exclusive SQLite connection owned by one storage worker.
pub struct SqliteStore {
    connection: Connection,
    database_id: String,
    boot_id: [u8; 16],
    boot_id_hex: String,
    run_no: Option<u64>,
    interval_no: Option<u64>,
    next_run_no: u64,
    next_interval_no: u64,
    next_record_sequence: u64,
    commit_no: u64,
    next_activation_no: u64,
    current_activation_no: Option<u64>,
    boot_sealed: bool,
    coverage_gap: bool,
    boot_anchor: TimeAnchor,
    next_anchor_no: u64,
    main_quota_pages: u64,
    wal_path: PathBuf,
    wal_threshold_bytes: u64,
    wal_checkpoints: u64,
    checkpoint_fault_once: bool,
}
