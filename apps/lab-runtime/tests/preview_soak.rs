//! Bounded developer-preview qualification workloads kept out of ordinary test latency.

use lab_core::{Command, Query, QueryResult, control::ControllerState};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState},
};
use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

struct SoakClock(Cell<Duration>);

impl SoakClock {
    fn new() -> Self {
        Self(Cell::new(Duration::ZERO))
    }

    fn advance(&self, by: Duration) {
        self.0.set(self.0.get() + by);
    }
}

impl Clock for SoakClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m11-preview-soak-{suffix}.sqlite"))
}

fn service_until(host: &mut HostCore, clock: &SoakClock, condition: impl Fn(&HostCore) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !condition(host) {
        assert!(Instant::now() < deadline, "bounded soak lifecycle stalled");
        host.service(clock).unwrap();
        thread::yield_now();
    }
}

#[test]
#[ignore = "bounded developer-preview soak; runs 2,000 scheduled turns and eight recording cycles"]
fn virtual_control_and_recorder_cycles_remain_bounded_and_close_cleanly() {
    const CYCLES: u64 = 8;
    const TURNS_PER_CYCLE: u64 = 250;

    let database = temporary_database();
    let worker = RecorderWorker::open(&database, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = SoakClock::new();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, clock.now())
        .unwrap();
    host.service(&clock).unwrap();
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: clock.now(),
    })
    .unwrap();

    let mut measurement_turns = 0u64;
    let mut controller_ticks = 0u64;
    for cycle in 1..=CYCLES {
        host.start_recording(&format!("preview soak {cycle}"), clock.now())
            .unwrap();
        service_until(&mut host, &clock, |host| {
            host.recording_status().unwrap().state == RecordingState::Recording
        });

        for turn in 1..=TURNS_PER_CYCLE {
            clock.advance(Duration::from_millis(100));
            let report = host.service(&clock).unwrap();
            measurement_turns += report.measurements;
            controller_ticks += report.controller_ticks;

            // Keep the stress substantial but avoid deliberately converting this
            // clean-close oracle into an ingress-exhaustion test.
            service_until(&mut host, &clock, |host| {
                host.recording_status().unwrap().outstanding_groups < 3
            });
            if turn % 25 == 0 {
                let QueryResult::Latest(Some(sample)) = host
                    .query(Query::GetLatestSignal(host.temperature()))
                    .unwrap()
                else {
                    panic!("periodic acquisition stopped")
                };
                assert_eq!(sample.at(), clock.now());
            }
        }

        service_until(&mut host, &clock, |host| {
            host.recording_status().unwrap().outstanding_groups == 0
        });
        host.stop_recording_at(clock.now()).unwrap();
        service_until(&mut host, &clock, |host| {
            host.recording_status().unwrap().state == RecordingState::Idle
        });
        let status = host.recording_status().unwrap();
        assert_eq!(status.coverage, "complete");
        assert_eq!(status.first_error, None);
    }

    let QueryResult::Controller(controller) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!()
    };
    assert_eq!(controller.state, ControllerState::Running);
    host.command(Command::PauseController {
        controller: host.controller_id(),
        at: clock.now(),
    })
    .unwrap();
    host.service(&clock).unwrap();
    host.finish_recorder().unwrap();
    service_until(&mut host, &clock, |host| {
        host.recording_status().unwrap().state == RecordingState::Closed
    });
    assert!(host.shutdown_status().recorder_flushed);
    drop(host);

    let archive_bytes = fs::metadata(&database).unwrap().len();
    let sqlite = rusqlite::Connection::open(&database).unwrap();
    let integrity: String = sqlite
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    let measurements: i64 = sqlite
        .query_row("SELECT COUNT(*) FROM measurements", [], |row| row.get(0))
        .unwrap();
    let sealed_runs: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE state='sealed' AND coverage='complete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let sealed_intervals: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM recording_intervals WHERE state='sealed' AND coverage='complete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(integrity, "ok");
    assert_eq!(sealed_runs, CYCLES as i64);
    assert_eq!(sealed_intervals, CYCLES as i64);
    assert!(measurements >= (CYCLES * TURNS_PER_CYCLE) as i64);
    assert!(measurement_turns >= CYCLES * TURNS_PER_CYCLE);
    assert!(controller_ticks > 1_000);
    eprintln!(
        "preview soak: turns={}, controller_ticks={controller_ticks}, measurements={measurements}, archive_bytes={archive_bytes}",
        CYCLES * TURNS_PER_CYCLE
    );
    drop(sqlite);
    assert!(!database.with_extension("sqlite-wal").exists());
    assert!(!database.with_extension("sqlite-shm").exists());
    fs::remove_file(database).unwrap();
}
