//! M5 Core contract: a nonblocking language-neutral component port and typed signals.

use lab_core::{
    Command, InstrumentId, Query, QueryResult, Runtime, SignalId, TEMPERATURE, Unit,
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, ComponentResult, ComponentStatus, Invocation, InvocationPhase,
        PlainData,
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
