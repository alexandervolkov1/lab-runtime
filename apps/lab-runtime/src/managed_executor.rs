//! Trusted implementation registry behind one bounded managed-component worker pool.
//!
//! Runtime submits every implementation through the same
//! [`ComponentExecutor`](lab_core::managed::ComponentExecutor)
//! port. Two fixed workers own execution for registered native Rust
//! implementations, so adding a native implementation does not add a second
//! queue, correlation path, or source of committed state.
//!
//! The active reference path is `native.moving_mean.v1`: its implementation ID,
//! property metadata and runner are registered here; configuration/host composition
//! builds a Core definition; Runtime creates an `Invocation`; `ManagedExecutor`
//! returns a correlated `ComponentResult`; and Runtime alone validates and commits
//! the result. This module contains no Lua/script loader and grants no output or
//! physical-evidence authority. Registration ceremony is intentionally unchanged in
//! M10.2.

use lab_core::managed::{
    ComponentCompletion, ComponentError, ComponentExecutor, ComponentKind, ComponentResult,
    ComponentStatus, Correlation, Invocation, InvocationPhase, PlainData, PlainValue,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Stable semantic identity of the reference native transform.
pub const MOVING_MEAN_IMPLEMENTATION: &str = "native.moving_mean.v1";

/// Registration-owned configuration metadata projected generically by the API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentPropertyMetadata {
    /// Stable property identity within the implementation configuration.
    pub id: &'static str,
    /// Language-neutral scalar type.
    pub value_type: &'static str,
    /// Inclusive numeric minimum, when applicable.
    pub minimum: Option<i64>,
    /// Inclusive numeric maximum, when applicable.
    pub maximum: Option<i64>,
    /// Runtime lifecycle class selected by trusted registration.
    pub mutation_class: &'static str,
}

const MOVING_MEAN_PROPERTIES: &[ComponentPropertyMetadata] = &[ComponentPropertyMetadata {
    id: "window",
    value_type: "integer",
    minimum: Some(2),
    maximum: Some(64),
    mutation_class: "reinitialize",
}];

/// Property metadata for one registered implementation. The wire layer has no
/// implementation-specific branch, so new registrations need no new operation.
pub fn component_property_metadata(implementation: &str) -> &'static [ComponentPropertyMetadata] {
    match implementation {
        MOVING_MEAN_IMPLEMENTATION => MOVING_MEAN_PROPERTIES,
        _ => &[],
    }
}

static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
const WORKERS: usize = 2;
const JOB_DEADLINE: Duration = Duration::from_millis(100);
const WORKER_STACK: usize = 2 * 1024 * 1024;
const WINDOW_FIELD: &str = "window";
const VALUES_FIELD: &str = "values";

/// Trusted fixture gate proving both fixed worker slots entered managed Step work.
/// It is unavailable to component code and exists only for deterministic tests.
pub struct WorkerBarrier {
    enabled: AtomicBool,
    entered: AtomicUsize,
    released: AtomicBool,
}

impl WorkerBarrier {
    /// Create a disabled gate so initialization may finish before Step blocking.
    pub const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            entered: AtomicUsize::new(0),
            released: AtomicBool::new(false),
        }
    }

    /// Gate future Step work on the common worker boundary.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Number of fixed workers that reached the gate.
    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::Acquire)
    }

    /// Release gated workers during test cleanup.
    pub fn release(&self) {
        self.released.store(true, Ordering::Release);
    }
}

impl Default for WorkerBarrier {
    fn default() -> Self {
        Self::new()
    }
}

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

struct Slot {
    jobs: Option<SyncSender<Work>>,
    completions: Receiver<ComponentCompletion>,
    busy: Option<Busy>,
    thread: Option<JoinHandle<()>>,
}

/// One compile-time implementation registry and exactly two bounded worker slots.
///
/// Each slot has one job and one completion mailbox. There is no backlog behind a
/// busy slot, no replacement thread for a quarantined job, and no worker-owned
/// Runtime or output-authority handle. Shutdown closes admission and never joins a
/// worker that has not already finished.
pub struct ManagedExecutor {
    slots: Vec<Slot>,
    cursor: usize,
    closed: bool,
}

impl ManagedExecutor {
    /// Launch the process-wide two-slot pool for all registered implementations.
    pub fn new() -> Result<Self, ComponentError> {
        Self::new_with_runner_and_barrier(run_registered, None)
    }

    /// Construct the production registry with a deterministic test-only Step gate.
    pub fn new_with_barrier(barrier: Arc<WorkerBarrier>) -> Result<Self, ComponentError> {
        Self::new_with_runner_and_barrier(run_registered, Some(barrier))
    }

    fn new_with_runner_and_barrier(
        runner: WorkerRunner,
        barrier: Option<Arc<WorkerBarrier>>,
    ) -> Result<Self, ComponentError> {
        ACTIVE_WORKERS
            .compare_exchange(0, WORKERS, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ComponentError::Busy)?;
        let mut slots = Vec::with_capacity(WORKERS);
        for id in 0..WORKERS {
            let (jobs, job_receiver) = mpsc::sync_channel::<Work>(1);
            let (results, completions) = mpsc::sync_channel::<ComponentCompletion>(1);
            let worker_barrier = barrier.clone();
            let worker = thread::Builder::new()
                .name(format!("lab-managed-{id}"))
                .stack_size(WORKER_STACK)
                .spawn(move || {
                    let _guard = WorkerGuard;
                    while let Ok(work) = job_receiver.recv() {
                        if let Some(gate) = &worker_barrier
                            && gate.enabled.load(Ordering::Acquire)
                            && work.invocation.phase == InvocationPhase::Step
                        {
                            gate.entered.fetch_add(1, Ordering::AcqRel);
                            while !gate.released.load(Ordering::Acquire) {
                                thread::yield_now();
                            }
                        }
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
        validate_registered(&invocation)?;
        if self.closed {
            return Err(ComponentError::Executor);
        }
        for offset in 0..WORKERS {
            let index = (self.cursor + offset) % WORKERS;
            let slot = &mut self.slots[index];
            if slot.busy.is_some() {
                continue;
            }
            let deadline = Instant::now()
                .checked_add(JOB_DEADLINE)
                .ok_or(ComponentError::Deadline)?;
            let cancelled = Arc::new(AtomicBool::new(false));
            let correlation = invocation.correlation;
            slot.jobs
                .as_ref()
                .ok_or(ComponentError::Executor)?
                .try_send(Work {
                    invocation,
                    deadline,
                    cancelled: cancelled.clone(),
                })
                .map_err(|_| ComponentError::Busy)?;
            slot.busy = Some(Busy {
                correlation,
                deadline,
                cancelled,
                expired: false,
            });
            self.cursor = (index + 1) % WORKERS;
            return Ok(());
        }
        Err(ComponentError::Busy)
    }

    fn try_poll(&mut self) -> Option<ComponentCompletion> {
        for offset in 0..WORKERS {
            let index = (self.cursor + offset) % WORKERS;
            let slot = &mut self.slots[index];
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
                Err(TryRecvError::Disconnected | TryRecvError::Empty) => {}
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

fn validate_registered(job: &Invocation) -> Result<(), ComponentError> {
    match job.definition.implementation.id().as_str() {
        MOVING_MEAN_IMPLEMENTATION => {
            if !matches!(
                job.definition.manifest.kind,
                ComponentKind::Transform { .. }
            ) || configured_window(job)? != job.definition.manifest.warmup_samples
            {
                return Err(ComponentError::InvalidConfiguration);
            }
        }
        _ => return Err(ComponentError::InvalidConfiguration),
    }
    Ok(())
}

fn run_registered(
    job: &Invocation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<ComponentResult, ComponentError> {
    validate_registered(job)?;
    match job.definition.implementation.id().as_str() {
        MOVING_MEAN_IMPLEMENTATION => run_moving_mean(job, deadline, &cancelled),
        _ => Err(ComponentError::InvalidConfiguration),
    }
}

fn configured_window(job: &Invocation) -> Result<usize, ComponentError> {
    if job.definition.config.fields.len() != 1 {
        return Err(ComponentError::InvalidConfiguration);
    }
    let Some(PlainValue::Number(window)) = job.definition.config.fields.get(WINDOW_FIELD) else {
        return Err(ComponentError::InvalidConfiguration);
    };
    if !window.is_finite() || window.fract() != 0.0 || !(2.0..=64.0).contains(window) {
        return Err(ComponentError::InvalidConfiguration);
    }
    Ok(*window as usize)
}

fn check_deadline(deadline: Instant, cancelled: &AtomicBool) -> Result<(), ComponentError> {
    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
        Err(ComponentError::Deadline)
    } else {
        Ok(())
    }
}

fn run_moving_mean(
    job: &Invocation,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<ComponentResult, ComponentError> {
    check_deadline(deadline, cancelled)?;
    let window = configured_window(job)?;
    if job.phase == InvocationPhase::Init {
        if job.input.is_some() {
            return Err(ComponentError::InvalidConfiguration);
        }
        let mut state = PlainData::default();
        state
            .fields
            .insert(VALUES_FIELD.into(), PlainValue::Numbers(Vec::new()));
        return Ok(ComponentResult {
            status: ComponentStatus::Init,
            value: None,
            unit: job.definition.manifest.unit,
            state,
            diagnostics: Vec::new(),
        });
    }
    let input = job.input.ok_or(ComponentError::InputUnavailable)?;
    if input.unit != job.definition.manifest.unit || !input.value.is_finite() {
        return Err(ComponentError::InputUnavailable);
    }
    if job.state.fields.len() != 1 {
        return Err(ComponentError::InvalidResult);
    }
    let Some(PlainValue::Numbers(previous)) = job.state.fields.get(VALUES_FIELD) else {
        return Err(ComponentError::InvalidResult);
    };
    if previous.len() > window || previous.iter().any(|value| !value.is_finite()) {
        return Err(ComponentError::InvalidResult);
    }
    let retained = previous.len().min(window.saturating_sub(1));
    let mut values = Vec::with_capacity(window);
    values.extend_from_slice(&previous[previous.len() - retained..]);
    values.push(input.value);
    check_deadline(deadline, cancelled)?;
    let mut state = PlainData::default();
    state
        .fields
        .insert(VALUES_FIELD.into(), PlainValue::Numbers(values.clone()));
    let ready = values.len() == window;
    let value = if ready {
        let mut mean = 0.0;
        for (index, sample) in values.iter().enumerate() {
            mean += (*sample - mean) / (index + 1) as f64;
            if !mean.is_finite() {
                return Err(ComponentError::InvalidResult);
            }
        }
        Some(mean)
    } else {
        None
    };
    Ok(ComponentResult {
        status: if ready {
            ComponentStatus::Ready
        } else {
            ComponentStatus::Warming
        },
        value,
        unit: job.definition.manifest.unit,
        state,
        diagnostics: Vec::new(),
    })
}
