//! M5 Core contract: a nonblocking language-neutral component port and typed signals.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, SampleQuality, SignalId, TEMPERATURE, Unit,
    Value, VirtualInstrumentConfig,
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, ComponentResult, ComponentState,
        ComponentStatus, Invocation, InvocationPhase, PlainData,
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
        implementation: ComponentImplementation::built_in("test.fake.v1").unwrap(),
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
    for (id, rate) in [(201, 1.0), (202, 2.0)] {
        runtime
            .command(Command::InvokeComponent {
                component: ComponentId::new(id),
                at: Duration::ZERO,
            })
            .unwrap();
        let job = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        assert_eq!(
            job.definition.config.fields.get("rate"),
            Some(&lab_core::managed::PlainValue::Number(rate))
        );
        complete(
            &mailbox,
            job,
            ComponentStatus::Ready,
            Some(20.0 + rate),
            PlainData::default(),
        );
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        let QueryResult::Latest(Some(observed)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(id),
                TEMPERATURE,
            )))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(observed.value(), Some(&Value::Float(20.0 + rate)));
    }
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
    let mut wrong_runtime = job.correlation;
    wrong_runtime.runtime += 1;
    let mut wrong_component = job.correlation;
    wrong_component.component = ComponentId::new(202);
    let mut wrong_generation = job.correlation;
    wrong_generation.generation += 1;
    let mut wrong_attempt = job.correlation;
    wrong_attempt.attempt += 1;
    let mut wrong_revision = job.correlation;
    wrong_revision.revision += 1;
    for forged in [
        wrong_runtime,
        wrong_component,
        wrong_generation,
        wrong_attempt,
        wrong_revision,
    ] {
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
    }
    for _ in 0..3 {
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
    }
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
    let original = job.correlation;
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
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: original,
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

#[test]
fn trusted_plain_data_and_source_boundary_admission_is_atomic() {
    let mut data = PlainData::default();
    data.fields.insert(
        "payload".into(),
        lab_core::managed::PlainValue::Text("x".repeat(128)),
    );
    data.fields.insert(
        "array".into(),
        lab_core::managed::PlainValue::Numbers(vec![1.0; 64]),
    );
    assert!(data.validate().is_ok());
    data.fields.insert(
        "extra".into(),
        lab_core::managed::PlainValue::Numbers(vec![2.0; 65]),
    );
    assert!(data.validate().is_err());
    data.fields.remove("extra");
    data.fields.insert(
        "payload".into(),
        lab_core::managed::PlainValue::Text("x".repeat(129)),
    );
    assert!(data.validate().is_err());
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    let exact = definition(201, 1.0);
    runtime
        .command(Command::StageComponent {
            definition: exact,
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    assert_eq!(mailbox.lock().unwrap().submitted.len(), 1);
}

#[test]
fn valid_old_inflight_completion_cannot_restore_good_after_generation_commit() {
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
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::from_secs(1),
        })
        .unwrap();
    let old_pending = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    runtime
        .command(Command::StageComponent {
            definition: definition(201, 2.0),
            replaces: Some(ComponentId::new(201)),
            at: Duration::from_secs(1),
        })
        .unwrap();
    let replacement = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    complete(
        &mailbox,
        replacement,
        ComponentStatus::Init,
        None,
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    let signal = SignalId::new(InstrumentId::new(201), TEMPERATURE);
    let QueryResult::Latest(Some(after_reload)) =
        runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(after_reload.quality(), SampleQuality::Unavailable);
    complete(
        &mailbox,
        old_pending,
        ComponentStatus::Ready,
        Some(999.0),
        PlainData::default(),
    );
    runtime
        .command(Command::PollComponents {
            at: Duration::from_secs(1),
        })
        .unwrap();
    let QueryResult::Component(new) = runtime
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(new.generation, 2);
    assert_eq!(new.state, ComponentState::Warming);
    assert_eq!(new.good_steps, 0);
    let QueryResult::Latest(Some(last)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(last, after_reload);
}

#[test]
fn eight_committed_components_and_one_stage_remain_fixed_capacity() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    for id in 201..209 {
        runtime
            .command(Command::StageComponent {
                definition: definition(id, 1.0),
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        let pending = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        assert!(matches!(
            runtime.command(Command::StageComponent {
                definition: definition(300, 1.0),
                replaces: None,
                at: Duration::ZERO
            }),
            Err(lab_core::Error::Component(
                lab_core::managed::ComponentError::Busy
            ))
        ));
        complete(
            &mailbox,
            pending,
            ComponentStatus::Init,
            None,
            PlainData::default(),
        );
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
    }
    assert!(
        runtime
            .command(Command::StageComponent {
                definition: definition(209, 1.0),
                replaces: None,
                at: Duration::ZERO
            })
            .is_err()
    );
    assert!(mailbox.lock().unwrap().submitted.is_empty());
    let QueryResult::Instruments(all) = runtime.query(Query::Discover).unwrap() else {
        panic!()
    };
    assert_eq!(
        all.iter()
            .filter(|item| (201..209).contains(&item.id.get()))
            .count(),
        8
    );
}

#[test]
fn core_rejects_out_of_range_and_premature_ready_before_state_or_good_commit() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    for (id, value, warmup) in [(201, 501.0, 1), (202, 40.0, 3)] {
        let mut candidate = definition(id, 1.0);
        candidate.manifest.warmup_samples = warmup;
        runtime
            .command(Command::StageComponent {
                definition: candidate,
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
                component: ComponentId::new(id),
                at: Duration::ZERO,
            })
            .unwrap();
        let step = mailbox.lock().unwrap().submitted.pop_front().unwrap();
        let mut candidate_state = PlainData::default();
        candidate_state.fields.insert(
            "attempt".into(),
            lab_core::managed::PlainValue::Number(99.0),
        );
        complete(
            &mailbox,
            step,
            ComponentStatus::Ready,
            Some(value),
            candidate_state,
        );
        runtime
            .command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        let QueryResult::Component(snapshot) = runtime
            .query(Query::Component(ComponentId::new(id)))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot.state, ComponentState::Failed);
        assert_eq!(snapshot.committed_state, PlainData::default());
        let QueryResult::Latest(Some(latest)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(id),
                TEMPERATURE,
            )))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(latest.quality(), SampleQuality::Unavailable);
    }
}
