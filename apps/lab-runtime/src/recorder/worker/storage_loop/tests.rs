//! Batching must catch up after a disk stall without changing FIFO or credits.
use super::*;
use lab_core::{InstrumentId, Sample, SignalId, Unit};

fn wait(worker: &mut RecorderWorker, done: impl Fn(&RecordingStatus) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = worker.poll();
        if done(&status) {
            return;
        }
        assert!(Instant::now() < deadline, "{status:?}");
        thread::yield_now();
    }
}

fn fact(sequence: u64) -> RecordingFact {
    RecordingFact::Measurement {
        sequence,
        sample: Sample::validated_good(
            SignalId::new(InstrumentId::new(1), lab_core::TEMPERATURE),
            Unit::CELSIUS,
            Duration::from_millis(sequence),
            Value::Float(20.0),
        )
        .unwrap(),
        generation: 1,
        revision: 1,
        state_revision: None,
        lineage: None,
    }
}

fn fixture() -> (RecorderWorker, WriterBarrier, std::path::PathBuf) {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).unwrap();
    let path = std::env::temp_dir().join(format!("recorder-backlog-{random:x?}.sqlite"));
    let barrier = WriterBarrier::held();
    let mut worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    worker.request_start("backlog").unwrap();
    wait(&mut worker, |s| s.state == RecordingState::Recording);
    (worker, barrier, path)
}

#[test]
fn expired_batch_wait_still_coalesces_queued_groups_up_to_the_cap() {
    let (mut worker, barrier, path) = fixture();
    // Intercept only the queue timestamp, retaining real owner admission,
    // assigned record IDs and credits. No wall-clock sleep or disk speed assumption.
    let (intercept, messages) = mpsc::sync_channel(MAX_GROUPS);
    let sender = std::mem::replace(&mut worker.sender, intercept);
    for sequence in 1..=5 {
        worker.try_admit(vec![fact(sequence)]).unwrap();
        let Message::Facts(facts, bytes, at, _, first) = messages.recv().unwrap() else {
            panic!("expected facts");
        };
        sender
            .send(Message::Facts(
                facts,
                bytes,
                at,
                Instant::now() - Duration::from_secs(1),
                first,
            ))
            .unwrap();
    }
    worker.sender = sender;
    assert_eq!(worker.poll().outstanding_groups, 5);
    barrier.release();
    wait(&mut worker, |s| s.outstanding_groups == 0);
    assert_eq!(worker.poll().outstanding_bytes, 0);
    worker.request_stop().unwrap();
    wait(&mut worker, |s| s.state == RecordingState::Idle);
    worker.request_finish().unwrap();
    wait(&mut worker, |s| s.state == RecordingState::Closed);
    drop(worker);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM measurements", [], |r| r.get(0))
        .unwrap();
    let commits: Vec<u8> = db
        .query_row("SELECT commit_no FROM durable_checkpoints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 5);
    assert_eq!(
        u64::from_be_bytes(commits.try_into().unwrap()),
        5,
        "start, two bounded fact batches, stop, finish"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn ordinary_burst_has_finite_headroom_and_still_fails_closed_at_capacity() {
    let (mut worker, barrier, path) = fixture();
    for sequence in 1..=32 {
        worker.try_admit(vec![fact(sequence)]).unwrap();
    }
    let pending = worker.poll();
    assert_eq!(pending.outstanding_groups, 32);
    assert_eq!(
        pending.limits.groups - pending.outstanding_groups,
        4,
        "Reference pairs keep their protected capacity"
    );
    assert!(worker.try_admit(vec![fact(33)]).is_err());
    assert_eq!(worker.poll().first_missing_fact, Some(33));
    assert_eq!(worker.poll().state, RecordingState::Failed);
    worker.request_finish().unwrap();
    barrier.release();
    wait(&mut worker, |s| s.worker_closed);
    let closed = worker.poll();
    assert!(closed.failure_persisted && closed.terminal_seal_committed);
    assert_eq!(closed.outstanding_groups, 0);
    drop(worker);
    std::fs::remove_file(path).unwrap();
}
