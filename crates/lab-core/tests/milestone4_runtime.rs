//! M4 acceptance for Runtime-owned plant and native controller lifecycle.

use lab_core::{
    Command, CommandResult, InstrumentId, Query, QueryResult, Runtime, SignalId, Unit, Value,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, OutputState,
        SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
};
use std::time::Duration;

const PLANT: InstrumentId = InstrumentId::new(100);
const REFERENCE: ReferenceId = ReferenceId::new(1);
const CONTROLLER: ControllerId = ControllerId::new(1);

fn actuator() -> ActuatorId {
    ActuatorId::new(PLANT, lab_core::HEATER_POWER)
}

fn output(runtime: &mut Runtime, command: OutputCommand, at: Duration) -> OutputResult {
    let CommandResult::Output(result) = runtime
        .command(Command::Output {
            actuator: actuator(),
            command,
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    result
}

fn setup(reference: ReferenceConfig) -> Runtime {
    setup_with_policy(reference, 1, Duration::from_secs(100))
}

fn setup_with_policy(
    reference: ReferenceConfig,
    warmup: usize,
    lease_lifetime: Duration,
) -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: PLANT,
            name: "Controlled thermal plant".into(),
            history_capacity: 64,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    output(
        &mut runtime,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(120),
            max_proposal_ttl: Duration::from_secs(2),
            required_evidence: EvidenceLevel::Readback,
        }),
        Duration::ZERO,
    );
    output(&mut runtime, OutputCommand::RequestSafe, Duration::ZERO);
    let OutputResult::Dispatched(safe) =
        output(&mut runtime, OutputCommand::BeginDispatch, Duration::ZERO)
    else {
        panic!()
    };
    output(
        &mut runtime,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
        Duration::ZERO,
    );
    runtime
        .command(Command::RegisterReference(reference))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: CONTROLLER,
            input: SignalId::new(PLANT, lab_core::TEMPERATURE),
            output: actuator(),
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_secs(1),
                warmup_samples: warmup,
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
            lease_lifetime,
            proposal_ttl: Duration::from_secs(1),
        }))
        .unwrap();
    runtime
        .command(Command::PrepareController(CONTROLLER))
        .unwrap();
    runtime
}

#[test]
fn three_distinct_observations_warm_without_actuator_authority_or_pid_work() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    for second in 0..=3 {
        refresh(&mut runtime, second);
        runtime
            .command(if second == 0 {
                Command::StartController {
                    controller: CONTROLLER,
                    at: Duration::ZERO,
                }
            } else {
                Command::TickController {
                    controller: CONTROLLER,
                    at: Duration::from_secs(second),
                }
            })
            .unwrap();
        let snapshot = state(&runtime);
        if second < 2 {
            assert_eq!(snapshot.state, ControllerState::Warming);
            assert!(snapshot.lease.is_none());
            assert!(snapshot.pid.latest.is_none());
            assert!(snapshot.latest_output.is_none());
            assert_eq!(snapshot.ema.good_samples, second as usize + 1);
        } else if second == 2 {
            assert_eq!(snapshot.state, ControllerState::Running);
            assert_eq!(snapshot.ema.good_samples, 3);
            assert!(snapshot.lease.is_some());
            assert!(snapshot.pid.latest.is_none());
            assert!(snapshot.latest_output.is_none());
        } else {
            assert_eq!(snapshot.state, ControllerState::Running);
            assert!(snapshot.latest_output.is_some());
        }
    }
}

#[test]
fn repeated_warming_sample_does_not_count_and_exclusive_freshness_fails_closed() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert_eq!(state(&runtime).ema.good_samples, 1);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(2),
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert!(state(&runtime).lease.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::Disarmed);
    assert!(snapshot.safe_confirmed);
}

#[test]
fn warming_guard_blocks_competing_acquire_and_pause_resume_restarts_warmup() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::Output {
                actuator: actuator(),
                command: OutputCommand::Acquire {
                    owner: lab_core::output::OutputOwner::Automatic(CONTROLLER.get()),
                    lifetime: Duration::from_secs(1),
                },
                at: Duration::ZERO,
            })
            .is_err()
    );
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    assert_eq!(state(&runtime).state, ControllerState::Paused);
    refresh(&mut runtime, 1);
    runtime
        .command(Command::ResumeController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert_eq!(state(&runtime).state, ControllerState::Warming);
    assert_eq!(state(&runtime).ema.good_samples, 1);
    assert_eq!(state(&runtime).pid.integral, 0.0);
}

#[test]
fn healthy_native_lease_replaces_only_its_token_and_remains_finite() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let old = state(&runtime).lease.unwrap();
    for second in 1..=30 {
        refresh(&mut runtime, second);
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(second),
            })
            .unwrap();
        let lease = state(&runtime).lease.unwrap();
        assert_eq!(state(&runtime).state, ControllerState::Running);
        assert_eq!(lease.epoch(), old.epoch());
        assert_eq!(lease.owner(), old.owner());
        assert_eq!(lease.expires(), Duration::from_secs(second + 5));
    }
    assert!(
        runtime
            .command(Command::Output {
                actuator: actuator(),
                command: OutputCommand::Release(old),
                at: Duration::from_secs(30),
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Running);
}

fn fixed(value: f64) -> ReferenceConfig {
    ReferenceConfig::Fixed {
        id: REFERENCE,
        value,
        unit: Unit::CELSIUS,
    }
}

fn refresh(runtime: &mut Runtime, seconds: u64) {
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(seconds),
        })
        .unwrap();
}

fn state(runtime: &Runtime) -> lab_core::control::ControllerSnapshot {
    let QueryResult::Controller(snapshot) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    snapshot
}

#[test]
fn controller_start_pause_and_resume_use_distinct_output_authority() {
    let mut runtime = setup(fixed(60.0));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let first_epoch = state(&runtime).lease.unwrap().epoch();
    assert_eq!(state(&runtime).state, ControllerState::Running);

    refresh(&mut runtime, 1);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let QueryResult::Output(running) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert!(running.sent.unwrap().value > 0.0);
    assert!(running.readback.is_some());

    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert_eq!(state(&runtime).state, ControllerState::Paused);
    let QueryResult::Output(paused) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(paused.state, OutputState::Disarmed);
    assert_eq!(paused.readback.unwrap().value, 0.0);

    refresh(&mut runtime, 2);
    runtime
        .command(Command::ResumeController {
            controller: CONTROLLER,
            at: Duration::from_secs(2),
        })
        .unwrap();
    let resumed = state(&runtime);
    assert_eq!(resumed.state, ControllerState::Running);
    assert!(resumed.lease.unwrap().epoch() > first_epoch);
    assert_eq!(resumed.pid.integral, 0.0);
}

#[test]
fn pause_settles_safe_output_when_watchdog_already_expired_the_lease() {
    let mut runtime = setup(fixed(60.0));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();

    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(100),
        })
        .unwrap();

    assert_eq!(state(&runtime).state, ControllerState::Paused);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::Disarmed);
    assert_eq!(snapshot.readback.unwrap().value, 0.0);
}

#[test]
fn stale_input_faults_controller_revokes_and_completes_safe() {
    let mut runtime = setup(fixed(60.0));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(3),
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::FaultLatched);
    assert!(snapshot.lease.is_none());
    assert_eq!(snapshot.readback.unwrap().value, 0.0);
}

#[test]
fn large_tick_gap_faults_even_with_a_fresh_sample() {
    let mut runtime = setup(fixed(60.0));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    refresh(&mut runtime, 3);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(3),
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
}

#[test]
fn closed_loop_moves_temperature_toward_reference_through_arbiter() {
    let mut runtime = setup(ReferenceConfig::Ramp {
        id: REFERENCE,
        start: 20.0,
        target: 60.0,
        rate: 10.0,
        unit: Unit::CELSIUS,
        at: Duration::ZERO,
    });
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    for second in 1..=30 {
        refresh(&mut runtime, second);
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(second),
            })
            .unwrap();
    }
    let QueryResult::Latest(Some(sample)) = runtime
        .query(Query::GetLatestSignal(SignalId::new(
            PLANT,
            lab_core::TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    let Some(Value::Float(temperature)) = sample.value() else {
        panic!()
    };
    assert!(*temperature > 45.0);
    assert!((60.0 - temperature).abs() < 40.0);

    assert!(
        runtime
            .command(Command::ConfigureParameter {
                instrument: PLANT,
                parameter: lab_core::HEATER_POWER,
                value: Value::Float(0.0),
            })
            .is_err()
    );
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert!(snapshot.sent.is_some());
    assert!(snapshot.readback.is_some());
}

#[test]
fn unavailable_input_faults_without_reusing_old_good_value() {
    let mut runtime = setup(fixed(60.0));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::InjectPlantMeasurementFailure {
            instrument: PLANT,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(1),
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
}
