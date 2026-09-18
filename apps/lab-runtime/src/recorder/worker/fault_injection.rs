//! Trusted SQLite-worker fault injection and its synchronization oracle.

use super::*;

/// Trusted fault-harness barrier that holds only the SQLite worker before work.
/// The Runtime owner never waits on this barrier or shares its mutable state.
#[derive(Debug)]
pub(super) struct BarrierState {
    gate: Mutex<BarrierGate>,
    changed: Condvar,
    pub(super) hold_start: bool,
    pub(super) hold_after_fact_commit: bool,
    pub(super) hold_terminal_operation: bool,
    pub(super) panic_before_fact_sql: bool,
    pub(super) hold_finish: bool,
    pub(super) fail_periodic_wall_read: AtomicBool,
    pub(super) low_wal_threshold: Option<u64>,
    pub(super) fail_checkpoint_once: AtomicBool,
    pub(super) fail_close_after_seal: bool,
}

#[derive(Debug)]
struct BarrierGate {
    held: bool,
    reached: bool,
}
/// Trusted fault-harness barrier for a confirmed held storage stage.
#[derive(Clone, Debug)]
pub struct WriterBarrier(pub(super) Arc<BarrierState>);
impl WriterBarrier {
    /// Create a barrier initially holding the storage worker.
    pub fn held() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: true,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Hold the Start SQL barrier as well, for boundary admission tests.
    pub fn held_start() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: true,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: true,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Hold after a real fact transaction commits but before its owner receipt.
    /// This is a trusted process-failure test seam, never Runtime owner work.
    pub fn held_after_fact_commit() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: true,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: true,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Hold only a terminal operation before SQL, after earlier acceptance commits.
    /// Used to kill a real process at the accepted-only durability boundary.
    pub fn held_terminal_operation_after_acceptance() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: true,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: true,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Terminate the storage thread before one fact transaction for fault tests.
    pub fn panic_before_fact_sql() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: false,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: true,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Hold only Finish before SQL, after its owner FIFO admission.
    pub fn held_finish() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: true,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: true,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Inject one failed later UTC read on the SQLite worker's periodic path.
    /// This trusted test seam does not supply or alter Runtime control time.
    pub fn fail_next_periodic_wall_read() -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: false,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(true),
            low_wal_threshold: None,
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Use a smaller real WAL checkpoint threshold in the storage fault harness.
    pub fn low_wal_threshold_for_testing(bytes: u64) -> Self {
        Self(Arc::new(BarrierState {
            gate: Mutex::new(BarrierGate {
                held: false,
                reached: false,
            }),
            changed: Condvar::new(),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
            fail_periodic_wall_read: AtomicBool::new(false),
            low_wal_threshold: Some(bytes),
            fail_checkpoint_once: AtomicBool::new(false),
            fail_close_after_seal: false,
        }))
    }
    /// Arm exactly one fault at the next fact's worker-only WAL threshold gate.
    pub fn fail_next_checkpoint(&self) {
        self.0.fail_checkpoint_once.store(true, Ordering::Release);
    }
    /// Simulate a storage close error after the real terminal SQL seal commits.
    /// This fault seam is confined to the worker; it never changes Runtime safety.
    pub fn fail_close_after_seal() -> Self {
        let mut barrier = Self::low_wal_threshold_for_testing(16 * 1024 * 1024);
        Arc::get_mut(&mut barrier.0)
            .expect("fresh fault barrier")
            .fail_close_after_seal = true;
        barrier
    }
    /// Release any worker held at a deterministic storage stage.
    pub fn release(&self) {
        let mut gate = self
            .0
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        gate.held = false;
        drop(gate);
        self.0.changed.notify_all();
    }
    /// Whether the worker actually reached a held storage stage.
    pub fn reached(&self) -> bool {
        self.0
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reached
    }
    /// Wait for the exact worker barrier predicate, with a finite hang guard.
    pub fn wait_until_reached(&self, timeout: Duration) -> bool {
        let guard = self
            .0
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (gate, _) = self
            .0
            .changed
            .wait_timeout_while(guard, timeout, |gate| !gate.reached)
            .unwrap_or_else(|error| error.into_inner());
        gate.reached
    }
    pub(super) fn await_release(&self) {
        self.await_release_before_wait(|| {});
    }

    fn await_release_before_wait<F>(&self, before_wait: F)
    where
        F: FnOnce(),
    {
        let mut guard = self
            .0
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.reached = true;
        self.0.changed.notify_all();
        let mut before_wait = Some(before_wait);
        while guard.held {
            if let Some(before_wait) = before_wait.take() {
                before_wait();
            }
            guard = self
                .0
                .changed
                .wait(guard)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    pub(super) fn mark_reached(&self) {
        let mut gate = self
            .0
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        gate.reached = true;
        drop(gate);
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod writer_barrier_tests {
    use super::*;

    #[test]
    fn release_attempt_at_the_reached_wait_boundary_cannot_be_lost() {
        let barrier = WriterBarrier::held();
        let releaser_barrier = barrier.clone();
        let (start_release, release_requested) = mpsc::channel();
        let (released, release_completed) = mpsc::channel();
        let releaser = thread::spawn(move || {
            release_requested.recv().unwrap();
            releaser_barrier.release();
            released.send(()).unwrap();
        });

        // The hook runs after reached=true and after the worker observed held=true,
        // while it still owns the predicate mutex. The releaser therefore attempts
        // the formerly racy transition at the narrowest boundary. Condvar::wait
        // atomically releases the mutex before sleeping, so the release cannot be
        // observed without its notification.
        barrier.await_release_before_wait(|| start_release.send(()).unwrap());
        release_completed
            .recv_timeout(Duration::from_secs(1))
            .expect("release remained blocked after the waiter proceeded");
        releaser.join().unwrap();
        assert!(barrier.reached());
    }

    #[test]
    fn early_and_duplicate_release_are_idempotent() {
        let barrier = WriterBarrier::held();
        barrier.release();
        barrier.release();
        barrier.await_release();
        assert!(barrier.reached());
    }
}
