//! Storage boot identity must match the serving Runtime and change on reopen.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::recorder::SqliteStore;
use std::{path::PathBuf, time::Duration};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-reopen-{suffix}.sqlite"))
}

#[test]
fn reopen_preserves_database_but_uses_new_exact_serving_boot_and_unknown_old_tail() {
    let path = temporary_database();
    let first_boot = "11111111111111111111111111111111";
    let second_boot = "22222222222222222222222222222222";
    let mut first = SqliteStore::open_with_boot(&path, first_boot).unwrap();
    let database_id = first.database_id().to_owned();
    assert_eq!(first.boot_id(), first_boot);
    first.start_run("interrupted").unwrap();
    drop(first);
    let second = SqliteStore::open_with_boot(&path, second_boot).unwrap();
    assert_eq!(second.database_id(), database_id);
    assert_eq!(second.boot_id(), second_boot);
    drop(second);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let state: String = connection
        .query_row(
            "SELECT state FROM runs WHERE boot_id=?1",
            [hex_bytes(first_boot)],
            |row| row.get(0),
        )
        .unwrap();
    let coverage: String = connection
        .query_row(
            "SELECT coverage FROM runs WHERE boot_id=?1",
            [hex_bytes(first_boot)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "interrupted");
    assert_eq!(coverage, "unknown_tail");
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reopen_rejects_multiple_unfinished_runs_without_changing_either_old_run() {
    let path = temporary_database();
    let first_boot = "abababababababababababababababab";
    let second_boot = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    let mut original = SqliteStore::open_with_boot(&path, first_boot).unwrap();
    original.start_run("first active").unwrap();
    drop(original);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO runs(boot_id,run_no,label,policy,state,coverage)
         VALUES(?1,?2,'second active','required','recording','complete')",
            rusqlite::params![hex_bytes(first_boot), 2u64.to_be_bytes().as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO recording_intervals(boot_id,interval_no,run_no,state,coverage)
         VALUES(?1,?2,?2,'recording','complete')",
            rusqlite::params![hex_bytes(first_boot), 2u64.to_be_bytes().as_slice()],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteStore::open_with_boot(&path, second_boot).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let still_active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE state='recording'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let boots: i64 = connection
        .query_row("SELECT COUNT(*) FROM runtime_boots", [], |row| row.get(0))
        .unwrap();
    assert_eq!((still_active, boots), (2, 1));
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reopen_rejects_an_unfinished_boot_checkpoint_ahead_of_its_committed_facts() {
    let path = temporary_database();
    let first_boot = "efefefefefefefefefefefefefefefef";
    let second_boot = "12121212121212121212121212121212";
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(97);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "checkpoint fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut original = SqliteStore::open_with_boot(&path, first_boot).unwrap();
    original.start_run("unfinished checkpoint").unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    original
        .append_facts(&runtime.take_recording_facts())
        .unwrap();
    drop(original);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE durable_checkpoints SET persisted_through_seq=?1 WHERE boot_id=?2",
            rusqlite::params![99u64.to_be_bytes().as_slice(), hex_bytes(first_boot)],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteStore::open_with_boot(&path, second_boot).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let state: String = connection
        .query_row("SELECT state FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(state, "recording");
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

fn hex_bytes(value: &str) -> Vec<u8> {
    (0..32)
        .step_by(2)
        .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
        .collect()
}
