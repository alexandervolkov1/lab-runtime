//! Real Lua VM trust-boundary acceptance, not a simulated parser.

use lab_core::{
    InstrumentId, TEMPERATURE, Unit,
    managed::{
        ComponentDefinition, ComponentId, ComponentKind, ComponentManifest, ComponentStatus,
        Correlation, Invocation, InvocationPhase, PlainData,
    },
};
use lab_lua::run_bounded;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn job(source: &str, phase: InvocationPhase) -> Invocation {
    let definition = ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: ComponentId::new(1),
            instrument: InstrumentId::new(1),
            name: "Lua virtual model".into(),
            parameter: TEMPERATURE,
            kind: ComponentKind::Source,
            unit: Unit::CELSIUS,
            min: -100.0,
            max: 500.0,
            warmup_samples: 1,
            max_input_age: Duration::from_secs(2),
            history_capacity: 8,
        },
        source: source.into(),
        config: PlainData::default(),
    };
    Invocation {
        correlation: Correlation {
            runtime: 1,
            component: ComponentId::new(1),
            generation: 1,
            attempt: 1,
            revision: 0,
        },
        phase,
        definition,
        state: PlainData::default(),
        at: Duration::from_secs(1),
        dt: Duration::from_secs(1),
        input: None,
    }
}

#[test]
fn real_vm_runs_plain_data_callback_in_a_fresh_environment() {
    let source = r#"
        return function(ctx)
          if ctx.phase == 'init' then
            return {state={}, diagnostics={}}
          end
          return {status='ready', value=42, unit_id='degC', state={}, diagnostics={}}
        end
    "#;
    let cancellation = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs(1);
    let init = run_bounded(
        &job(source, InvocationPhase::Init),
        deadline,
        cancellation.clone(),
    )
    .unwrap();
    assert_eq!(init.status, ComponentStatus::Init);
    assert!(init.value.is_none());
    let step = run_bounded(&job(source, InvocationPhase::Step), deadline, cancellation).unwrap();
    assert_eq!(step.status, ComponentStatus::Ready);
    assert_eq!(step.value, Some(42.0));
}

#[test]
fn real_vm_cannot_reach_io_os_package_debug_or_hidden_string_libraries() {
    let source = r#"
        return function(ctx)
          if _G ~= nil or io ~= nil or os ~= nil or package ~= nil or
             require ~= nil or debug ~= nil or load ~= nil or pcall ~= nil or
             coroutine ~= nil or ('x').sub ~= nil then error('capability') end
          return {status='ready', value=1, unit_id='degC', state={}, diagnostics={}}
        end
    "#;
    let result = run_bounded(
        &job(source, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.value, Some(1.0));
}
