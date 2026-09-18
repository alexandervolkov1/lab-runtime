//! Required recording is a Runtime-owned prerequisite, not a client lifetime.

use lab_core::{
    Command, CommandResult, Error, InstrumentId, Query, QueryResult, VirtualInstrumentConfig,
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputResult,
        OutputState, SafeProfile,
    },
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

#[test]
fn required_stop_checks_a_second_tracked_actuator_before_coverage_closes() {
    let path = temporary_database();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    let instrument = InstrumentId::new(901);
    host.command(Command::RegisterVirtual(VirtualInstrumentConfig {
        id: instrument,
        name: "second output".into(),
        history_capacity: 1,
        base_temperature: 20.0,
        measurement_enabled: true,
    }))
    .unwrap();
    let actuator = ActuatorId::new(instrument, lab_core::HEATER_POWER);
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(2),
            max_proposal_ttl: Duration::from_millis(200),
            required_evidence: EvidenceLevel::Readback,
        }),
    })
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::RequestSafe,
    })
    .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(safe)) = host
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BeginDispatch,
        })
        .unwrap()
    else {
        panic!("second safe procedure absent")
    };
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    })
    .unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    host.start_recording("two outputs", Duration::ZERO).unwrap();
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
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(901),
            lifetime: Duration::from_secs(1),
        },
    })
    .unwrap();
    assert!(
        host.stop_recording().is_err(),
        "every tracked output must prove safe before Required stop"
    );
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::RequestSafe,
    })
    .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(safe)) = host
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BeginDispatch,
        })
        .unwrap()
    else {
        panic!("second safe transition absent")
    };
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    })
    .unwrap();
    host.stop_recording().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    host.finish_recorder().unwrap();
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    drop(host);
    std::fs::remove_file(path).unwrap();
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-required-{suffix}.sqlite"))
}

#[test]
fn required_host_refuses_control_until_start_commits_then_records_autonomous_units() {
    let path = temporary_database();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    assert_eq!(
        host.command(Command::StartController {
            controller: host.controller_id(),
            at: Duration::ZERO,
        }),
        Err(Error::RecordingUnavailable)
    );
    host.start_recording("host fixture", Duration::ZERO)
        .unwrap();
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
    let start_watermark = host.recording_status().unwrap().persisted_through_sequence;
    let controller = host
        .query(lab_core::Query::Controller(host.controller_id()))
        .unwrap();
    assert!(
        matches!(&controller, lab_core::QueryResult::Controller(snapshot) if
        snapshot.state == lab_core::control::ControllerState::Ready),
        "controller changed before Required start: {controller:?}; recording status: {:?}",
        host.recording_status()
    );
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
    for step in 1..=5 {
        let at = Duration::from_millis(step * 100);
        host.service(&FakeClock(at)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while host.recording_status().unwrap().outstanding_groups != 0 && Instant::now() < deadline
        {
            host.service(&FakeClock(at)).unwrap();
            std::thread::yield_now();
        }
        assert_eq!(
            host.recording_status().unwrap().outstanding_groups,
            0,
            "fixture writer did not complete its accepted group"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().persisted_through_sequence <= start_watermark
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(500)))
            .unwrap();
        std::thread::yield_now();
    }
    assert!(host.recording_status().unwrap().persisted_through_sequence > start_watermark);
    host.command(Command::PauseController {
        controller: host.controller_id(),
        at: Duration::from_millis(500),
    })
    .unwrap();
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording,
        "recording failed before stop: {:?}",
        host.recording_status()
    );
    host.stop_recording().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(500)))
            .unwrap();
        std::thread::yield_now();
    }
    assert_eq!(host.recording_status().unwrap().state, RecordingState::Idle);
    host.finish_recorder().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(500)))
            .unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Closed
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn required_worker_panic_faults_native_control_without_waiting_for_sql() {
    let path = temporary_database();
    let barrier = WriterBarrier::panic_before_fact_sql();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("Required panic", Duration::ZERO)
        .unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
    assert!(barrier.wait_until_reached(Duration::from_secs(2)));
    while host.recording_status().unwrap().state != RecordingState::Failed {
        assert!(Instant::now() < by);
        host.service(&FakeClock(Duration::from_millis(10))).unwrap();
        std::thread::yield_now();
    }
    let QueryResult::Controller(controller) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!("native controller query unavailable")
    };
    assert_eq!(controller.state, lab_core::control::ControllerState::Failed);
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!("output query unavailable")
    };
    assert!(output.fault_latched);
    assert!(output.lease.is_none());
    assert!(matches!(
        output.state,
        OutputState::SafePending | OutputState::FaultLatched
    ));
    assert_eq!(host.recording_status().unwrap().coverage, "unknown_tail");
    drop(host);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn required_stop_rejects_a_manual_lease_until_rust_confirms_safe_evidence() {
    let path = temporary_database();
    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("manual lease fixture", Duration::ZERO)
        .unwrap();
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
    let actuator = ActuatorId::new(InstrumentId::new(1), lab_core::HEATER_POWER);
    host.command(Command::Output {
        actuator,
        command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(73),
            lifetime: Duration::from_secs(1),
        },
        at: Duration::ZERO,
    })
    .unwrap();
    assert!(
        host.stop_recording().is_err(),
        "manual lease cannot outlive Required coverage"
    );
    host.command(Command::Output {
        actuator,
        command: OutputCommand::RequestSafe,
        at: Duration::ZERO,
    })
    .unwrap();
    host.service(&FakeClock(Duration::from_millis(10))).unwrap();
    assert!(
        host.stop_recording().is_ok(),
        "confirmed Rust safe evidence permits stop"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Idle
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(10))).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(host.recording_status().unwrap().state, RecordingState::Idle);
    host.finish_recorder().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Closed
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::from_millis(10))).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Closed
    );
    drop(host);
    std::fs::remove_file(path).unwrap();
}
