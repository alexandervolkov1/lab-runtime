//! Fixed worker ledger and deadline quarantine; Runtime never waits for a Lua VM.

use crate::runner::run_bounded;
use lab_core::managed::{
    ComponentCompletion, ComponentError, ComponentExecutor, ComponentResult, Correlation,
    Invocation, InvocationPhase,
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

/// Trusted fixture gate proving both fixed worker slots have entered a Step.
/// It is never given to a guest or exposed as a network command. Release it even
/// after an assertion failure so the global worker ledger can unwind.
pub struct WorkerBarrier {
    enabled: AtomicBool,
    entered: AtomicUsize,
    released: AtomicBool,
}
impl WorkerBarrier {
    /// Create a disabled gate; ordinary Init can finish before Step exhaustion.
    pub const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            entered: AtomicUsize::new(0),
            released: AtomicBool::new(false),
        }
    }
    /// Gate future Step execution on both real fixed worker threads.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }
    /// Count worker slots that reached the gate, before guest VM work.
    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::Acquire)
    }
    /// Let gated work finish or observe its already-expired cancellation.
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
        Self::new_with_runner(run_bounded)
    }
    /// Construct the same two-worker supervisor with a trusted test barrier.
    /// After release every admitted Step still executes the real bounded Lua VM.
    pub fn new_with_barrier(barrier: Arc<WorkerBarrier>) -> Result<Self, ComponentError> {
        Self::new_with_runner_and_barrier(run_bounded, Some(barrier))
    }

    // A private runner substitution lets the unit harness block the actual
    // supervisor slots. Runtime and scripts never receive this test seam.
    fn new_with_runner(runner: WorkerRunner) -> Result<Self, ComponentError> {
        Self::new_with_runner_and_barrier(runner, None)
    }
    fn new_with_runner_and_barrier(
        runner: WorkerRunner,
        barrier: Option<Arc<WorkerBarrier>>,
    ) -> Result<Self, ComponentError> {
        // Reserve the whole pool atomically: concurrent constructors cannot
        // each admit one worker while believing they own the fixed capacity.
        ACTIVE_WORKERS
            .compare_exchange(0, WORKERS, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ComponentError::Busy)?;
        let mut slots = Vec::with_capacity(WORKERS);
        for id in 0..WORKERS {
            let (jobs, job_receiver) = mpsc::sync_channel::<Work>(1);
            let (results, completions) = mpsc::sync_channel::<ComponentCompletion>(1);
            let worker_barrier = barrier.clone();
            let worker = thread::Builder::new()
                .name(format!("lab-lua-{id}"))
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
        self.begin_shutdown();
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
        let unfinished = self.unfinished_workers();
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

#[cfg(test)]
mod tests {
    use super::*;
    use lab_core::{
        InstrumentId, TEMPERATURE, Unit,
        managed::{
            ComponentDefinition, ComponentId, ComponentKind, ComponentManifest, Correlation,
            InvocationPhase, PlainData,
        },
    };

    static REACHED: AtomicUsize = AtomicUsize::new(0);
    static RELEASE: AtomicBool = AtomicBool::new(false);

    fn blocked_runner(
        _: &Invocation,
        _: Instant,
        _: Arc<AtomicBool>,
    ) -> Result<ComponentResult, ComponentError> {
        REACHED.fetch_add(1, Ordering::AcqRel);
        while !RELEASE.load(Ordering::Acquire) {
            thread::yield_now();
        }
        Err(ComponentError::Executor)
    }

    fn init(id: u64) -> Invocation {
        Invocation {
            correlation: Correlation {
                runtime: 1,
                component: ComponentId::new(id),
                generation: 1,
                attempt: 0,
                revision: 0,
            },
            phase: InvocationPhase::Init,
            definition: ComponentDefinition {
                manifest: ComponentManifest {
                    schema_version: 1,
                    id: ComponentId::new(id),
                    instrument: InstrumentId::new(id),
                    name: "test-only worker barrier".into(),
                    parameter: TEMPERATURE,
                    kind: ComponentKind::Source,
                    unit: Unit::CELSIUS,
                    min: 0.0,
                    max: 100.0,
                    warmup_samples: 1,
                    max_input_age: Duration::from_secs(1),
                    history_capacity: 8,
                },
                source: "return function(ctx) return ctx end".into(),
                config: PlainData::default(),
            },
            state: PlainData::default(),
            at: Duration::ZERO,
            dt: Duration::ZERO,
            input: None,
        }
    }

    #[test]
    fn actual_supervisor_shutdown_reports_two_unfinished_workers_without_losing_the_ledger() {
        REACHED.store(0, Ordering::Release);
        RELEASE.store(false, Ordering::Release);
        let mut pool = LuaSupervisor::new_with_runner(blocked_runner).unwrap();
        pool.try_submit(init(201)).unwrap();
        pool.try_submit(init(202)).unwrap();
        let reached_deadline = Instant::now() + Duration::from_secs(2);
        while REACHED.load(Ordering::Acquire) != 2 {
            assert!(
                Instant::now() < reached_deadline,
                "barrier workers never started"
            );
            thread::yield_now();
        }
        assert_eq!(pool.try_submit(init(203)), Err(ComponentError::Busy));
        thread::sleep(Duration::from_millis(105));
        assert!(pool.try_expire(init(201).correlation));
        assert!(pool.try_expire(init(202).correlation));
        assert!(pool.try_poll().is_none());
        let shutdown_at = Instant::now();
        assert_eq!(pool.request_shutdown(), 2);
        assert!(shutdown_at.elapsed() < Duration::from_millis(300));
        assert_eq!(LuaSupervisor::active_workers(), 2);
        assert!(matches!(LuaSupervisor::new(), Err(ComponentError::Busy)));
        RELEASE.store(true, Ordering::Release);
        let cleanup_deadline = Instant::now() + Duration::from_secs(2);
        while LuaSupervisor::active_workers() != 0 {
            assert!(
                Instant::now() < cleanup_deadline,
                "blocked workers failed to leave after release"
            );
            thread::yield_now();
        }
        assert_eq!(pool.request_shutdown(), 0); // Joins only finished test threads.
        drop(pool);
        let mut recovered = LuaSupervisor::new().unwrap();
        assert_eq!(recovered.request_shutdown(), 0);
    }
}
