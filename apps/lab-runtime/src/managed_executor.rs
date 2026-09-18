//! Trusted implementation registry behind one bounded managed-component worker pool.
//!
//! Runtime submits every implementation through the same
//! [`ComponentExecutor`](lab_core::managed::ComponentExecutor)
//! port. Two fixed workers own execution for registered native Rust
//! implementations, so adding a native implementation does not add a second
//! queue, correlation path, or source of committed state.
//!
//! The active reference path is `native.moving_mean.v1`. Its bounded algorithm and
//! metadata live in `managed_executor/moving_mean.rs`, while
//! `managed_executor/registry.rs` is the one compile-time
//! selection and composition point. Runtime creates an `Invocation`;
//! `ManagedExecutor` returns a correlated `ComponentResult`; and Runtime alone
//! validates and commits the result. This module contains no Lua/script loader and
//! grants no output or physical-evidence authority.
//!
//! # Adding a trusted native component
//!
//! 1. Implement bounded logic in a sibling module. It may consume only the finite
//!    `Invocation` and must return bounded `PlainData`; it has no Runtime, transport
//!    or OutputAuthority access.
//! 2. Assign a stable semantic implementation ID and declare configuration property
//!    metadata, units, output bounds, warm-up and input shape beside the logic.
//! 3. Add one explicit entry to `managed_executor/registry.rs`. Generic discovery, properties,
//!    current/history/subscriptions and Recorder provenance then follow from the
//!    ordinary component definition and signal paths.
//! 4. Test finite inputs/results, units, warm-up, state bounds and generic API
//!    projection. Registration is static trusted Rust composition, not a plugin API.

mod moving_mean;
mod registry;

#[cfg(test)]
mod reference_component;

pub use registry::MOVING_MEAN_IMPLEMENTATION;
pub(crate) use registry::{
    ComponentPropertyMetadata, NativeComponentDefinition, build_component_definition,
    component_property_metadata, validate_component_configuration,
};

use lab_core::managed::{
    ComponentCompletion, ComponentError, ComponentExecutor, ComponentResult, Correlation,
    Invocation,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static EXECUTOR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const WORKERS: usize = 2;
const JOB_DEADLINE: Duration = Duration::from_millis(100);
const WORKER_STACK: usize = 2 * 1024 * 1024;

type WorkerRunner =
    fn(&Invocation, Instant, Arc<AtomicBool>) -> Result<ComponentResult, ComponentError>;

struct WorkerGuard;

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Work {
    invocation: Invocation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

struct Busy {
    correlation: Correlation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    expired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotLifecycle {
    Alive,
    Dead,
}

struct Slot {
    lifecycle: SlotLifecycle,
    jobs: Option<SyncSender<Work>>,
    completions: Receiver<ComponentCompletion>,
    busy: Option<Busy>,
    thread: Option<JoinHandle<()>>,
}

/// One compile-time implementation registry and exactly two bounded worker slots.
///
/// Each slot has one job and one completion mailbox. There is no backlog behind a
/// busy slot, no replacement thread for a quarantined job, and no worker-owned
/// Runtime or output-authority handle. A worker-channel disconnect permanently
/// marks that fixed slot dead and terminalizes its one pending correlation with
/// [`ComponentError::Executor`]; the slot is neither reported as pending capacity
/// nor respawned. Shutdown closes admission and never joins a worker that has not
/// already finished.
pub struct ManagedExecutor {
    slots: Vec<Slot>,
    cursor: usize,
    closed: bool,
}

impl ManagedExecutor {
    /// Launch the process-wide two-slot pool for all registered implementations.
    pub fn new() -> Result<Self, ComponentError> {
        Self::new_with_runner(registry::run_registered)
    }

    fn new_with_runner(runner: WorkerRunner) -> Result<Self, ComponentError> {
        ACTIVE_WORKERS
            .compare_exchange(0, WORKERS, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ComponentError::Busy)?;
        let mut slots = Vec::with_capacity(WORKERS);
        for id in 0..WORKERS {
            let (jobs, job_receiver) = mpsc::sync_channel::<Work>(1);
            let (results, completions) = mpsc::sync_channel::<ComponentCompletion>(1);
            let worker = thread::Builder::new()
                .name(format!("lab-managed-{id}"))
                .stack_size(WORKER_STACK)
                .spawn(move || {
                    let _guard = WorkerGuard;
                    while let Ok(work) = job_receiver.recv() {
                        let outcome = runner(&work.invocation, work.deadline, work.cancelled);
                        let timely = Instant::now() < work.deadline;
                        let completion = ComponentCompletion {
                            correlation: work.invocation.correlation,
                            timely,
                            outcome,
                        };
                        let _ = results.try_send(completion);
                    }
                });
            match worker {
                Ok(worker) => slots.push(Slot {
                    lifecycle: SlotLifecycle::Alive,
                    jobs: Some(jobs),
                    completions,
                    busy: None,
                    thread: Some(worker),
                }),
                Err(_) => {
                    ACTIVE_WORKERS.fetch_sub(WORKERS - slots.len(), Ordering::AcqRel);
                    return Err(ComponentError::Executor);
                }
            }
        }
        Ok(Self {
            slots,
            cursor: 0,
            closed: false,
        })
    }

    /// Current process-wide worker count; it never exceeds two.
    pub fn active_workers() -> usize {
        ACTIVE_WORKERS.load(Ordering::Acquire)
    }
}

impl ComponentExecutor for ManagedExecutor {
    fn try_submit(&mut self, invocation: Invocation) -> Result<(), ComponentError> {
        registry::validate_invocation(&invocation)?;
        if self.closed {
            return Err(ComponentError::Executor);
        }
        let mut invocation = Some(invocation);
        let mut alive = 0usize;
        for offset in 0..WORKERS {
            let index = (self.cursor + offset) % WORKERS;
            let slot = &mut self.slots[index];
            if slot.lifecycle == SlotLifecycle::Dead {
                continue;
            }
            alive += 1;
            if slot.busy.is_some() {
                continue;
            }
            let deadline = Instant::now()
                .checked_add(JOB_DEADLINE)
                .ok_or(ComponentError::Deadline)?;
            let cancelled = Arc::new(AtomicBool::new(false));
            let job = Work {
                invocation: invocation.take().expect("one unsubmitted invocation"),
                deadline,
                cancelled: cancelled.clone(),
            };
            let correlation = job.invocation.correlation;
            match slot
                .jobs
                .as_ref()
                .ok_or(ComponentError::Executor)?
                .try_send(job)
            {
                Ok(()) => {}
                Err(TrySendError::Full(job)) => {
                    invocation = Some(job.invocation);
                    continue;
                }
                Err(TrySendError::Disconnected(job)) => {
                    invocation = Some(job.invocation);
                    slot.lifecycle = SlotLifecycle::Dead;
                    slot.jobs.take();
                    alive -= 1;
                    continue;
                }
            }
            slot.busy = Some(Busy {
                correlation,
                deadline,
                cancelled,
                expired: false,
            });
            self.cursor = (index + 1) % WORKERS;
            return Ok(());
        }
        Err(if alive == 0 {
            ComponentError::Executor
        } else {
            ComponentError::Busy
        })
    }

    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        for offset in 0..WORKERS {
            let index = (self.cursor + offset) % WORKERS;
            let slot = &mut self.slots[index];
            if slot.lifecycle == SlotLifecycle::Dead {
                continue;
            }
            match slot.completions.try_recv() {
                Ok(mut completion) => {
                    if let Some(busy) = slot.busy.take() {
                        completion.timely &= !busy.expired
                            && Instant::now() < busy.deadline
                            && busy.correlation == completion.correlation;
                    } else {
                        completion.timely = false;
                    }
                    self.cursor = (index + 1) % WORKERS;
                    return Some(completion);
                }
                Err(TryRecvError::Disconnected) => {
                    slot.lifecycle = SlotLifecycle::Dead;
                    slot.jobs.take();
                    if let Some(busy) = slot.busy.take() {
                        busy.cancelled.store(true, Ordering::Release);
                        self.cursor = (index + 1) % WORKERS;
                        return Some(ComponentCompletion {
                            correlation: busy.correlation,
                            timely: false,
                            outcome: Err(ComponentError::Executor),
                        });
                    }
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        None
    }

    fn try_cancel(&mut self, correlation: Correlation) {
        for slot in &mut self.slots {
            if let Some(busy) = &slot.busy
                && busy.correlation == correlation
            {
                busy.cancelled.store(true, Ordering::Release);
            }
        }
    }

    fn try_expire(&mut self, correlation: Correlation) -> bool {
        for slot in &mut self.slots {
            if let Some(busy) = &mut slot.busy
                && busy.correlation == correlation
                && Instant::now() >= busy.deadline
            {
                busy.expired = true;
                busy.cancelled.store(true, Ordering::Release);
                return true;
            }
        }
        false
    }

    fn begin_shutdown(&mut self) {
        self.closed = true;
        for slot in &mut self.slots {
            if let Some(busy) = &slot.busy {
                busy.cancelled.store(true, Ordering::Release);
            }
            slot.jobs.take();
        }
    }

    fn unfinished_workers(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| {
                slot.thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
            })
            .count()
    }
}

impl Drop for ManagedExecutor {
    fn drop(&mut self) {
        self.begin_shutdown();
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use lab_core::{
        Command, InstrumentId, Query, QueryResult, Runtime, SignalId, TEMPERATURE, Unit,
        VirtualInstrumentConfig,
        control::ControllerState,
        managed::{
            CapturedInput, ComponentDefinition, ComponentId, ComponentImplementation,
            ComponentKind, ComponentManifest, ComponentState, Correlation, InvocationPhase,
            PlainData, PlainValue,
        },
    };
    use std::{cell::Cell, sync::MutexGuard};

    use crate::host::{Clock, HostCore};

    const INPUT: InstrumentId = InstrumentId::new(9_101);
    const FILTER: ComponentId = ComponentId::new(9_102);
    const OTHER_FILTER: ComponentId = ComponentId::new(9_103);

    struct TestClock(Cell<Duration>);

    impl TestClock {
        fn set(&self, at: Duration) {
            self.0.set(at);
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
    }

    fn test_guard() -> MutexGuard<'static, ()> {
        let guard = EXECUTOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(ManagedExecutor::active_workers(), 0);
        guard
    }

    fn definition_for(id: ComponentId) -> ComponentDefinition {
        ComponentDefinition {
            manifest: ComponentManifest {
                schema_version: 1,
                id,
                instrument: InstrumentId::new(id.get()),
                name: "Panic containment fixture".into(),
                parameter: TEMPERATURE,
                kind: ComponentKind::Transform {
                    input: SignalId::new(INPUT, TEMPERATURE),
                },
                unit: Unit::CELSIUS,
                min: -100.0,
                max: 500.0,
                warmup_samples: 2,
                max_input_age: Duration::from_secs(2),
                history_capacity: 8,
            },
            implementation: ComponentImplementation::built_in(MOVING_MEAN_IMPLEMENTATION).unwrap(),
            config: PlainData {
                fields: [("window".into(), PlainValue::Number(2.0))]
                    .into_iter()
                    .collect(),
            },
        }
    }

    fn definition() -> ComponentDefinition {
        definition_for(FILTER)
    }

    fn panic_on_step(
        invocation: &Invocation,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ComponentResult, ComponentError> {
        if invocation.phase == InvocationPhase::Step {
            panic!("intentional managed-worker panic")
        }
        moving_mean::run(invocation, deadline, &cancelled)
    }

    fn panic_one_on_step(
        invocation: &Invocation,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ComponentResult, ComponentError> {
        if invocation.phase == InvocationPhase::Step && invocation.correlation.component == FILTER {
            panic!("intentional scoped managed-worker panic")
        }
        moving_mean::run(invocation, deadline, &cancelled)
    }

    fn invocation(attempt: u64) -> Invocation {
        Invocation {
            correlation: Correlation {
                runtime: 1,
                component: FILTER,
                generation: 1,
                attempt,
                revision: 0,
            },
            phase: InvocationPhase::Step,
            definition: definition(),
            state: PlainData {
                fields: [("values".into(), PlainValue::Numbers(Vec::new()))]
                    .into_iter()
                    .collect(),
            },
            at: Duration::from_millis(attempt),
            dt: Duration::from_millis(1),
            input: Some(CapturedInput {
                signal: SignalId::new(INPUT, TEMPERATURE),
                value: 20.0,
                unit: Unit::CELSIUS,
                at: Duration::from_millis(attempt),
                freshness_at: Duration::from_millis(attempt),
                source_generation: 1,
                source_revision: 1,
                source_state_revision: None,
            }),
        }
    }

    fn poll_until(runtime: &mut Runtime, at: Duration, predicate: impl Fn(&Runtime) -> bool) {
        for _ in 0..100_000 {
            runtime.command(Command::PollComponents { at }).unwrap();
            if predicate(runtime) {
                return;
            }
            std::thread::yield_now();
        }
        panic!("bounded managed-worker lifecycle did not settle")
    }

    fn wait_for_workers(expected: usize) {
        for _ in 0..100_000 {
            if ManagedExecutor::active_workers() == expected {
                return;
            }
            std::thread::yield_now();
        }
        panic!("managed-worker count did not settle")
    }

    fn wait_for_unfinished_workers(executor: &ManagedExecutor, expected: usize) {
        for _ in 0..100_000 {
            if executor.unfinished_workers() == expected {
                return;
            }
            std::thread::yield_now();
        }
        panic!("managed-worker thread handle did not settle")
    }

    fn wait_for_completion(executor: &mut ManagedExecutor) -> ComponentCompletion {
        for _ in 0..100_000 {
            if let Some(completion) = executor.try_poll() {
                return completion;
            }
            std::thread::yield_now();
        }
        panic!("managed-worker death did not produce a terminal completion")
    }

    #[test]
    fn disconnected_completion_terminalizes_each_dead_slot_once() {
        let _guard = test_guard();
        let mut executor = ManagedExecutor::new_with_runner(panic_on_step).unwrap();

        executor.try_submit(invocation(1)).unwrap();
        wait_for_workers(1);
        let first = wait_for_completion(&mut executor);
        assert_eq!(first.correlation, invocation(1).correlation);
        assert!(!first.timely);
        assert_eq!(first.outcome, Err(ComponentError::Executor));
        assert!(executor.try_poll().is_none(), "death is reported once");

        executor.try_submit(invocation(2)).unwrap();
        wait_for_workers(0);
        let second = wait_for_completion(&mut executor);
        assert_eq!(second.correlation, invocation(2).correlation);
        assert_eq!(second.outcome, Err(ComponentError::Executor));
        assert_eq!(
            executor.try_submit(invocation(3)),
            Err(ComponentError::Executor),
            "dead capacity must not masquerade as a pending Busy slot"
        );
        // The worker decrements the test-only active counter immediately before its
        // thread function returns. Wait on the production shutdown predicate too;
        // equality of those two observations is not an atomic guarantee.
        wait_for_unfinished_workers(&executor, 0);
    }

    #[test]
    fn panicked_worker_terminalizes_pending_component_and_leaves_bounded_capacity() {
        let _guard = test_guard();
        let mut runtime = Runtime::new();
        runtime
            .command(Command::RegisterVirtual(VirtualInstrumentConfig {
                id: INPUT,
                name: "Panic fixture input".into(),
                history_capacity: 8,
                base_temperature: 20.0,
                measurement_enabled: true,
            }))
            .unwrap();
        runtime
            .install_component_executor(Box::new(
                ManagedExecutor::new_with_runner(panic_on_step).unwrap(),
            ))
            .unwrap();
        runtime
            .command(Command::StageComponent {
                definition: definition(),
                replaces: None,
                at: Duration::ZERO,
            })
            .unwrap();
        poll_until(&mut runtime, Duration::ZERO, |runtime| {
            matches!(
                runtime.query(Query::Component(FILTER)),
                Ok(QueryResult::Component(snapshot)) if snapshot.pending.is_none()
            )
        });
        runtime
            .command(Command::RefreshMeasurement {
                instrument: INPUT,
                parameter: TEMPERATURE,
                at: Duration::from_millis(1),
            })
            .unwrap();
        runtime
            .command(Command::InvokeComponent {
                component: FILTER,
                at: Duration::from_millis(1),
            })
            .unwrap();

        poll_until(&mut runtime, Duration::from_millis(1), |runtime| {
            matches!(
                runtime.query(Query::Component(FILTER)),
                Ok(QueryResult::Component(snapshot))
                    if snapshot.state == ComponentState::Failed && snapshot.pending.is_none()
            )
        });
        wait_for_workers(1);

        runtime
            .command(Command::QuiesceManaged {
                at: Duration::from_millis(2),
            })
            .unwrap();
        drop(runtime);
        wait_for_workers(0);
    }

    #[test]
    fn one_worker_panic_is_scoped_while_acquisition_other_component_and_facts_progress() {
        let _guard = test_guard();
        let mut runtime = Runtime::new();
        runtime.enable_recording_facts();
        runtime
            .command(Command::RegisterVirtual(VirtualInstrumentConfig {
                id: INPUT,
                name: "Scoped panic input".into(),
                history_capacity: 8,
                base_temperature: 20.0,
                measurement_enabled: true,
            }))
            .unwrap();
        runtime
            .install_component_executor(Box::new(
                ManagedExecutor::new_with_runner(panic_one_on_step).unwrap(),
            ))
            .unwrap();
        for component in [FILTER, OTHER_FILTER] {
            runtime
                .command(Command::StageComponent {
                    definition: definition_for(component),
                    replaces: None,
                    at: Duration::ZERO,
                })
                .unwrap();
            poll_until(&mut runtime, Duration::ZERO, |runtime| {
                matches!(
                    runtime.query(Query::Component(component)),
                    Ok(QueryResult::Component(snapshot)) if snapshot.pending.is_none()
                )
            });
        }
        runtime.take_recording_facts();

        runtime
            .command(Command::RefreshMeasurement {
                instrument: INPUT,
                parameter: TEMPERATURE,
                at: Duration::from_millis(1),
            })
            .unwrap();
        for component in [FILTER, OTHER_FILTER] {
            runtime
                .command(Command::InvokeComponent {
                    component,
                    at: Duration::from_millis(1),
                })
                .unwrap();
        }
        poll_until(&mut runtime, Duration::from_millis(1), |runtime| {
            [FILTER, OTHER_FILTER].iter().all(|component| {
                matches!(
                    runtime.query(Query::Component(*component)),
                    Ok(QueryResult::Component(snapshot)) if snapshot.pending.is_none()
                )
            })
        });
        let QueryResult::Component(failed) = runtime.query(Query::Component(FILTER)).unwrap()
        else {
            panic!()
        };
        let QueryResult::Component(unrelated) =
            runtime.query(Query::Component(OTHER_FILTER)).unwrap()
        else {
            panic!()
        };
        assert_eq!(failed.state, ComponentState::Failed);
        assert_eq!(unrelated.state, ComponentState::Warming);
        assert_eq!(ManagedExecutor::active_workers(), 1);

        runtime
            .command(Command::RefreshMeasurement {
                instrument: INPUT,
                parameter: TEMPERATURE,
                at: Duration::from_millis(2),
            })
            .unwrap();
        runtime
            .command(Command::InvokeComponent {
                component: OTHER_FILTER,
                at: Duration::from_millis(2),
            })
            .unwrap();
        poll_until(&mut runtime, Duration::from_millis(2), |runtime| {
            matches!(
                runtime.query(Query::Component(OTHER_FILTER)),
                Ok(QueryResult::Component(snapshot))
                    if snapshot.state == ComponentState::Ready && snapshot.pending.is_none()
            )
        });
        let QueryResult::Latest(Some(input)) = runtime
            .query(Query::GetLatestSignal(SignalId::new(INPUT, TEMPERATURE)))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(input.at(), Duration::from_millis(2));
        assert_eq!(input.quality(), lab_core::SampleQuality::Good);
        assert!(
            !runtime.take_recording_facts().is_empty(),
            "semantic Recorder facts must keep advancing after scoped worker death"
        );

        runtime
            .command(Command::QuiesceManaged {
                at: Duration::from_millis(3),
            })
            .unwrap();
        drop(runtime);
        wait_for_workers(0);
    }

    #[test]
    fn managed_worker_panic_cannot_starve_unrelated_native_controller_or_shutdown() {
        let _guard = test_guard();
        let mut host = HostCore::virtual_demo().unwrap();
        host.install_component_executor(Box::new(
            ManagedExecutor::new_with_runner(panic_on_step).unwrap(),
        ))
        .unwrap();
        host.stage_standard_components(Duration::ZERO).unwrap();
        let clock = TestClock(Cell::new(Duration::ZERO));
        for milliseconds in (0..=200).step_by(10) {
            clock.set(Duration::from_millis(milliseconds));
            host.service(&clock).unwrap();
            if host.standard_components_initialized() {
                break;
            }
        }
        assert!(host.standard_components_initialized());
        host.activate_standard_components(clock.now()).unwrap();
        host.command(Command::StartController {
            controller: host.controller_id(),
            at: clock.now(),
        })
        .unwrap();

        let start = clock.now().as_millis() as u64;
        let mut controller_ticks = 0;
        for step in 1..=20 {
            clock.set(Duration::from_millis(start + step * 100));
            controller_ticks += host.service(&clock).unwrap().controller_ticks;
        }
        let component_id = host.component_catalog()[0].0;
        let QueryResult::Component(component) = host.query(Query::Component(component_id)).unwrap()
        else {
            panic!()
        };
        assert_eq!(component.state, ComponentState::Failed);
        let QueryResult::Controller(controller) =
            host.query(Query::Controller(host.controller_id())).unwrap()
        else {
            panic!()
        };
        assert_eq!(controller.state, ControllerState::Running);
        assert!(
            controller_ticks >= 10,
            "native controller ticks={controller_ticks}"
        );
        assert!(controller.pid.latest.is_some());

        host.begin_shutdown(&clock).unwrap();
        for step in 1..=20 {
            clock.set(clock.now() + Duration::from_millis(10));
            host.service(&clock).unwrap();
            if host.shutdown_status().exit_success {
                break;
            }
            assert!(step < 20, "managed death prevented finite shutdown");
            std::thread::yield_now();
        }
        let shutdown = host.shutdown_status();
        assert!(shutdown.exit_success, "{shutdown:?}");
        assert_eq!(shutdown.unfinished_workers, 0);
        drop(host);
        wait_for_workers(0);
    }
}
