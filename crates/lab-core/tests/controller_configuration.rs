//! M6 PID configuration remains a safe domain mutation with one owner.

use lab_core::{
    Command, CommandResult, InstrumentId, Query, QueryResult, Runtime, SignalId, Unit,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
};
use std::time::Duration;

const PLANT: InstrumentId = InstrumentId::new(22);
const REFERENCE: ReferenceId = ReferenceId::new(23);
const CONTROLLER: ControllerId = ControllerId::new(24);
fn actuator() -> ActuatorId {
    ActuatorId::new(PLANT, lab_core::HEATER_POWER)
}
fn candidate() -> PidConfig {
    PidConfig {
        kp: 2.0,
        ki: 0.2,
        kd: 0.1,
        output_min: 0.0,
        output_max: 80.0,
    }
}

fn setup() -> Runtime {
    setup_with_warmup(1)
}

fn setup_with_warmup(warmup_samples: usize) -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: PLANT,
            name: "Configuration plant".into(),
            history_capacity: 16,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: actuator(),
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(5),
                max_proposal_ttl: Duration::from_secs(1),
                required_evidence: EvidenceLevel::Readback,
            }),
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: actuator(),
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(safe)) = runtime
        .command(Command::Output {
            actuator: actuator(),
            at: Duration::ZERO,
            command: OutputCommand::BeginDispatch,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator: actuator(),
            at: Duration::ZERO,
            command: OutputCommand::Complete {
                dispatch_id: safe.id(),
                outcome: DispatchOutcome::ReadbackVerified,
            },
        })
        .unwrap();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: REFERENCE,
            value: 50.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: CONTROLLER,
            input: SignalId::new(PLANT, lab_core::TEMPERATURE),
            output: actuator(),
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_secs(1),
                warmup_samples,
                unit: Unit::CELSIUS,
            },
            pid: PidConfig {
                kp: 3.0,
                ki: 0.4,
                kd: 0.2,
                output_min: 0.0,
                output_max: 100.0,
            },
            max_input_age: Duration::from_secs(2),
            max_tick_gap: Duration::from_secs(2),
            lease_lifetime: Duration::from_secs(5),
            proposal_ttl: Duration::from_secs(1),
        }))
        .unwrap();
    runtime
        .command(Command::PrepareController(CONTROLLER))
        .unwrap();
    runtime
}

fn controller(runtime: &Runtime) -> (lab_core::control::ControllerSnapshot, PidConfig) {
    let QueryResult::Controller(snapshot) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    let QueryResult::ControllerConfig(config) =
        runtime.query(Query::ControllerConfig(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    (snapshot, config.pid)
}

#[test]
fn ready_configuration_resets_memory_and_keeps_bindings_without_starting_output() {
    let mut runtime = setup();
    let before = controller(&runtime);
    assert_eq!(before.0.state, ControllerState::Ready);
    assert_eq!(before.0.config_revision, 1);
    let CommandResult::ControllerUpdated(committed) = runtime
        .command(Command::ConfigureControllerPid {
            controller: CONTROLLER,
            pid: candidate(),
            expected_revision: 1,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(committed.state, ControllerState::Ready);
    assert_eq!(committed.config_revision, 2);
    assert_eq!(committed.pid.integral, 0.0);
    assert!(committed.lease.is_none());
    let after = controller(&runtime);
    assert_eq!(after.1, candidate());
    assert_eq!(after.0.config_revision, 2);
    let QueryResult::ControllerConfig(config) =
        runtime.query(Query::ControllerConfig(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    assert_eq!(config.input, SignalId::new(PLANT, lab_core::TEMPERATURE));
    assert_eq!(config.output, actuator());
    assert_eq!(config.reference, REFERENCE);
}

#[test]
fn invalid_revisions_limits_and_active_state_reject_without_config_or_memory_change() {
    let mut runtime = setup_with_warmup(2);
    let before = controller(&runtime);
    for (pid, expected_revision) in [
        (
            PidConfig {
                kp: f64::NAN,
                ..candidate()
            },
            1,
        ),
        (
            PidConfig {
                output_max: 101.0,
                ..candidate()
            },
            1,
        ),
        (candidate(), 2),
    ] {
        assert!(
            runtime
                .command(Command::ConfigureControllerPid {
                    controller: CONTROLLER,
                    pid,
                    expected_revision,
                })
                .is_err()
        );
        assert_eq!(controller(&runtime), before);
    }
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let warming = controller(&runtime);
    assert_eq!(warming.0.state, ControllerState::Warming);
    assert!(
        runtime
            .command(Command::ConfigureControllerPid {
                controller: CONTROLLER,
                pid: candidate(),
                expected_revision: 1,
            })
            .is_err()
    );
    assert_eq!(controller(&runtime), warming);
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert_eq!(controller(&runtime).0.state, ControllerState::Running);
    assert!(
        runtime
            .command(Command::ConfigureControllerPid {
                controller: CONTROLLER,
                pid: candidate(),
                expected_revision: 1,
            })
            .is_err()
    );
}

#[test]
fn safe_paused_configuration_is_atomic_and_resets_the_earlier_pid_diagnostics() {
    let mut runtime = setup();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(2),
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(2),
        })
        .unwrap();
    assert!(controller(&runtime).0.pid.latest.is_some());
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(2),
        })
        .unwrap();
    assert_eq!(controller(&runtime).0.state, ControllerState::Paused);
    runtime
        .command(Command::ConfigureControllerPid {
            controller: CONTROLLER,
            pid: candidate(),
            expected_revision: 1,
        })
        .unwrap();
    let after = controller(&runtime);
    assert_eq!(after.0.state, ControllerState::Paused);
    assert_eq!(after.0.config_revision, 2);
    assert_eq!(after.0.pid.integral, 0.0);
    assert_eq!(after.0.pid.previous_measurement, None);
    assert_eq!(after.0.last_tick, None);
    assert_eq!(after.0.latest_output, None);
    assert_eq!(after.1, candidate());
}
