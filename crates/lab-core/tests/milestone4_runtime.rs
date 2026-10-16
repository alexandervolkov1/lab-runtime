//! M4 acceptance for Runtime-owned plant and native controller lifecycle.

use lab_core::{
    Command, CommandResult, InstrumentId, Query, QueryResult, Runtime, SignalId, Unit, Value,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputProposal, OutputResult,
        OutputState, SafeProfile,
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
    output_for(runtime, actuator(), command, at)
}

fn output_for(
    runtime: &mut Runtime,
    actuator: ActuatorId,
    command: OutputCommand,
    at: Duration,
) -> OutputResult {
    let CommandResult::Output(result) = runtime
        .command(Command::Output {
            actuator,
            command,
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    result
}

#[test]
fn unrelated_manual_authority_expires_while_native_controller_renews() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    let other_id = InstrumentId::new(101);
    let other = ActuatorId::new(other_id, lab_core::HEATER_POWER);
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: other_id,
            name: "Independent plant".into(),
            history_capacity: 8,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    output_for(
        &mut runtime,
        other,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(5),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Readback,
        }),
        Duration::ZERO,
    );
    output_for(
        &mut runtime,
        other,
        OutputCommand::RequestSafe,
        Duration::ZERO,
    );
    let OutputResult::Dispatched(safe) = output_for(
        &mut runtime,
        other,
        OutputCommand::BeginDispatch,
        Duration::ZERO,
    ) else {
        panic!()
    };
    output_for(
        &mut runtime,
        other,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
        Duration::ZERO,
    );
    let OutputResult::Lease(manual) = output_for(
        &mut runtime,
        other,
        OutputCommand::Acquire {
            owner: lab_core::output::OutputOwner::Manual(1),
            lifetime: Duration::from_secs(2),
        },
        Duration::ZERO,
    ) else {
        panic!()
    };
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    for second in 1..=3 {
        refresh(&mut runtime, second);
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(second),
            })
            .unwrap();
    }
    assert_eq!(state(&runtime).state, ControllerState::Running);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(other)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::SafePending);
    assert!(snapshot.lease.is_none());
    assert!(
        runtime
            .command(Command::Output {
                actuator: other,
                command: OutputCommand::Release(manual),
                at: Duration::from_secs(3),
            })
            .is_err()
    );
}

#[test]
fn pause_after_multi_sample_activation_resets_pid_and_requires_new_warmup() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    for second in 1..=3 {
        refresh(&mut runtime, second);
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(second),
            })
            .unwrap();
    }
    assert!(state(&runtime).pid.latest.is_some());
    let old_epoch = state(&runtime).lease.unwrap().epoch();
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(3),
        })
        .unwrap();
    refresh(&mut runtime, 4);
    runtime
        .command(Command::ResumeController {
            controller: CONTROLLER,
            at: Duration::from_secs(4),
        })
        .unwrap();
    assert_eq!(state(&runtime).state, ControllerState::Warming);
    assert_eq!(state(&runtime).ema.good_samples, 1);
    assert!(state(&runtime).pid.latest.is_none());
    assert_eq!(state(&runtime).pid.integral, 0.0);
    for second in 5..=6 {
        refresh(&mut runtime, second);
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(second),
            })
            .unwrap();
    }
    assert_eq!(state(&runtime).state, ControllerState::Running);
    assert!(state(&runtime).lease.unwrap().epoch() > old_epoch);
    assert!(state(&runtime).pid.latest.is_none());
    refresh(&mut runtime, 7);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(7),
        })
        .unwrap();
    assert_eq!(state(&runtime).pid.latest.unwrap().d, 0.0);
}

#[test]
fn independent_safety_service_expires_stalled_native_owner_without_pid_work() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let before = state(&runtime);
    assert_eq!(
        runtime
            .command(Command::ServiceSafety {
                at: Duration::from_secs(2)
            })
            .unwrap(),
        CommandResult::SafetyServiced
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert!(state(&runtime).lease.is_none());
    assert!(state(&runtime).pid.latest.is_none());
    assert_eq!(state(&runtime).ema, before.ema);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::FaultLatched);
    assert_eq!(snapshot.readback.unwrap().value, 0.0);
}

#[test]
fn independent_safety_service_expires_manual_owner_and_keeps_queries_pure() {
    let mut runtime = setup(fixed(60.0));
    let OutputResult::Lease(manual) = output(
        &mut runtime,
        OutputCommand::Acquire {
            owner: lab_core::output::OutputOwner::Manual(17),
            lifetime: Duration::from_secs(2),
        },
        Duration::ZERO,
    ) else {
        panic!()
    };
    let QueryResult::Output(before) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    for _ in 0..2 {
        let QueryResult::Output(copy) = runtime.query(Query::Output(actuator())).unwrap() else {
            panic!()
        };
        assert_eq!(copy, before);
    }
    assert_eq!(
        runtime
            .command(Command::ServiceSafety {
                at: Duration::from_secs(2)
            })
            .unwrap(),
        CommandResult::SafetyServiced
    );
    let QueryResult::Output(after) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert!(after.lease.is_none());
    assert_eq!(after.state, OutputState::Disarmed);
    assert_eq!(after.readback.unwrap().value, 0.0);
    assert!(manual.expires() < Duration::from_secs(3));
}

#[test]
fn unsuccessful_native_delivery_never_renews_or_retains_authority() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let initial = state(&runtime).lease.unwrap();
    refresh(&mut runtime, 1);
    output(
        &mut runtime,
        OutputCommand::Propose(OutputProposal {
            lease: initial,
            value: Value::Float(10.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_secs(1),
        }),
        Duration::from_secs(1),
    );
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(1)
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert!(state(&runtime).lease.is_none());
    assert!(state(&runtime).latest_output.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::FaultLatched);
    assert!(snapshot.lease.is_none());
    assert_eq!(snapshot.readback.unwrap().value, 0.0);
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
        if second == 1 {
            assert!(
                runtime
                    .command(Command::Output {
                        actuator: actuator(),
                        command: OutputCommand::Release(old),
                        at: Duration::from_secs(1),
                    })
                    .is_err()
            );
        }
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

#[test]
fn a_stale_native_controller_never_releases_a_later_manual_owner() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let first = state(&runtime).lease.unwrap();
    output(&mut runtime, OutputCommand::Tick, Duration::from_secs(5));
    let OutputResult::Dispatched(safe) = output(
        &mut runtime,
        OutputCommand::BeginDispatch,
        Duration::from_secs(5),
    ) else {
        panic!()
    };
    output(
        &mut runtime,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
        Duration::from_secs(5),
    );
    let OutputResult::Lease(manual) = output(
        &mut runtime,
        OutputCommand::Acquire {
            owner: lab_core::output::OutputOwner::Manual(200),
            lifetime: Duration::from_secs(2),
        },
        Duration::from_secs(5),
    ) else {
        panic!()
    };
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(5),
        })
        .unwrap();
    assert_eq!(state(&runtime).state, ControllerState::Paused);
    assert!(state(&runtime).lease.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.lease, Some(manual));
    assert!(manual.epoch() > first.epoch());
    assert_eq!(snapshot.state, OutputState::ArmedManual);
}

#[test]
fn safe_dispatch_busy_retains_failed_controller_and_unknown_output_state() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    output(
        &mut runtime,
        OutputCommand::RequestSafe,
        Duration::from_secs(1),
    );
    let OutputResult::Dispatched(safe) = output(
        &mut runtime,
        OutputCommand::BeginDispatch,
        Duration::from_secs(1),
    ) else {
        panic!()
    };
    refresh(&mut runtime, 1);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(1)
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert!(state(&runtime).lease.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::SafePending);
    assert_eq!(
        snapshot.in_flight.map(|dispatch| dispatch.id()),
        Some(safe.id())
    );
    assert!(!snapshot.safe_confirmed);
}

#[test]
fn pause_safe_dispatch_busy_keeps_paused_controller_queryable() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    output(&mut runtime, OutputCommand::Tick, Duration::from_secs(5));
    let OutputResult::Dispatched(safe) = output(
        &mut runtime,
        OutputCommand::BeginDispatch,
        Duration::from_secs(5),
    ) else {
        panic!()
    };
    assert!(
        runtime
            .command(Command::PauseController {
                controller: CONTROLLER,
                at: Duration::from_secs(5)
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Paused);
    assert!(state(&runtime).lease.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(
        snapshot.in_flight.map(|dispatch| dispatch.id()),
        Some(safe.id())
    );
    assert!(!snapshot.safe_confirmed);
}

#[test]
fn independent_ramp_spans_warmup_and_pause_without_query_side_effects() {
    let mut runtime = setup_with_policy(
        ReferenceConfig::Ramp {
            id: REFERENCE,
            start: 20.0,
            target: 60.0,
            rate: 2.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        },
        3,
        Duration::from_secs(5),
    );
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    refresh(&mut runtime, 1);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let QueryResult::Reference(before) = runtime.query(Query::Reference(REFERENCE)).unwrap() else {
        panic!()
    };
    for _ in 0..3 {
        let QueryResult::Reference(copy) = runtime.query(Query::Reference(REFERENCE)).unwrap()
        else {
            panic!()
        };
        assert_eq!(copy, before);
    }
    refresh(&mut runtime, 2);
    runtime
        .command(Command::ResumeController {
            controller: CONTROLLER,
            at: Duration::from_secs(2),
        })
        .unwrap();
    refresh(&mut runtime, 3);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(3),
        })
        .unwrap();
    refresh(&mut runtime, 4);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(4),
        })
        .unwrap();
    refresh(&mut runtime, 5);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(5),
        })
        .unwrap();
    let QueryResult::Reference(after) = runtime.query(Query::Reference(REFERENCE)).unwrap() else {
        panic!()
    };
    let lab_core::reference::ReferenceSnapshot::Ramp { state: after, .. } = after else {
        panic!()
    };
    assert_eq!(after.current, 30.0);
    assert_eq!(after.last_at, Duration::from_secs(5));
    assert!(
        (state(&runtime).pid.latest.unwrap().p - 3.0 * (30.0 - state(&runtime).ema.value.unwrap()))
            .abs()
            < 1e-10
    );
}

#[test]
fn invalid_start_preserves_ready_memory_and_warming_unavailable_releases_guard() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    let ready = state(&runtime);
    assert!(
        runtime
            .command(Command::StartController {
                controller: CONTROLLER,
                at: Duration::ZERO
            })
            .is_err()
    );
    assert_eq!(state(&runtime), ready);
    refresh(&mut runtime, 1);
    assert!(
        runtime
            .command(Command::StartController {
                controller: CONTROLLER,
                at: Duration::ZERO
            })
            .is_err()
    );
    assert_eq!(state(&runtime), ready);
    runtime
        .command(Command::InjectPlantMeasurementFailure {
            instrument: PLANT,
            at: Duration::from_secs(2),
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::StartController {
                controller: CONTROLLER,
                at: Duration::from_secs(2)
            })
            .is_err()
    );
    assert_eq!(state(&runtime), ready);

    refresh(&mut runtime, 3);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::from_secs(3),
        })
        .unwrap();
    runtime
        .command(Command::InjectPlantMeasurementFailure {
            instrument: PLANT,
            at: Duration::from_secs(4),
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(4)
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
    let OutputResult::Lease(_) = output(
        &mut runtime,
        OutputCommand::Acquire {
            owner: lab_core::output::OutputOwner::Manual(99),
            lifetime: Duration::from_secs(1),
        },
        Duration::from_secs(4),
    ) else {
        panic!()
    };
}

#[test]
fn another_native_controller_cannot_take_the_warming_actuator() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    let other = ControllerId::new(2);
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: other,
            input: SignalId::new(PLANT, lab_core::TEMPERATURE),
            output: actuator(),
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_secs(1),
                warmup_samples: 1,
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
    runtime.command(Command::PrepareController(other)).unwrap();
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::StartController {
                controller: other,
                at: Duration::ZERO
            })
            .is_err()
    );
    let QueryResult::Controller(snapshot) = runtime.query(Query::Controller(other)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, ControllerState::Ready);
    assert!(snapshot.lease.is_none());
    assert_eq!(state(&runtime).state, ControllerState::Warming);
}

#[test]
fn warming_request_safe_and_trip_cancel_delayed_activation() {
    for trip in [false, true] {
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
                    command: OutputCommand::BindProfile(SafeProfile {
                        min: 0.0,
                        max: 100.0,
                        safe_value: 0.0,
                        max_lease: Duration::from_secs(5),
                        max_proposal_ttl: Duration::from_secs(1),
                        required_evidence: EvidenceLevel::Readback,
                    }),
                    at: Duration::ZERO,
                })
                .is_err()
        );
        output(
            &mut runtime,
            if trip {
                OutputCommand::Trip
            } else {
                OutputCommand::RequestSafe
            },
            Duration::ZERO,
        );
        assert_eq!(
            state(&runtime).state,
            if trip {
                ControllerState::Failed
            } else {
                ControllerState::Paused
            }
        );
        refresh(&mut runtime, 1);
        assert!(
            runtime
                .command(Command::TickController {
                    controller: CONTROLLER,
                    at: Duration::from_secs(1)
                })
                .is_err()
        );
        assert!(state(&runtime).lease.is_none());
    }
}

#[test]
fn warmup_uses_uneven_observation_intervals_and_rejects_backwards_service() {
    let mut runtime = setup_with_policy(fixed(60.0), 3, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let previous = state(&runtime);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::ZERO
            })
            .is_err()
    );
    assert_eq!(state(&runtime), previous);
    refresh(&mut runtime, 1);
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    assert_eq!(state(&runtime).ema.good_samples, 2);
    let later = state(&runtime);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::ZERO
            })
            .is_err()
    );
    assert_eq!(state(&runtime), later);
    refresh(&mut runtime, 4);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(4)
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert_eq!(state(&runtime).ema.good_samples, 2);
}

#[test]
fn exact_lease_expiry_cannot_be_renewed_and_pure_queries_do_not_service_watchdog() {
    let mut runtime = setup_with_policy(fixed(60.0), 1, Duration::from_secs(5));
    refresh(&mut runtime, 0);
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let before = state(&runtime);
    for _ in 0..3 {
        assert_eq!(state(&runtime), before);
    }
    refresh(&mut runtime, 5);
    assert!(
        runtime
            .command(Command::TickController {
                controller: CONTROLLER,
                at: Duration::from_secs(5)
            })
            .is_err()
    );
    assert_eq!(state(&runtime).state, ControllerState::Failed);
    assert!(state(&runtime).lease.is_none());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator())).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::FaultLatched);
    assert!(snapshot.safe_confirmed);
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
