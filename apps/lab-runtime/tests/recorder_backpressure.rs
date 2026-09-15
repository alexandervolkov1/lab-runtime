//! Recorder owner-to-storage handoff remains finite and nonblocking.

use lab_core::{Command, InstrumentId, Runtime, VirtualInstrumentConfig};
use lab_runtime::recorder::{RecorderLimits, RecorderWorker, RecordingState};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-worker-{suffix}.sqlite"))
}

fn await_state(worker: &mut RecorderWorker, state: RecordingState) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != state && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let status = worker.poll();
    assert_eq!(status.state, state, "{status:?}");
}

#[test]
fn bounded_worker_admits_a_group_without_waiting_for_its_sqlite_commit() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    worker.request_start("worker fixture").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(76);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "worker fixture".into(),
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
    let facts = runtime.take_recording_facts();
    assert_eq!(facts.len(), 1);
    worker.try_admit(facts).unwrap();
    assert_eq!(worker.poll().outstanding_records, 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().persisted_through_sequence <= start_watermark && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        worker.poll().persisted_through_sequence,
        start_watermark + 1
    );
    assert_eq!(worker.poll().outstanding_records, 0);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn quiet_recording_progress_requires_a_committed_sqlite_probe() {
    let path = temporary_database();
    let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    worker.request_start("quiet fixture").unwrap();
    await_state(&mut worker, RecordingState::Recording);
    let start_watermark = worker.poll().persisted_through_sequence;
    worker.request_probe_at(Duration::from_millis(250)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().confirmed_submission != Some(Duration::from_millis(250))
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        worker.poll().confirmed_submission,
        Some(Duration::from_millis(250))
    );
    assert_eq!(worker.poll().persisted_through_sequence, start_watermark);
    worker.request_stop().unwrap();
    await_state(&mut worker, RecordingState::Idle);
    worker.request_finish().unwrap();
    await_state(&mut worker, RecordingState::Closed);
    std::fs::remove_file(path).unwrap();
}
