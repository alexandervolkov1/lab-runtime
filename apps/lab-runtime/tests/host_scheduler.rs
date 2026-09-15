//! The M6 host must progress native work without a network client or Lua callback.

use lab_core::{
    Command, Query, QueryResult, SampleQuality,
    control::ControllerState,
    output::{ActuatorId, OutputState},
};
use lab_runtime::host::{Clock, HostCore, SchedulePlan};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{
    cell::Cell,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct TestClock(Cell<Duration>);
impl TestClock {
    fn at(at: Duration) -> Self {
        Self(Cell::new(at))
    }
    fn set(&self, at: Duration) {
        self.0.set(at);
    }
}
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

fn controller(host: &HostCore) -> lab_core::control::ControllerSnapshot {
    let QueryResult::Controller(snapshot) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!()
    };
    snapshot
}

#[test]
fn host_refreshes_measurement_reference_and_native_controller_without_clients() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock::at(Duration::ZERO);
    let first = host.service(&clock).unwrap();
    assert_eq!(
        (first.safety, first.measurements, first.references),
        (1, 1, 1)
    );
    let QueryResult::Latest(Some(initial)) = host
        .query(Query::GetLatestSignal(host.temperature()))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(initial.quality(), SampleQuality::Good);
    assert_eq!(controller(&host).state, ControllerState::Ready);
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: clock.now(),
    })
    .unwrap();
    assert_eq!(controller(&host).state, ControllerState::Warming);
    assert!(controller(&host).lease.is_none());
    for tenth in 1..=70 {
        clock.set(Duration::from_millis(tenth * 100));
        host.service(&clock).unwrap();
    }
    assert_eq!(controller(&host).state, ControllerState::Running);
    let QueryResult::Latest(Some(later)) = host
        .query(Query::GetLatestSignal(host.temperature()))
        .unwrap()
    else {
        panic!()
    };
    assert!(later.at() > initial.at());
    let measured = match later.value().unwrap() {
        lab_core::Value::Float(v) => *v,
        _ => panic!(),
    };
    assert!(
        measured > 20.0,
        "the trusted native loop must move the Rust plant"
    );
    let QueryResult::Reference(reference) =
        host.query(Query::Reference(host.reference_id())).unwrap()
    else {
        panic!()
    };
    let lab_core::reference::ReferenceSnapshot::Ramp { state, .. } = reference else {
        panic!()
    };
    assert!(state.current > 20.0);
}

#[test]
fn scheduler_skips_missed_deadlines_and_fails_an_expired_native_gap_before_pid_replay() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock::at(Duration::ZERO);
    host.service(&clock).unwrap();
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
    for tenth in 1..=4 {
        clock.set(Duration::from_millis(tenth * 100));
        host.service(&clock).unwrap();
    }
    let before = controller(&host);
    assert_eq!(before.state, ControllerState::Running);
    let expires = before.lease.unwrap().expires();
    clock.set(Duration::from_secs(2));
    let report = host.service(&clock).unwrap();
    assert_eq!(report.measurements, 1, "old plant slots coalesce");
    assert_eq!(report.references, 1, "old Reference slots coalesce");
    assert_eq!(report.controller_ticks, 0, "old PID slots never replay");
    assert!(report.skipped_deadlines > 0);
    assert_eq!(controller(&host).state, ControllerState::Failed);
    assert!(controller(&host).lease.is_none());
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::FaultLatched);
    assert!(output.readback.is_some());
    assert!(expires > Duration::from_millis(400));
    let next = host.service(&clock).unwrap();
    assert_eq!(
        next.measurements, 0,
        "deadline advance prevents hot catch-up"
    );
}

#[test]
fn repeated_or_unavailable_input_cannot_advance_pid_or_renew_lease() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock::at(Duration::ZERO);
    host.service(&clock).unwrap();
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
    for tenth in 1..=4 {
        clock.set(Duration::from_millis(tenth * 100));
        host.service(&clock).unwrap();
    }
    let before = controller(&host);
    assert_eq!(before.state, ControllerState::Running);
    let plan = SchedulePlan::without_measurements(
        host.plant_id(),
        host.reference_id(),
        host.controller_id(),
        host.temperature(),
    );
    host.replace_plan(plan).unwrap();
    clock.set(Duration::from_millis(500));
    let report = host.service(&clock).unwrap();
    assert_eq!(report.controller_ticks, 0);
    let after = controller(&host);
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.lease, before.lease);
    clock.set(Duration::from_millis(1000));
    host.service(&clock).unwrap();
    assert_eq!(controller(&host).state, ControllerState::Failed);
}

#[test]
fn safety_service_runs_once_per_due_slot_before_native_work() {
    let mut host = HostCore::virtual_demo().unwrap();
    let clock = TestClock::at(Duration::ZERO);
    let first = host.service(&clock).unwrap();
    assert_eq!(first.safety, 1);
    assert_eq!(host.service(&clock).unwrap().safety, 0);
    clock.set(Duration::from_millis(100));
    let next = host.service(&clock).unwrap();
    assert_eq!(
        next.safety, 1,
        "late safety slots coalesce into one opportunity"
    );
    assert!(next.skipped_deadlines >= 9);
    assert_eq!(next.measurements, 1);
}

#[test]
fn actual_owner_thread_progresses_without_any_tcp_client_and_shuts_down_safely() {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (running_tx, running_rx) = mpsc::sync_channel(1);
    let owner = thread::spawn(move || {
        let options =
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap();
        let mut service = ServiceHost::startup(options).unwrap();
        let clock = service.clock_copy();
        ready_tx.send(service.bound_address()).unwrap();
        service.owner_mut().service(&clock).unwrap();
        service
            .owner_mut()
            .command(Command::StartController {
                controller: lab_core::control::ControllerId::new(1),
                at: clock.now(),
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            service.owner_mut().service(&clock).unwrap();
            let state = controller(service.owner());
            if state.state == ControllerState::Running && state.pid.latest.is_some() {
                let lease = state.lease.unwrap();
                running_tx.send((clock.now(), lease.expires())).unwrap();
                break;
            }
            assert!(
                Instant::now() < until,
                "autonomous native path did not progress"
            );
            thread::sleep(Duration::from_millis(5));
        }
        service.owner_mut().begin_shutdown(&clock).unwrap();
        service.owner_mut().service(&clock).unwrap();
        service.owner().shutdown_status()
    });
    let addr = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    let (now, expiry) = running_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    assert!(expiry > now);
    assert!(owner.join().unwrap().exit_success);
}
