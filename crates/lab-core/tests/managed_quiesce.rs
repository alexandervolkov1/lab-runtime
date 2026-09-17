//! M6 shutdown fences managed admission without waiting for a stalled callback.

use lab_core::{
    Command, CommandResult, InstrumentId, Runtime, TEMPERATURE, Unit,
    managed::{
        ComponentCompletion, ComponentDefinition, ComponentError, ComponentExecutor, ComponentId,
        ComponentImplementation, ComponentKind, ComponentManifest, Correlation, Invocation,
        PlainData,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

struct StalledExecutor {
    closed: Arc<AtomicBool>,
    cancelled: Arc<AtomicUsize>,
}
impl ComponentExecutor for StalledExecutor {
    fn try_submit(&mut self, _: Invocation) -> Result<(), ComponentError> {
        Ok(())
    }
    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        None
    }
    fn try_cancel(&mut self, _: Correlation) {
        self.cancelled.fetch_add(1, Ordering::AcqRel);
    }
    fn begin_shutdown(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
    fn unfinished_workers(&self) -> usize {
        2
    }
}

fn source() -> ComponentDefinition {
    ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: ComponentId::new(44),
            instrument: InstrumentId::new(44),
            name: "Pending startup source".into(),
            parameter: TEMPERATURE,
            kind: ComponentKind::Source,
            unit: Unit::CELSIUS,
            min: 0.0,
            max: 100.0,
            warmup_samples: 1,
            max_input_age: Duration::from_secs(1),
            history_capacity: 8,
        },
        implementation: ComponentImplementation::text(
            "test.stalled.v1",
            "return function(ctx) return ctx end",
        )
        .unwrap(),
        config: PlainData::default(),
    }
}

#[test]
fn quiesce_cancels_pending_admission_without_wait_and_never_reopens_it() {
    let closed = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let mut runtime = Runtime::new();
    runtime
        .install_component_executor(Box::new(StalledExecutor {
            closed: closed.clone(),
            cancelled: cancelled.clone(),
        }))
        .unwrap();
    runtime
        .command(Command::StageComponent {
            definition: source(),
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    let result = runtime
        .command(Command::QuiesceManaged {
            at: Duration::from_millis(1),
        })
        .unwrap();
    assert_eq!(result, CommandResult::ManagedQuiesced);
    assert!(closed.load(Ordering::Acquire));
    assert!(cancelled.load(Ordering::Acquire) > 0);
    assert_eq!(runtime.unfinished_component_workers(), 2);
    assert!(
        runtime
            .command(Command::StageComponent {
                definition: source(),
                replaces: None,
                at: Duration::from_millis(2),
            })
            .is_err()
    );
    runtime
        .command(Command::PollComponents {
            at: Duration::from_millis(3),
        })
        .unwrap();
    assert_eq!(runtime.unfinished_component_workers(), 2);
}
