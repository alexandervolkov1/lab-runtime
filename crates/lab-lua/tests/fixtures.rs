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
    at: u64,
    config: PlainData,
    state: PlainData,
    input: Option<CapturedInput>,
) -> Invocation {
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
        dt: Duration::from_secs(if at == 0 { 0 } else { 1 }),
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
        0,
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
        0,
        PlainData::default(),
        PlainData::default(),
        None,
    ))
    .state;
    let mut model_state = model_state;
    for at in 0..=2 {
        let model = run(&invocation(
            VIRTUAL_MODEL_SOURCE,
            ComponentKind::Source,
            InvocationPhase::Step,
            at,
            config.clone(),
            model_state,
            None,
        ));
        assert_eq!(model.status, ComponentStatus::Ready);
        assert_eq!(model.value, Some((at + 1) as f64));
        model_state = model.state;
        let input = CapturedInput {
            signal: input_signal,
            value: model.value.unwrap(),
            unit: Unit::CELSIUS,
            at: Duration::from_secs(at),
            freshness_at: Duration::from_secs(at),
        };
        let mean = run(&invocation(
            MOVING_MEAN_SOURCE,
            kind,
            InvocationPhase::Step,
            at,
            PlainData::default(),
            mean_state,
            Some(input),
        ));
        assert_eq!(
            mean.status,
            if at == 2 {
                ComponentStatus::Ready
            } else {
                ComponentStatus::Warming
            }
        );
        assert_eq!(mean.value, if at == 2 { Some(2.0) } else { None });
        mean_state = mean.state;
    }
}
