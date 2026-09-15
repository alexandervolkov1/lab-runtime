//! Shutdown must erect a producer barrier before waiting for output safety.

use lab_core::instrument::{
    DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
    MetakonInstrumentConfig,
};
use lab_core::output::{ActuatorId, EvidenceLevel, OutputCommand, SafeProfile};
use lab_core::transport::{
    ByteTransport, ExecutorState, RecoveryStatus, ResourceId, TransportIoError,
};
use lab_core::{
    AccessMode, InstrumentId, ParameterId, ParameterRole, Unit, ValueSpec, WriteEffect,
};
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

struct NeverClean;
impl ByteTransport for NeverClean {
    fn try_write(&mut self, _: &[u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }
    fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Pending)
    }
}

#[test]
fn failed_m3_recovery_and_two_stalled_workers_expire_grace_with_unconfirmed_safe_evidence() {
    let mut host = HostCore::virtual_demo().unwrap();
    host.install_component_executor(Box::new(TwoStalledWorkers(Arc::new(AtomicBool::new(
        false,
    )))))
    .unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
        host,
    )
    .unwrap();
    let resource = ResourceId::new(9);
    let instrument = InstrumentId::new(90);
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    let at = service.clock_copy().now();
    let owner = service.owner_mut();
    owner
        .register_transport(resource, Box::new(NeverClean))
        .unwrap();
    owner
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "failed recovery fixture".into(),
                parameters: vec![
                    DataParameterDefinition {
                        id: ParameterId::new(1),
                        name: "probe".into(),
                        value_spec: ValueSpec::Float {
                            min: -100.0,
                            max: 100.0,
                        },
                        unit: Unit::CELSIUS,
                        access: AccessMode::ReadOnly,
                        role: ParameterRole::Measurement,
                        write_effect: WriteEffect::None,
                        operation: KnownOperation::Temperature,
                        scale: 1.0,
                    },
                    DataParameterDefinition {
                        id: parameter,
                        name: "safe target".into(),
                        value_spec: ValueSpec::Float {
                            min: 0.0,
                            max: 100.0,
                        },
                        unit: Unit::PERCENT,
                        access: AccessMode::ReadWrite,
                        role: ParameterRole::Actuator,
                        write_effect: WriteEffect::OutputAffecting,
                        operation: KnownOperation::Output,
                        scale: 1.0,
                    },
                ],
            },
            binding: MetakonBinding {
                resource,
                device: 15,
                channel: 0,
                binding_generation: 1,
                mapping_revision: 1,
                expected_output_unit: Some(Unit::PERCENT),
            },
            history_capacity: 2,
        }))
        .unwrap();
    owner
        .command(Command::Output {
            actuator,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(1),
                max_proposal_ttl: Duration::from_millis(100),
                required_evidence: EvidenceLevel::Acknowledgement,
            }),
            at,
        })
        .unwrap();
    owner.track_trusted_output(actuator).unwrap();
    owner
        .command(Command::Output {
            actuator,
            command: OutputCommand::RequestSafe,
            at,
        })
        .unwrap();
    owner
        .command(Command::QueueMetakonRead {
            instrument,
            parameter: ParameterId::new(1),
            at,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(50),
        })
        .unwrap();
    owner.command(Command::PollTransports { at }).unwrap();
    let QueryResult::Transport(started) = owner.query(Query::Transport(resource)).unwrap() else {
        panic!()
    };
    assert_eq!(started.state, ExecutorState::InFlight);
    service.request_shutdown().unwrap();
    let began = std::time::Instant::now();
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(
            began.elapsed() < Duration::from_millis(2500),
            "bounded safe grace exceeded"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!terminal.safe_confirmed);
    assert_eq!(terminal.unfinished_workers, 2);
    assert!(!terminal.exit_success);
    let outputs = service.owner().output_safe_records();
    assert!(
        outputs
            .iter()
            .any(|r| r["instrument"] == "90" && r["safe_confirmed"] == false)
    );
    let resources = service.owner().resource_records();
    assert!(
        resources
            .iter()
            .any(|r| r["data"]["state"] == "recovering" || r["data"]["state"] == "offline"),
        "resources={resources:?}"
    );
    let QueryResult::Transport(snapshot) =
        service.owner().query(Query::Transport(resource)).unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        snapshot.state,
        ExecutorState::Recovering | ExecutorState::Offline
    ));
}
