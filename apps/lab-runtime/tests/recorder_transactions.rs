//! Real SQLite constraint failure must roll back a whole mixed transaction.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::recorder::SqliteStore;
use std::{path::PathBuf, time::Duration};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-transaction-{suffix}.sqlite"))
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
    assert!(checkpoints.contains(&2u64.to_be_bytes().to_vec()));
    drop(connection);
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
