//! Fixed worker ledger and deadline quarantine; Runtime never waits for a Lua VM.

use crate::runner::run_bounded;
use lab_core::managed::{
    ComponentCompletion, ComponentError, ComponentExecutor, Correlation, Invocation,
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

/// Process-wide count makes dropping/recreating a facade unable to bypass quarantine.
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
const WORKERS: usize = 2;
const JOB_DEADLINE: Duration = Duration::from_millis(100);
const WORKER_STACK: usize = 2 * 1024 * 1024;

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

/// The two fixed worker slots own all Lua VM allocation, execution and cleanup.
/// No callback or worker handle is shared with Runtime's output authority.
pub struct LuaSupervisor {
    slots: Vec<Slot>,
    cursor: usize,
    closed: bool,
}

impl LuaSupervisor {
    /// Launch exactly two slots, or reject if a previous abandoned pool is still alive.
    pub fn new() -> Result<Self, ComponentError> {
        // Reserve the whole pool atomically: concurrent constructors cannot
        // each admit one worker while believing they own the fixed capacity.
        ACTIVE_WORKERS
            .compare_exchange(0, WORKERS, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ComponentError::Busy)?;
        let mut slots = Vec::with_capacity(WORKERS);
        for id in 0..WORKERS {
            let (jobs, job_receiver) = mpsc::sync_channel::<Work>(1);
            let (results, completions) = mpsc::sync_channel::<ComponentCompletion>(1);
            let worker = thread::Builder::new()
                .name(format!("lab-lua-{id}"))
                .stack_size(WORKER_STACK)
                .spawn(move || {
                    let _guard = WorkerGuard;
                    while let Ok(work) = job_receiver.recv() {
                        let outcome = run_bounded(&work.invocation, work.deadline, work.cancelled);
                        // This timestamp follows conversion and VM destruction.
                        let timely = Instant::now() < work.deadline;
                        let completion = ComponentCompletion {
                            correlation: work.invocation.correlation,
                            timely,
                            outcome,
                        };
                        // A cancelled receiver must never block this worker's cleanup.
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
                    // Already started threads own their WorkerGuard; return
                    // the reservation for this and any unstarted workers.
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

    /// Return fixed worker capacity without creating any new worker.
    pub fn active_workers() -> usize {
        ACTIVE_WORKERS.load(Ordering::Acquire)
    }

    /// Fence admission and request cancellation; report unfinished slots after
    /// at most 200 ms. Join only completed threads off the safety lane.
    pub fn request_shutdown(&mut self) -> usize {
        self.closed = true;
        for slot in &mut self.slots {
            if let Some(busy) = &slot.busy {
                busy.cancelled.store(true, Ordering::Release);
            }
            slot.jobs.take();
        }
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            if self
                .slots
                .iter()
                .all(|slot| slot.thread.as_ref().is_none_or(JoinHandle::is_finished))
            {
                break;
            }
            thread::yield_now();
        }
        let unfinished = self
            .slots
            .iter()
            .filter(|slot| {
                slot.thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
            })
            .count();
        for slot in &mut self.slots {
            if slot.thread.as_ref().is_some_and(JoinHandle::is_finished)
                && let Some(thread) = slot.thread.take()
            {
                let _ = thread.join();
            }
        }
        unfinished
    }
}

impl ComponentExecutor for LuaSupervisor {
    fn try_submit(&mut self, invocation: Invocation) -> Result<(), ComponentError> {
        if self.closed {
            return Err(ComponentError::Executor);
        }
        for offset in 0..WORKERS {
            let index = (self.cursor + offset) % WORKERS;
            let slot = &mut self.slots[index];
            if slot.busy.is_some() {
                continue;
            }
            let sender = slot.jobs.as_ref().ok_or(ComponentError::Executor)?;
            let deadline = Instant::now()
                .checked_add(JOB_DEADLINE)
                .ok_or(ComponentError::Deadline)?;
            let cancelled = Arc::new(AtomicBool::new(false));
            let correlation = invocation.correlation;
            sender
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
                // Still quarantined until this same worker reports VM cleanup.
                return true;
            }
        }
        false
    }
}

impl Drop for LuaSupervisor {
    fn drop(&mut self) {
        self.closed = true;
        for slot in &mut self.slots {
            if let Some(busy) = &slot.busy {
                busy.cancelled.store(true, Ordering::Release);
            }
            // Dropping a sender lets an idle worker exit; dropping a JoinHandle
            // never waits on a VM stuck in native work or an uncaught script.
            slot.jobs.take();
        }
    }
}
