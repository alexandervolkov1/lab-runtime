//! SQLite version-one storage on a dedicated host worker, outside `lab-core`.
//!
//! [`SqliteStore`] is deliberately synchronous: only the storage worker or an
//! offline test may own it. The Runtime owner must hand groups to a bounded
//! nonblocking ingress rather than call these methods on its safety lane.

use lab_core::{
    InstrumentId, ParameterId, SampleQuality, Value,
    recording::{OutputStage, RecordingFact},
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, limits::Limit, params};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

mod time;
mod worker;
pub use time::TimeAnchor;
pub use worker::{RecorderLimits, RecorderWorker, RecordingState, RecordingStatus, WriterBarrier};

/// Recording availability policy selected at trusted host startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingPolicy {
    /// Preserve experiment progress while retaining a visible coverage failure.
    BestEffort,
    /// Recorder failure closes ordinary control and invokes Rust safe/fault work.
    Required,
}

/// Owner-frozen loss boundary; an absent count is an honest unknown count.
#[derive(Clone, Debug)]
pub struct RecorderGap {
    /// First stable failure code/context, capped before entering the seal slot.
    pub reason: String,
    /// Runtime monotonic cutoff of this coverage interval.
    pub at: Duration,
    /// First source fact identity not transferred to durable ingress, if known.
    pub first_missing_fact: Option<u64>,
    /// Known count of rejected source facts at this cutoff, if known.
    pub known_missing_count: Option<u64>,
    /// Last source fact identity accepted into ingress before the gap.
    pub last_accepted_fact: Option<u64>,
}

/// Immutable owner-captured run boundary. Existing samples inside it are
/// snapshots, never newly observed measurement rows or physical evidence.
#[derive(Clone, Debug)]
pub struct BoundarySnapshot {
    /// Actual owner monotonic time of capture.
    pub at: Duration,
    /// Canonical bounded JSON of config revisions, latest attempts and pending IDs.
    pub data: String,
}
impl BoundarySnapshot {
    /// Empty trusted boundary for offline adapter tests with no Runtime owner.
    pub fn empty(at: Duration) -> Self {
        Self {
            at,
            data: serde_json::json!({"latest_samples":[],
            "controller_revisions":[],"reference_revisions":[],
            "managed_revisions":[],"pending_operations":[]})
            .to_string(),
        }
    }
    fn valid(&self) -> bool {
        !self.data.is_empty()
            && self.data.len() <= 64 * 1024
            && serde_json::from_str::<serde_json::Value>(&self.data).is_ok()
    }
}
impl RecorderGap {
    fn valid(&self) -> bool {
        !self.reason.is_empty()
            && self.reason.len() <= 512
            && self.known_missing_count.is_none_or(|count| count > 0)
    }
}

/// Immutable application-boundary fact; it is a Recorder domain payload, not a
/// wire frame or an M6 event-ring entry. Its original request identity survives
/// client disconnect and its phase never predicts a future physical outcome.
#[derive(Clone, Debug)]
pub struct OperationRecord {
    /// Original logical client scope, independent of the connection lifetime.
    pub scope: String,
    /// Original checked request sequence.
    pub request_seq: u64,
    /// Stable application command name.
    pub command: &'static str,
    /// `accepted`, `completed`, or `failed` at the true application boundary.
    pub phase: &'static str,
    /// Canonical bounded JSON of typed intent or typed terminal result.
    pub data: String,
    /// Truth source of this fact, never caller-supplied evidence.
    pub outcome_basis: &'static str,
    /// Owner monotonic publication time within this boot.
    pub at: Duration,
}
impl OperationRecord {
    fn charge(&self) -> Option<usize> {
        self.scope
            .capacity()
            .checked_add(self.data.capacity())?
            .checked_add(256)
    }
    fn valid(&self) -> bool {
        self.scope.len() <= 64
            && self.request_seq > 0
            && self.data.len() <= 16 * 1024
            && matches!(self.phase, "accepted" | "completed" | "failed")
            && matches!(
                self.outcome_basis,
                "application_admission" | "domain_result"
            )
    }
}

const APPLICATION_ID: i32 = 0x4c41_4252; // "LABR"; unrelated SQLite files are rejected.
const SCHEMA_VERSION: i64 = 1;
const MAX_RAW_PAGE: usize = 128;
const MAIN_FILE_QUOTA_BYTES: u64 = 1024 * 1024 * 1024;
const WAL_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;

/// Stable, nonrecursive storage failure surfaced to the host policy.
#[derive(Debug)]
pub struct StorageError(String);
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

/// Cooperative VM/time limit for one indexed history selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryBudget {
    vm_ops: u32,
    elapsed: Duration,
    stride: i32,
}
impl Default for HistoryBudget {
    fn default() -> Self {
        Self {
            vm_ops: 100_000,
            elapsed: Duration::from_millis(50),
            stride: 1_000,
        }
    }
}
impl HistoryBudget {
    /// Smaller trusted fault-profile limit; no public client can choose budgets.
    pub fn for_testing(vm_ops: u32, elapsed: Duration) -> Self {
        Self {
            vm_ops: vm_ops.clamp(1, 100_000),
            elapsed: elapsed
                .min(Duration::from_millis(50))
                .max(Duration::from_micros(1)),
            stride: vm_ops.clamp(1, 1_000) as i32,
        }
    }
}

// Reset the callback before the worker resumes fact commits. A budget failure
// must never turn an ordinary SQLite transaction into a history interrupt.
struct HistoryProgress<'a>(&'a Connection);
impl<'a> HistoryProgress<'a> {
    fn install(connection: &'a Connection, budget: HistoryBudget) -> Result<Self, StorageError> {
        let started = Instant::now();
        let mut executed = 0u32;
        connection.progress_handler(
            budget.stride,
            Some(move || {
                executed = executed.saturating_add(budget.stride as u32);
                executed >= budget.vm_ops || started.elapsed() >= budget.elapsed
            }),
        )?;
        Ok(Self(connection))
    }
}
impl Drop for HistoryProgress<'_> {
    fn drop(&mut self) {
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

/// One raw committed measurement; publication and source observation times differ.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementRow {
    /// Immutable canonical record identity within its boot.
    pub record_sequence: u64,
    /// Monotonic publication time scoped to the archived boot.
    pub published_at: Duration,
    /// Monotonic source/model observation time.
    pub observed_at: Duration,
    /// Exact engineering unit symbol captured at commit.
    pub unit: String,
    /// `good` or `unavailable`; no interpolation is performed.
    pub quality: String,
    /// Exact typed value, absent for a failed attempt.
    pub value: Option<Value>,
    /// Failure reason when unavailable.
    pub failure: Option<String>,
    /// Source generation at commit.
    pub generation: u64,
    /// Source revision at commit.
    pub revision: u64,
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

/// Exact indexed archive filter; elapsed times are scoped to the selected boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryFilter {
    /// Archived process boot, which may differ from the serving boot.
    pub boot_id: String,
    /// Checked run counter within that archived boot.
    pub run_no: u64,
    /// Stable instrument identity.
    pub instrument: InstrumentId,
    /// Stable parameter identity within the instrument.
    pub parameter: ParameterId,
    /// Inclusive monotonic publication bound.
    pub from: Duration,
    /// Exclusive monotonic publication bound.
    pub to: Duration,
}

/// One server-retained keyset continuation at a frozen durable checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    filter: HistoryFilter,
    database_id: String,
    watermark: u64,
    last: (u64, u64),
    upper: (u64, u64),
}

/// At most 128 complete raw rows and one continuation token.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryPage {
    /// Matching committed rows, with Unavailable preserved as a raw row.
    pub rows: Vec<MeasurementRow>,
    /// Durable record checkpoint frozen on the first page.
    pub watermark: u64,
    /// Continuation only if more matching rows existed at that checkpoint.
    pub next_cursor: Option<HistoryCursor>,
    /// `complete`, `gap`, or `unknown_tail` interval/run coverage.
    pub coverage: String,
}

/// One archived run discoverable after a fresh process boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// Archive boot containing this run.
    pub boot_id: String,
    /// Checked run counter scoped to that boot.
    pub run_no: u64,
    /// Human-readable label, never an identity.
    pub label: String,
    /// Selected trusted recording policy.
    pub policy: String,
    /// `recording`, `sealed`, `interrupted`, or failed lifecycle state.
    pub state: String,
    /// `complete`, `gap`, or `unknown_tail` coverage.
    pub coverage: String,
}

/// One retained run-list keyset continuation, scoped to this database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunsCursor {
    database_id: String,
    last: ([u8; 16], u64),
    upper: ([u8; 16], u64),
}

/// At most 32 indexed run summaries and one continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunsPage {
    /// Archived runs in immutable boot/run key order.
    pub runs: Vec<RunSummary>,
    /// Continuation only if another run existed below the frozen upper key.
    pub next_cursor: Option<RunsCursor>,
}

/// Exact already-loaded trusted source/definition bytes; a path is never reread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceEntry {
    /// Stable semantic class, such as `managed_lua_source`.
    pub kind: String,
    /// Exact byte encoding, such as `utf8` or `json_v1`.
    pub encoding: String,
    /// Owned bytes captured from the active candidate before later file changes.
    pub content: Vec<u8>,
}

/// One frozen object baseline attached to an immutable activation. Entry links
/// are indices into the transferred source set; the worker hashes exact bytes.
#[derive(Clone, Debug)]
pub struct ProvenanceObject {
    /// Stable object family (`instrument`, `controller`, `reference`, `actuator`).
    pub kind: &'static str,
    /// Stable ID: eight bytes for one ID, sixteen for a composite signal/actuator.
    pub id: Vec<u8>,
    /// Stable logical key, separate from a display label.
    pub logical_key: String,
    /// Human-readable label captured at activation.
    pub label: String,
    /// Bounded canonical JSON of exact committed descriptor/configuration fields.
    pub descriptor: String,
    /// Unit identity, if this object has one.
    pub unit_key: Option<String>,
    /// Component generation or native immutable generation one.
    pub generation: Option<u64>,
    /// Immutable instance/output binding description if relevant.
    pub binding: Option<String>,
    /// Index of the exact definition/source entry covering this object.
    pub source_entry_index: usize,
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
}

impl SqliteStore {
    /// Create or reopen one local version-one file, keeping its database identity.
    /// Reopen never restores control authority, controller state or safe evidence.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|error| StorageError(format!("boot entropy unavailable: {error}")))?;
        let boot: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        Self::open_with_boot(path, &boot)
    }

    /// Open with the serving M6 Runtime boot ID; no second storage boot is invented.
    pub fn open_with_boot(path: &Path, boot: &str) -> Result<Self, StorageError> {
        let origin = Instant::now();
        let anchor = TimeAnchor::capture(|| origin.elapsed(), || Ok(SystemTime::now()))?;
        Self::open_with_boot_anchor(path, boot, anchor)
    }

    /// Open with an anchor bracketed against the serving process's monotonic
    /// origin. The worker owns all subsequent SQL; UTC remains display metadata.
    pub fn open_with_boot_anchor(
        path: &Path,
        boot: &str,
        anchor: TimeAnchor,
    ) -> Result<Self, StorageError> {
        if anchor.wall_us().is_none() {
            return Err(StorageError("boot UTC anchor unavailable".into()));
        }
        let boot_id = parse_boot_id(boot)?;
        if !path.is_absolute() || path.to_string_lossy().starts_with("\\\\") {
            return Err(StorageError(
                "storage path must be a local absolute path".into(),
            ));
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_millis(100))?;
        let application_id: i32 =
            connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let table_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        let new_file = application_id == 0 && version == 0 && table_count == 0;
        if !new_file && (application_id != APPLICATION_ID || version != SCHEMA_VERSION) {
            return Err(StorageError(
                "unknown or incompatible sqlite application/schema".into(),
            ));
        }
        if !new_file {
            let required: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
                 ('schema_version','runtime_boots','runs','recording_intervals',\
                  'configurations','provenance_content','object_snapshots','records',\
                  'measurements','operation_events','controller_events','reference_events',\
                  'output_events','runtime_events','gaps','durable_checkpoints','clock_anchors')",
                [],
                |row| row.get(0),
            )?;
            if required != 17 {
                return Err(StorageError(
                    "version-one schema tables are incomplete".into(),
                ));
            }
            let indexes: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN
                 ('runtime_boots_unfinished','runs_unfinished','intervals_unfinished',
                  'measurements_history','measurements_all_history','operation_identity')",
                [],
                |row| row.get(0),
            )?;
            if indexes != 6 {
                return Err(StorageError(
                    "version-one required indexes are incomplete".into(),
                ));
            }
            let columns: Vec<String> = connection
                .prepare("PRAGMA index_info('measurements_history')")?
                .query_map([], |row| row.get::<_, String>(2))?
                .collect::<Result<_, _>>()?;
            if columns
                != [
                    "boot_id",
                    "run_no",
                    "instrument_id",
                    "parameter_id",
                    "published_at",
                    "record_seq",
                ]
            {
                return Err(StorageError(
                    "version-one history index is incompatible".into(),
                ));
            }
            let (declared_schema, record_encoding, identity): (i64, i64, String) = connection
                .query_row(
                    "SELECT schema_version,record_encoding,database_id FROM schema_version
                     WHERE singleton=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            if declared_schema != SCHEMA_VERSION
                || record_encoding != 1
                || parse_boot_id(&identity).is_err()
            {
                return Err(StorageError(
                    "incompatible version-one record encoding/identity".into(),
                ));
            }
            validate_unfinished_entries(&connection)?;
        }
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let page_count: i64 =
            connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let page_size = u64::try_from(page_size)
            .map_err(|_| StorageError("invalid SQLite page size".into()))?;
        let page_count = u64::try_from(page_count)
            .map_err(|_| StorageError("invalid SQLite page count".into()))?;
        if !(512..=65_536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(StorageError("incompatible SQLite page size".into()));
        }
        let main_quota_pages = MAIN_FILE_QUOTA_BYTES / page_size;
        if page_count > main_quota_pages {
            return Err(StorageError(
                "existing main database exceeds one GiB".into(),
            ));
        }
        connection.pragma_update(
            None,
            "max_page_count",
            i64::try_from(main_quota_pages)
                .map_err(|_| StorageError("main page quota exceeds SQLite range".into()))?,
        )?;
        let actual_max: i64 =
            connection.pragma_query_value(None, "max_page_count", |row| row.get(0))?;
        if u64::try_from(actual_max).ok() != Some(main_quota_pages) {
            return Err(StorageError("SQLite main page quota unavailable".into()));
        }
        let locking_mode: String =
            connection.pragma_query_value(None, "locking_mode", |row| row.get(0))?;
        if locking_mode != "exclusive" {
            connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        }
        let actual: String =
            connection.pragma_query_value(None, "locking_mode", |row| row.get(0))?;
        if actual != "exclusive" {
            return Err(StorageError(
                "exclusive database ownership unavailable".into(),
            ));
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "wal_autocheckpoint", 1_000i64)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "cache_size", -2048)?;
        connection.pragma_update(None, "mmap_size", 0)?;
        connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 64 * 1024)?;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, 128 * 1024)?;
        // The optional rusqlite statement-cache feature is disabled, so there
        // are no retained prepared statements beyond the active operation.
        if new_file {
            create_schema(&mut connection)?;
        }
        let database_id: String = connection.query_row(
            "SELECT database_id FROM schema_version WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let transaction = connection.transaction()?;
        // An unfinished prior process is an unknown tail, not a failed action.
        transaction.execute(
            "UPDATE recording_intervals SET state='interrupted', coverage='unknown_tail' \
             WHERE state IN ('starting','recording','stopping')",
            [],
        )?;
        transaction.execute(
            "UPDATE runs SET state='interrupted', coverage='unknown_tail' \
             WHERE state IN ('starting','recording','stopping')",
            [],
        )?;
        transaction.execute(
            "UPDATE runtime_boots SET state='interrupted' WHERE state='active'",
            [],
        )?;
        transaction.execute(
            "INSERT INTO runtime_boots(boot_id,build_version,state,started_wall_us,
             anchor_before,anchor_after,anchor_uncertainty_ns)
             VALUES(?1,?2,'active',?3,?4,?5,?6)",
            params![
                boot_id.as_slice(),
                env!("CARGO_PKG_VERSION"),
                anchor.wall_us(),
                duration_blob(anchor.before())?.as_slice(),
                duration_blob(anchor.after())?.as_slice(),
                u64_blob(anchor.uncertainty_ns()).as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO clock_anchors(boot_id,anchor_no,kind,monotonic_before,
             monotonic_after,uncertainty_ns,wall_us,unavailable_reason)
             VALUES(?1,?2,'boot',?3,?4,?5,?6,NULL)",
            params![
                boot_id.as_slice(),
                u64_blob(1).as_slice(),
                duration_blob(anchor.before())?.as_slice(),
                duration_blob(anchor.after())?.as_slice(),
                u64_blob(anchor.uncertainty_ns()).as_slice(),
                anchor.wall_us()
            ],
        )?;
        transaction.execute(
            "INSERT INTO durable_checkpoints(boot_id,commit_no,persisted_through_seq) \
             VALUES(?1,?2,?3)",
            params![
                boot_id.as_slice(),
                u64_blob(0).as_slice(),
                u64_blob(0).as_slice()
            ],
        )?;
        transaction.commit()?;
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push("-wal");
        Ok(Self {
            connection,
            database_id,
            boot_id,
            boot_id_hex: boot.to_owned(),
            run_no: None,
            interval_no: None,
            next_run_no: 1,
            next_interval_no: 1,
            next_record_sequence: 0,
            commit_no: 0,
            next_activation_no: 1,
            current_activation_no: None,
            boot_sealed: false,
            coverage_gap: false,
            boot_anchor: anchor,
            next_anchor_no: 2,
            main_quota_pages,
            wal_path: PathBuf::from(sidecar),
            wal_threshold_bytes: WAL_THRESHOLD_BYTES,
            wal_checkpoints: 0,
        })
    }

    /// Worker-side observation of page size, logical main page count and the
    /// active hard cap. This is storage metadata, never a Runtime Query.
    pub fn storage_pages(&self) -> Result<(u64, u64, u64), StorageError> {
        let page_size: i64 = self
            .connection
            .pragma_query_value(None, "page_size", |row| row.get(0))?;
        let page_count: i64 = self
            .connection
            .pragma_query_value(None, "page_count", |row| row.get(0))?;
        let max_pages: i64 = self
            .connection
            .pragma_query_value(None, "max_page_count", |row| row.get(0))?;
        Ok((
            u64::try_from(page_size).map_err(|_| StorageError("invalid page size".into()))?,
            u64::try_from(page_count).map_err(|_| StorageError("invalid page count".into()))?,
            u64::try_from(max_pages).map_err(|_| StorageError("invalid max page count".into()))?,
        ))
    }

    /// Trusted real-file fault profile for testing the reserve boundary with
    /// small files; it cannot enlarge the production one-GiB cap.
    pub fn lower_main_quota_for_testing(&mut self, pages: u64) -> Result<(), StorageError> {
        let (_, current, _) = self.storage_pages()?;
        if pages < current || pages > self.main_quota_pages {
            return Err(StorageError("invalid smaller main page quota".into()));
        }
        self.connection.pragma_update(
            None,
            "max_page_count",
            i64::try_from(pages)
                .map_err(|_| StorageError("test page quota exceeds range".into()))?,
        )?;
        let (_, _, actual) = self.storage_pages()?;
        if actual != pages {
            return Err(StorageError("test page quota was not applied".into()));
        }
        self.main_quota_pages = pages;
        Ok(())
    }

    /// Worker-only WAL observation: actual sidecar bytes and the count of
    /// successful explicit threshold checkpoints in this boot.
    pub fn wal_health(&self) -> Result<(u64, u64), StorageError> {
        let bytes = match std::fs::metadata(&self.wal_path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(StorageError(format!("WAL metadata: {error}"))),
        };
        Ok((bytes, self.wal_checkpoints))
    }

    /// Trusted small real-file limit for checkpoint fault acceptance. It may
    /// lower but never enlarge the production sixteen-MiB threshold.
    pub fn lower_wal_threshold_for_testing(&mut self, bytes: u64) -> Result<(), StorageError> {
        if bytes == 0 || bytes > WAL_THRESHOLD_BYTES {
            return Err(StorageError("invalid smaller WAL threshold".into()));
        }
        self.wal_threshold_bytes = bytes;
        Ok(())
    }

    fn require_wal_budget(&mut self) -> Result<(), StorageError> {
        let (bytes, _) = self.wal_health()?;
        if bytes < self.wal_threshold_bytes {
            return Ok(());
        }
        let (busy, _log, _checkpointed): (i64, i64, i64) =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        if busy != 0 || self.wal_health()?.0 >= self.wal_threshold_bytes {
            return Err(StorageError("WAL threshold checkpoint failed".into()));
        }
        self.wal_checkpoints = self
            .wal_checkpoints
            .checked_add(1)
            .ok_or_else(|| StorageError("WAL checkpoint counter exhausted".into()))?;
        Ok(())
    }

    fn require_main_reserve(&self) -> Result<(), StorageError> {
        let (_, page_count, max_pages) = self.storage_pages()?;
        if max_pages != self.main_quota_pages
            || page_count >= self.main_quota_pages.saturating_mul(95) / 100
        {
            return Err(StorageError(
                "main database five-percent reserve exhausted".into(),
            ));
        }
        Ok(())
    }

    /// Commit one independently observed UTC anchor without changing the
    /// initial boot mapping or any domain fact's monotonic timestamp.
    pub fn append_clock_anchor(
        &mut self,
        kind: &str,
        anchor: &TimeAnchor,
    ) -> Result<(), StorageError> {
        if !matches!(kind, "periodic" | "interval_start" | "interval_end") {
            return Err(StorageError("invalid clock anchor kind".into()));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let next = self
            .next_anchor_no
            .checked_add(1)
            .ok_or_else(|| StorageError("clock anchor identity exhausted".into()))?;
        let sequence = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("clock record identity exhausted".into()))?;
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("clock commit identity exhausted".into()))?;
        let time = duration_blob(anchor.after())?;
        let wall_estimate = self.boot_anchor.estimate_us(anchor.after())?;
        let payload = serde_json::json!({
            "kind":kind,
            "monotonic_before_ns":anchor.before().as_nanos().to_string(),
            "monotonic_after_ns":anchor.after().as_nanos().to_string(),
            "uncertainty_ns":anchor.uncertainty_ns().to_string(),
            "actual_wall_us":anchor.wall_us(),
            "unavailable_reason":anchor.unavailable_reason(),
            "wall_basis":"boot_anchor"
        })
        .to_string();
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,
             published_at,observed_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
             VALUES(?1,?2,?3,?4,'clock_anchor',1,?5,?5,?5,?6,'boot_anchor',
             'recorder_clock',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                self.run_no.map(u64_blob).map(|value| value.to_vec()),
                self.interval_no.map(u64_blob).map(|value| value.to_vec()),
                time.as_slice(),
                wall_estimate,
                payload.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
             VALUES(?1,?2,'clock','info',?3,?4)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                kind,
                payload.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO clock_anchors(boot_id,anchor_no,record_seq,kind,monotonic_before,
             monotonic_after,uncertainty_ns,wall_us,unavailable_reason)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                self.boot_id.as_slice(),
                u64_blob(self.next_anchor_no).as_slice(),
                u64_blob(sequence).as_slice(),
                kind,
                duration_blob(anchor.before())?.as_slice(),
                duration_blob(anchor.after())?.as_slice(),
                u64_blob(anchor.uncertainty_ns()).as_slice(),
                anchor.wall_us(),
                anchor.unavailable_reason()
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3
             WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                u64_blob(sequence).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_anchor_no = next;
        self.next_record_sequence = sequence;
        self.commit_no = commit;
        Ok(())
    }

    /// Return the stable identity retained across process reopen.
    pub fn database_id(&self) -> &str {
        &self.database_id
    }

    /// Exact process boot identity shared with the M6 public server.
    pub fn boot_id(&self) -> &str {
        &self.boot_id_hex
    }

    /// Checked current run counter within this process boot, when recording.
    pub fn current_run_no(&self) -> Option<u64> {
        self.run_no
    }

    /// Checked current interval counter within this process boot, when recording.
    pub fn current_interval_no(&self) -> Option<u64> {
        self.interval_no
    }

    /// Last committed canonical record identity for this serving boot.
    pub fn current_record_sequence(&self) -> u64 {
        self.next_record_sequence
    }

    /// Commit one bounded immutable activation manifest and deduplicated bytes.
    /// SQL is performed only on the storage worker or in an offline test.
    pub fn commit_provenance(
        &mut self,
        entries: &[ProvenanceEntry],
    ) -> Result<[u8; 32], StorageError> {
        self.commit_activation(entries, &[])
    }

    /// Commit one frozen source set and object baseline in the same transaction.
    pub fn commit_activation(
        &mut self,
        entries: &[ProvenanceEntry],
        objects: &[ProvenanceObject],
    ) -> Result<[u8; 32], StorageError> {
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        if entries.is_empty() || entries.len() > 128 {
            return Err(StorageError("provenance count must be 1..=128".into()));
        }
        let mut charge = 0usize;
        let mut indexed = Vec::with_capacity(entries.len());
        let mut entry_hashes = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.kind.is_empty()
                || entry.kind.len() > 64
                || entry.encoding.is_empty()
                || entry.encoding.len() > 64
                || entry.content.is_empty()
                || entry.content.len() > 64 * 1024
            {
                return Err(StorageError("invalid bounded provenance entry".into()));
            }
            charge = charge
                .checked_add(
                    entry.kind.capacity()
                        + entry.encoding.capacity()
                        + entry.content.capacity()
                        + 128,
                )
                .ok_or_else(|| StorageError("provenance credit exhausted".into()))?;
            if charge > 1024 * 1024 {
                return Err(StorageError("provenance credit exhausted".into()));
            }
            let content_hash: [u8; 32] = Sha256::digest(&entry.content).into();
            entry_hashes.push(content_hash);
            indexed.push((entry, content_hash));
        }
        if objects.len() > 256 {
            return Err(StorageError("object baseline count exhausted".into()));
        }
        let mut object_charge = 0usize;
        for object in objects {
            if !matches!(object.id.len(), 8 | 16)
                || object.kind.len() > 64
                || object.logical_key.len() > 128
                || object.label.len() > 128
                || object.descriptor.len() > 4096
                || object.descriptor.is_empty()
                || object.unit_key.as_ref().is_some_and(|unit| unit.len() > 32)
                || object
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.len() > 128)
                || object.source_entry_index >= entry_hashes.len()
            {
                return Err(StorageError("invalid bounded object baseline".into()));
            }
            serde_json::from_str::<serde_json::Value>(&object.descriptor)
                .map_err(|_| StorageError("invalid object descriptor JSON".into()))?;
            object_charge = object_charge
                .checked_add(
                    object.id.capacity()
                        + object.logical_key.capacity()
                        + object.label.capacity()
                        + object.descriptor.capacity()
                        + object.unit_key.as_ref().map_or(0, String::capacity)
                        + object.binding.as_ref().map_or(0, String::capacity)
                        + 256,
                )
                .ok_or_else(|| StorageError("object baseline credit exhausted".into()))?;
            if object_charge > 256 * 1024 {
                return Err(StorageError("object baseline credit exhausted".into()));
            }
        }
        indexed.sort_by(|left, right| {
            (&left.0.kind, &left.0.encoding, left.1).cmp(&(
                &right.0.kind,
                &right.0.encoding,
                right.1,
            ))
        });
        for pair in indexed.windows(2) {
            if pair[0].0.kind == pair[1].0.kind
                && pair[0].0.encoding == pair[1].0.encoding
                && pair[0].1 == pair[1].1
                && pair[0].0.content != pair[1].0.content
            {
                return Err(StorageError("provenance hash/content collision".into()));
            }
        }
        indexed.dedup_by(|left, right| {
            left.0.kind == right.0.kind
                && left.0.encoding == right.0.encoding
                && left.1 == right.1
                && left.0.content == right.0.content
        });
        // The root is the hash of actual stored canonical JSON, so inspection
        // can reconstruct the typed entry references without executable code.
        let refs: Vec<serde_json::Value> = indexed
            .iter()
            .map(|(entry, hash)| {
                serde_json::json!({"kind":entry.kind,"encoding":entry.encoding,
                "sha256":hex_hash(hash)})
            })
            .collect();
        let manifest = serde_json::json!({"encoding_version":1,
            "hash_algorithm":"sha256","entries":refs})
        .to_string()
        .into_bytes();
        if manifest.len() > 64 * 1024
            || charge
                .checked_add(manifest.len() + 128)
                .is_none_or(|total| total > 1024 * 1024)
        {
            return Err(StorageError("provenance manifest credit exhausted".into()));
        }
        let root: [u8; 32] = Sha256::digest(&manifest).into();
        let activation_no = self.next_activation_no;
        let next = activation_no
            .checked_add(1)
            .ok_or_else(|| StorageError("activation identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        for (entry, hash) in indexed {
            let existing: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT content FROM provenance_content WHERE content_hash=?1",
                    params![hash.as_slice()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(content) = existing {
                if content != entry.content {
                    return Err(StorageError("provenance hash/content collision".into()));
                }
            } else {
                transaction.execute(
                    "INSERT INTO provenance_content(content_hash,encoding,kind,content)
                     VALUES(?1,?2,?3,?4)",
                    params![hash.as_slice(), entry.encoding, entry.kind, entry.content],
                )?;
            }
        }
        let existing_root: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT content FROM provenance_content WHERE content_hash=?1",
                params![root.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(content) = existing_root {
            if content != manifest {
                return Err(StorageError(
                    "provenance root hash/content collision".into(),
                ));
            }
        } else {
            transaction.execute(
                "INSERT INTO provenance_content(content_hash,encoding,kind,content)
                 VALUES(?1,'json_v1','activation_manifest',?2)",
                params![root.as_slice(), manifest],
            )?;
        }
        transaction.execute(
            "INSERT INTO configurations(boot_id,activation_no,manifest_root_hash,
             manifest_content_hash,encoding) VALUES(?1,?2,?3,?3,'manifest_v1')",
            params![
                self.boot_id.as_slice(),
                u64_blob(activation_no).as_slice(),
                root.as_slice()
            ],
        )?;
        for object in objects {
            let source = entry_hashes[object.source_entry_index];
            let source_hash = (object.kind == "managed_component").then_some(source.to_vec());
            let safety_hash = (object.kind == "actuator").then_some(source.to_vec());
            transaction.execute(
                "INSERT INTO object_snapshots(boot_id,activation_no,object_kind,object_id,
                 logical_key,label,generation,descriptor,unit_key,instance_binding,
                 definition_hash,source_hash,safety_hash)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    self.boot_id.as_slice(),
                    u64_blob(activation_no).as_slice(),
                    object.kind,
                    object.id.as_slice(),
                    object.logical_key,
                    object.label,
                    object.generation.map(u64_blob).map(|id| id.to_vec()),
                    object.descriptor,
                    object.unit_key,
                    object.binding,
                    source.as_slice(),
                    source_hash,
                    safety_hash
                ],
            )?;
        }
        transaction.commit()?;
        self.next_activation_no = next;
        self.current_activation_no = Some(activation_no);
        Ok(root)
    }

    /// Verify the worker connection's actual WAL/FULL/foreign-key settings.
    pub fn sqlite_settings(&self) -> Result<SqliteSettings, StorageError> {
        let journal_mode: String =
            self.connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let synchronous: i64 = self
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))?;
        let foreign_keys: i64 =
            self.connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        let user_version: i64 =
            self.connection
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(SqliteSettings {
            journal_mode,
            synchronous,
            foreign_keys: foreign_keys == 1,
            user_version,
        })
    }

    /// Commit a new run and recording interval before accepting its facts.
    pub fn start_run(&mut self, label: &str) -> Result<(), StorageError> {
        self.start_run_with_policy(label, RecordingPolicy::Required)
    }

    /// Commit the host-selected immutable policy with the run's start barrier.
    pub fn start_run_with_policy(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
    ) -> Result<(), StorageError> {
        self.start_run_with_boundary(label, policy, &BoundarySnapshot::empty(Duration::ZERO))
    }

    /// Atomically bind a new run, its interval and the owner-frozen boundary.
    pub fn start_run_with_boundary(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
    ) -> Result<(), StorageError> {
        self.start_run_impl(label, policy, boundary, None)
    }

    /// Commit the actual interval-start UTC bracket in the same durable
    /// transaction as the frozen boundary and its checkpoint.
    pub fn start_run_with_boundary_anchor(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
        anchor: &TimeAnchor,
    ) -> Result<(), StorageError> {
        self.start_run_impl(label, policy, boundary, Some(anchor))
    }

    fn start_run_impl(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
        anchor: Option<&TimeAnchor>,
    ) -> Result<(), StorageError> {
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        if self.run_no.is_some()
            || label.len() > 128
            || label.trim().is_empty()
            || !boundary.valid()
        {
            return Err(StorageError("invalid or overlapping recording run".into()));
        }
        let run_no = self.next_run_no;
        let interval_no = self.next_interval_no;
        let following_run = run_no
            .checked_add(1)
            .ok_or_else(|| StorageError("run identity exhausted".into()))?;
        let following_interval = interval_no
            .checked_add(1)
            .ok_or_else(|| StorageError("interval identity exhausted".into()))?;
        let boundary_record = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("boundary record identity exhausted".into()))?;
        let anchor_record = anchor
            .map(|_| {
                boundary_record
                    .checked_add(1)
                    .ok_or_else(|| StorageError("start anchor record identity exhausted".into()))
            })
            .transpose()?;
        let final_record = anchor_record.unwrap_or(boundary_record);
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("start clock anchor identity exhausted".into()))
            })
            .transpose()?;
        let boundary_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("boundary commit identity exhausted".into()))?;
        let at = duration_blob(boundary.at)?;
        let boundary_wall = self.boot_anchor.estimate_us(boundary.at)?;
        let transaction = self.connection.transaction()?;
        let policy = match policy {
            RecordingPolicy::Required => "required",
            RecordingPolicy::BestEffort => "best-effort",
        };
        transaction.execute(
            "INSERT INTO runs(boot_id,run_no,label,policy,state,coverage,initial_activation_id,
             started_wall_us) VALUES(?1,?2,?3,?4,'recording','complete',?5,?6)",
            params![
                self.boot_id.as_slice(),
                u64_blob(run_no).as_slice(),
                label,
                policy,
                self.current_activation_no
                    .map(u64_blob)
                    .map(|id| id.to_vec()),
                anchor.and_then(TimeAnchor::wall_us),
            ],
        )?;
        transaction.execute(
            "INSERT INTO recording_intervals(boot_id,interval_no,run_no,state,coverage) \
             VALUES(?1,?2,?3,'recording','complete')",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                u64_blob(run_no).as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,
            kind,version,published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
            VALUES(?1,?2,?3,?4,'boundary_snapshot',1,?5,?5,?6,
            'boot_anchor','host_lifecycle',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_record).as_slice(),
                u64_blob(run_no).as_slice(),
                u64_blob(interval_no).as_slice(),
                at.as_slice(),
                boundary_wall,
                boundary.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,
            severity,code,data) VALUES(?1,?2,'boundary','info','run_start',?3)",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_record).as_slice(),
                boundary.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE recording_intervals SET boundary_seq=?3,start_seq=?3
            WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                u64_blob(boundary_record).as_slice()
            ],
        )?;
        if let (Some(anchor), Some(sequence)) = (anchor, anchor_record) {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                sequence,
                self.next_anchor_no,
                "interval_start",
                anchor,
                Some(run_no),
                Some(interval_no),
                &self.boot_anchor,
            )?;
        }
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,
            persisted_through_seq=?3,coverage='complete' WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_commit).as_slice(),
                u64_blob(final_record).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.run_no = Some(run_no);
        self.interval_no = Some(interval_no);
        self.next_run_no = following_run;
        self.next_interval_no = following_interval;
        self.next_record_sequence = final_record;
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.commit_no = boundary_commit;
        Ok(())
    }

    /// Atomically commit one bounded fact group and its cumulative checkpoint.
    /// Call only on the storage worker; the Runtime owner never waits here.
    pub fn append_facts(&mut self, facts: &[RecordingFact]) -> Result<u64, StorageError> {
        let capture = facts
            .last()
            .map(|fact| match fact {
                RecordingFact::Measurement { sample, .. } => sample.at(),
                RecordingFact::Output { at, .. }
                | RecordingFact::Controller { at, .. }
                | RecordingFact::Reference { at, .. } => *at,
            })
            .unwrap_or(Duration::ZERO);
        self.append_facts_with_capture(facts, capture)
    }

    /// Commit an owner group with its original host-admission time, independent
    /// of each observation's occurrence and publication timestamps.
    pub fn append_facts_with_capture(
        &mut self,
        facts: &[RecordingFact],
        captured_at: Duration,
    ) -> Result<u64, StorageError> {
        self.append_fact_groups(&[(facts, captured_at)])
    }

    /// Commit several causal owner groups without splitting any group. The
    /// worker retains their original capture times and one bounded batch.
    pub fn append_fact_groups(
        &mut self,
        groups: &[(&[RecordingFact], Duration)],
    ) -> Result<u64, StorageError> {
        let count = groups
            .iter()
            .try_fold(0usize, |total, (facts, _)| total.checked_add(facts.len()))
            .ok_or_else(|| StorageError("batch count arithmetic exhausted".into()))?;
        if groups.is_empty()
            || groups.len() > 4
            || count == 0
            || count > 256
            || groups.iter().any(|(facts, _)| facts.is_empty())
            || self.run_no.is_none()
        {
            return Err(StorageError("invalid or oversized recording batch".into()));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let mut next_sequence = self.next_record_sequence;
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("commit identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        for &(facts, captured_at) in groups {
            for fact in facts {
                validate_storage_fact(fact)?;
                next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("record identity exhausted".into()))?;
                let record_id = u64_blob(next_sequence);
                let fact_id = u64_blob(fact.sequence());
                let run_no = u64_blob(self.run_no.expect("run checked above"));
                let interval_no = u64_blob(self.interval_no.expect("run checked above"));
                let (kind, time) = match fact {
                    RecordingFact::Measurement { sample, .. } => ("measurement", sample.at()),
                    RecordingFact::Output { at, .. } => ("output", *at),
                    RecordingFact::Controller { at, .. } => ("controller", *at),
                    RecordingFact::Reference { at, .. } => ("reference", *at),
                };
                let at = duration_blob(time)?;
                let observed = match fact {
                    RecordingFact::Measurement { sample, .. } => sample.freshness_at(),
                    _ => time,
                };
                let observed = duration_blob(observed)?;
                let captured = duration_blob(captured_at)?;
                let wall_estimate = self.boot_anchor.estimate_us(time)?;
                transaction.execute(
                    "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
                 fact_seq,published_at,observed_at,captured_at,wall_estimate_us,wall_basis)
                 VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,'boot_anchor')",
                    params![
                        self.boot_id.as_slice(),
                        record_id.as_slice(),
                        run_no.as_slice(),
                        interval_no.as_slice(),
                        kind,
                        fact_id.as_slice(),
                        at.as_slice(),
                        observed.as_slice(),
                        captured.as_slice(),
                        wall_estimate
                    ],
                )?;
                match fact {
                    RecordingFact::Measurement {
                        sample,
                        generation,
                        revision,
                        ..
                    } => {
                        let (value_kind, float_value, integer_value, bool_value, text_value) =
                            match sample.value() {
                                Some(Value::Float(value)) if value.is_finite() => {
                                    ("float", Some(*value), None, None, None)
                                }
                                Some(Value::Integer(value)) => {
                                    ("integer", None, Some(*value), None, None)
                                }
                                Some(Value::Boolean(value)) => {
                                    ("boolean", None, None, Some(i64::from(*value)), None)
                                }
                                Some(Value::Text(value)) => {
                                    ("text", None, None, None, Some(value.as_str()))
                                }
                                Some(Value::Enum(value)) => {
                                    ("enum", None, None, None, Some(value.as_str()))
                                }
                                Some(Value::Float(_)) => {
                                    return Err(StorageError("nonfinite measurement".into()));
                                }
                                None => ("none", None, None, None, None),
                            };
                        let quality = match sample.quality() {
                            SampleQuality::Good => "good",
                            SampleQuality::Unavailable => "unavailable",
                        };
                        let failure = sample.failure().map(|reason| format!("{reason:?}"));
                        transaction.execute(
                        "INSERT INTO measurements(boot_id,record_seq,run_no,instrument_id,\
                         parameter_id,generation,revision,observed_at,published_at,unit_key,\
                         quality,failure,value_kind,float_value,integer_value,bool_value,text_value) \
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                        params![self.boot_id.as_slice(), record_id.as_slice(), run_no.as_slice(),
                            u64_blob(sample.signal().instrument().get()).as_slice(),
                            u64_blob(sample.signal().parameter().get()).as_slice(),
                            u64_blob(*generation).as_slice(), u64_blob(*revision).as_slice(),
                            duration_blob(sample.freshness_at())?.as_slice(), at.as_slice(),
                            sample.unit().id(), quality, failure, value_kind,
                            float_value, integer_value, bool_value, text_value],
                    )?;
                    }
                    RecordingFact::Output {
                        actuator,
                        attempt_id,
                        dispatch_id,
                        stage,
                        value,
                        source,
                        ..
                    } => {
                        let stage = match stage {
                            OutputStage::RejectedBeforeSend => "rejected_before_send",
                            OutputStage::ExpiredBeforeSend => "expired_before_send",
                            OutputStage::Requested => "requested",
                            OutputStage::Authorized => "authorized",
                            OutputStage::SendStarted => "send_started",
                            OutputStage::Acknowledged => "acknowledged",
                            OutputStage::ReadbackVerified => "readback_verified",
                            OutputStage::Failed => "failed",
                            OutputStage::Ambiguous => "ambiguous",
                            OutputStage::TransportUncertain => "transport_uncertain",
                            OutputStage::SafeRequested => "safe_requested",
                            OutputStage::SafeSendStarted => "safe_send_started",
                            OutputStage::SafeAcknowledged => "safe_acknowledged",
                            OutputStage::SafeReadbackVerified => "safe_readback_verified",
                            OutputStage::Revoked => "revoked",
                        };
                        let evidence_source = match source {
                            lab_core::recording::OutputEvidenceSource::None => "none",
                            lab_core::recording::OutputEvidenceSource::VirtualSimulation => {
                                "virtual_simulation"
                            }
                            lab_core::recording::OutputEvidenceSource::TransportProtocol => {
                                "transport_protocol"
                            }
                        };
                        let attempt_blob = attempt_id.map(u64_blob);
                        let dispatch_blob = dispatch_id.map(|id| {
                            let (instance, sequence) = id.diagnostic_parts();
                            let mut bytes = [0u8; 16];
                            bytes[..8].copy_from_slice(&instance.to_be_bytes());
                            bytes[8..].copy_from_slice(&sequence.to_be_bytes());
                            bytes
                        });
                        transaction.execute(
                            "INSERT INTO output_events(boot_id,record_seq,attempt_id,dispatch_id,\
                         instrument_id,parameter_id,stage,value,evidence_source,evidence_basis) \
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'runtime')",
                            params![
                                self.boot_id.as_slice(),
                                record_id.as_slice(),
                                attempt_blob.as_ref().map(|blob| blob.as_slice()),
                                dispatch_blob.as_ref().map(|blob| blob.as_slice()),
                                u64_blob(actuator.instrument().get()).as_slice(),
                                u64_blob(actuator.parameter().get()).as_slice(),
                                stage,
                                value,
                                evidence_source
                            ],
                        )?;
                    }
                    RecordingFact::Controller {
                        controller,
                        state,
                        config_revision,
                        pid,
                        ..
                    } => {
                        let state = format!("{state:?}").to_ascii_lowercase();
                        let settings = pid.map(|pid| {
                            serde_json::json!({
                                "kp":pid.kp,"ki":pid.ki,"kd":pid.kd,
                                "output_min":pid.output_min,"output_max":pid.output_max,
                            })
                            .to_string()
                        });
                        transaction.execute(
                        "INSERT INTO controller_events(boot_id,record_seq,controller_id,after_state,
                         event_kind,config_revision,diagnostics)
                         VALUES(?1,?2,?3,?4,?5,?6,?7)",
                        params![self.boot_id.as_slice(),record_id.as_slice(),
                            u64_blob(controller.get()).as_slice(),state,
                            if pid.is_some() {"configuration"} else {"lifecycle"},
                            u64_blob(*config_revision).as_slice(),settings],
                    )?;
                    }
                    RecordingFact::Reference {
                        reference,
                        revision,
                        value,
                        target,
                        rate,
                        unit,
                        at,
                        ..
                    } => {
                        transaction.execute(
                            "INSERT INTO reference_events(boot_id,record_seq,reference_id,revision,
                         event_kind,value,target,rate,unit_key,progress_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                            params![
                                self.boot_id.as_slice(),
                                record_id.as_slice(),
                                u64_blob(reference.get()).as_slice(),
                                u64_blob(*revision).as_slice(),
                                if target.is_some() { "ramp" } else { "fixed" },
                                value,
                                target,
                                rate,
                                unit.id(),
                                duration_blob(*at)?.as_slice()
                            ],
                        )?;
                    }
                }
            }
        }
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                u64_blob(next_sequence).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = next_sequence;
        self.commit_no = next_commit;
        Ok(next_sequence)
    }

    /// Atomically commit one application fact and its checkpoint on the worker.
    /// The transaction can preserve an accepted-only result after a process crash.
    pub fn append_operation(&mut self, operation: &OperationRecord) -> Result<u64, StorageError> {
        if self.run_no.is_none() || !operation.valid() {
            return Err(StorageError("invalid operation recording fact".into()));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let sequence = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("operation record identity exhausted".into()))?;
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("operation commit identity exhausted".into()))?;
        let run = self.run_no.expect("checked above");
        let interval = self.interval_no.expect("active run owns interval");
        let wall_estimate = self.boot_anchor.estimate_us(operation.at)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
             published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
             VALUES(?1,?2,?3,?4,'operation',1,?5,?5,?6,'boot_anchor',
             'application',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                u64_blob(run).as_slice(),
                u64_blob(interval).as_slice(),
                duration_blob(operation.at)?.as_slice(),
                wall_estimate,
                operation.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO operation_events(boot_id,record_seq,request_scope,request_seq,\
             phase,command,result,outcome_basis) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                operation.scope,
                operation.request_seq.to_string(),
                operation.phase,
                operation.command,
                operation.data,
                operation.outcome_basis
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                u64_blob(sequence).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = sequence;
        self.commit_no = commit;
        Ok(sequence)
    }

    /// Commit a real durable progress marker even during an otherwise quiet run.
    /// A queue heartbeat is not a substitute for this SQL transaction.
    pub fn probe(&mut self, submitted_at: Duration) -> Result<(), StorageError> {
        if self.run_no.is_none() {
            return Err(StorageError("progress probe outside recording".into()));
        }
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("commit identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,last_probe_at=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                duration_blob(submitted_at)?.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.commit_no = next_commit;
        Ok(())
    }

    /// Seal the current run in a transaction; no later facts enter that interval.
    pub fn stop_run(&mut self) -> Result<(), StorageError> {
        self.stop_run_impl(None)
    }

    /// Seal an interval with its actual UTC end bracket and final clock fact
    /// in the same transaction as the sealed coverage/checkpoint rows.
    pub fn stop_run_with_anchor(&mut self, anchor: &TimeAnchor) -> Result<(), StorageError> {
        self.stop_run_impl(Some(anchor))
    }

    fn stop_run_impl(&mut self, anchor: Option<&TimeAnchor>) -> Result<(), StorageError> {
        let Some(run_no) = self.run_no else {
            return Err(StorageError("no active recording run".into()));
        };
        let interval_no = self.interval_no.expect("run exists");
        let anchor_record = anchor
            .map(|_| {
                self.next_record_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("end anchor record identity exhausted".into()))
            })
            .transpose()?;
        let anchor_commit = anchor
            .map(|_| {
                self.commit_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("end anchor commit identity exhausted".into()))
            })
            .transpose()?;
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("end clock anchor identity exhausted".into()))
            })
            .transpose()?;
        let transaction = self.connection.transaction()?;
        if let (Some(anchor), Some(sequence)) = (anchor, anchor_record) {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                sequence,
                self.next_anchor_no,
                "interval_end",
                anchor,
                Some(run_no),
                Some(interval_no),
                &self.boot_anchor,
            )?;
        }
        transaction.execute(
            "UPDATE recording_intervals SET state='sealed',end_seq=?3
             WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                anchor_record.map(u64_blob).map(|value| value.to_vec())
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state='sealed',ended_wall_us=?3 WHERE boot_id=?1 AND run_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(run_no).as_slice(),
                anchor.and_then(TimeAnchor::wall_us)
            ],
        )?;
        if let (Some(sequence), Some(commit)) = (anchor_record, anchor_commit) {
            transaction.execute(
                "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3
                 WHERE boot_id=?1",
                params![
                    self.boot_id.as_slice(),
                    u64_blob(commit).as_slice(),
                    u64_blob(sequence).as_slice()
                ],
            )?;
        }
        transaction.commit()?;
        if let Some(sequence) = anchor_record {
            self.next_record_sequence = sequence;
        }
        if let Some(commit) = anchor_commit {
            self.commit_no = commit;
        }
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.run_no = None;
        self.interval_no = None;
        Ok(())
    }

    /// Seal a writable failed interval after its already-admitted FIFO prefix.
    /// No missing fact is fabricated; the typed gap names the known source
    /// boundary and the checkpoint names only bytes this transaction committed.
    pub fn fail_run(&mut self, gap: &RecorderGap) -> Result<u64, StorageError> {
        if !gap.valid() {
            return Err(StorageError("invalid bounded recorder gap".into()));
        }
        let run = self
            .run_no
            .ok_or_else(|| StorageError("gap seal without active run".into()))?;
        let interval = self
            .interval_no
            .ok_or_else(|| StorageError("gap interval missing".into()))?;
        let sequence = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("gap record identity exhausted".into()))?;
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("gap commit identity exhausted".into()))?;
        let last_missing = gap.first_missing_fact.and_then(|first| {
            gap.known_missing_count
                .and_then(|count| first.checked_add(count - 1))
        });
        let summary = serde_json::json!({"reason":gap.reason,
            "first_missing_fact":gap.first_missing_fact.map(|id|id.to_string()),
            "known_missing_count":gap.known_missing_count.map(|count|count.to_string()),
            "last_accepted_fact":gap.last_accepted_fact.map(|id|id.to_string()),
            "cutoff_monotonic_ns":gap.at.as_nanos().to_string(),
            "last_confirmed_record_seq":self.next_record_sequence.to_string()})
        .to_string();
        let record = u64_blob(sequence);
        let cutoff = duration_blob(gap.at)?;
        let wall_estimate = self.boot_anchor.estimate_us(gap.at)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,
            kind,version,published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
            VALUES(?1,?2,?3,?4,'recorder_gap',1,?5,?5,?6,
            'boot_anchor','host_lifecycle',?7)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                u64_blob(run).as_slice(),
                u64_blob(interval).as_slice(),
                cutoff.as_slice(),
                wall_estimate,
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO gaps(boot_id,record_seq,interval_no,first_missing,
            last_missing,known_count,unknown_tail,reason,last_confirmed)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                u64_blob(interval).as_slice(),
                gap.first_missing_fact.map(u64_blob).map(|id| id.to_vec()),
                last_missing.map(u64_blob).map(|id| id.to_vec()),
                gap.known_missing_count.map(u64_blob).map(|id| id.to_vec()),
                i64::from(gap.known_missing_count.is_none()),
                gap.reason,
                u64_blob(self.next_record_sequence).as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,
            severity,code,data) VALUES(?1,?2,'recorder_gap','error','coverage_lost',?3)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE recording_intervals SET state='failed',coverage='gap',
            end_seq=?3,loss_summary=?4 WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval).as_slice(),
                record.as_slice(),
                summary
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state='failed',coverage='gap' WHERE boot_id=?1
            AND run_no=?2",
            params![self.boot_id.as_slice(), u64_blob(run).as_slice()],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,
            persisted_through_seq=?3,coverage='gap' WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                record.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = sequence;
        self.commit_no = commit;
        self.run_no = None;
        self.interval_no = None;
        self.coverage_gap = true;
        Ok(sequence)
    }

    /// Commit a terminal boot seal after every accepted run/fact barrier.
    /// Presence of this row proves SQL commit, independently of its later receipt.
    pub fn finish_boot(&mut self, at: Duration) -> Result<(), StorageError> {
        self.finish_boot_with_summary(
            at,
            &serde_json::json!({"safety_evidence":"unprovided"}),
            None,
        )
    }

    /// Atomically seal the frozen trusted shutdown observation and actual UTC anchor.
    pub fn finish_boot_with_summary(
        &mut self,
        at: Duration,
        evidence: &serde_json::Value,
        anchor: Option<&TimeAnchor>,
    ) -> Result<(), StorageError> {
        if self.run_no.is_some() || self.boot_sealed {
            return Err(StorageError(
                "boot finish requires a stopped, unsealed recorder".into(),
            ));
        }
        let evidence_bytes = serde_json::to_vec(evidence)
            .map_err(|error| StorageError(format!("shutdown evidence: {error}")))?;
        if evidence_bytes.len() > 16 * 1024 || !evidence.is_object() {
            return Err(StorageError("invalid bounded shutdown evidence".into()));
        }
        let anchor_sequence = anchor
            .map(|_| {
                self.next_record_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("terminal clock identity exhausted".into()))
            })
            .transpose()?;
        let anchor_no = anchor.map(|_| self.next_anchor_no);
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("terminal anchor identity exhausted".into()))
            })
            .transpose()?;
        let next_record = self
            .next_record_sequence
            .checked_add(if anchor.is_some() { 2 } else { 1 })
            .ok_or_else(|| StorageError("terminal record identity exhausted".into()))?;
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("terminal commit identity exhausted".into()))?;
        let record = u64_blob(next_record);
        let published = duration_blob(at)?;
        let mut summary = evidence.as_object().expect("validated object").clone();
        summary.insert(
            "accepted_prefix_through_seq".into(),
            serde_json::Value::String(self.next_record_sequence.to_string()),
        );
        summary.insert(
            "coverage".into(),
            serde_json::Value::String(if self.coverage_gap { "gap" } else { "complete" }.into()),
        );
        let summary = serde_json::Value::Object(summary).to_string();
        let transaction = self.connection.transaction()?;
        if let (Some(anchor), Some(anchor_sequence), Some(anchor_no)) =
            (anchor, anchor_sequence, anchor_no)
        {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                anchor_sequence,
                anchor_no,
                "boot_end",
                anchor,
                None,
                None,
                &self.boot_anchor,
            )?;
        }
        let changed = transaction.execute(
            "UPDATE runtime_boots SET state='sealed',exit_summary=?2,ended_wall_us=?3 \
             WHERE boot_id=?1 AND state='active'",
            params![
                self.boot_id.as_slice(),
                summary,
                anchor.and_then(TimeAnchor::wall_us)
            ],
        )?;
        if changed != 1 {
            return Err(StorageError("active boot seal target missing".into()));
        }
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,kind,version,published_at,
             captured_at,origin) VALUES(?1,?2,'shutdown',1,?3,?3,'host_lifecycle')",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                published.as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
             VALUES(?1,?2,'shutdown','info','boot_sealed',?3)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3
             WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                record.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = next_record;
        self.commit_no = next_commit;
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.boot_sealed = true;
        Ok(())
    }

    /// Explicit close reports a failure after a committed seal separately.
    pub fn close(self) -> Result<(), StorageError> {
        self.connection
            .close()
            .map_err(|(_connection, error)| StorageError(format!("sqlite close: {error}")))
    }

    /// Discover archived runs through the primary-key index without COUNT/OFFSET.
    /// The first page freezes the upper boot/run key; summaries are time-of-page.
    pub fn read_history_runs(
        &self,
        cursor: Option<&RunsCursor>,
        limit: usize,
    ) -> Result<RunsPage, StorageError> {
        let _progress = HistoryProgress::install(&self.connection, HistoryBudget::default())?;
        if !(1..=32).contains(&limit) {
            return Err(StorageError("run page limit must be 1..=32".into()));
        }
        let (last, upper) = if let Some(cursor) = cursor {
            if cursor.database_id != self.database_id || cursor.last > cursor.upper {
                return Err(StorageError("run cursor does not match database".into()));
            }
            (Some(cursor.last), Some(cursor.upper))
        } else {
            let upper = self
                .connection
                .query_row(
                    "SELECT boot_id,run_no FROM runs ORDER BY boot_id DESC,run_no DESC LIMIT 1",
                    [],
                    |row| Ok((read_boot_blob(row.get(0)?)?, read_u64_blob(row.get(1)?)?)),
                )
                .optional()?;
            (None, upper)
        };
        let Some(upper) = upper else {
            return Ok(RunsPage {
                runs: Vec::new(),
                next_cursor: None,
            });
        };
        let last_boot = last.map_or([0u8; 16], |key| key.0);
        let last_run = u64_blob(last.map_or(0, |key| key.1));
        let upper_run = u64_blob(upper.1);
        let mut statement = self.connection.prepare(
            "SELECT boot_id,run_no,label,policy,state,coverage FROM runs
             WHERE (?1=0 OR boot_id>?2 OR (boot_id=?2 AND run_no>?3))
               AND (boot_id<?4 OR (boot_id=?4 AND run_no<=?5))
             ORDER BY boot_id,run_no LIMIT ?6",
        )?;
        let selected = statement.query_map(
            params![
                i64::from(last.is_some()),
                last_boot.as_slice(),
                last_run.as_slice(),
                upper.0.as_slice(),
                upper_run.as_slice(),
                i64::try_from(limit + 1).expect("bounded run lookahead")
            ],
            |row| {
                let boot = read_boot_blob(row.get(0)?)?;
                Ok(RunSummary {
                    boot_id: hex_boot(boot),
                    run_no: read_u64_blob(row.get(1)?)?,
                    label: row.get(2)?,
                    policy: row.get(3)?,
                    state: row.get(4)?,
                    coverage: row.get(5)?,
                })
            },
        )?;
        let mut runs = Vec::with_capacity(limit);
        let mut bytes = 0usize;
        let mut has_more = false;
        for selected_run in selected {
            let run = selected_run?;
            let charge =
                192 + run.label.len() + run.policy.len() + run.state.len() + run.coverage.len();
            if runs.len() == limit || bytes.saturating_add(charge) > 8 * 1024 {
                if runs.is_empty() {
                    return Err(StorageError("whole run exceeds page budget".into()));
                }
                has_more = true;
                break;
            }
            bytes += charge;
            runs.push(run);
        }
        let next_cursor = if has_more {
            let last_run = runs.last().expect("lookahead follows returned run");
            Some(RunsCursor {
                database_id: self.database_id.clone(),
                last: (parse_boot_id(&last_run.boot_id)?, last_run.run_no),
                upper,
            })
        } else {
            None
        };
        Ok(RunsPage { runs, next_cursor })
    }

    /// Select one indexed keyset page at a frozen durable checkpoint.
    /// The caller retains the returned cursor in a bounded connection slot;
    /// no read transaction or SQLite statement remains open between pages.
    pub fn read_history_measurements(
        &self,
        filter: &HistoryFilter,
        cursor: Option<&HistoryCursor>,
        limit: usize,
    ) -> Result<HistoryPage, StorageError> {
        self.read_history_measurements_with_budget(filter, cursor, limit, HistoryBudget::default())
    }

    /// Trusted smaller test budget; the production worker always uses defaults.
    pub fn read_history_measurements_with_budget(
        &self,
        filter: &HistoryFilter,
        cursor: Option<&HistoryCursor>,
        limit: usize,
        budget: HistoryBudget,
    ) -> Result<HistoryPage, StorageError> {
        let _progress = HistoryProgress::install(&self.connection, budget)?;
        if !(1..=MAX_RAW_PAGE).contains(&limit) || filter.from >= filter.to {
            return Err(StorageError(
                "invalid bounded history range or page limit".into(),
            ));
        }
        let archive_boot = parse_boot_id(&filter.boot_id)?;
        let run_no = u64_blob(filter.run_no);
        let instrument_id = u64_blob(filter.instrument.get());
        let parameter_id = u64_blob(filter.parameter.get());
        let from = duration_blob(filter.from)?;
        let to = duration_blob(filter.to)?;
        let coverage: String = self
            .connection
            .query_row(
                "SELECT coverage FROM runs WHERE boot_id=?1 AND run_no=?2",
                params![archive_boot.as_slice(), run_no.as_slice()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StorageError("unknown archived run".into()))?;
        let current_watermark = self.connection.query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints WHERE boot_id=?1",
            params![archive_boot.as_slice()],
            |row| read_u64_blob(row.get(0)?),
        )?;
        let (watermark, last, upper) = if let Some(cursor) = cursor {
            if cursor.filter != *filter
                || cursor.database_id != self.database_id
                || cursor.watermark > current_watermark
                || cursor.last > cursor.upper
            {
                return Err(StorageError(
                    "history cursor does not match archive/filter".into(),
                ));
            }
            (cursor.watermark, Some(cursor.last), Some(cursor.upper))
        } else {
            let frozen = u64_blob(current_watermark);
            let upper = self
                .connection
                .query_row(
                    "SELECT published_at,record_seq FROM measurements
                 WHERE boot_id=?1 AND run_no=?2 AND instrument_id=?3 AND parameter_id=?4
                   AND published_at>=?5 AND published_at<?6 AND record_seq<=?7
                 ORDER BY published_at DESC,record_seq DESC LIMIT 1",
                    params![
                        archive_boot.as_slice(),
                        run_no.as_slice(),
                        instrument_id.as_slice(),
                        parameter_id.as_slice(),
                        from.as_slice(),
                        to.as_slice(),
                        frozen.as_slice()
                    ],
                    |row| Ok((read_u64_blob(row.get(0)?)?, read_u64_blob(row.get(1)?)?)),
                )
                .optional()?;
            (current_watermark, None, upper)
        };
        let Some(upper) = upper else {
            return Ok(HistoryPage {
                rows: Vec::new(),
                watermark,
                next_cursor: None,
                coverage,
            });
        };
        let frozen = u64_blob(watermark);
        let upper_at = u64_blob(upper.0);
        let upper_seq = u64_blob(upper.1);
        let last_at = u64_blob(last.map_or(0, |key| key.0));
        let last_seq = u64_blob(last.map_or(0, |key| key.1));
        let mut statement = self.connection.prepare(
            "SELECT record_seq,published_at,observed_at,unit_key,quality,failure,
                    value_kind,float_value,integer_value,bool_value,text_value,generation,revision
             FROM measurements
             WHERE boot_id=?1 AND run_no=?2 AND instrument_id=?3 AND parameter_id=?4
               AND published_at>=?5 AND published_at<?6 AND record_seq<=?7
               AND (?8=0 OR published_at>?9 OR (published_at=?9 AND record_seq>?10))
               AND (published_at<?11 OR (published_at=?11 AND record_seq<=?12))
             ORDER BY published_at,record_seq LIMIT ?13",
        )?;
        let selected = statement.query_map(
            params![
                archive_boot.as_slice(),
                run_no.as_slice(),
                instrument_id.as_slice(),
                parameter_id.as_slice(),
                from.as_slice(),
                to.as_slice(),
                frozen.as_slice(),
                i64::from(last.is_some()),
                last_at.as_slice(),
                last_seq.as_slice(),
                upper_at.as_slice(),
                upper_seq.as_slice(),
                i64::try_from(limit + 1).expect("bounded lookahead")
            ],
            decode_measurement_row,
        )?;
        let mut rows = Vec::with_capacity(limit);
        let mut page_bytes = 0usize;
        let mut has_more = false;
        for selected_row in selected {
            let row = selected_row?;
            let bytes = 256
                + row.unit.len()
                + row.failure.as_ref().map_or(0, String::len)
                + match &row.value {
                    Some(Value::Text(text) | Value::Enum(text)) => text.len() * 2,
                    _ => 0,
                };
            if rows.len() == limit || page_bytes.saturating_add(bytes) > 8 * 1024 {
                if rows.is_empty() {
                    return Err(StorageError("whole history row exceeds page budget".into()));
                }
                has_more = true;
                break;
            }
            page_bytes += bytes;
            rows.push(row);
        }
        let next_cursor = if has_more {
            let last_row = rows.last().expect("lookahead follows at least one row");
            Some(HistoryCursor {
                filter: filter.clone(),
                database_id: self.database_id.clone(),
                watermark,
                last: (
                    duration_blob(last_row.published_at).map(u64::from_be_bytes)?,
                    last_row.record_sequence,
                ),
                upper,
            })
        } else {
            None
        };
        Ok(HistoryPage {
            rows,
            watermark,
            next_cursor,
            coverage,
        })
    }

    /// Read a bounded raw page for one signal using its indexed publication order.
    /// This worker-only primitive is not an application Query or an owner-lane call.
    pub fn read_measurements(
        &self,
        instrument: InstrumentId,
        parameter: ParameterId,
        limit: usize,
    ) -> Result<Vec<MeasurementRow>, StorageError> {
        if !(1..=MAX_RAW_PAGE).contains(&limit) {
            return Err(StorageError("history page limit must be 1..=128".into()));
        }
        let mut statement = self.connection.prepare(
            "SELECT record_seq,published_at,observed_at,unit_key,quality,failure,\
             value_kind,float_value,integer_value,bool_value,text_value,generation,revision \
             FROM measurements WHERE instrument_id=?1 AND parameter_id=?2 \
             ORDER BY published_at,record_seq LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                u64_blob(instrument.get()).as_slice(),
                u64_blob(parameter.get()).as_slice(),
                i64::try_from(limit).expect("bounded limit")
            ],
            decode_measurement_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn u64_blob(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

// A malformed native fact must roll back the entire mixed batch before any
// checkpoint advances; SQLite may otherwise coerce NaN to NULL silently.
fn validate_storage_fact(fact: &RecordingFact) -> Result<(), StorageError> {
    match fact {
        RecordingFact::Measurement { sample, .. } => {
            if sample.value().is_some_and(|value| {
                matches!(value,
                Value::Float(number) if !number.is_finite())
            }) {
                return Err(StorageError("nonfinite measurement fact".into()));
            }
        }
        RecordingFact::Output { value, .. } => {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(StorageError("nonfinite output fact".into()));
            }
        }
        RecordingFact::Controller { pid, .. } => {
            if pid.is_some_and(|pid| {
                !pid.kp.is_finite()
                    || !pid.ki.is_finite()
                    || !pid.kd.is_finite()
                    || !pid.output_min.is_finite()
                    || !pid.output_max.is_finite()
            }) {
                return Err(StorageError("nonfinite controller fact".into()));
            }
        }
        RecordingFact::Reference {
            value,
            target,
            rate,
            ..
        } => {
            if !value.is_finite()
                || target.is_some_and(|target| !target.is_finite())
                || rate.is_some_and(|rate| !rate.is_finite())
            {
                return Err(StorageError("nonfinite Reference fact".into()));
            }
        }
    }
    Ok(())
}

// Clock facts follow the same records/projection transaction as every other
// durable source fact; actual UTC remains independent of the fixed boot map.
// The full boot/run/interval scope is passed into one atomic projection so an
// actual UTC anchor cannot silently attach to another recording interval.
#[expect(clippy::too_many_arguments, reason = "explicit clock fact scope")]
fn insert_clock_projection(
    transaction: &rusqlite::Transaction<'_>,
    boot_id: &[u8; 16],
    sequence: u64,
    anchor_no: u64,
    kind: &str,
    anchor: &TimeAnchor,
    run_no: Option<u64>,
    interval_no: Option<u64>,
    boot_anchor: &TimeAnchor,
) -> Result<(), StorageError> {
    let time = duration_blob(anchor.after())?;
    let wall_estimate = boot_anchor.estimate_us(anchor.after())?;
    let payload = serde_json::json!({
        "kind":kind,
        "monotonic_before_ns":anchor.before().as_nanos().to_string(),
        "monotonic_after_ns":anchor.after().as_nanos().to_string(),
        "uncertainty_ns":anchor.uncertainty_ns().to_string(),
        "actual_wall_us":anchor.wall_us(),
        "unavailable_reason":anchor.unavailable_reason(),
        "wall_basis":"boot_anchor"
    })
    .to_string();
    transaction.execute(
        "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,
         published_at,observed_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
         VALUES(?1,?2,?3,?4,'clock_anchor',1,?5,?5,?5,?6,'boot_anchor',
         'recorder_clock',?7)",
        params![
            boot_id.as_slice(),
            u64_blob(sequence).as_slice(),
            run_no.map(u64_blob).map(|value| value.to_vec()),
            interval_no.map(u64_blob).map(|value| value.to_vec()),
            time.as_slice(),
            wall_estimate,
            payload.as_bytes()
        ],
    )?;
    transaction.execute(
        "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
         VALUES(?1,?2,'clock','info',?3,?4)",
        params![
            boot_id.as_slice(),
            u64_blob(sequence).as_slice(),
            kind,
            payload.as_bytes()
        ],
    )?;
    transaction.execute(
        "INSERT INTO clock_anchors(boot_id,anchor_no,record_seq,kind,monotonic_before,
         monotonic_after,uncertainty_ns,wall_us,unavailable_reason)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            boot_id.as_slice(),
            u64_blob(anchor_no).as_slice(),
            u64_blob(sequence).as_slice(),
            kind,
            duration_blob(anchor.before())?.as_slice(),
            time.as_slice(),
            u64_blob(anchor.uncertainty_ns()).as_slice(),
            anchor.wall_us(),
            anchor.unavailable_reason()
        ],
    )?;
    Ok(())
}

fn parse_boot_id(text: &str) -> Result<[u8; 16], StorageError> {
    if text.len() != 32
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StorageError(
            "boot identity must be 32 lowercase hex digits".into(),
        ));
    }
    let mut result = [0u8; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| StorageError("invalid boot identity".into()))?;
    }
    Ok(result)
}

fn duration_blob(value: Duration) -> Result<[u8; 8], StorageError> {
    let nanos = u64::try_from(value.as_nanos())
        .map_err(|_| StorageError("monotonic time exceeds version-one range".into()))?;
    Ok(u64_blob(nanos))
}

fn read_u64_blob(value: Vec<u8>) -> Result<u64, rusqlite::Error> {
    let bytes: [u8; 8] = value
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(u64::from_be_bytes(bytes))
}

fn read_boot_blob(value: Vec<u8>) -> Result<[u8; 16], rusqlite::Error> {
    value.try_into().map_err(|_| rusqlite::Error::InvalidQuery)
}

fn hex_boot(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_hash(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_measurement_row(row: &rusqlite::Row<'_>) -> Result<MeasurementRow, rusqlite::Error> {
    let kind: String = row.get(6)?;
    let value = match kind.as_str() {
        "none" => None,
        "float" => Some(Value::Float(row.get(7)?)),
        "integer" => Some(Value::Integer(row.get(8)?)),
        "boolean" => Some(Value::Boolean(row.get::<_, i64>(9)? != 0)),
        "text" => Some(Value::Text(row.get(10)?)),
        "enum" => Some(Value::Enum(row.get(10)?)),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(MeasurementRow {
        record_sequence: read_u64_blob(row.get(0)?)?,
        published_at: Duration::from_nanos(read_u64_blob(row.get(1)?)?),
        observed_at: Duration::from_nanos(read_u64_blob(row.get(2)?)?),
        unit: row.get(3)?,
        quality: row.get(4)?,
        failure: row.get(5)?,
        value,
        generation: read_u64_blob(row.get(11)?)?,
        revision: read_u64_blob(row.get(12)?)?,
    })
}

// An invalid archive must be rejected before WAL conversion or new-boot
// recovery. Indexed LIMIT 2 proves the at-most-one invariant without scanning
// the history of all earlier sealed runs.
fn validate_unfinished_entries(connection: &Connection) -> Result<(), StorageError> {
    let mut boots = Vec::with_capacity(2);
    let mut statement =
        connection.prepare("SELECT boot_id FROM runtime_boots WHERE state='active' LIMIT 2")?;
    for row in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
        boots.push(read_boot_blob(row?)?);
    }
    let mut runs = Vec::with_capacity(2);
    let mut statement = connection.prepare(
        "SELECT boot_id,run_no FROM runs
         WHERE state IN ('starting','recording','stopping') LIMIT 2",
    )?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
    })? {
        let (boot, run) = row?;
        runs.push((read_boot_blob(boot)?, read_u64_blob(run)?));
    }
    let mut intervals = Vec::with_capacity(2);
    let mut statement = connection.prepare(
        "SELECT boot_id,run_no FROM recording_intervals
         WHERE state IN ('starting','recording','stopping') LIMIT 2",
    )?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
    })? {
        let (boot, run) = row?;
        intervals.push((read_boot_blob(boot)?, read_u64_blob(run)?));
    }
    if boots.len() > 1
        || runs.len() > 1
        || intervals.len() > 1
        || runs
            .first()
            .is_some_and(|(boot, _)| boots.first() != Some(boot))
        || runs != intervals
    {
        return Err(StorageError(
            "invalid multiple or inconsistent unfinished entries".into(),
        ));
    }
    if let Some(boot) = boots.first() {
        let checkpoint: Option<Vec<u8>> = connection
            .query_row(
                "SELECT persisted_through_seq FROM durable_checkpoints WHERE boot_id=?1",
                params![boot.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let checkpoint = checkpoint.ok_or_else(|| {
            StorageError("storage_corrupt: active boot checkpoint missing".into())
        })?;
        let checkpoint = read_u64_blob(checkpoint)?;
        let last: Option<Vec<u8>> = connection
            .query_row(
                "SELECT record_seq FROM records WHERE boot_id=?1
             ORDER BY record_seq DESC LIMIT 1",
                params![boot.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let last = last.map(read_u64_blob).transpose()?.unwrap_or(0);
        if checkpoint != last {
            return Err(StorageError(
                "storage_corrupt: checkpoint/fact mismatch".into(),
            ));
        }
    }
    Ok(())
}

fn create_schema(connection: &mut Connection) -> Result<(), StorageError> {
    let mut database_id = [0u8; 16];
    getrandom::fill(&mut database_id)
        .map_err(|error| StorageError(format!("database entropy unavailable: {error}")))?;
    let identity: String = database_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE schema_version(singleton INTEGER PRIMARY KEY CHECK(singleton=1),
             database_id TEXT NOT NULL UNIQUE, schema_version INTEGER NOT NULL,
             record_encoding INTEGER NOT NULL, created_wall_us INTEGER);
         CREATE TABLE runtime_boots(boot_id BLOB PRIMARY KEY CHECK(length(boot_id)=16),
             build_version TEXT NOT NULL, started_wall_us INTEGER, ended_wall_us INTEGER,
             anchor_before BLOB, anchor_after BLOB, anchor_uncertainty_ns BLOB,
             state TEXT NOT NULL, exit_summary TEXT, recovered_by_boot BLOB);
         CREATE INDEX runtime_boots_unfinished ON runtime_boots(state,boot_id);
         CREATE TABLE clock_anchors(boot_id BLOB NOT NULL,anchor_no BLOB NOT NULL,
             record_seq BLOB,
             kind TEXT NOT NULL,monotonic_before BLOB NOT NULL,
             monotonic_after BLOB NOT NULL,uncertainty_ns BLOB NOT NULL,
             wall_us INTEGER,unavailable_reason TEXT,
             PRIMARY KEY(boot_id,anchor_no),
             FOREIGN KEY(boot_id) REFERENCES runtime_boots(boot_id),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE runs(boot_id BLOB NOT NULL, run_no BLOB NOT NULL CHECK(length(run_no)=8),
             label TEXT NOT NULL, policy TEXT NOT NULL, state TEXT NOT NULL,
             coverage TEXT NOT NULL, started_wall_us INTEGER, ended_wall_us INTEGER,
             initial_activation_id BLOB,
             PRIMARY KEY(boot_id,run_no), FOREIGN KEY(boot_id) REFERENCES runtime_boots(boot_id));
         CREATE INDEX runs_unfinished ON runs(state,boot_id,run_no);
         CREATE TABLE recording_intervals(boot_id BLOB NOT NULL, interval_no BLOB NOT NULL,
             run_no BLOB NOT NULL, state TEXT NOT NULL, coverage TEXT NOT NULL,
             boundary_seq BLOB, start_seq BLOB, end_seq BLOB, loss_summary TEXT,
             PRIMARY KEY(boot_id,interval_no),
             FOREIGN KEY(boot_id,run_no) REFERENCES runs(boot_id,run_no));
         CREATE INDEX intervals_unfinished ON recording_intervals(state,boot_id,run_no);
         CREATE TABLE configurations(boot_id BLOB NOT NULL, activation_no BLOB NOT NULL,
             manifest_root_hash BLOB, manifest_content_hash BLOB, encoding TEXT,
             committed_at BLOB, object_revisions TEXT,
             PRIMARY KEY(boot_id,activation_no),
             FOREIGN KEY(boot_id) REFERENCES runtime_boots(boot_id));
         CREATE TABLE provenance_content(content_hash BLOB PRIMARY KEY CHECK(length(content_hash)=32),
             encoding TEXT NOT NULL, kind TEXT NOT NULL, content BLOB NOT NULL);
         CREATE TABLE object_snapshots(boot_id BLOB NOT NULL, activation_no BLOB NOT NULL,
             object_kind TEXT NOT NULL, object_id BLOB NOT NULL, logical_key TEXT,
             label TEXT, generation BLOB, descriptor TEXT, unit_key TEXT,
             instance_binding TEXT, definition_hash BLOB, source_hash BLOB, safety_hash BLOB,
             PRIMARY KEY(boot_id,activation_no,object_kind,object_id),
             FOREIGN KEY(boot_id,activation_no) REFERENCES configurations(boot_id,activation_no));
         CREATE TABLE records(boot_id BLOB NOT NULL, record_seq BLOB NOT NULL CHECK(length(record_seq)=8),
             run_no BLOB, interval_no BLOB, kind TEXT NOT NULL,
             version INTEGER NOT NULL CHECK(version=1), fact_seq BLOB,
             published_at BLOB, observed_at BLOB, captured_at BLOB, wall_estimate_us INTEGER,
             wall_basis TEXT,
             origin TEXT, target TEXT, cause TEXT, payload BLOB,
             PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,run_no) REFERENCES runs(boot_id,run_no),
             FOREIGN KEY(boot_id,interval_no) REFERENCES recording_intervals(boot_id,interval_no));
         CREATE TABLE measurements(boot_id BLOB NOT NULL, record_seq BLOB NOT NULL,
             run_no BLOB NOT NULL, instrument_id BLOB NOT NULL, parameter_id BLOB NOT NULL,
             generation BLOB NOT NULL, revision BLOB NOT NULL, observed_at BLOB NOT NULL,
             published_at BLOB NOT NULL, unit_key TEXT NOT NULL CHECK(length(unit_key) BETWEEN 1 AND 64),
             quality TEXT NOT NULL CHECK(quality IN ('good','unavailable')),
             failure TEXT, value_kind TEXT NOT NULL, float_value REAL,
             integer_value INTEGER, bool_value INTEGER, text_value TEXT, lineage BLOB,
             PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq),
             CHECK((quality='good' AND failure IS NULL AND value_kind!='none') OR
                   (quality='unavailable' AND failure IS NOT NULL AND value_kind='none')),
             CHECK((value_kind='none' AND float_value IS NULL AND integer_value IS NULL
                    AND bool_value IS NULL AND text_value IS NULL) OR
                   (value_kind='float' AND float_value IS NOT NULL AND integer_value IS NULL
                    AND bool_value IS NULL AND text_value IS NULL) OR
                   (value_kind='integer' AND float_value IS NULL AND integer_value IS NOT NULL
                    AND bool_value IS NULL AND text_value IS NULL) OR
                   (value_kind='boolean' AND float_value IS NULL AND integer_value IS NULL
                    AND bool_value IN (0,1) AND text_value IS NULL) OR
                   (value_kind IN ('text','enum') AND float_value IS NULL
                    AND integer_value IS NULL AND bool_value IS NULL
                    AND text_value IS NOT NULL)));
         CREATE INDEX measurements_history ON measurements(
             boot_id,run_no,instrument_id,parameter_id,published_at,record_seq);
         CREATE INDEX measurements_all_history ON measurements(
             instrument_id,parameter_id,published_at,record_seq);
         CREATE TABLE operation_events(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,
             request_scope TEXT,request_seq TEXT,phase TEXT,command TEXT,result TEXT,
             outcome_basis TEXT,PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE INDEX operation_identity ON operation_events(request_scope,request_seq,record_seq);
         CREATE TABLE controller_events(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,
             controller_id BLOB,before_state TEXT,after_state TEXT,event_kind TEXT,
             config_revision BLOB,input_correlation TEXT,reference_correlation TEXT,
             output_correlation TEXT,diagnostics TEXT,PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE reference_events(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,
             reference_id BLOB,revision BLOB,event_kind TEXT,value REAL,target REAL,
             rate REAL,unit_key TEXT,progress_at BLOB,PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE output_events(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,
             attempt_id BLOB,dispatch_id BLOB,resource_id BLOB,instrument_id BLOB NOT NULL,
             parameter_id BLOB NOT NULL,authority_epoch BLOB,generation BLOB,revision BLOB,
             stage TEXT NOT NULL,value REAL,unit_key TEXT,evidence_source TEXT,
             evidence_basis TEXT,failure TEXT,ambiguous INTEGER,settled INTEGER,
             PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE runtime_events(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,
             category TEXT,severity TEXT,code TEXT,data BLOB,PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE gaps(boot_id BLOB NOT NULL,record_seq BLOB NOT NULL,interval_no BLOB,
             first_missing BLOB,last_missing BLOB,known_count BLOB,unknown_tail INTEGER,
             reason TEXT,last_confirmed BLOB,PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,record_seq) REFERENCES records(boot_id,record_seq));
         CREATE TABLE durable_checkpoints(boot_id BLOB PRIMARY KEY,commit_no BLOB NOT NULL,
             persisted_through_seq BLOB NOT NULL,last_probe_at BLOB,coverage TEXT,
             FOREIGN KEY(boot_id) REFERENCES runtime_boots(boot_id));
         PRAGMA application_id=1279345234;
         PRAGMA user_version=1;",
    )?;
    transaction.execute(
        "INSERT INTO schema_version(singleton,database_id,schema_version,record_encoding) \
         VALUES(1,?1,1,1)",
        params![identity],
    )?;
    transaction.commit()?;
    let actual: i32 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if actual != APPLICATION_ID {
        return Err(StorageError("created application id mismatch".into()));
    }
    Ok(())
}
