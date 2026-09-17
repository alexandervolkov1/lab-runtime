//! M9A contract: managed definitions select implementations without assuming Lua source.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, SampleQuality, SignalId, TEMPERATURE, Unit,
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentError, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, ComponentResult, ComponentState,
        ComponentStatus, Correlation, Invocation, PlainData,
    },
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Mailbox {
    submitted: VecDeque<Invocation>,
    completed: VecDeque<ComponentCompletion>,
}

struct FakeExecutor(Arc<Mutex<Mailbox>>);

impl ComponentExecutor for FakeExecutor {
    fn try_submit(&mut self, job: Invocation) -> Result<(), ComponentError> {
        self.0.lock().unwrap().submitted.push_back(job);
        Ok(())
    }

    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        self.0.lock().unwrap().completed.pop_front()
    }

    fn try_cancel(&mut self, _: Correlation) {}
}

fn manifest(id: u64) -> ComponentManifest {
    ComponentManifest {
        schema_version: 1,
        id: ComponentId::new(id),
        instrument: InstrumentId::new(id),
        name: format!("neutral component {id}"),
        parameter: TEMPERATURE,
        kind: ComponentKind::Source,
        unit: Unit::CELSIUS,
        min: -100.0,
        max: 500.0,
        warmup_samples: 1,
        max_input_age: Duration::from_secs(1),
        history_capacity: 8,
    }
}

fn commit_init(runtime: &mut Runtime, mailbox: &Arc<Mutex<Mailbox>>) {
    let job = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: job.correlation,
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
}

#[test]
fn native_definition_requires_no_executable_source_text() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest: manifest(901),
                implementation: ComponentImplementation::built_in("native.moving_mean.v1").unwrap(),
                config: PlainData::default(),
            },
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let admitted = mailbox.lock().unwrap().submitted.front().unwrap().clone();
    assert!(admitted.definition.implementation.artifact().is_built_in());
    commit_init(&mut runtime, &mailbox);
    let QueryResult::Component(snapshot) = runtime
        .query(Query::Component(ComponentId::new(901)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.implementation.as_str(), "native.moving_mean.v1");
}

#[test]
fn source_adapter_uses_the_same_neutral_definition_and_invocation_contract() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest: manifest(902),
                implementation: ComponentImplementation::text(
                    "lua.v1",
                    "return function(ctx) return ctx end",
                )
                .unwrap(),
                config: PlainData::default(),
            },
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let admitted = mailbox.lock().unwrap().submitted.front().unwrap().clone();
    assert_eq!(admitted.definition.implementation.id().as_str(), "lua.v1");
    assert!(
        admitted
            .definition
            .implementation
            .artifact()
            .text()
            .is_some()
    );
    commit_init(&mut runtime, &mailbox);
}

#[test]
fn built_in_identity_cannot_bypass_shared_finite_range_validation() {
    let mailbox = Arc::new(Mutex::new(Mailbox::default()));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(FakeExecutor(mailbox.clone())))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest: manifest(903),
                implementation: ComponentImplementation::built_in("native.test.v1").unwrap(),
                config: PlainData::default(),
            },
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    commit_init(&mut runtime, &mailbox);
    runtime
        .command(Command::InvokeComponent {
            component: ComponentId::new(903),
            at: Duration::ZERO,
        })
        .unwrap();
    let step = mailbox.lock().unwrap().submitted.pop_front().unwrap();
    mailbox
        .lock()
        .unwrap()
        .completed
        .push_back(ComponentCompletion {
            correlation: step.correlation,
            timely: true,
            outcome: Ok(ComponentResult {
                status: ComponentStatus::Ready,
                value: Some(f64::INFINITY),
                unit: Unit::CELSIUS,
                state: PlainData::default(),
                diagnostics: vec![],
            }),
        });
    runtime
        .command(Command::PollComponents { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Latest(Some(sample)) = runtime
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(903),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.quality(), SampleQuality::Unavailable);
    let QueryResult::Component(snapshot) = runtime
        .query(Query::Component(ComponentId::new(903)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.state, ComponentState::Failed);
    assert_eq!(snapshot.committed_state, PlainData::default());
}
