//! Real Lua VM trust-boundary acceptance, not a simulated parser.

use lab_core::{
    InstrumentId, TEMPERATURE, Unit,
    managed::{
        ComponentDefinition, ComponentError, ComponentId, ComponentKind, ComponentManifest,
        ComponentStatus, Correlation, Invocation, InvocationPhase, PlainData,
    },
};
use lab_lua::run_bounded;
use std::{
    process::Command as ProcessCommand,
    sync::{Arc, atomic::AtomicBool},
    thread,
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
          local code = 0
          if _G ~= nil then code = code + 1 end
          if io ~= nil then code = code + 2 end
          if os ~= nil then code = code + 4 end
          if package ~= nil then code = code + 8 end
          if require ~= nil then code = code + 16 end
          if debug ~= nil then code = code + 32 end
          if load ~= nil then code = code + 64 end
          if pcall ~= nil then code = code + 128 end
          if coroutine ~= nil then code = code + 256 end
          return {status='ready', value=code, unit_id='degC', state={}, diagnostics={}}
        end
    "#;
    let result = run_bounded(
        &job(source, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.value, Some(0.0));
}

#[test]
fn string_metatable_cannot_resolve_a_hidden_standard_library() {
    let source = "return function(ctx) local hidden = ('x').sub; return {status='ready', value=1, unit_id='degC', state={}, diagnostics={}} end";
    let result = run_bounded(
        &job(source, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(result.is_err());
}

#[test]
fn forbidden_evidence_keys_and_invalid_scalar_or_state_are_rejected() {
    for bad in [
        "status='ready', value='42', unit_id='degC', state={}, diagnostics={}",
        "status='ready', value=true, unit_id='degC', state={}, diagnostics={}",
        "status='ready', value=42, unit_id='percent', state={}, diagnostics={}",
        "status='ready', value=42, unit_id='degC', state={}, diagnostics={}, ack=true",
        "status='ready', value=42, unit_id='degC', state={}, diagnostics={}, safe_confirmed=true",
        "status='ready', value=42, unit_id='degC', state={}, diagnostics={}, bytes={1,2}",
        "status='ready', value=42, unit_id='degC', state={}, diagnostics={}, actuator=301",
        "status='warming', value=42, unit_id='degC', state={}, diagnostics={}",
        "status='ready', value=1/0, unit_id='degC', state={}, diagnostics={}",
        "status='ready', value=42, unit_id='degC', state={nested={a=1}}, diagnostics={}",
        "status='ready', value=42, unit_id='degC', state={}, diagnostics={'a','b','c','d','e'}",
    ] {
        let source = format!("return function(ctx) return {{{bad}}} end");
        let result = run_bounded(
            &job(&source, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(result.is_err(), "invalid Lua return was accepted: {bad}");
    }
}

#[test]
fn result_and_plain_state_limits_are_checked_independently_of_the_vm_heap() {
    let exact = format!(
        "return function(ctx) return {{status='ready',value=1,unit_id='degC',state={{}},diagnostics={{'{}'}}}} end",
        "x".repeat(256)
    );
    assert!(
        run_bounded(
            &job(&exact, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false))
        )
        .is_ok()
    );
    let excess = exact.replacen(&"x".repeat(256), &"x".repeat(257), 1);
    assert!(
        run_bounded(
            &job(&excess, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    for source in [
        "return function(ctx) local t={}; for i=1,17 do t['key'..i]=i end; return {status='ready',value=1,unit_id='degC',state=t,diagnostics={}} end",
        "return function(ctx) local s='x'; for i=1,9 do s=s..s end; return {status='ready',value=1,unit_id='degC',state={},diagnostics={s}} end",
        "return function(ctx) return {status='ready',value=1,unit_id='degC',state={list={[1]=1,[65]=2}},diagnostics={}} end",
    ] {
        assert!(
            run_bounded(
                &job(source, InvocationPhase::Step),
                Instant::now() + Duration::from_secs(1),
                Arc::new(AtomicBool::new(false)),
            )
            .is_err()
        );
    }
    let mut too_large = job(
        "return function(ctx) return {state={},diagnostics={}} end",
        InvocationPhase::Init,
    );
    too_large.definition.source.push_str(&" ".repeat(32 * 1024));
    assert_eq!(
        run_bounded(
            &too_large,
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false))
        ),
        Err(ComponentError::DataLimit)
    );
}

#[test]
fn globals_and_mutated_input_tables_do_not_persist_between_fresh_vm_jobs() {
    let source = r#"
      counter = (counter or 0) + 1
      return function(ctx)
        if ctx.phase == 'init' then return {state={}, diagnostics={}} end
        ctx.config.rate = 999
        ctx.state.changed = true
        return {status='ready', value=counter, unit_id='degC', state={}, diagnostics={}}
      end
    "#;
    for _ in 0..3 {
        let result = run_bounded(
            &job(source, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(result.value, Some(1.0));
        assert!(result.state.fields.is_empty());
    }
}

#[test]
fn real_host_call_budget_has_a_valid_boundary_and_fails_above_it() {
    for (calls, accepted) in [(120, true), (129, false)] {
        let source = format!(
            r#"
          return function(ctx)
            for i=1,{calls} do type(i) end
            return {{status='ready', value=42, unit_id='degC', state={{}}, diagnostics={{}}}}
          end
        "#
        );
        let result = run_bounded(
            &job(&source, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        );
        if accepted {
            assert_eq!(result.unwrap().value, Some(42.0));
        } else {
            assert_eq!(result, Err(ComponentError::HostCallLimit));
        }
    }
}

#[test]
fn private_arity_gate_rejects_even_extra_nil_without_exposing_select() {
    let valid = "return function(ctx) return {status='ready', value=math.min(2,4)+math.abs(-3), unit_id='degC', state={}, diagnostics={}} end";
    let result = run_bounded(
        &job(valid, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.value, Some(5.0));
    for source in [
        "return function(ctx) return {status='ready', value=math.abs(1,nil), unit_id='degC', state={}, diagnostics={}} end",
        "return function(ctx) return {status='ready', value=math.min(1), unit_id='degC', state={}, diagnostics={}} end",
        "return function(ctx) return {status='ready', value=1, unit_id='degC', state={}, diagnostics={select('#',1)} end",
    ] {
        assert!(
            run_bounded(
                &job(source, InvocationPhase::Step),
                Instant::now() + Duration::from_secs(1),
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
    }
}

#[test]
fn cyclic_and_sparse_plain_state_are_rejected_before_unbounded_copy() {
    for source in [
        "return function(ctx) local t={}; t.self=t; return {status='ready', value=1, unit_id='degC', state=t, diagnostics={}} end",
        "return function(ctx) return {status='ready', value=1, unit_id='degC', state={list={[1]=1,[3]=3}}, diagnostics={}} end",
        "return function(ctx) return {status='ready', value=1, unit_id='degC', state={list={[100000000]=1}}, diagnostics={}} end",
    ] {
        let result = run_bounded(
            &job(source, InvocationPhase::Step),
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(result.is_err());
    }
}

#[test]
fn expired_wall_clock_deadline_rejects_before_vm_construction() {
    let source = "return function(ctx) return {state={}, diagnostics={}} end";
    let result = run_bounded(
        &job(source, InvocationPhase::Init),
        Instant::now(),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(result, Err(ComponentError::Deadline));
}

#[test]
fn real_lua_heap_quota_and_text_only_parser_are_isolated() {
    let large = r#"
      return function(ctx)
        local text = 'x'
        for i=1,27 do text = text .. text end
        return {status='ready', value=42, unit_id='degC', state={}, diagnostics={}}
      end
    "#;
    let result = run_bounded(
        &job(large, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(result, Err(ComponentError::MemoryLimit));
    let invalid = run_bounded(
        &job("\u{1b}Lua-not-text", InvocationPhase::Init),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(invalid, Err(ComponentError::Syntax));
}

#[test]
fn child_infinite_loop_quota_probe() {
    if std::env::var("LAB_LUA_CHILD_QUOTA").as_deref() != Ok("yes") {
        return;
    }
    let source = "return function(ctx) while true do end end";
    let result = run_bounded(
        &job(source, InvocationPhase::Step),
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(result, Err(ComponentError::InstructionLimit));
}

#[test]
fn infinite_lua_loop_is_stopped_under_an_outer_process_watchdog() {
    let executable = std::env::current_exe().unwrap();
    let mut child = ProcessCommand::new(executable)
        .args(["--exact", "child_infinite_loop_quota_probe", "--nocapture"])
        .env("LAB_LUA_CHILD_QUOTA", "yes")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("real Lua instruction hook exceeded the outer process watchdog");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
