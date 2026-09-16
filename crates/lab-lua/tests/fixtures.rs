//! Actual disposable VMs run both chosen M5 observation fixtures.

use lab_core::{
    InstrumentId, SignalId, TEMPERATURE, Unit,
    managed::{
        CapturedInput, ComponentDefinition, ComponentId, ComponentKind, ComponentManifest,
        ComponentStatus, Correlation, Invocation, InvocationPhase, PlainData, PlainValue,
    },
};
use lab_lua::{
    fixtures::{MOVING_MEAN_SOURCE, VIRTUAL_MODEL_SOURCE},
    run_bounded,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn invocation(
    source: &str,
    kind: ComponentKind,
    phase: InvocationPhase,
    clock: (u64, u64),
    config: PlainData,
    state: PlainData,
    input: Option<CapturedInput>,
) -> Invocation {
    let (at, dt) = clock;
    let id = ComponentId::new(210);
    Invocation {
        correlation: Correlation {
            runtime: 1,
            component: id,
            generation: 1,
            attempt: at + 1,
            revision: at,
        },
        phase,
        definition: ComponentDefinition {
            manifest: ComponentManifest {
                schema_version: 1,
                id,
                instrument: InstrumentId::new(210),
                name: "Lua model or mean".into(),
                parameter: TEMPERATURE,
                kind,
                unit: Unit::CELSIUS,
                min: -100.0,
                max: 500.0,
                warmup_samples: if matches!(kind, ComponentKind::Source) {
                    1
                } else {
                    3
                },
                max_input_age: Duration::from_secs(2),
                history_capacity: 8,
            },
            source: source.into(),
            config,
        },
        state,
        at: Duration::from_secs(at),
        dt: Duration::from_secs(dt),
        input,
    }
}

fn run(job: &Invocation) -> lab_core::managed::ComponentResult {
    run_bounded(
        job,
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap()
}

#[test]
fn virtual_temperature_model_feeds_three_sample_mean_through_plain_state() {
    let mut config = PlainData::default();
    config
        .fields
        .insert("baseline".into(), PlainValue::Number(1.0));
    config.fields.insert("rate".into(), PlainValue::Number(1.0));
    let model_state = run(&invocation(
        VIRTUAL_MODEL_SOURCE,
        ComponentKind::Source,
        InvocationPhase::Init,
        (0, 0),
        config.clone(),
        PlainData::default(),
        None,
    ))
    .state;
    let input_signal = SignalId::new(InstrumentId::new(201), TEMPERATURE);
    let kind = ComponentKind::Transform {
        input: input_signal,
    };
    let mut mean_state = run(&invocation(
        MOVING_MEAN_SOURCE,
        kind,
        InvocationPhase::Init,
        (0, 0),
        PlainData::default(),
        PlainData::default(),
        None,
    ))
    .state;
    let mut model_state = model_state;
    for (step, at) in [0, 2, 5].into_iter().enumerate() {
        let model = run(&invocation(
            VIRTUAL_MODEL_SOURCE,
            ComponentKind::Source,
            InvocationPhase::Step,
            (step as u64, u64::from(step != 0)),
            config.clone(),
            model_state,
            None,
        ));
        assert_eq!(model.status, ComponentStatus::Ready);
        assert_eq!(model.value, Some((step + 1) as f64));
        model_state = model.state;
        let input = CapturedInput {
            signal: input_signal,
            value: model.value.unwrap(),
            unit: Unit::CELSIUS,
            at: Duration::from_secs(at),
            freshness_at: Duration::from_secs(at),
            source_generation: 1,
            source_revision: 1,
            source_state_revision: Some(step as u64 + 1),
        };
        let mean = run(&invocation(
            MOVING_MEAN_SOURCE,
            kind,
            InvocationPhase::Step,
            (
                at,
                match step {
                    0 => 0,
                    1 => 2,
                    _ => 3,
                },
            ),
            PlainData::default(),
            mean_state,
            Some(input),
        ));
        assert_eq!(
            mean.status,
            if step == 2 {
                ComponentStatus::Ready
            } else {
                ComponentStatus::Warming
            }
        );
        assert_eq!(mean.value, if step == 2 { Some(2.0) } else { None });
        mean_state = mean.state;
    }
    let mut variant = PlainData::default();
    variant
        .fields
        .insert("baseline".into(), PlainValue::Number(5.0));
    variant
        .fields
        .insert("rate".into(), PlainValue::Number(2.0));
    let second = run(&invocation(
        VIRTUAL_MODEL_SOURCE,
        ComponentKind::Source,
        InvocationPhase::Step,
        (2, 2),
        variant,
        PlainData::default(),
        None,
    ));
    assert_eq!(second.value, Some(9.0));
}
