//! M5 Core contract: a nonblocking language-neutral component port and typed signals.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, SampleQuality, SignalId, TEMPERATURE, Unit,
    Value, VirtualInstrumentConfig,
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, ComponentResult, ComponentState, ComponentStatus, Invocation,
        InvocationPhase, PlainData,
    },
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Mailbox {
    submitted: VecDeque<Invocation>,
    completed: VecDeque<ComponentCompletion>,
}

struct ClockedExecutor {
    mailbox: Arc<Mutex<Mailbox>>,
    expired: Arc<AtomicBool>,
}

impl ComponentExecutor for ClockedExecutor {
    fn try_submit(&mut self, job: Invocation) -> Result<(), lab_core::managed::ComponentError> {
        self.mailbox.lock().unwrap().submitted.push_back(job);
        Ok(())
    }

    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        self.mailbox.lock().unwrap().completed.pop_front()
    }

    fn try_cancel(&mut self, _: lab_core::managed::Correlation) {}

    fn try_expire(&mut self, _: lab_core::managed::Correlation) -> bool {
        self.expired.load(Ordering::Acquire)
    }
}

struct FakeExecutor(Arc<Mutex<Mailbox>>);

impl ComponentExecutor for FakeExecutor {
    fn try_submit(&mut self, job: Invocation) -> Result<(), lab_core::managed::ComponentError> {
        self.0.lock().unwrap().submitted.push_back(job);
        Ok(())
    }

    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        self.0.lock().unwrap().completed.pop_front()
    }

    fn try_cancel(&mut self, _: lab_core::managed::Correlation) {}
}

fn definition(id: u64, rate: f64) -> ComponentDefinition {
    let mut config = PlainData::default();
    config
        .fields
        .insert("rate".into(), lab_core::managed::PlainValue::Number(rate));
    ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: ComponentId::new(id),
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
        source: "return function(ctx) return ctx end".into(),
        config,
    }
}

#[test]
fn staged_models_share_generic_discovery_and_never_publish_init_as_good() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    for (id, rate) in [(201, 1.0), (202, 2.0)] {
        runtime
            .command(Command::StageComponent {
                definition: definition(id, rate),
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        assert_eq!(init.phase, InvocationPhase::Init);
        mailbox
            .lock()
            .unwrap()
            .completed
            .push_back(ComponentCompletion {
                correlation: init.correlation,
                timely: true,
                outcome: Ok(ComponentResult {
                    status: ComponentStatus::Init,
                    value: None,
                    unit: Unit::CELSIUS,
                    state: PlainData::default(),
                    diagnostics: vec![],
                }),
            });
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        let QueryResult::Latest(latest) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(id),
                TEMPERATURE,
            )))
            .unwrap()
        else {
            panic!()
        };
        assert!(latest.is_none());
    }
    let QueryResult::Instruments(found) = runtime.query(Query::Discover).unwrap() else {
        panic!()
    };
    assert!(found.iter().any(|item| item.id == InstrumentId::new(201)));
    assert!(found.iter().any(|item| item.id == InstrumentId::new(202)));
}

fn complete(
    mailbox: &Arc<Mutex<Mailbox>>,
    job: Invocation,
    status: ComponentStatus,
    value: Option<f64>,
    state: PlainData,
) {
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: job.correlation,
            timely: true,
            outcome: Ok(ComponentResult {
                status,
                value,
                unit: job.definition.manifest.unit,
                state,
                diagnostics: vec![],
            }),
        });
}

#[test]
fn three_sample_transform_keeps_native_input_lineage_and_atomic_committed_state() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    let input = InstrumentId::new(100);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: input,
            name: "native measurement".into(),
            history_capacity: 8,
            base_temperature: 1.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    let mut filter = definition(220, 0.0);
    filter.manifest.kind = ComponentKind::Transform {
        input: SignalId::new(input, TEMPERATURE),
    };
    filter.manifest.warmup_samples = 3;
    runtime
        .command(Command::StageComponent {
            definition: filter,
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        init,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    for second in 0..=2 {
        runtime
            .command(Command::RefreshMeasurement {
                instrument: input,
                parameter: TEMPERATURE,
                at: Duration::from_secs(second),
            })
            .unwrap();
        runtime
            .command(Command::InvokeComponent {
                component: ComponentId::new(220),
                at: Duration::from_secs(second),
            })
            .unwrap();
        let job = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        assert_eq!(job.input.unwrap().value, (second + 1) as f64);
        assert_eq!(job.input.unwrap().freshness_at, Duration::from_secs(second));
        assert_eq!(
            job.dt,
            if second == 0 {
                Duration::ZERO
            } else {
                Duration::from_secs(1)
            }
        );
        let mut state = PlainData::default();
        state.fields.insert(
            "count".into(),
            lab_core::managed::PlainValue::Number((second + 1) as f64),
        );
        complete(
            &mailbox,
            job,
            if second == 2 {
                ComponentStatus::Ready
            } else {
                ComponentStatus::Warming
            },
            if second == 2 { Some(2.0) } else { None },
            state,
        );
        runtime
            .command(Command::PollComponents {
                at: Duration::from_secs(second),
            })
            .unwrap();
        let QueryResult::Component(snapshot) = runtime
            .query(Query::Component(ComponentId::new(220)))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot.good_steps, second as usize + 1);
        assert_eq!(
            snapshot.state,
            if second == 2 {
                ComponentState::Ready
            } else {
                ComponentState::Warming
            }
        );
        let QueryResult::Latest(latest) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(220),
                TEMPERATURE,
            )))
            .unwrap()
        else {
            panic!()
        };
        let latest = latest.unwrap();
        assert_eq!(latest.at(), Duration::from_secs(second));
        assert_eq!(latest.freshness_at(), Duration::from_secs(second));
        if second == 2 {
            assert_eq!(latest.value(), Some(&Value::Float(2.0)));
        } else {
            assert_eq!(latest.quality(), SampleQuality::Unavailable);
            assert!(latest.value().is_none());
        }
    }
}

#[test]
fn invalid_stage_preserves_committed_model_and_valid_reload_invalidates_same_time_good() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: definition(201, 1.0),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        init,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::ZERO,
        })
        .unwrap();
    let first = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        first,
        ComponentStatus::Ready,
        Some(42.0),
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let signal = SignalId::new(InstrumentId::new(201), TEMPERATURE);
    let QueryResult::Latest(old) = runtime.query(Query::GetLatestSignal(signal)).unwrap() else {
        panic!()
    };
    let old = old.unwrap();
    assert_eq!(old.value(), Some(&Value::Float(42.0)));
    let mut invalid = definition(201, 9.0);
    invalid.manifest.unit = Unit::PERCENT;
    assert!(
        runtime
            .command(Command::StageComponent {
                definition: invalid,
                replaces: Some(ComponentId::new(201)),
                at: Duration::ZERO
            })
            .is_err()
    );
    let QueryResult::Component(before) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(before.generation, 1);
    assert_eq!(before.state, ComponentState::Ready);

    runtime
        .command(Command::StageComponent {
            definition: definition(201, 9.0),
            replaces: Some(ComponentId::new(201)),
            at: Duration::ZERO,
        })
        .unwrap();
    let candidate = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    assert_eq!(candidate.correlation.generation, 2);
    complete(
        &mailbox,
        candidate,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Component(after) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(after.generation, 2);
    assert_eq!(after.state, ComponentState::Warming);
    assert_eq!(after.good_steps, 0);
    let QueryResult::Latest(latest) = runtime.query(Query::GetLatestSignal(signal)).unwrap() else {
        panic!()
    };
    let latest = latest.unwrap();
    assert_eq!(latest.at(), Duration::ZERO);
    assert_eq!(latest.quality(), SampleQuality::Unavailable);
    assert_eq!(old.value(), Some(&Value::Float(42.0)));
}

#[test]
fn forged_completion_identity_and_invalid_candidate_never_commit_partial_state() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: definition(201, 1.0),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        init,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::ZERO,
        })
        .unwrap();
    let job = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    let mut forged = job.correlation;
    forged.generation += 1;
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: forged,
            timely: true,
            outcome: Ok(ComponentResult {
                status: ComponentStatus::Ready,
                value: Some(999.0),
                unit: Unit::CELSIUS,
                state: PlainData::default(),
                diagnostics: vec![],
            }),
        });
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Component(pending) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(pending.pending, Some(job.correlation));
    assert_eq!(pending.committed_state, PlainData::default());
    let mut candidate = PlainData::default();
    candidate.fields.insert(
        "mutated".into(),
        lab_core::managed::PlainValue::Number(99.0),
    );
    complete(
        &mailbox,
        job,
        ComponentStatus::Ready,
        Some(f64::NAN),
        candidate,
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Component(failed) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(failed.state, ComponentState::Failed);
    assert_eq!(failed.committed_state, PlainData::default());
    assert!(failed.pending.is_none());
    let QueryResult::Latest(latest) = runtime
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(201),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(latest.unwrap().quality(), SampleQuality::Unavailable);
}

#[test]
fn late_transform_result_retains_old_input_freshness_and_exact_age_is_unusable() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    let input = InstrumentId::new(100);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: input,
            name: "native input".into(),
            history_capacity: 8,
            base_temperature: 1.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    let mut definition = definition(220, 0.0);
    definition.manifest.kind = ComponentKind::Transform {
        input: SignalId::new(input, TEMPERATURE),
    };
    runtime
        .command(Command::StageComponent {
            definition,
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        init,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: input,
            parameter: TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(220),
            at: Duration::ZERO,
        })
        .unwrap();
    let step = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        step,
        ComponentStatus::Ready,
        Some(1.0),
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    let output = SignalId::new(InstrumentId::new(220), TEMPERATURE);
    let QueryResult::Latest(good) = runtime.query(Query::GetLatestSignal(output)).unwrap() else {
        panic!()
    };
    let good = good.unwrap();
    assert_eq!(good.at(), Duration::from_secs(1));
    assert_eq!(good.freshness_at(), Duration::ZERO);
    assert_eq!(good.value(), Some(&Value::Float(1.0)));
    assert!(
        runtime
            .command(Command::InvokeComponent {
                component: ComponentId::new(220),
                at: Duration::from_secs(2),
            })
            .is_err()
    );
    let QueryResult::Component(snapshot) = runtime
        .query(Query::Component(ComponentId::new(220)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.state, ComponentState::Failed);
    let QueryResult::Latest(failed) = runtime.query(Query::GetLatestSignal(output)).unwrap() else {
        panic!()
    };
    assert_eq!(failed.unwrap().quality(), SampleQuality::Unavailable);
    assert_eq!(good.value(), Some(&Value::Float(1.0)));
}

#[test]
fn adapter_deadline_fences_even_a_late_completion_from_the_current_generation() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let expired = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(ClockedExecutor {
            mailbox: mailbox.clone(),
            expired: expired.clone(),
        }))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: definition(201, 1.0),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let init = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        init,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::ZERO,
        })
        .unwrap();
    let current = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    // The adapter clock has not reached its exclusive acceptance boundary.
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Component(before) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(before.pending, Some(current.correlation));
    expired.store(true, Ordering::Release);
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let current_generation = current.correlation.generation;
    complete(
        &mailbox,
        current,
        ComponentStatus::Ready,
        Some(999.0),
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Component(after) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(after.generation, current_generation);
    assert_eq!(after.state, ComponentState::Failed);
    assert!(after.pending.is_none());
    assert_eq!(after.good_steps, 0);
    let QueryResult::Latest(Some(latest)) = runtime
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(201),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(latest.quality(), SampleQuality::Unavailable);
}
