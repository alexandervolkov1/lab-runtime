//! Fixed real worker slots; no Runtime/safety mutex is shared with Lua execution.

use lab_core::{
    InstrumentId, TEMPERATURE, Unit,
    managed::{
        ComponentDefinition, ComponentError, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, Correlation, Invocation, InvocationPhase, PlainData,
    },
};
use lab_lua::{LuaSupervisor, fixtures::VIRTUAL_MODEL_SOURCE};
use std::{
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

fn init(id: u64) -> Invocation {
    let component = ComponentId::new(id);
    Invocation {
        correlation: Correlation {
            runtime: 1,
            component,
            generation: 1,
            attempt: 0,
            revision: 0,
        },
        phase: InvocationPhase::Init,
        definition: ComponentDefinition {
            manifest: ComponentManifest {
                schema_version: 1,
                id: component,
                instrument: InstrumentId::new(id),
                name: format!("model {id}"),
                parameter: TEMPERATURE,
                kind: ComponentKind::Source,
                unit: Unit::CELSIUS,
                min: -100.0,
                max: 500.0,
                warmup_samples: 1,
                max_input_age: Duration::from_secs(2),
                history_capacity: 8,
            },
            source: VIRTUAL_MODEL_SOURCE.into(),
            config: PlainData::default(),
        },
        state: PlainData::default(),
        at: Duration::ZERO,
        dt: Duration::ZERO,
        input: None,
    }
}

#[test]
fn exactly_two_real_slots_remain_bounded_and_recover_after_cleanup() {
    let mut supervisor = LuaSupervisor::new().unwrap();
    assert_eq!(LuaSupervisor::active_workers(), 2);
    supervisor.try_submit(init(201)).unwrap();
    supervisor.try_submit(init(202)).unwrap();
    assert_eq!(supervisor.try_submit(init(203)), Err(ComponentError::Busy));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut completed = 0;
    while completed < 2 && Instant::now() < deadline {
        if let Some(result) = supervisor.try_poll() {
            assert!(result.timely);
            assert!(result.outcome.is_ok());
            completed += 1;
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(completed, 2);
    supervisor.try_submit(init(203)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut third = false;
    while Instant::now() < deadline {
        if let Some(result) = supervisor.try_poll() {
            assert_eq!(result.correlation.component, ComponentId::new(203));
            third = true;
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(third);
    let first = init(204);
    let second = init(205);
    supervisor.try_submit(first.clone()).unwrap();
    supervisor.try_submit(second.clone()).unwrap();
    // Acceptance includes host polling: a result finished by the worker but
    // collected at/after 100 ms must not acquire a new publication deadline.
    thread::sleep(Duration::from_millis(110));
    assert!(supervisor.try_expire(first.correlation));
    assert!(supervisor.try_expire(second.correlation));
    assert_eq!(supervisor.try_submit(init(206)), Err(ComponentError::Busy));
    let mut late = 0;
    let late_deadline = Instant::now() + Duration::from_secs(2);
    while late < 2 {
        if let Some(result) = supervisor.try_poll() {
            assert!(!result.timely);
            late += 1;
        } else {
            assert!(Instant::now() < late_deadline, "expired slots never freed");
            thread::yield_now();
        }
    }
    supervisor.try_submit(init(206)).unwrap();
    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    while supervisor.try_poll().is_none() {
        assert!(
            Instant::now() < recovery_deadline,
            "recovered slot never completed"
        );
        thread::yield_now();
    }
    assert_eq!(supervisor.request_shutdown(), 0);
    assert_eq!(LuaSupervisor::active_workers(), 0);
    simultaneous_pool_constructors_cannot_split_the_two_worker_reservation();
}

fn simultaneous_pool_constructors_cannot_split_the_two_worker_reservation() {
    let begin = Arc::new(Barrier::new(3));
    let end = Arc::new(Barrier::new(3));
    let (results, receiver) = mpsc::channel();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let begin = begin.clone();
            let end = end.clone();
            let results = results.clone();
            thread::spawn(move || {
                begin.wait();
                let mut pool = LuaSupervisor::new();
                results.send(pool.is_ok()).unwrap();
                end.wait();
                if let Ok(pool) = pool.as_mut() {
                    assert_eq!(pool.request_shutdown(), 0);
                }
            })
        })
        .collect();
    begin.wait();
    let accepted = [receiver.recv().unwrap(), receiver.recv().unwrap()];
    assert_eq!(accepted.iter().filter(|accepted| **accepted).count(), 1);
    assert_eq!(LuaSupervisor::active_workers(), 2);
    end.wait();
    for worker in threads {
        worker.join().unwrap();
    }
    assert_eq!(LuaSupervisor::active_workers(), 0);
}
