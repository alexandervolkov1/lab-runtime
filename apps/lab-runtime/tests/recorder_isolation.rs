//! A stalled SQLite worker must not halt native safety or BestEffort control.

use lab_core::{
    Command, Query, QueryResult,
    output::{ActuatorId, OutputState},
};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, WriterBarrier},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-isolation-{suffix}.sqlite"))
}

fn remove_after_worker_close(path: PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

fn start(host: &mut HostCore) {
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("isolation", Duration::ZERO).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
}

#[test]
fn best_effort_native_pid_keeps_progressing_across_three_leases_while_writer_is_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    start(&mut host);
    let mut ticks = 0;
    for step in 1..=70 {
        let report = host
            .service(&FakeClock(Duration::from_millis(step * 100)))
            .unwrap();
        ticks += report.controller_ticks;
    }
    assert!(ticks > 30, "ticks={ticks}");
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Failed
    );
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::ArmedAuto);
    assert!(output.sent.is_some());
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
}

#[test]
fn required_progress_deadline_trips_native_output_even_with_writer_still_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    start(&mut host);
    host.service(&FakeClock(Duration::from_millis(100)))
        .unwrap();
    host.service(&FakeClock(Duration::from_secs(2))).unwrap();
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert!(output.fault_latched);
    assert!(output.lease.is_none());
    assert!(matches!(
        output.state,
        OutputState::SafePending | OutputState::FaultLatched
    ));
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
}
