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
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
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
