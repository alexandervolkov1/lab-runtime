//! Original submission times may be out of order within a committed FIFO prefix.

use lab_core::{InstrumentId, Sample, SignalId, Unit, Value, recording::RecordingFact};
use lab_runtime::recorder::{
    AnnotationRecord, ConfigurationLifecycleRecord, OperationRecord, ProvenanceEntry, RecorderGap,
    RecorderLimits, RecorderWorker, RecordingState, RecordingStatus, WriterBarrier,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(3);

fn database() -> PathBuf {
    let mut entropy = [0; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-receipt-{suffix}.sqlite"))
}

fn wait(
    worker: &mut RecorderWorker,
    predicate: impl Fn(&RecordingStatus) -> bool,
) -> RecordingStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = worker.poll();
        if predicate(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "receipt deadline: {status:?}");
        std::thread::yield_now();
    }
}

// Release the trusted writer gate even when a regression assertion panics.
struct ReleaseOnDrop(WriterBarrier);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn operation(phase: &'static str, at: u64) -> OperationRecord {
    OperationRecord {
        scope: "receipt-test".into(),
        request_seq: 1,
        command: "reference_retune",
        phase,
        data: "{}".into(),
        outcome_basis: if phase == "accepted" {
            "application_admission"
        } else {
            "domain_result"
        },
        at: Duration::from_millis(at),
    }
}

fn fact(sequence: u64, at: u64) -> RecordingFact {
    RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(
            SignalId::new(InstrumentId::new(7), lab_core::TEMPERATURE),
            Unit::CELSIUS,
            Duration::from_millis(at),
            Value::Float(sequence as f64),
        )
        .unwrap(),
        generation: 2,
        revision: 3,
        state_revision: None,
        lineage: None,
    }
}

fn provenance(bytes: &[u8]) -> Vec<ProvenanceEntry> {
    vec![ProvenanceEntry {
        kind: "runtime_toml".into(),
        encoding: "toml_utf8_v1".into(),
        content: bytes.to_vec(),
    }]
}

fn start(worker: &mut RecorderWorker) {
    worker.request_start("receipt regression").unwrap();
    wait(worker, |s| s.state == RecordingState::Recording);
}

fn close(worker: &mut RecorderWorker) {
    if worker.poll().state == RecordingState::Recording {
        worker.request_stop().unwrap();
        wait(worker, |s| s.state == RecordingState::Idle);
    }
    worker.request_finish().unwrap();
    let status = wait(worker, |s| s.worker_closed);
    assert!(status.terminal_seal_committed, "{status:?}");
    assert_eq!(status.outstanding_groups, 0);
    assert_eq!(status.outstanding_records, 0);
    assert_eq!(status.outstanding_bytes, 0);
}

fn verify_archive(path: &std::path::Path) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path).unwrap();
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        !db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let sequences: Vec<Vec<u8>> = db
        .prepare("SELECT record_seq FROM records ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let sequences: Vec<u64> = sequences
        .into_iter()
        .map(|s| u64::from_be_bytes(s.try_into().unwrap()))
        .collect();
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    db
}

#[test]
fn older_fact_receipts_release_exact_credits_without_confirming_held_terminal_or_generation() {
    let path = database();
    let barrier = WriterBarrier::held_terminal_operation_after_acceptance();
    let _release = ReleaseOnDrop(barrier.clone());
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker
        .request_activation(provenance(b"old=true"), vec![])
        .unwrap();
    wait(&mut worker, |s| s.activation_generation == 1);
    start(&mut worker);
    worker
        .try_admit_operation(operation("accepted", 20))
        .unwrap();
    wait(&mut worker, |s| {
        s.outstanding_groups == 0 && s.confirmed_submission == Some(Duration::from_millis(20))
    });
    for (sequence, at) in [(1, 10), (2, 11), (3, 12)] {
        worker
            .try_admit_at(vec![fact(sequence, at)], Duration::from_millis(at))
            .unwrap();
    }
    worker
        .try_admit_operation(operation("completed", 30))
        .unwrap();
    assert!(barrier.wait_until_reached(TIMEOUT));
    // All preceding facts committed; only the terminal is blocked before SQL.
    let held = wait(&mut worker, |s| s.outstanding_groups == 1);
    assert_eq!(held.outstanding_records, 1);
    assert!(held.outstanding_bytes > 0);
    assert_eq!(held.persisted_through_sequence, 6);
    assert_eq!(held.confirmed_submission, Some(Duration::from_millis(20)));
    assert_eq!(
        worker.poll(),
        held,
        "a repeated cumulative receipt must not release credit twice"
    );
    worker
        .try_admit_at(vec![fact(4, 13)], Duration::from_millis(13))
        .unwrap();
    let lifecycle = ConfigurationLifecycleRecord {
        operation_id: 41,
        operation_kind: "reconnect_resource",
        base_revision: 1,
        committed_revision: 1,
        toml_hash: [0x5a; 32],
        affected: vec!["resource:7:binding_generation:2".into()],
        reason: None,
        at: Duration::from_millis(15),
    };
    let generation = worker
        .try_reserve_live_activation(&lifecycle)
        .unwrap()
        .unwrap();
    assert_eq!(generation, 2);
    assert!(
        worker
            .commit_reserved_live_activation(3, provenance(b"new=true"), vec![], lifecycle.clone())
            .is_err()
    );
    worker
        .commit_reserved_live_activation(generation, provenance(b"new=true"), vec![], lifecycle)
        .unwrap();
    let pending = worker.poll();
    assert_eq!(
        pending.activation_generation, 1,
        "queued activation is not committed generation"
    );
    assert_eq!(pending.outstanding_groups, 3);
    assert_eq!(pending.persisted_through_sequence, 6);
    assert_eq!(
        pending.confirmed_submission,
        Some(Duration::from_millis(20)),
        "queued 30 ms is not confirmed"
    );
    barrier.release();
    let committed = wait(&mut worker, |s| {
        s.outstanding_groups == 0 && s.activation_generation == 2
    });
    assert_eq!(
        committed.confirmed_submission,
        Some(Duration::from_millis(30))
    );
    assert_eq!(committed.persisted_through_sequence, 9);
    assert_eq!(committed.state, RecordingState::Recording);
    assert_eq!(committed.coverage, "complete");
    close(&mut worker);
    assert_eq!(worker.poll().state, RecordingState::Closed);
    let database_id = worker.database_id().to_owned();
    let previous_boot = worker.boot_id().to_owned();
    drop(worker);
    let db = verify_archive(&path);
    let phases: Vec<String> = db
        .prepare("SELECT phase FROM operation_events ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(phases, ["accepted", "completed"]);
    let captures: Vec<Vec<u8>> = db
        .prepare("SELECT captured_at FROM records WHERE kind='measurement' ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        captures,
        [10u64, 11, 12, 13].map(|ms| (ms * 1_000_000).to_be_bytes().to_vec())
    );
    let identities: Vec<(Vec<u8>, Vec<u8>)> = db
        .prepare("SELECT generation,revision FROM measurements ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        identities,
        vec![(2u64.to_be_bytes().to_vec(), 3u64.to_be_bytes().to_vec()); 4]
    );
    drop(db);
    let mut reopened = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    assert_eq!(reopened.database_id(), database_id);
    assert_ne!(reopened.boot_id(), previous_boot);
    assert_eq!(
        reopened.poll().activation_generation,
        0,
        "generation belongs to the new worker boot"
    );
    close(&mut reopened);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn older_committed_fact_does_not_hide_durable_gap_or_terminal_boot_seal() {
    let path = database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    start(&mut worker);
    worker
        .try_admit_operation(operation("accepted", 20))
        .unwrap();
    wait(&mut worker, |s| {
        s.outstanding_groups == 0 && s.confirmed_submission == Some(Duration::from_millis(20))
    });
    worker
        .try_admit_at(vec![fact(1, 10)], Duration::from_millis(10))
        .unwrap();
    worker.fail_with_gap(RecorderGap {
        reason: "explicit receipt-test loss".into(),
        at: Duration::from_millis(21),
        first_missing_fact: Some(2),
        known_missing_count: Some(1),
        last_accepted_fact: Some(1),
    });
    let sealed = wait(&mut worker, |s| {
        s.failure_persisted && s.outstanding_groups == 0
    });
    assert_eq!(sealed.confirmed_submission, Some(Duration::from_millis(20)));
    assert_eq!(sealed.persisted_through_sequence, 5);
    assert_eq!(sealed.state, RecordingState::Failed);
    assert_eq!(sealed.coverage, "gap");
    close(&mut worker);
    assert_eq!(
        worker.poll().state,
        RecordingState::Failed,
        "sealing must not erase the failure"
    );
    drop(worker);
    let db = verify_archive(&path);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM gaps", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT state FROM runtime_boots", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "sealed"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn fact_batch_confirms_maximum_original_time_and_preserves_every_member_timestamp() {
    let path = database();
    let barrier = WriterBarrier::held();
    let _release = ReleaseOnDrop(barrier.clone());
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    start(&mut worker);
    for (sequence, at) in [(1, 30), (2, 10), (3, 20)] {
        worker
            .try_admit_at(vec![fact(sequence, at)], Duration::from_millis(at))
            .unwrap();
    }
    assert!(barrier.wait_until_reached(TIMEOUT));
    assert_eq!(worker.poll().confirmed_submission, Some(Duration::ZERO));
    barrier.release();
    let status = wait(&mut worker, |s| s.outstanding_groups == 0);
    assert_eq!(status.confirmed_submission, Some(Duration::from_millis(30)));
    close(&mut worker);
    drop(worker);
    let db = verify_archive(&path);
    let captures: Vec<Vec<u8>> = db
        .prepare("SELECT captured_at FROM records WHERE kind='measurement' ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        captures,
        [30u64, 10, 20].map(|ms| (ms * 1_000_000).to_be_bytes().to_vec())
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn older_operation_annotation_and_probe_keep_committed_high_water_mark_across_intervals() {
    let path = database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    start(&mut worker);
    worker
        .try_admit_operation(operation("accepted", 800))
        .unwrap();
    wait(&mut worker, |s| {
        s.outstanding_groups == 0 && s.confirmed_submission == Some(Duration::from_millis(800))
    });
    worker
        .try_admit_operation(operation("completed", 700))
        .unwrap();
    wait(&mut worker, |s| s.outstanding_groups == 0);
    worker
        .try_admit_annotation(AnnotationRecord {
            scope: "receipt-test".into(),
            request_seq: 2,
            name: "older original time".into(),
            data_json: "{}".into(),
            at: Duration::from_millis(600),
        })
        .unwrap();
    let annotation = wait(&mut worker, |s| s.outstanding_groups == 0);
    assert_eq!(
        annotation.confirmed_submission,
        Some(Duration::from_millis(800))
    );
    worker.request_probe_at(Duration::from_millis(500)).unwrap();
    // Stop is a FIFO barrier after the probe, so Idle proves it was committed.
    worker.request_stop().unwrap();
    let stopped = wait(&mut worker, |s| s.state == RecordingState::Idle);
    assert_eq!(
        stopped.confirmed_submission,
        Some(Duration::from_millis(800))
    );
    worker
        .request_start_at("next interval", Duration::from_millis(900))
        .unwrap();
    let next = wait(&mut worker, |s| s.state == RecordingState::Recording);
    assert_eq!(next.run_no, Some(2));
    assert_eq!(next.interval_no, Some(2));
    assert_eq!(next.confirmed_submission, Some(Duration::from_millis(900)));
    assert!(next.persisted_through_sequence > stopped.persisted_through_sequence);
    close(&mut worker);
    drop(worker);
    drop(verify_archive(&path));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn four_real_uncommitted_groups_still_fail_without_confirming_queued_submissions() {
    let path = database();
    let barrier = WriterBarrier::held();
    let _release = ReleaseOnDrop(barrier.clone());
    let mut worker = RecorderWorker::open_with_barrier(
        &path,
        RecorderLimits {
            groups: 4,
            ..RecorderLimits::default()
        },
        barrier.clone(),
    )
    .unwrap();
    start(&mut worker);
    for at in 1..=4 {
        worker
            .try_admit_at(vec![fact(at, at)], Duration::from_millis(at))
            .unwrap();
    }
    assert!(barrier.wait_until_reached(TIMEOUT));
    let full = worker.poll();
    assert_eq!(full.outstanding_groups, 4);
    assert_eq!(full.outstanding_records, 4);
    assert_eq!(full.confirmed_submission, Some(Duration::ZERO));
    assert_eq!(full.persisted_through_sequence, 2);
    assert_eq!(
        worker
            .try_admit_at(vec![fact(5, 5)], Duration::from_millis(5))
            .unwrap_err()
            .to_string(),
        "recorder ingress capacity exhausted"
    );
    assert_eq!(worker.poll().outstanding_groups, 4);
    barrier.release();
    let failed = wait(&mut worker, |s| {
        s.failure_persisted && s.outstanding_groups == 0
    });
    assert_eq!(failed.confirmed_submission, Some(Duration::from_millis(4)));
    assert_eq!(failed.first_missing_fact, Some(5));
    close(&mut worker);
    drop(worker);
    let db = verify_archive(&path);
    assert_eq!(
        db.query_row("SELECT count(*) FROM measurements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
