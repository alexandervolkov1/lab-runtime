//! Real SQLite constraint failure must roll back a whole mixed transaction.

use lab_core::{
    Command, InstrumentId, Runtime, Sample, SignalId, Unit, Value, VirtualInstrumentConfig,
    recording::RecordingFact, reference::ReferenceId,
};
use lab_runtime::recorder::{HistoryFilter, SqliteStore};
use std::{path::PathBuf, time::Duration};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-transaction-{suffix}.sqlite"))
}

#[test]
fn duplicate_source_fact_identity_with_changed_value_rolls_back_its_whole_batch() {
    let path = temporary_database();
    let boot = "bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc";
    let instrument = InstrumentId::new(812);
    let signal = SignalId::new(instrument, lab_core::TEMPERATURE);
    let fact = |sequence, second, value| RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(
            signal,
            Unit::CELSIUS,
            Duration::from_secs(second),
            Value::Float(value),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
    };
    let mut store = SqliteStore::open_with_boot(&path, boot).unwrap();
    store.start_run("source identity").unwrap();
    store.append_facts(&[fact(7, 1, 23.5)]).unwrap();
    let prefix = store.current_record_sequence();
    assert!(
        store
            .append_facts(&[fact(8, 2, 24.0), fact(7, 3, 99.0)])
            .is_err()
    );
    assert_eq!(store.current_record_sequence(), prefix);
    let current = store
        .read_measurements(instrument, lab_core::TEMPERATURE, 8)
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].value, Some(Value::Float(23.5)));
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(4)).unwrap();
    store.close().unwrap();
    let reopened = SqliteStore::open_with_boot(&path, "bdbdbdbdbdbdbdbdbdbdbdbdbdbdbdbd").unwrap();
    let archive = reopened
        .read_history_measurements(
            &HistoryFilter {
                boot_id: boot.into(),
                run_no: 1,
                instrument,
                parameter: lab_core::TEMPERATURE,
                from: Duration::ZERO,
                to: Duration::from_secs(4),
            },
            None,
            8,
        )
        .unwrap();
    assert_eq!(archive.rows.len(), 1);
    assert_eq!(archive.rows[0].value, Some(Value::Float(23.5)));
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn real_insert_error_rolls_back_all_new_rows_and_checkpoint_without_erasing_prefix() {
    let path = temporary_database();
    let instrument = InstrumentId::new(81);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "transaction fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut prefix = SqliteStore::open(&path).unwrap();
    prefix.start_run("prefix").unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    prefix
        .append_facts(&runtime.take_recording_facts())
        .unwrap();
    prefix.stop_run().unwrap();
    let prefix_checkpoint = prefix.current_record_sequence();
    drop(prefix);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_second_measurement BEFORE INSERT ON measurements
         WHEN (SELECT COUNT(*) FROM measurements)=2
         BEGIN SELECT RAISE(ABORT,'injected insert failure'); END;",
        )
        .unwrap();
    drop(connection);
    let mut store = SqliteStore::open(&path).unwrap();
    let settings = store.sqlite_settings().unwrap();
    assert_eq!(settings.journal_mode, "wal");
    assert_eq!(settings.synchronous, 2);
    assert!(settings.foreign_keys);
    assert_eq!(settings.user_version, 1);
    store.start_run("failing batch").unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(2),
        })
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(3),
        })
        .unwrap();
    assert!(store.append_facts(&runtime.take_recording_facts()).is_err());
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let measurements: i64 = connection
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(measurements, 1);
    let checkpoints: Vec<Vec<u8>> = connection
        .prepare("SELECT persisted_through_seq FROM durable_checkpoints ORDER BY boot_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints.contains(&1u64.to_be_bytes().to_vec()));
    assert!(checkpoints.contains(&prefix_checkpoint.to_be_bytes().to_vec()));
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn checkpoint_update_failure_rolls_back_fact_and_preserves_prior_boot_prefix() {
    let path = temporary_database();
    let signal = SignalId::new(InstrumentId::new(481), lab_core::TEMPERATURE);
    let fact = |sequence, at| RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(signal, Unit::CELSIUS, at, Value::Float(27.5)).unwrap(),
        generation: 1,
        revision: 1,
    };
    let old_boot = "48484848484848484848484848484848";
    let mut old = SqliteStore::open_with_boot(&path, old_boot).unwrap();
    old.start_run("durable prefix").unwrap();
    old.append_facts(&[fact(1, Duration::from_secs(1))])
        .unwrap();
    old.stop_run().unwrap();
    old.finish_boot(Duration::from_secs(2)).unwrap();
    let old_checkpoint = old.current_record_sequence();
    old.close().unwrap();
    let external = rusqlite::Connection::open(&path).unwrap();
    external
        .execute_batch(
            "CREATE TRIGGER fail_final_checkpoint BEFORE UPDATE OF persisted_through_seq ON durable_checkpoints
             WHEN (SELECT count(*) FROM measurements)>=2
             BEGIN SELECT RAISE(ABORT,'injected checkpoint failure'); END;",
        )
        .unwrap();
    drop(external);
    let mut next = SqliteStore::open_with_boot(&path, "49494949494949494949494949494949").unwrap();
    next.start_run("checkpoint failure").unwrap();
    let boundary = next.current_record_sequence();
    assert!(
        next.append_facts(&[fact(2, Duration::from_secs(3))])
            .is_err()
    );
    assert_eq!(next.current_record_sequence(), boundary);
    drop(next);
    let db = rusqlite::Connection::open(&path).unwrap();
    let fact_rows: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fact_rows, 1);
    let checkpoints = db
        .prepare("SELECT persisted_through_seq FROM durable_checkpoints ORDER BY boot_id")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints.contains(&old_checkpoint.to_be_bytes().to_vec()));
    assert!(checkpoints.contains(&boundary.to_be_bytes().to_vec()));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn nonfinite_second_group_rejects_one_batch_without_committing_its_valid_prefix() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("nonfinite batch").unwrap();
    let boundary = store.current_record_sequence();
    let instrument = InstrumentId::new(881);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "valid first group".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let first = runtime.take_recording_facts();
    let invalid = RecordingFact::Reference {
        sequence: first[0].sequence() + 1,
        reference: ReferenceId::new(882),
        revision: 1,
        value: f64::NAN,
        target: None,
        rate: None,
        unit: Unit::CELSIUS,
        at: Duration::from_secs(2),
    };
    assert!(
        store
            .append_fact_groups(&[
                (&first, Duration::from_secs(1)),
                (&[invalid], Duration::from_secs(2)),
            ])
            .is_err()
    );
    assert_eq!(store.current_record_sequence(), boundary);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let checkpoint: Vec<u8> = db
        .query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint, boundary.to_be_bytes());
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unknown_nonempty_database_is_rejected_without_converting_it_to_wal() {
    let path = temporary_database();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE alien(value TEXT); INSERT INTO alien VALUES('kept');")
        .unwrap();
    drop(connection);
    assert!(SqliteStore::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM alien", [], |row| row.get(0))
        .unwrap();
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(mode, "delete");
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn real_sqlite_connection_caps_main_file_at_one_gib_by_checked_page_count() {
    let path = temporary_database();
    let store = SqliteStore::open(&path).unwrap();
    let (page_size, page_count, max_pages) = store.storage_pages().unwrap();
    assert!(page_size >= 512);
    assert!(page_count > 0);
    assert_eq!(max_pages, (1024u64 * 1024 * 1024 / page_size));
    assert!(page_count < max_pages);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn five_percent_page_reserve_rejects_new_run_before_mutating_history() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    let (_, pages, _) = store.storage_pages().unwrap();
    store.lower_main_quota_for_testing(pages + 2).unwrap();
    assert!(store.start_run("must not start inside reserve").is_err());
    assert_eq!(store.current_record_sequence(), 0);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let runs: i64 = db
        .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 0);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn wal_threshold_checkpoints_between_ordinary_transactions_and_reports_actual_bytes() {
    let path = temporary_database();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("wal budget").unwrap();
    let before = store.wal_health().unwrap();
    assert!(
        before.0 > 0,
        "a real WAL file should contain the start transaction"
    );
    store.lower_wal_threshold_for_testing(1).unwrap();
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(982);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "wal sample".into(),
            history_capacity: 2,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    for second in 1..=2 {
        runtime
            .command(Command::RefreshMeasurement {
                instrument,
                parameter: lab_core::TEMPERATURE,
                at: Duration::from_secs(second),
            })
            .unwrap();
        store.append_facts(&runtime.take_recording_facts()).unwrap();
    }
    let after = store.wal_health().unwrap();
    assert!(
        after.1 >= 2,
        "both ordinary transactions must check the real WAL"
    );
    assert!(
        after.0 > 0,
        "WAL bytes are reported, not treated as a hard cap"
    );
    store.stop_run().unwrap();
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn incompatible_record_encoding_is_rejected_before_recovering_the_old_boot() {
    let path = temporary_database();
    let mut original =
        SqliteStore::open_with_boot(&path, "88888888888888888888888888888888").unwrap();
    original.start_run("unfinished encoding fixture").unwrap();
    drop(original);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE schema_version SET record_encoding=2 WHERE singleton=1",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteStore::open_with_boot(&path, "99999999999999999999999999999999").is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let boots: i64 = connection
        .query_row("SELECT COUNT(*) FROM runtime_boots", [], |row| row.get(0))
        .unwrap();
    let old_state: String = connection
        .query_row("SELECT state FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(boots, 1, "incompatible files must remain unchanged");
    assert_eq!(old_state, "recording");
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn measurement_type_check_rejects_two_simultaneous_value_columns() {
    let path = temporary_database();
    let boot = "13131313131313131313131313131313";
    let mut store = SqliteStore::open_with_boot(&path, boot).unwrap();
    store.start_run("typed constraint fixture").unwrap();
    drop(store);
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let transaction = connection.transaction().unwrap();
    let boot_bytes: Vec<u8> = (0..32)
        .step_by(2)
        .map(|at| u8::from_str_radix(&boot[at..at + 2], 16).unwrap())
        .collect();
    let one = 1u64.to_be_bytes();
    let two = 2u64.to_be_bytes();
    transaction
        .execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version)
         VALUES(?1,?2,?3,?3,'measurement',1)",
            rusqlite::params![boot_bytes, two.as_slice(), one.as_slice()],
        )
        .unwrap();
    let inserted = transaction.execute(
        "INSERT INTO measurements(boot_id,record_seq,run_no,instrument_id,
         parameter_id,generation,revision,observed_at,published_at,unit_key,
         quality,value_kind,float_value,integer_value)
         VALUES(?1,?2,?3,?3,?3,?3,?3,?3,?3,'C','good','float',1.0,2)",
        rusqlite::params![boot_bytes, two.as_slice(), one.as_slice()],
    );
    assert!(
        inserted.is_err(),
        "a float row cannot also carry an integer value"
    );
    transaction.rollback().unwrap();
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reopen_rejects_a_missing_required_history_index_before_new_boot_recovery() {
    let path = temporary_database();
    let boot = "14141414141414141414141414141414";
    let original = SqliteStore::open_with_boot(&path, boot).unwrap();
    drop(original);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("DROP INDEX measurements_history")
        .unwrap();
    drop(connection);
    assert!(SqliteStore::open_with_boot(&path, "15151515151515151515151515151515").is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let boots: i64 = connection
        .query_row("SELECT COUNT(*) FROM runtime_boots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(boots, 1);
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_second_storage_owner_cannot_recover_or_change_live_first_boot() {
    let path = temporary_database();
    let mut first = SqliteStore::open_with_boot(&path, "66666666666666666666666666666666").unwrap();
    first.start_run("live").unwrap();
    assert!(SqliteStore::open_with_boot(&path, "77777777777777777777777777777777").is_err());
    drop(first);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let state: String = connection
        .query_row("SELECT state FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(state, "recording");
    drop(connection);
    std::fs::remove_file(path).unwrap();
}
