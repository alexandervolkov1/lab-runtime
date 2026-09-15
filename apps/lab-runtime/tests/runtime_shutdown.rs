//! Shutdown must erect a producer barrier before waiting for output safety.

use lab_core::{
    Command, Query, QueryResult,
    control::ControllerState,
    managed::{ComponentCompletion, ComponentError, ComponentExecutor, Correlation, Invocation},
};
use lab_runtime::host::{Clock, HostCore};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct TestClock(Cell<Duration>);
impl TestClock {
    fn set(&self, at: Duration) {
        self.0.set(at);
    }
}
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

fn state(host: &HostCore) -> lab_core::control::ControllerSnapshot {
    let QueryResult::Controller(snapshot) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!()
    };
    snapshot
}

#[test]
fn running_controller_stops_production_then_rust_confirms_safe_before_completion() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock(Cell::new(Duration::ZERO));
    host.service(&clock).unwrap();
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: clock.now(),
    })
    .unwrap();
    for tenth in 1..=4 {
        clock.set(Duration::from_millis(tenth * 100));
        host.service(&clock).unwrap();
    }
    assert_eq!(state(&host).state, ControllerState::Running);
    host.begin_shutdown(&clock).unwrap();
    assert!(
        !host.shutdown_status().safe_confirmed,
        "a queued safe request is not completion evidence"
    );
    assert!(
        host.command(Command::StartController {
            controller: host.controller_id(),
            at: clock.now(),
        })
        .is_err(),
        "stop barrier precedes any queued/new Start"
    );
    host.service(&clock).unwrap();
    let status = host.shutdown_status();
    assert_eq!(state(&host).state, ControllerState::Paused);
    assert!(state(&host).lease.is_none());
    assert!(status.safe_confirmed);
    assert_eq!(status.unfinished_workers, 0);
    assert!(status.exit_success);
    clock.set(Duration::from_millis(500));
    let report = host.service(&clock).unwrap();
    assert_eq!(report.measurements, 0);
    assert_eq!(report.controller_ticks, 0);
    assert_eq!(report.references, 0);
}

#[test]
fn warming_controller_has_no_lease_and_cannot_activate_after_shutdown_barrier() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock(Cell::new(Duration::ZERO));
    host.service(&clock).unwrap();
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: clock.now(),
    })
    .unwrap();
    assert_eq!(state(&host).state, ControllerState::Warming);
    assert!(state(&host).lease.is_none());
    clock.set(Duration::from_millis(50));
    host.begin_shutdown(&clock).unwrap();
    for tenth in 1..=8 {
        clock.set(Duration::from_millis(tenth * 100));
        host.service(&clock).unwrap();
    }
    assert_eq!(state(&host).state, ControllerState::Paused);
    assert!(state(&host).lease.is_none());
    assert!(host.shutdown_status().safe_confirmed);
}

struct TwoStalledWorkers(Arc<AtomicBool>);
impl ComponentExecutor for TwoStalledWorkers {
    fn try_submit(&mut self, _: Invocation) -> Result<(), ComponentError> {
        Err(ComponentError::Busy)
    }
    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        None
    }
    fn try_cancel(&mut self, _: Correlation) {}
    fn begin_shutdown(&mut self) {
        self.0.store(true, Ordering::Release);
    }
    fn unfinished_workers(&self) -> usize {
        2
    }
}

#[test]
fn unfinished_workers_are_reported_without_delaying_rust_safe_work() {
    let mut host = HostCore::virtual_demo().unwrap();
    let begun = Arc::new(AtomicBool::new(false));
    host.install_component_executor(Box::new(TwoStalledWorkers(begun.clone())))
        .unwrap();
    let clock = TestClock(Cell::new(Duration::ZERO));
    host.begin_shutdown(&clock).unwrap();
    assert!(begun.load(Ordering::Acquire));
    host.service(&clock).unwrap();
    let status = host.shutdown_status();
    assert!(status.safe_confirmed);
    assert_eq!(status.unfinished_workers, 2);
    assert!(
        !status.exit_success,
        "unfinished cleanup is not a full success"
    );
}

#[test]
fn service_shutdown_grace_stays_nonblocking_and_reports_stalled_cleanup() {
    let mut host = HostCore::virtual_demo().unwrap();
    let begun = Arc::new(AtomicBool::new(false));
    host.install_component_executor(Box::new(TwoStalledWorkers(begun.clone())))
        .unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
        host,
    )
    .unwrap();
    service.request_shutdown().unwrap();
    assert!(begun.load(Ordering::Acquire));
    let one_step_at = std::time::Instant::now();
    assert!(service.shutdown_step().unwrap().is_none());
    assert!(
        one_step_at.elapsed() < Duration::from_millis(30),
        "owner step cannot wait for Lua"
    );
    let until = std::time::Instant::now() + Duration::from_secs(1);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < until,
            "cleanup grace never became terminal"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(terminal.safe_confirmed);
    assert_eq!(terminal.unfinished_workers, 2);
    assert!(!terminal.exit_success);
}
