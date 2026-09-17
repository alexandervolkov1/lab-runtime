//! M9A acceptance for the native implementation behind the neutral executor port.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, SignalId, TEMPERATURE, Unit,
    VirtualInstrumentConfig,
    managed::{
        ComponentDefinition, ComponentId, ComponentImplementation, ComponentKind,
        ComponentManifest, ComponentState, PlainData, PlainValue,
    },
};
use lab_runtime::managed_executor::{MOVING_MEAN_IMPLEMENTATION, ManagedExecutor};
use std::{
    thread,
    time::{Duration, Instant},
};

const INPUT: InstrumentId = InstrumentId::new(901);
const FILTER: ComponentId = ComponentId::new(902);

fn definition(unit: Unit) -> ComponentDefinition {
    let mut config = PlainData::default();
    config
        .fields
        .insert("window".into(), PlainValue::Number(3.0));
    ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: FILTER,
            instrument: InstrumentId::new(FILTER.get()),
            name: "Native moving mean".into(),
            parameter: TEMPERATURE,
            kind: ComponentKind::Transform {
                input: SignalId::new(INPUT, TEMPERATURE),
            },
            unit,
            min: -100.0,
            max: 500.0,
            warmup_samples: 3,
            max_input_age: Duration::from_secs(2),
            history_capacity: 8,
        },
        implementation: ComponentImplementation::built_in(MOVING_MEAN_IMPLEMENTATION).unwrap(),
        config,
    }
}

fn wait_settled(runtime: &mut Runtime, at: Duration, generation: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        runtime.command(Command::PollComponents { at }).unwrap();
        if matches!(runtime.query(Query::Component(FILTER)),
            Ok(QueryResult::Component(snapshot))
                if snapshot.pending.is_none() && snapshot.generation == generation)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native managed work did not settle"
        );
        thread::yield_now();
    }
}

#[test]
fn moving_mean_uses_plain_bounded_state_and_resets_on_one_generation_replacement() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: INPUT,
            name: "Native input".into(),
            history_capacity: 8,
            base_temperature: 10.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime
        .install_component_executor(Box::new(ManagedExecutor::new().unwrap()))
        .unwrap();
    assert_eq!(ManagedExecutor::active_workers(), 2);

    assert!(
        runtime
            .command(Command::StageComponent {
                definition: definition(Unit::PERCENT),
                replaces: None,
                at: Duration::ZERO,
            })
            .is_err()
    );

    runtime
        .command(Command::StageComponent {
            definition: definition(Unit::CELSIUS),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    wait_settled(&mut runtime, Duration::ZERO, 1);

    let mut observed = Vec::new();
    for second in 0..3 {
        let at = Duration::from_secs(second);
        runtime
            .command(Command::RefreshMeasurement {
                instrument: INPUT,
                parameter: TEMPERATURE,
                at,
            })
            .unwrap();
        let QueryResult::Latest(Some(input)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(INPUT, TEMPERATURE)))
            .unwrap()
        else {
            panic!()
        };
        let lab_core::Value::Float(value) = input.value().unwrap() else {
            panic!()
        };
        observed.push(*value);
        runtime
            .command(Command::InvokeComponent {
                component: FILTER,
                at,
            })
            .unwrap();
        wait_settled(&mut runtime, at, 1);
    }

    let QueryResult::Component(ready) = runtime.query(Query::Component(FILTER)).unwrap() else {
        panic!()
    };
    assert_eq!(ready.state, ComponentState::Ready);
    assert_eq!(ready.generation, 1);
    assert_eq!(
        ready.committed_state.fields.get("values"),
        Some(&PlainValue::Numbers(observed.clone()))
    );
    let QueryResult::Latest(Some(mean)) = runtime
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(FILTER.get()),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    let expected = observed.iter().sum::<f64>() / observed.len() as f64;
    assert_eq!(mean.value(), Some(&lab_core::Value::Float(expected)));

    runtime
        .command(Command::StageComponent {
            definition: definition(Unit::CELSIUS),
            replaces: Some(FILTER),
            at: Duration::from_secs(3),
        })
        .unwrap();
    wait_settled(&mut runtime, Duration::from_secs(3), 2);
    let QueryResult::Component(replaced) = runtime.query(Query::Component(FILTER)).unwrap() else {
        panic!()
    };
    assert_eq!(replaced.generation, 2);
    assert_eq!(replaced.state, ComponentState::Warming);
    assert_eq!(replaced.good_steps, 0);
    assert_eq!(
        replaced.committed_state.fields.get("values"),
        Some(&PlainValue::Numbers(Vec::new()))
    );

    runtime
        .command(Command::QuiesceManaged {
            at: Duration::from_secs(4),
        })
        .unwrap();
    drop(runtime);
    let shutdown_deadline = Instant::now() + Duration::from_secs(1);
    while ManagedExecutor::active_workers() != 0 {
        assert!(
            Instant::now() < shutdown_deadline,
            "managed workers did not retire finitely"
        );
        thread::yield_now();
    }
}
