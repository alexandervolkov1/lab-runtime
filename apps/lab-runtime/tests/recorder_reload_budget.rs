//! C20 configuration activation uses M7's unchanged four-group ingress credit.

use lab_runtime::recorder::{
    ConfigurationLifecycleRecord, OperationRecord, ProvenanceEntry, RecorderLimits, RecorderWorker,
    RecordingState, WriterBarrier,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-reload-budget-{suffix}.sqlite"))
}

fn provenance(content: &[u8]) -> Vec<ProvenanceEntry> {
    vec![ProvenanceEntry {
        kind: "runtime_toml".into(),
        encoding: "toml_utf8_v1".into(),
        content: content.to_vec(),
    }]
}

fn await_state(worker: &mut RecorderWorker, expected: RecordingState) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != expected {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn c20_live_activation_reserves_one_of_the_same_four_groups_and_commits_one_fact() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker
        .request_activation(provenance(b"old=true"), vec![])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 1 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    worker.request_start("reload budget").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    for seq in 1..=4 {
        worker
            .try_admit_operation(OperationRecord {
                scope: "reload-budget".into(),
                request_seq: seq,
                command: "reference_retune",
                phase: "accepted",
                data: "{}".into(),
                outcome_basis: "application_admission",
                at: Duration::from_millis(seq),
            })
            .unwrap();
    }
    assert_eq!(worker.poll().outstanding_groups, 4);
    let lifecycle = ConfigurationLifecycleRecord {
        operation_id: 7,
        operation_kind: "reload_configuration",
        base_revision: 1,
        committed_revision: 2,
        toml_hash: [0x5a; 32],
        affected: vec!["instrument:41:generation:1".into()],
        reason: None,
        at: Duration::from_millis(10),
    };
    assert_eq!(
        worker.try_reserve_live_activation(&lifecycle).unwrap(),
        None
    );
    assert_eq!(worker.poll().state, RecordingState::Recording);

    barrier.release();
    while worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let reservation = worker
        .try_reserve_live_activation(&lifecycle)
        .unwrap()
        .expect("one ordinary group must now be reserved");
    assert_eq!(worker.poll().outstanding_groups, 1);
    worker
        .commit_reserved_live_activation(reservation, provenance(b"new=true"), vec![], lifecycle)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().activation_generation != 2 || worker.poll().outstanding_groups != 0 {
        assert!(Instant::now() < deadline, "{:?}", worker.poll());
        std::thread::yield_now();
    }
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    drop(worker);

    let db = rusqlite::Connection::open(&path).unwrap();
    let payload: Vec<u8> = db
        .query_row(
            "SELECT payload FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["base_revision"], "1");
    assert_eq!(payload["committed_revision"], "2");
    assert_eq!(payload["activation_generation"], "2");
    assert_eq!(payload["activation_root"].as_str().unwrap().len(), 64);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
