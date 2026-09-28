use super::*;
use crate::client::types::{
    EventCursor, HelloState, KnownAdmission, MutationIdentity, RecoveryRecord,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static TEST_ID: AtomicU64 = AtomicU64::new(1);

fn test_dir(name: &str) -> PathBuf {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "lab-workbench-journal-{name}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn record(seq: u64) -> RecoveryRecord {
    RecoveryRecord {
        boot_id: "boot".into(),
        identity: MutationIdentity {
            scope: "scope".into(),
            seq,
        },
        op: "reference_retune".into(),
        args: json!({"reference":"1","target":12.5,"nested":{"exact":true}}),
        admission: KnownAdmission::Ambiguous,
    }
}

fn hello(boot_id: &str, scope: &str) -> HelloState {
    HelloState {
        boot_id: boot_id.into(),
        scope: scope.into(),
        next_seq: 2,
        operations: Vec::new(),
        capabilities: json!([]),
        limits: json!({}),
        event_oldest: EventCursor {
            boot_id: boot_id.into(),
            seq: 0,
        },
        event_latest: EventCursor {
            boot_id: boot_id.into(),
            seq: 0,
        },
    }
}

#[test]
fn journal_round_trip_preserves_exact_payload_and_excludes_connection_tokens() {
    let directory = test_dir("round-trip");
    let path = directory.join("recovery.json");
    let journal =
        RecoveryJournal::from_records("boot".into(), "scope".into(), 2, &[record(1)]).unwrap();
    save_journal(&path, &journal).unwrap();
    let loaded = load_journal(&path).unwrap();
    assert_eq!(loaded, journal);
    let records = loaded.to_recovery_records();
    assert_eq!(records[0].op, "reference_retune");
    assert_eq!(records[0].args, record(1).args);

    let encoded = String::from_utf8(fs::read(&path).unwrap()).unwrap();
    assert!(!encoded.contains("msg_id"));
    assert!(!encoded.contains("subscription"));
    assert!(!encoded.contains("page_token"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn journal_enforces_eight_records_and_sixty_four_kibibytes() {
    let records = (1..=8).map(record).collect::<Vec<_>>();
    RecoveryJournal::from_records("boot".into(), "scope".into(), 9, &records).unwrap();
    let too_many = (1..=9).map(record).collect::<Vec<_>>();
    assert!(matches!(
        RecoveryJournal::from_records("boot".into(), "scope".into(), 10, &too_many),
        Err(JournalError::Limit("records"))
    ));

    let directory = test_dir("oversize");
    let path = directory.join("recovery.json");
    fs::write(&path, vec![b' '; super::journal::JOURNAL_FILE_BYTES + 1]).unwrap();
    assert!(matches!(load_journal(&path), Err(JournalError::Io(_))));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_truncated_wrong_root_and_versions_are_rejected() {
    let directory = test_dir("corrupt");
    let path = directory.join("recovery.json");
    for bytes in [
        Vec::new(),
        b"{".to_vec(),
        vec![0xff],
        b"[]".to_vec(),
        br#"{"format_version":2,"boot_id":"b","scope":"s","next_seq":"1","records":[]}"#.to_vec(),
        br#"{"boot_id":"b","scope":"s","next_seq":"1","records":[]}"#.to_vec(),
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(load_journal(&path).is_err());
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn journal_requires_authoritative_same_boot_scope_reconciliation_and_never_executes() {
    let journal =
        RecoveryJournal::from_records("boot".into(), "scope".into(), 2, &[record(1)]).unwrap();
    assert_eq!(
        journal.reconcile(&hello("other", "scope")),
        JournalDisposition::DifferentBoot
    );
    assert_eq!(
        journal.reconcile(&hello("boot", "other")),
        JournalDisposition::DifferentScope
    );
    let JournalDisposition::NeedsAuthoritativeStatus(records) =
        journal.reconcile(&hello("boot", "scope"))
    else {
        panic!("matching journal must require status reconciliation")
    };
    assert_eq!(records, vec![record(1)]);
    // Reconciliation returns data only. Sending remains exclusively an explicit
    // M14.2 worker command, so loading cannot execute or retry a mutation.
}

#[test]
fn duplicate_or_invalid_request_id_is_rejected() {
    let mut journal =
        RecoveryJournal::from_records("boot".into(), "scope".into(), 3, &[record(1), record(2)])
            .unwrap();
    journal.records[1].seq = "1".into();
    assert!(matches!(
        journal.validate(),
        Err(JournalError::DuplicateSequence(1))
    ));
    journal.records[1].seq = "01".into();
    assert!(matches!(
        journal.validate(),
        Err(JournalError::InvalidSequence)
    ));

    let invalid: Value = json!({
        "format_version":1,
        "boot_id":"boot",
        "scope":"scope",
        "next_seq":"2",
        "records":[{"seq":"1","op":"x","args":{},"admission":"pending","msg_id":"7"}]
    });
    assert!(serde_json::from_value::<RecoveryJournal>(invalid).is_err());
}
