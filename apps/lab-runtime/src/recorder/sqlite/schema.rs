//! Fixed schema-v1 DDL and reopen integrity validation.
//!
//! Historical table and column names are intentionally preserved.

use super::*;

pub(super) fn validate_unfinished_entries(connection: &Connection) -> Result<(), StorageError> {
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

pub(super) fn create_schema(connection: &mut Connection) -> Result<(), StorageError> {
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
         CREATE INDEX intervals_by_run ON recording_intervals(boot_id,run_no);
         CREATE TABLE configurations(boot_id BLOB NOT NULL, activation_no BLOB NOT NULL,
             manifest_root_hash BLOB, manifest_content_hash BLOB, encoding TEXT,
             committed_at BLOB, object_revisions TEXT,
             PRIMARY KEY(boot_id,activation_no),
             FOREIGN KEY(boot_id) REFERENCES runtime_boots(boot_id));
         CREATE TABLE provenance_content(content_hash BLOB NOT NULL CHECK(length(content_hash)=32),
             encoding TEXT NOT NULL, kind TEXT NOT NULL, content BLOB NOT NULL,
             PRIMARY KEY(content_hash,kind,encoding));
         CREATE TABLE object_snapshots(boot_id BLOB NOT NULL, activation_no BLOB NOT NULL,
             object_kind TEXT NOT NULL, object_id BLOB NOT NULL, logical_key TEXT,
             label TEXT, generation BLOB, descriptor TEXT, unit_key TEXT,
             instance_binding TEXT, definition_hash BLOB, source_hash BLOB, safety_hash BLOB,
             PRIMARY KEY(boot_id,activation_no,object_kind,object_id),
             FOREIGN KEY(boot_id,activation_no) REFERENCES configurations(boot_id,activation_no));
         CREATE TABLE records(boot_id BLOB NOT NULL, record_seq BLOB NOT NULL CHECK(length(record_seq)=8),
             run_no BLOB, interval_no BLOB, kind TEXT NOT NULL,
             version INTEGER NOT NULL CHECK(version=1), fact_seq BLOB
             CHECK(fact_seq IS NULL OR length(fact_seq)=8),
             published_at BLOB, observed_at BLOB, captured_at BLOB, wall_estimate_us INTEGER,
             wall_basis TEXT,
             origin TEXT, target TEXT, cause TEXT, payload BLOB,
             PRIMARY KEY(boot_id,record_seq),
             FOREIGN KEY(boot_id,run_no) REFERENCES runs(boot_id,run_no),
             FOREIGN KEY(boot_id,interval_no) REFERENCES recording_intervals(boot_id,interval_no));
         CREATE UNIQUE INDEX records_source_fact ON records(boot_id,fact_seq)
             WHERE fact_seq IS NOT NULL;
         CREATE TABLE measurements(boot_id BLOB NOT NULL, record_seq BLOB NOT NULL,
             run_no BLOB NOT NULL, instrument_id BLOB NOT NULL, parameter_id BLOB NOT NULL,
             generation BLOB NOT NULL, revision BLOB NOT NULL, state_revision BLOB,
             observed_at BLOB NOT NULL,
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
