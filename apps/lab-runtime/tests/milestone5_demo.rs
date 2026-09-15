//! Finite M5: two real Lua workers, dependent native authority and unrelated Rust control.

use lab_core::{
    Command, CommandResult, InstrumentId, Query, QueryResult, Runtime, SignalId, TEMPERATURE, Unit,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    managed::{
        ComponentDefinition, ComponentId, ComponentKind, ComponentManifest, ComponentState,
        PlainData, PlainValue,
    },
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputResult, OutputState,
        SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
};
use lab_lua::{
    LuaSupervisor,
    fixtures::{MOVING_MEAN_SOURCE, VIRTUAL_MODEL_SOURCE},
};
use std::{
    thread,
    time::{Duration, Instant},
};

const MODEL: ComponentId = ComponentId::new(201);
const FILTER: ComponentId = ComponentId::new(202);
const CONTROLLED: InstrumentId = InstrumentId::new(301);
const INDEPENDENT: InstrumentId = InstrumentId::new(302);
const CONTROL: ControllerId = ControllerId::new(41);
const NATIVE: ControllerId = ControllerId::new(42);

fn output(
    runtime: &mut Runtime,
    actuator: ActuatorId,
    command: OutputCommand,
    at: u64,
) -> OutputResult {
    let CommandResult::Output(result) = runtime
        .command(Command::Output {
            actuator,
            command,
            at: Duration::from_secs(at),
        })
        .unwrap()
    else {
        panic!()
    };
    result
}

fn actuator(id: InstrumentId) -> ActuatorId {
    ActuatorId::new(id, lab_core::HEATER_POWER)
}

fn plant(
    runtime: &mut Runtime,
    id: InstrumentId,
    reference: ReferenceId,
    controller: ControllerId,
    input: SignalId,
) {
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id,
            name: format!("Rust virtual plant {}", id.get()),
            history_capacity: 64,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    let target = actuator(id);
    output(
        runtime,
        target,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(10),
            max_proposal_ttl: Duration::from_secs(2),
            required_evidence: EvidenceLevel::Readback,
        }),
        0,
    );
    output(runtime, target, OutputCommand::RequestSafe, 0);
    let OutputResult::Dispatched(safe) = output(runtime, target, OutputCommand::BeginDispatch, 0)
    else {
        panic!()
    };
    output(
        runtime,
        target,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
        0,
    );
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: reference,
            value: 60.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: controller,
            input,
            output: target,
            reference,
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
    runtime
        .command(Command::PrepareController(controller))
        .unwrap();
}

fn component(
    id: ComponentId,
    kind: ComponentKind,
    warmup: usize,
    source: &str,
    config: PlainData,
) -> ComponentDefinition {
    ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id,
            instrument: InstrumentId::new(id.get()),
            name: format!("Lua observation {}", id.get()),
            parameter: TEMPERATURE,
            kind,
            unit: Unit::CELSIUS,
            min: -100.0,
            max: 500.0,
            warmup_samples: warmup,
            max_input_age: Duration::from_secs(2),
            history_capacity: 32,
        },
        source: source.into(),
        config,
    }
}

fn wait_for(
    runtime: &mut Runtime,
    id: ComponentId,
    at: u64,
    expected: ComponentState,
    publication: bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        runtime
            .command(Command::PollComponents {
                at: Duration::from_secs(at),
            })
            .unwrap();
        let QueryResult::Component(snapshot) = runtime.query(Query::Component(id)).unwrap() else {
            panic!()
        };
        let published = if publication {
            let signal = SignalId::new(snapshot.instrument, TEMPERATURE);
            let QueryResult::Latest(latest) =
                runtime.query(Query::GetLatestSignal(signal)).unwrap()
            else {
                panic!()
            };
            latest.is_some_and(|sample| sample.at() == Duration::from_secs(at))
        } else {
            true
        };
        if snapshot.state == expected && snapshot.pending.is_none() && published {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Lua worker did not settle for {}",
            id.get()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn refresh_and_tick_native(runtime: &mut Runtime, at: u64) {
    runtime
        .command(Command::RefreshMeasurement {
            instrument: INDEPENDENT,
            parameter: TEMPERATURE,
            at: Duration::from_secs(at),
        })
        .unwrap();
    if at == 0 {
        runtime
            .command(Command::StartController {
                controller: NATIVE,
                at: Duration::ZERO,
            })
            .unwrap();
    } else {
        runtime
            .command(Command::TickController {
                controller: NATIVE,
                at: Duration::from_secs(at),
            })
            .unwrap();
    }
}

#[test]
fn lua_model_filter_native_authority_failure_reload_and_independent_control() {
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(LuaSupervisor::new().unwrap()))
        .unwrap();
    let mut config = PlainData::default();
    config
        .fields
        .insert("baseline".into(), PlainValue::Number(1.0));
    config.fields.insert("rate".into(), PlainValue::Number(1.0));
    let failing_model = VIRTUAL_MODEL_SOURCE.replace(
        "local baseline =",
        "if ctx.now_seconds >= 4 then error('model failure') end\n  local baseline =",
    );
    let source = component(
        MODEL,
        ComponentKind::Source,
        1,
        &failing_model,
        config.clone(),
    );
    runtime
        .command(Command::StageComponent {
            definition: source,
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    wait_for(&mut runtime, MODEL, 0, ComponentState::Warming, false);
    let mean = component(
        FILTER,
        ComponentKind::Transform {
            input: SignalId::new(InstrumentId::new(MODEL.get()), TEMPERATURE),
        },
        3,
        MOVING_MEAN_SOURCE,
        PlainData::default(),
    );
    runtime
        .command(Command::StageComponent {
            definition: mean,
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    wait_for(&mut runtime, FILTER, 0, ComponentState::Warming, false);
    plant(
        &mut runtime,
        CONTROLLED,
        ReferenceId::new(11),
        CONTROL,
        SignalId::new(InstrumentId::new(FILTER.get()), TEMPERATURE),
    );
    plant(
        &mut runtime,
        INDEPENDENT,
        ReferenceId::new(12),
        NATIVE,
        SignalId::new(INDEPENDENT, TEMPERATURE),
    );

    for at in 0..=3 {
        refresh_and_tick_native(&mut runtime, at);
        runtime
            .command(Command::InvokeComponent {
                component: MODEL,
                at: Duration::from_secs(at),
            })
            .unwrap();
        wait_for(&mut runtime, MODEL, at, ComponentState::Ready, true);
        runtime
            .command(Command::InvokeComponent {
                component: FILTER,
                at: Duration::from_secs(at),
            })
            .unwrap();
        wait_for(
            &mut runtime,
            FILTER,
            at,
            if at < 2 {
                ComponentState::Warming
            } else {
                ComponentState::Ready
            },
            true,
        );
        if at == 2 {
            runtime
                .command(Command::StartController {
                    controller: CONTROL,
                    at: Duration::from_secs(at),
                })
                .unwrap();
        } else if at == 3 {
            runtime
                .command(Command::TickController {
                    controller: CONTROL,
                    at: Duration::from_secs(at),
                })
                .unwrap();
        }
    }
    let QueryResult::Controller(running) = runtime.query(Query::Controller(CONTROL)).unwrap()
    else {
        panic!()
    };
    assert_eq!(running.state, ControllerState::Running);
    assert!(running.latest_output.is_some());
    let prior_epoch = running.lease.unwrap().epoch();

    refresh_and_tick_native(&mut runtime, 4);
    runtime
        .command(Command::InvokeComponent {
            component: MODEL,
            at: Duration::from_secs(4),
        })
        .unwrap();
    wait_for(&mut runtime, MODEL, 4, ComponentState::Failed, true);
    let QueryResult::Controller(failed) = runtime.query(Query::Controller(CONTROL)).unwrap() else {
        panic!()
    };
    assert_eq!(failed.state, ControllerState::Failed);
    assert!(failed.lease.is_none());
    let QueryResult::Output(safe) = runtime.query(Query::Output(actuator(CONTROLLED))).unwrap()
    else {
        panic!()
    };
    assert_eq!(safe.state, OutputState::FaultLatched);
    assert_eq!(safe.readback.unwrap().value, 0.0);
    let QueryResult::Controller(other) = runtime.query(Query::Controller(NATIVE)).unwrap() else {
        panic!()
    };
    assert_eq!(other.state, ControllerState::Running);

    runtime
        .command(Command::StageComponent {
            definition: component(
                MODEL,
                ComponentKind::Source,
                1,
                VIRTUAL_MODEL_SOURCE,
                config,
            ),
            replaces: Some(MODEL),
            at: Duration::from_secs(4),
        })
        .unwrap();
    wait_for(&mut runtime, MODEL, 4, ComponentState::Warming, true);
    runtime
        .command(Command::StageComponent {
            definition: component(
                FILTER,
                ComponentKind::Transform {
                    input: SignalId::new(InstrumentId::new(MODEL.get()), TEMPERATURE),
                },
                3,
                MOVING_MEAN_SOURCE,
                PlainData::default(),
            ),
            replaces: Some(FILTER),
            at: Duration::from_secs(4),
        })
        .unwrap();
    wait_for(&mut runtime, FILTER, 4, ComponentState::Warming, true);
    for at in 5..=8 {
        refresh_and_tick_native(&mut runtime, at);
        runtime
            .command(Command::InvokeComponent {
                component: MODEL,
                at: Duration::from_secs(at),
            })
            .unwrap();
        wait_for(&mut runtime, MODEL, at, ComponentState::Ready, true);
        runtime
            .command(Command::InvokeComponent {
                component: FILTER,
                at: Duration::from_secs(at),
            })
            .unwrap();
        wait_for(
            &mut runtime,
            FILTER,
            at,
            if at < 7 {
                ComponentState::Warming
            } else {
                ComponentState::Ready
            },
            true,
        );
        if at == 7 {
            output(
                &mut runtime,
                actuator(CONTROLLED),
                OutputCommand::AcknowledgeFault,
                at,
            );
            runtime
                .command(Command::ResetFailedController {
                    controller: CONTROL,
                    at: Duration::from_secs(at),
                })
                .unwrap();
            runtime
                .command(Command::ResumeController {
                    controller: CONTROL,
                    at: Duration::from_secs(at),
                })
                .unwrap();
        } else if at == 8 {
            runtime
                .command(Command::TickController {
                    controller: CONTROL,
                    at: Duration::from_secs(at),
                })
                .unwrap();
        }
    }
    let QueryResult::Controller(recovered) = runtime.query(Query::Controller(CONTROL)).unwrap()
    else {
        panic!()
    };
    assert_eq!(recovered.state, ControllerState::Running);
    assert!(recovered.lease.unwrap().epoch() > prior_epoch);
    assert!(recovered.latest_output.is_some());
    let QueryResult::Component(model) = runtime.query(Query::Component(MODEL)).unwrap() else {
        panic!()
    };
    let QueryResult::Component(filter) = runtime.query(Query::Component(FILTER)).unwrap() else {
        panic!()
    };
    assert_eq!(model.generation, 2);
    assert_eq!(filter.generation, 2);
    drop(runtime);
    let deadline = Instant::now() + Duration::from_secs(1);
    while LuaSupervisor::active_workers() != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(LuaSupervisor::active_workers(), 0);
}
