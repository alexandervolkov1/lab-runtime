//! Connection opening, recovery, storage health and fixed SQLite settings.

use super::*;

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
                  'intervals_by_run',
                  'measurements_history','measurements_all_history','operation_identity',
                  'records_source_fact')",
                [],
                |row| row.get(0),
            )?;
            if indexes != 8 {
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
            let measurement_columns: Vec<String> = connection
                .prepare("PRAGMA table_info('measurements')")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<_, _>>()?;
            if !measurement_columns
                .iter()
                .any(|column| column == "state_revision")
                || !measurement_columns.iter().any(|column| column == "lineage")
            {
                return Err(StorageError(
                    "version-one measurement provenance columns are incomplete".into(),
                ));
            }
            let loss_index: Vec<String> = connection
                .prepare("PRAGMA index_info('intervals_by_run')")?
                .query_map([], |row| row.get::<_, String>(2))?
                .collect::<Result<_, _>>()?;
            if loss_index != ["boot_id", "run_no"] {
                return Err(StorageError("incompatible interval history index".into()));
            }
            let provenance_key: Vec<(String, i64)> = connection
                .prepare("PRAGMA table_info('provenance_content')")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
                })?
                .collect::<Result<_, _>>()?;
            let mut provenance_key = provenance_key
                .iter()
                .filter(|(_, key)| *key > 0)
                .cloned()
                .collect::<Vec<_>>();
            provenance_key.sort_by_key(|(_, key)| *key);
            if provenance_key
                != [
                    ("content_hash".into(), 1),
                    ("kind".into(), 2),
                    ("encoding".into(), 3),
                ]
            {
                return Err(StorageError("incompatible provenance identity key".into()));
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
            checkpoint_fault_once: false,
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

    /// Capture an owned footprint on the storage thread after a SQL boundary.
    pub fn storage_health(&self) -> Result<StorageHealth, StorageError> {
        let (page_size, page_count, quota_pages) = self.storage_pages()?;
        let (wal_bytes, wal_checkpoints) = self.wal_health()?;
        Ok(StorageHealth {
            main_logical_bytes: page_size
                .checked_mul(page_count)
                .ok_or_else(|| StorageError("main footprint overflow".into()))?,
            main_quota_bytes: page_size
                .checked_mul(quota_pages)
                .ok_or_else(|| StorageError("main quota overflow".into()))?,
            wal_bytes,
            wal_threshold_bytes: self.wal_threshold_bytes,
            wal_checkpoints,
        })
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

    /// Fail the next required threshold checkpoint at its worker-only gate.
    /// Trusted fault harnesses use this to verify the durable-prefix response.
    pub fn fail_next_checkpoint_for_testing(&mut self) {
        self.checkpoint_fault_once = true;
    }

    pub(super) fn require_wal_budget(&mut self) -> Result<(), StorageError> {
        let (bytes, _) = self.wal_health()?;
        if bytes < self.wal_threshold_bytes {
            return Ok(());
        }
        if self.checkpoint_fault_once {
            self.checkpoint_fault_once = false;
            return Err(StorageError("WAL threshold checkpoint failed".into()));
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

    pub(super) fn require_main_reserve(&self) -> Result<(), StorageError> {
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

    /// Commit only the exact owner-reserved clock identity in FIFO order.
    pub fn append_clock_anchor_assigned(
        &mut self,
        kind: &str,
        anchor: &TimeAnchor,
        record_seq: u64,
    ) -> Result<(), StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(record_seq) {
            return Err(StorageError("clock record reservation mismatch".into()));
        }
        self.append_clock_anchor(kind, anchor)
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
}
