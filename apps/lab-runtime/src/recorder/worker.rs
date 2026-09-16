//! One storage thread, finite ingress credit, and coalesced durable receipts.
//!
//! Only the storage worker can block on SQL. The owner uses try-send and brief
//! receipt inspection; shutdown requests never join a worker stuck in OS I/O.

use super::{
    AnnotationRecord, BoundarySnapshot, HistoryCursor, HistoryFilter, HistoryPage, OperationRecord,
    ProvenanceEntry, ProvenanceObject, RecorderGap, RecordingPolicy, RunsCursor, RunsPage,
    SqliteStore, StorageError, StorageHealth, TimeAnchor,
};
use crate::host::{Clock, SystemClock};
use lab_core::{Value, recording::RecordingFact};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::RangeInclusive,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

const MAX_GROUPS: usize = 4;
const MAX_HISTORY_JOBS: usize = 8;
const MAX_RECORDS: usize = 1024;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_GROUP_BYTES: usize = 512 * 1024;

// The worker reads UTC against the exact serving M6 monotonic origin. Offline
// workers keep their own single origin; neither source enters Core decisions.
#[derive(Clone, Copy)]
enum MonotonicSource {
    Serving(SystemClock),
    Offline(Instant),
}
impl MonotonicSource {
    fn now(self) -> Duration {
        match self {
            Self::Serving(clock) => clock.now(),
            Self::Offline(origin) => origin.elapsed(),
        }
    }
}

/// Trusted fault-harness barrier that holds only the SQLite worker before work.
/// The Runtime owner never waits on this barrier or shares its mutable state.
#[derive(Debug)]
struct BarrierState {
    held: AtomicBool,
    reached: AtomicBool,
    hold_start: bool,
    hold_after_fact_commit: bool,
    hold_terminal_operation: bool,
    panic_before_fact_sql: bool,
    hold_finish: bool,
}
/// Trusted fault-harness barrier for a confirmed held storage stage.
#[derive(Clone, Debug)]
pub struct WriterBarrier(Arc<BarrierState>);
impl WriterBarrier {
    /// Create a barrier initially holding the storage worker.
    pub fn held() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(true),
            reached: AtomicBool::new(false),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
        }))
    }
    /// Hold the Start SQL barrier as well, for boundary admission tests.
    pub fn held_start() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(true),
            reached: AtomicBool::new(false),
            hold_start: true,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
        }))
    }
    /// Hold after a real fact transaction commits but before its owner receipt.
    /// This is a trusted process-failure test seam, never Runtime owner work.
    pub fn held_after_fact_commit() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(true),
            reached: AtomicBool::new(false),
            hold_start: false,
            hold_after_fact_commit: true,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: false,
        }))
    }
    /// Hold only a terminal operation before SQL, after earlier acceptance commits.
    /// Used to kill a real process at the accepted-only durability boundary.
    pub fn held_terminal_operation_after_acceptance() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(true),
            reached: AtomicBool::new(false),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: true,
            panic_before_fact_sql: false,
            hold_finish: false,
        }))
    }
    /// Terminate the storage thread before one fact transaction for fault tests.
    pub fn panic_before_fact_sql() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(false),
            reached: AtomicBool::new(false),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: true,
            hold_finish: false,
        }))
    }
    /// Hold only Finish before SQL, after its owner FIFO admission.
    pub fn held_finish() -> Self {
        Self(Arc::new(BarrierState {
            held: AtomicBool::new(true),
            reached: AtomicBool::new(false),
            hold_start: false,
            hold_after_fact_commit: false,
            hold_terminal_operation: false,
            panic_before_fact_sql: false,
            hold_finish: true,
        }))
    }
    /// Release any worker held at a deterministic storage stage.
    pub fn release(&self) {
        self.0.held.store(false, Ordering::Release);
    }
    /// Whether the worker actually reached a held storage stage.
    pub fn reached(&self) -> bool {
        self.0.reached.load(Ordering::Acquire)
    }
    fn await_release(&self) {
        self.0.reached.store(true, Ordering::Release);
        while self.0.held.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Hard M7 ingress limits; smaller test profiles are permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecorderLimits {
    /// Records including those inside a worker transaction.
    pub records: usize,
    /// Accounted owned bytes including those inside a worker transaction.
    pub bytes: usize,
    /// Complete groups awaiting or executing a commit.
    pub groups: usize,
}
impl Default for RecorderLimits {
    fn default() -> Self {
        Self {
            records: MAX_RECORDS,
            bytes: MAX_BYTES,
            groups: MAX_GROUPS,
        }
    }
}
impl RecorderLimits {
    fn valid(self) -> bool {
        (1..=MAX_RECORDS).contains(&self.records)
            && (256..=MAX_BYTES).contains(&self.bytes)
            && (1..=MAX_GROUPS).contains(&self.groups)
    }
}

/// Owner-visible recording lifecycle; `Recording` follows a committed start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingState {
    /// Storage is open but no run is recording.
    Idle,
    /// A start barrier is queued and Required control remains inhibited.
    Starting,
    /// Start was committed and bounded fact admission is open.
    Recording,
    /// Stop barrier was queued; further ordinary admission is closed.
    Stopping,
    /// A sticky admission, worker, or SQLite failure closed ordinary coverage.
    Failed,
    /// Worker has closed its connection.
    Closed,
}

/// Owned status sampled without any SQL or worker wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingStatus {
    /// Current recording state.
    pub state: RecordingState,
    /// Highest record sequence actually committed and receipted.
    pub persisted_through_sequence: u64,
    /// Records still charged, including a batch inside SQL.
    pub outstanding_records: usize,
    /// Accounted bytes still charged, including a batch inside SQL.
    pub outstanding_bytes: usize,
    /// Groups still charged, including a batch inside SQL.
    pub outstanding_groups: usize,
    /// First stable failure message, retained through reconnect.
    pub first_error: Option<String>,
    /// Original owner time of the latest committed start/group/probe receipt.
    pub confirmed_submission: Option<Duration>,
    /// Current checked run counter within the serving boot.
    pub run_no: Option<u64>,
    /// Current checked coverage interval counter within the serving boot.
    pub interval_no: Option<u64>,
    /// A terminal boot seal committed and its receipt reached the owner.
    pub terminal_seal_committed: bool,
    /// Root of the committed frozen active composition before listener readiness.
    pub activation_root: Option<[u8; 32]>,
    /// Owned coverage is `complete` or `gap`; failed storage may leave it unsealed.
    pub coverage: &'static str,
    /// First source fact known to have been lost, if the source identity is known.
    pub first_missing_fact: Option<u64>,
    /// A failure seal actually committed and its receipt reached the owner.
    pub failure_persisted: bool,
    /// Worker thread closed its SQLite connection; independent of coverage.
    pub worker_closed: bool,
    /// Hard owner ingress limits selected before the worker starts.
    pub limits: RecorderLimits,
    /// Last worker-observed main/WAL footprint; absent only before readiness.
    pub storage: Option<StorageHealth>,
}

#[derive(Clone)]
struct Receipt {
    state: RecordingState,
    persisted: u64,
    released_records: usize,
    released_bytes: usize,
    released_groups: usize,
    first_error: Option<String>,
    confirmed_submission: Option<Duration>,
    run_no: Option<u64>,
    interval_no: Option<u64>,
    terminal_seal_committed: bool,
    activation_root: Option<[u8; 32]>,
    failure_persisted: bool,
    storage: Option<StorageHealth>,
}
impl Default for Receipt {
    fn default() -> Self {
        Self {
            state: RecordingState::Idle,
            persisted: 0,
            released_records: 0,
            released_bytes: 0,
            released_groups: 0,
            first_error: None,
            confirmed_submission: None,
            run_no: None,
            interval_no: None,
            terminal_seal_committed: false,
            activation_root: None,
            failure_persisted: false,
            storage: None,
        }
    }
}

enum Message {
    Activation(Vec<ProvenanceEntry>, Vec<ProvenanceObject>),
    Start(String, RecordingPolicy, Duration, BoundarySnapshot, u64),
    Facts(Vec<RecordingFact>, usize, Duration, Instant, u64),
    ClockAnchor(u64),
    Operation(OperationRecord, usize, u64),
    Annotation(AnnotationRecord, usize, u64),
    GapSeal(RecorderGap, u64),
    Probe(Duration),
    History {
        job: u64,
        filter: HistoryFilter,
        cursor: Option<HistoryCursor>,
        limit: usize,
    },
    Runs {
        job: u64,
        cursor: Option<RunsCursor>,
        limit: usize,
    },
    Stop(serde_json::Value, Duration, u64),
    Finish(serde_json::Value, Duration, u64),
}

/// Runtime-owned ingress to one SQLite worker. No method executes disk I/O.
pub struct RecorderWorker {
    sender: SyncSender<Message>,
    receipt: Arc<Mutex<Receipt>>,
    alive: Arc<AtomicBool>,
    /// Dropping this handle detaches rather than waiting for a blocked OS call.
    _thread: thread::JoinHandle<()>,
    limits: RecorderLimits,
    charged_records: usize,
    charged_bytes: usize,
    charged_groups: usize,
    seen_released_records: usize,
    seen_released_bytes: usize,
    seen_released_groups: usize,
    last_probe_requested: Option<Duration>,
    probe_pending: bool,
    cached: RecordingStatus,
    database_id: String,
    boot_id: String,
    selected_policy: RecordingPolicy,
    policy_configured: bool,
    history_mailbox: Arc<Mutex<BTreeMap<u64, Result<HistoryPage, StorageError>>>>,
    runs_mailbox: Arc<Mutex<BTreeMap<u64, Result<RunsPage, StorageError>>>>,
    active_history_jobs: Arc<Mutex<BTreeSet<u64>>>,
    next_history_job: u64,
    live_history_jobs: BTreeSet<u64>,
    pending_cancellations: BTreeSet<u64>,
    reserved_through: u64,
    source: MonotonicSource,
    last_periodic: Duration,
    periodic_pending: Option<u64>,
    last_owner_submission: Option<Duration>,
    last_accepted_fact: Option<u64>,
    gap_scheduled: bool,
    finish_requested: bool,
}

impl RecorderWorker {
    /// Open SQLite on the worker before readiness, with a finite startup barrier.
    pub fn open(path: &Path, limits: RecorderLimits) -> Result<Self, StorageError> {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|error| StorageError(format!("boot entropy unavailable: {error}")))?;
        let boot: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        Self::open_with_boot(path, limits, &boot)
    }

    /// Open with the exact serving Runtime boot ID, before listener readiness.
    pub fn open_with_boot(
        path: &Path,
        limits: RecorderLimits,
        boot_id: &str,
    ) -> Result<Self, StorageError> {
        Self::open_internal(path, limits, boot_id, None, None, None)
    }

    /// Open using the authoritative M6 clock origin's bracketed boot UTC.
    /// This is a trusted host-only seam; no client may choose an anchor.
    pub fn open_with_boot_clock(
        path: &Path,
        limits: RecorderLimits,
        boot_id: &str,
        anchor: TimeAnchor,
        clock: SystemClock,
    ) -> Result<Self, StorageError> {
        Self::open_internal(path, limits, boot_id, None, Some(anchor), Some(clock))
    }

    /// Fault-harness composition for testing blocked disk without blocking Core.
    pub fn open_with_barrier(
        path: &Path,
        limits: RecorderLimits,
        barrier: WriterBarrier,
    ) -> Result<Self, StorageError> {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|error| StorageError(format!("boot entropy unavailable: {error}")))?;
        let boot: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        Self::open_internal(path, limits, &boot, Some(barrier), None, None)
    }

    fn open_internal(
        path: &Path,
        limits: RecorderLimits,
        boot_id: &str,
        barrier: Option<WriterBarrier>,
        anchor: Option<TimeAnchor>,
        clock: Option<SystemClock>,
    ) -> Result<Self, StorageError> {
        if !limits.valid() {
            return Err(StorageError("invalid bounded recorder limits".into()));
        }
        // Ordinary group/job caps leave two physical FIFO slots for a failure
        // seal and finish request even while a writer is stalled.
        let (sender, receiver) = mpsc::sync_channel(MAX_GROUPS + MAX_HISTORY_JOBS + 6);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let receipt = Arc::new(Mutex::new(Receipt::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let history_mailbox = Arc::new(Mutex::new(BTreeMap::new()));
        let runs_mailbox = Arc::new(Mutex::new(BTreeMap::new()));
        let active_history_jobs = Arc::new(Mutex::new(BTreeSet::new()));
        let thread_receipt = Arc::clone(&receipt);
        let thread_alive = Arc::clone(&alive);
        let thread_history = Arc::clone(&history_mailbox);
        let thread_runs = Arc::clone(&runs_mailbox);
        let thread_jobs = Arc::clone(&active_history_jobs);
        let worker_path = path.to_path_buf();
        let worker_boot = boot_id.to_owned();
        let source = clock
            .map(MonotonicSource::Serving)
            .unwrap_or_else(|| MonotonicSource::Offline(Instant::now()));
        let last_periodic = source.now();
        let handle = thread::Builder::new()
            .name("lab-recorder-sqlite".into())
            .spawn(move || {
                let opened = anchor
                    .map(Ok)
                    .unwrap_or_else(|| {
                        TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    })
                    .and_then(|anchor| {
                        SqliteStore::open_with_boot_anchor(&worker_path, &worker_boot, anchor)
                    })
                    .and_then(|store| {
                        let health = store.storage_health()?;
                        thread_receipt
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .storage = Some(health);
                        Ok(store)
                    });
                let _ = ready_sender.send(
                    opened
                        .as_ref()
                        .map(|store| store.database_id().to_owned())
                        .map_err(|error| StorageError(error.to_string())),
                );
                if let Ok(store) = opened {
                    let _ = worker_loop(
                        store,
                        receiver,
                        &thread_receipt,
                        &thread_history,
                        &thread_runs,
                        &thread_jobs,
                        barrier.as_ref(),
                        source,
                    );
                }
                // The connection and WAL handles have been dropped before a
                // terminal close receipt becomes visible to the owner.
                {
                    let mut status = thread_receipt.lock().unwrap_or_else(|p| p.into_inner());
                    if status.state != RecordingState::Failed && status.terminal_seal_committed {
                        status.state = RecordingState::Closed;
                    } else if status.state != RecordingState::Failed {
                        status.state = RecordingState::Failed;
                        status.first_error.get_or_insert_with(|| {
                            "storage worker exited without a terminal seal".into()
                        });
                    }
                }
                thread_alive.store(false, Ordering::Release);
            })
            .map_err(|error| StorageError(format!("storage worker spawn: {error}")))?;
        let database_id = ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| StorageError("storage startup deadline exceeded".into()))??;
        Ok(Self {
            sender,
            receipt,
            alive,
            _thread: handle,
            limits,
            charged_records: 0,
            charged_bytes: 0,
            charged_groups: 0,
            seen_released_records: 0,
            seen_released_bytes: 0,
            seen_released_groups: 0,
            last_probe_requested: None,
            probe_pending: false,
            cached: RecordingStatus {
                state: RecordingState::Idle,
                persisted_through_sequence: 0,
                outstanding_records: 0,
                outstanding_bytes: 0,
                outstanding_groups: 0,
                first_error: None,
                confirmed_submission: None,
                run_no: None,
                interval_no: None,
                terminal_seal_committed: false,
                activation_root: None,
                coverage: "complete",
                first_missing_fact: None,
                failure_persisted: false,
                worker_closed: false,
                limits,
                storage: None,
            },
            database_id,
            boot_id: boot_id.to_owned(),
            selected_policy: RecordingPolicy::Required,
            policy_configured: false,
            history_mailbox,
            runs_mailbox,
            active_history_jobs,
            next_history_job: 1,
            live_history_jobs: BTreeSet::new(),
            pending_cancellations: BTreeSet::new(),
            reserved_through: 0,
            source,
            last_periodic,
            periodic_pending: None,
            last_owner_submission: None,
            last_accepted_fact: None,
            gap_scheduled: false,
            finish_requested: false,
        })
    }

    /// Schedule one bounded archived read without SQL on the Runtime owner.
    /// The eight slots include queued, executing and retained results.
    pub fn request_history(
        &mut self,
        filter: HistoryFilter,
        cursor: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.drain_history_cancellations();
        if !self.pending_cancellations.is_empty() {
            return Err(StorageError("history cancellation pending".into()));
        }
        if !(1..=128).contains(&limit) || filter.from >= filter.to || filter.boot_id.len() != 32 {
            return Err(StorageError("invalid bounded history request".into()));
        }
        let job = self.next_history_job;
        let following = job
            .checked_add(1)
            .ok_or_else(|| StorageError("history job identity exhausted".into()))?;
        let mut active = self
            .active_history_jobs
            .try_lock()
            .map_err(|_| StorageError("history slots busy".into()))?;
        if active.len() >= MAX_HISTORY_JOBS {
            return Err(StorageError("history slots exhausted".into()));
        }
        active.insert(job);
        match self.sender.try_send(Message::History {
            job,
            filter,
            cursor,
            limit,
        }) {
            Ok(()) => {
                self.next_history_job = following;
                self.live_history_jobs.insert(job);
                Ok(job)
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                active.remove(&job);
                Err(StorageError("history worker busy".into()))
            }
        }
    }

    /// Queue one frozen startup activation set before listener readiness.
    /// The owned source bytes are transferred once; hashing and SQL stay on the
    /// worker. The caller must await its receipt before advertising readiness.
    pub fn request_activation(
        &mut self,
        entries: Vec<ProvenanceEntry>,
        objects: Vec<ProvenanceObject>,
    ) -> Result<(), StorageError> {
        if self.poll().state != RecordingState::Idle || self.cached.activation_root.is_some() {
            return Err(StorageError(
                "activation requires idle unactivated worker".into(),
            ));
        }
        self.send_control(Message::Activation(entries, objects))
    }

    /// Schedule one indexed archived-run discovery page on the storage worker.
    pub fn request_runs(
        &mut self,
        cursor: Option<RunsCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.drain_history_cancellations();
        if !self.pending_cancellations.is_empty() {
            return Err(StorageError("history cancellation pending".into()));
        }
        if !(1..=32).contains(&limit) {
            return Err(StorageError("invalid run page limit".into()));
        }
        let job = self.next_history_job;
        let next = job
            .checked_add(1)
            .ok_or_else(|| StorageError("history job identity exhausted".into()))?;
        let mut active = self
            .active_history_jobs
            .try_lock()
            .map_err(|_| StorageError("history slots busy".into()))?;
        if active.len() >= MAX_HISTORY_JOBS {
            return Err(StorageError("history slots exhausted".into()));
        }
        active.insert(job);
        match self.sender.try_send(Message::Runs { job, cursor, limit }) {
            Ok(()) => {
                self.next_history_job = next;
                self.live_history_jobs.insert(job);
                Ok(job)
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                active.remove(&job);
                Err(StorageError("history worker busy".into()))
            }
        }
    }

    /// Take only a completed result; a pending SQL read never blocks this call.
    pub fn try_take_history(&mut self, job: u64) -> Option<Result<HistoryPage, StorageError>> {
        self.drain_history_cancellations();
        if self.pending_cancellations.contains(&job) {
            return None;
        }
        let mut active = self.active_history_jobs.try_lock().ok()?;
        let result = self.history_mailbox.try_lock().ok()?.remove(&job)?;
        active.remove(&job);
        self.live_history_jobs.remove(&job);
        Some(result)
    }

    /// Take only a completed archived-run page, releasing its bounded slot.
    pub fn try_take_runs(&mut self, job: u64) -> Option<Result<RunsPage, StorageError>> {
        self.drain_history_cancellations();
        if self.pending_cancellations.contains(&job) {
            return None;
        }
        let mut active = self.active_history_jobs.try_lock().ok()?;
        let result = self.runs_mailbox.try_lock().ok()?.remove(&job)?;
        active.remove(&job);
        self.live_history_jobs.remove(&job);
        Some(result)
    }

    /// Fence an expired/disconnected job; its late worker result is discarded.
    pub fn cancel_history(&mut self, job: u64) {
        // An owner must not wait for a transient worker mutex. Retain at most
        // the eight already-admitted jobs and retry on every later owner poll.
        if !self.live_history_jobs.remove(&job) {
            return;
        }
        self.pending_cancellations.insert(job);
        self.drain_history_cancellations();
    }

    fn drain_history_cancellations(&mut self) {
        if self.pending_cancellations.is_empty() {
            return;
        }
        let Ok(mut active) = self.active_history_jobs.try_lock() else {
            return;
        };
        let Ok(mut history) = self.history_mailbox.try_lock() else {
            return;
        };
        let Ok(mut runs) = self.runs_mailbox.try_lock() else {
            return;
        };
        for job in &self.pending_cancellations {
            active.remove(job);
            history.remove(job);
            runs.remove(job);
        }
        self.pending_cancellations.clear();
    }

    /// Stable archive database identity retained across process reopen.
    pub fn database_id(&self) -> &str {
        &self.database_id
    }

    /// Serving process boot identity shared with the public server.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }

    /// Freeze the trusted host policy before the first run; clients cannot set it.
    pub fn configure_policy(&mut self, policy: RecordingPolicy) -> Result<(), StorageError> {
        if self.policy_configured || self.poll().state != RecordingState::Idle {
            return Err(StorageError(
                "recording policy already configured or active".into(),
            ));
        }
        self.selected_policy = policy;
        self.policy_configured = true;
        Ok(())
    }

    /// Queue a start barrier; `Recording` is visible only after SQL commits it.
    pub fn request_start(&mut self, label: &str) -> Result<(), StorageError> {
        self.request_start_at(label, Duration::ZERO)
    }

    /// Queue a start with its original monotonic submission time.
    pub fn request_start_at(
        &mut self,
        label: &str,
        submitted_at: Duration,
    ) -> Result<(), StorageError> {
        self.request_start_with_boundary(label, submitted_at, BoundarySnapshot::empty(submitted_at))
    }

    /// Queue the owner-frozen boundary before all post-boundary fact groups.
    pub fn request_start_with_boundary(
        &mut self,
        label: &str,
        submitted_at: Duration,
        boundary: BoundarySnapshot,
    ) -> Result<(), StorageError> {
        if self.poll().state != RecordingState::Idle
            || label.trim().is_empty()
            || label.len() > 128
            || boundary.at != submitted_at
            || !boundary.valid()
        {
            return Err(StorageError("invalid recording start".into()));
        }
        let assigned = self.planned_range(2)?;
        self.change_state(RecordingState::Starting);
        self.send_control(Message::Start(
            label.to_owned(),
            self.selected_policy,
            submitted_at,
            boundary,
            *assigned.start(),
        ))?;
        self.reserved_through = *assigned.end();
        self.last_owner_submission = Some(submitted_at);
        self.last_probe_requested = Some(submitted_at);
        Ok(())
    }

    /// Try to transfer one complete capture group, charging in-flight payloads.
    /// Return owner-assigned record identities while durability remains pending.
    /// Queue saturation latches failure; no producer waits for disk capacity.
    pub fn try_admit(
        &mut self,
        facts: Vec<RecordingFact>,
    ) -> Result<RangeInclusive<u64>, StorageError> {
        self.try_admit_at(facts, Duration::ZERO)
    }

    /// Try to admit a complete group with its original owner submission time.
    /// The returned range is reserved only after the whole FIFO transfer succeeds.
    pub fn try_admit_at(
        &mut self,
        facts: Vec<RecordingFact>,
        submitted_at: Duration,
    ) -> Result<RangeInclusive<u64>, StorageError> {
        self.poll();
        if !matches!(
            self.cached.state,
            RecordingState::Starting | RecordingState::Recording
        ) || facts.is_empty()
            || facts.len() > 256
        {
            return Err(StorageError("recording group not admissible".into()));
        }
        // The transferred Vec keeps its allocation until the transaction has
        // completed. Charge its capacity and worst-case escaped value bytes,
        // including the worker's bounded encoding scratch, before transfer.
        let vec_bytes = facts
            .capacity()
            .saturating_mul(std::mem::size_of::<RecordingFact>());
        let (bytes, oversized_record) = facts.iter().fold((vec_bytes, false), |state, fact| {
            let charge = fact_charge(fact);
            (
                state.0.saturating_add(charge),
                state.1 || charge > 64 * 1024,
            )
        });
        if oversized_record
            || bytes > MAX_GROUP_BYTES
            || self.charged_groups >= self.limits.groups
            || facts.len() > self.limits.records.saturating_sub(self.charged_records)
            || bytes > self.limits.bytes.saturating_sub(self.charged_bytes)
        {
            self.fail_with_gap(RecorderGap {
                reason: "recorder ingress capacity exhausted".into(),
                at: submitted_at,
                first_missing_fact: Some(facts[0].sequence()),
                known_missing_count: u64::try_from(facts.len()).ok(),
                last_accepted_fact: self.last_accepted_fact,
            });
            return Err(StorageError("recorder ingress capacity exhausted".into()));
        }
        let record_count = facts.len();
        let assigned = self.planned_range(record_count)?;
        let last_fact = facts.last().map(RecordingFact::sequence);
        let message = Message::Facts(
            facts,
            bytes,
            submitted_at,
            Instant::now(),
            *assigned.start(),
        );
        match self.sender.try_send(message) {
            Ok(()) => {
                self.reserved_through = *assigned.end();
                self.last_owner_submission = Some(
                    self.last_owner_submission
                        .map_or(submitted_at, |latest| latest.max(submitted_at)),
                );
                self.charged_records += record_count;
                self.charged_bytes += bytes;
                self.charged_groups += 1;
                self.last_accepted_fact = last_fact;
                Ok(assigned)
            }
            Err(
                TrySendError::Full(Message::Facts(facts, _, _, _, _))
                | TrySendError::Disconnected(Message::Facts(facts, _, _, _, _)),
            ) => {
                self.fail_with_gap(RecorderGap {
                    reason: "recorder ingress unavailable".into(),
                    at: submitted_at,
                    first_missing_fact: Some(facts[0].sequence()),
                    known_missing_count: u64::try_from(facts.len()).ok(),
                    last_accepted_fact: self.last_accepted_fact,
                });
                Err(StorageError("recorder ingress unavailable".into()))
            }
            Err(_) => unreachable!("fact transfer returns its own message"),
        }
    }

    /// Charge and transfer one original application-boundary fact without disk I/O.
    /// A rejected transfer is a sticky coverage failure; it never edits the
    /// already committed application outcome.
    pub fn try_admit_operation(&mut self, operation: OperationRecord) -> Result<(), StorageError> {
        self.poll();
        let lifecycle_terminal = self.cached.state == RecordingState::Idle
            && operation.command == "recording_stop"
            && matches!(operation.phase, "completed" | "failed");
        if !(matches!(
            self.cached.state,
            RecordingState::Starting | RecordingState::Recording
        ) || lifecycle_terminal)
            || !operation.valid()
        {
            return Err(StorageError("operation fact not admissible".into()));
        }
        let bytes = operation
            .charge()
            .ok_or_else(|| StorageError("operation credit arithmetic exhausted".into()))?;
        if bytes > MAX_GROUP_BYTES
            || self.charged_groups >= self.limits.groups
            || self.charged_records >= self.limits.records
            || bytes > self.limits.bytes.saturating_sub(self.charged_bytes)
        {
            self.fail_with_gap(RecorderGap {
                reason: "recorder ingress capacity exhausted".into(),
                at: operation.at,
                first_missing_fact: None,
                known_missing_count: Some(1),
                last_accepted_fact: self.last_accepted_fact,
            });
            return Err(StorageError("recorder ingress capacity exhausted".into()));
        }
        let at = operation.at;
        let assigned = self.planned_range(1)?;
        match self
            .sender
            .try_send(Message::Operation(operation, bytes, *assigned.start()))
        {
            Ok(()) => {
                self.reserved_through = *assigned.end();
                self.last_owner_submission = Some(
                    self.last_owner_submission
                        .map_or(at, |latest| latest.max(at)),
                );
                self.charged_records += 1;
                self.charged_bytes += bytes;
                self.charged_groups += 1;
                Ok(())
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.fail_with_gap(RecorderGap {
                    reason: "recorder ingress unavailable".into(),
                    at,
                    first_missing_fact: None,
                    known_missing_count: Some(1),
                    last_accepted_fact: self.last_accepted_fact,
                });
                Err(StorageError("recorder ingress unavailable".into()))
            }
        }
    }

    /// Reserve the exact annotation record ID after an atomic bounded transfer.
    /// Its returned ID is an ingress receipt, never a durable confirmation.
    pub fn try_admit_annotation(
        &mut self,
        annotation: AnnotationRecord,
    ) -> Result<u64, StorageError> {
        self.poll();
        if self.cached.state != RecordingState::Recording || !annotation.valid() {
            return Err(StorageError("annotation outside active interval".into()));
        }
        let bytes = annotation
            .charge()
            .ok_or_else(|| StorageError("annotation credit arithmetic exhausted".into()))?;
        if bytes > 64 * 1024
            || bytes > MAX_GROUP_BYTES
            || self.charged_groups >= self.limits.groups
            || self.charged_records >= self.limits.records
            || bytes > self.limits.bytes.saturating_sub(self.charged_bytes)
        {
            self.fail_with_gap(RecorderGap {
                reason: "recorder annotation ingress capacity exhausted".into(),
                at: annotation.at,
                first_missing_fact: None,
                known_missing_count: Some(1),
                last_accepted_fact: self.last_accepted_fact,
            });
            return Err(StorageError("annotation ingress capacity exhausted".into()));
        }
        let assigned = *self.planned_range(1)?.start();
        let at = annotation.at;
        match self
            .sender
            .try_send(Message::Annotation(annotation, bytes, assigned))
        {
            Ok(()) => {
                self.reserved_through = assigned;
                self.last_owner_submission = Some(
                    self.last_owner_submission
                        .map_or(at, |latest| latest.max(at)),
                );
                self.charged_records += 1;
                self.charged_bytes += bytes;
                self.charged_groups += 1;
                Ok(assigned)
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.fail_with_gap(RecorderGap {
                    reason: "recorder annotation ingress unavailable".into(),
                    at,
                    first_missing_fact: None,
                    known_missing_count: Some(1),
                    last_accepted_fact: self.last_accepted_fact,
                });
                Err(StorageError("annotation ingress unavailable".into()))
            }
        }
    }

    /// Queue a durable stop barrier after all previously admitted groups.
    pub fn request_stop(&mut self) -> Result<(), StorageError> {
        self.request_stop_with_summary(serde_json::json!({"pending_operations":[]}), Duration::ZERO)
    }

    /// Seal the owner's bounded pending-operation barrier after prior FIFO work.
    pub fn request_stop_with_summary(
        &mut self,
        summary: serde_json::Value,
        requested_at: Duration,
    ) -> Result<(), StorageError> {
        if self.poll().state != RecordingState::Recording {
            return Err(StorageError("recording stop requires active run".into()));
        }
        if serde_json::to_vec(&summary)
            .map_err(|error| StorageError(format!("stop summary: {error}")))?
            .len()
            > 16 * 1024 - 512
        {
            return Err(StorageError(
                "stop summary exceeds reserved seal credit".into(),
            ));
        }
        let assigned = self.planned_range(2)?;
        self.change_state(RecordingState::Stopping);
        self.send_control(Message::Stop(summary, requested_at, *assigned.start()))?;
        self.reserved_through = *assigned.end();
        Ok(())
    }

    /// Coalesce quiet-run probes at 250 ms and await their actual commit receipt.
    pub fn request_probe_at(&mut self, submitted_at: Duration) -> Result<(), StorageError> {
        self.poll();
        if self.cached.state != RecordingState::Recording {
            return Err(StorageError("progress probe outside recording".into()));
        }
        if self.probe_pending
            || self.last_probe_requested.is_some_and(|old| {
                old.checked_add(Duration::from_millis(250))
                    .is_none_or(|due| submitted_at < due)
            })
        {
            return Ok(());
        }
        self.send_control(Message::Probe(submitted_at))?;
        self.last_owner_submission = Some(
            self.last_owner_submission
                .map_or(submitted_at, |latest| latest.max(submitted_at)),
        );
        self.last_probe_requested = Some(submitted_at);
        self.probe_pending = true;
        Ok(())
    }

    /// Ask the worker to close after accepted work and lifecycle barriers.
    pub fn request_finish(&mut self) -> Result<(), StorageError> {
        self.request_finish_with_summary(
            serde_json::json!({"safety_evidence":"unprovided"}),
            Duration::ZERO,
        )
    }

    /// Seal a trusted frozen shutdown observation after the accepted prefix.
    pub fn request_finish_with_summary(
        &mut self,
        summary: serde_json::Value,
        at: Duration,
    ) -> Result<(), StorageError> {
        if self.finish_requested {
            return Ok(());
        }
        if !matches!(
            self.poll().state,
            RecordingState::Idle | RecordingState::Failed
        ) {
            return Err(StorageError("finish requires idle or failed run".into()));
        }
        if serde_json::to_vec(&summary)
            .map_err(|error| StorageError(format!("shutdown evidence: {error}")))?
            .len()
            > 16 * 1024
        {
            return Err(StorageError("shutdown evidence exceeds 16 KiB".into()));
        }
        let assigned = self.planned_range(2)?;
        self.send_control(Message::Finish(summary, at, *assigned.start()))?;
        self.reserved_through = *assigned.end();
        self.finish_requested = true;
        Ok(())
    }

    /// Consume a cumulative receipt and release credit exactly once.
    /// This reads no SQLite state and never waits for the writer's lock.
    pub fn poll(&mut self) -> RecordingStatus {
        self.drain_history_cancellations();
        let fresh_receipt = self.receipt.try_lock().ok().map(|receipt| receipt.clone());
        let status = self.reconcile_receipt(fresh_receipt);
        self.schedule_periodic();
        status
    }

    fn reconcile_receipt(&mut self, fresh_receipt: Option<Receipt>) -> RecordingStatus {
        if let Some(receipt) = fresh_receipt {
            self.apply_receipt(receipt);
        }
        let alive = self.alive.load(Ordering::Acquire);
        let panicked = alive && self._thread.is_finished();
        if panicked {
            // A Rust panic bypasses the worker's final alive/receipt stores.
            // Detect it without joining or claiming that queued facts committed.
            self.fail("storage worker panicked");
        }
        let worker_closed = !alive || panicked;
        if worker_closed
            && !matches!(
                self.cached.state,
                RecordingState::Closed | RecordingState::Failed
            )
        {
            // The worker may close between the first receipt clone and this
            // alive check. Read its final published receipt once more before
            // calling a successful seal/close an unexpected exit.
            let final_receipt = self.receipt.try_lock().ok().map(|receipt| receipt.clone());
            if let Some(receipt) = final_receipt {
                self.apply_receipt(receipt);
                if !matches!(
                    self.cached.state,
                    RecordingState::Closed | RecordingState::Failed
                ) {
                    self.fail("recorder worker exited unexpectedly");
                }
            }
        }
        self.cached.outstanding_records = self.charged_records;
        self.cached.outstanding_bytes = self.charged_bytes;
        self.cached.outstanding_groups = self.charged_groups;
        self.cached.worker_closed = worker_closed;
        self.cached.clone()
    }

    fn apply_receipt(&mut self, receipt: Receipt) {
        if receipt.persisted < self.cached.persisted_through_sequence {
            // A stale clone cannot lower a watermark or undo a later receipt.
            return;
        }
        if receipt.persisted > self.reserved_through {
            self.fail("future storage record receipt");
            return;
        }
        if receipt.confirmed_submission.is_some_and(|confirmed| {
            self.last_owner_submission
                .is_none_or(|latest| confirmed > latest)
        }) {
            self.fail("future storage submission receipt");
            return;
        }
        if receipt.confirmed_submission < self.cached.confirmed_submission {
            // A stale heartbeat clone cannot move the Required progress clock back.
            return;
        }
        if receipt.released_records < self.seen_released_records
            || receipt.released_bytes < self.seen_released_bytes
            || receipt.released_groups < self.seen_released_groups
        {
            self.fail("regressing storage receipt");
        } else {
            let new_records = receipt.released_records - self.seen_released_records;
            let new_bytes = receipt.released_bytes - self.seen_released_bytes;
            let new_groups = receipt.released_groups - self.seen_released_groups;
            if new_records > self.charged_records
                || new_bytes > self.charged_bytes
                || new_groups > self.charged_groups
            {
                self.fail("future storage receipt");
            } else {
                self.charged_records -= new_records;
                self.charged_bytes -= new_bytes;
                self.charged_groups -= new_groups;
                self.seen_released_records = receipt.released_records;
                self.seen_released_bytes = receipt.released_bytes;
                self.seen_released_groups = receipt.released_groups;
                self.cached.persisted_through_sequence = receipt.persisted;
                self.cached.confirmed_submission = receipt.confirmed_submission;
                self.cached.run_no = receipt.run_no;
                self.cached.interval_no = receipt.interval_no;
                self.cached.terminal_seal_committed = receipt.terminal_seal_committed;
                self.cached.activation_root = receipt.activation_root;
                self.cached.failure_persisted = receipt.failure_persisted;
                self.cached.storage = receipt.storage;
                if self
                    .periodic_pending
                    .is_some_and(|id| receipt.persisted >= id)
                {
                    self.periodic_pending = None;
                }
                if self.probe_pending
                    && self.last_probe_requested.is_some_and(|requested| {
                        receipt
                            .confirmed_submission
                            .is_some_and(|confirmed| confirmed >= requested)
                    })
                {
                    self.probe_pending = false;
                }
                // An owner-submitted lifecycle barrier remains pending
                // until the worker publishes its committed successor.
                let pending_start = self.cached.state == RecordingState::Starting
                    && receipt.state == RecordingState::Idle;
                let pending_stop = self.cached.state == RecordingState::Stopping
                    && receipt.state == RecordingState::Recording;
                let sticky_failure = self.cached.state == RecordingState::Failed;
                if !pending_start && !pending_stop && !sticky_failure {
                    self.cached.state = receipt.state;
                }
                if receipt.state == RecordingState::Failed
                    && self.cached.coverage == "complete"
                    && self.cached.run_no.is_some()
                {
                    self.cached.coverage = "unknown_tail";
                }
                if self.cached.first_error.is_none() {
                    self.cached.first_error = receipt.first_error;
                }
            }
        }
    }

    fn planned_range(&self, count: usize) -> Result<RangeInclusive<u64>, StorageError> {
        let first = self
            .reserved_through
            .checked_add(1)
            .ok_or_else(|| StorageError("record identity exhausted".into()))?;
        let last = self
            .reserved_through
            .checked_add(
                u64::try_from(count).map_err(|_| StorageError("record count exhausted".into()))?,
            )
            .ok_or_else(|| StorageError("record identity exhausted".into()))?;
        Ok(first..=last)
    }

    fn schedule_periodic(&mut self) {
        if self.periodic_pending.is_some()
            || self.finish_requested
            || !matches!(
                self.cached.state,
                RecordingState::Idle | RecordingState::Recording
            )
        {
            return;
        }
        let now = self.source.now();
        if now.saturating_sub(self.last_periodic) < Duration::from_secs(1) {
            return;
        }
        let Ok(assigned) = self.planned_range(1) else {
            self.fail("clock record identity exhausted");
            return;
        };
        if self
            .sender
            .try_send(Message::ClockAnchor(*assigned.start()))
            .is_ok()
        {
            self.reserved_through = *assigned.end();
            self.last_periodic = now;
            self.periodic_pending = Some(*assigned.start());
        }
    }

    fn send_control(&mut self, message: Message) -> Result<(), StorageError> {
        match self.sender.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.fail("recorder lifecycle slot unavailable");
                Err(StorageError("recorder lifecycle slot unavailable".into()))
            }
        }
    }

    fn change_state(&mut self, state: RecordingState) {
        self.cached.state = state;
    }

    fn fail(&mut self, message: &str) {
        if self.cached.coverage == "complete" && self.cached.run_no.is_some() {
            self.cached.coverage = "unknown_tail";
        }
        self.cached.state = RecordingState::Failed;
        self.cached
            .first_error
            .get_or_insert_with(|| message.to_owned());
        if let Ok(mut receipt) = self.receipt.try_lock() {
            receipt.state = RecordingState::Failed;
            receipt
                .first_error
                .get_or_insert_with(|| message.to_owned());
        }
    }

    /// Freeze coverage on the first known loss and reserve a FIFO failure seal.
    /// SQL failure or a disconnected worker leaves `failure_persisted=false`.
    pub fn fail_with_gap(&mut self, gap: RecorderGap) {
        if self.gap_scheduled || self.cached.state == RecordingState::Failed {
            return;
        }
        if !gap.valid() {
            self.fail("invalid recorder failure seal");
            return;
        }
        self.cached.coverage = "gap";
        self.cached.first_missing_fact = gap.first_missing_fact;
        self.fail(&gap.reason);
        let Ok(assigned) = self.planned_range(1) else {
            return;
        };
        if self
            .sender
            .try_send(Message::GapSeal(gap, *assigned.start()))
            .is_ok()
        {
            self.reserved_through = *assigned.end();
            self.gap_scheduled = true;
        }
    }
}

fn fact_charge(fact: &RecordingFact) -> usize {
    match fact {
        RecordingFact::Output { .. } => 512,
        RecordingFact::Controller { .. } | RecordingFact::Reference { .. } => 512,
        RecordingFact::Measurement { sample, .. } => {
            512 + match sample.value() {
                Some(Value::Text(value) | Value::Enum(value)) => value
                    .capacity()
                    .saturating_add(value.len().saturating_mul(6)),
                _ => 0,
            }
        }
    }
}

// Keep the immutable clock source and each bounded mailbox explicit at the one
// storage thread boundary; none is shared with the Runtime state owner.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit storage thread ownership"
)]
fn worker_loop(
    mut store: SqliteStore,
    receiver: mpsc::Receiver<Message>,
    receipt: &Arc<Mutex<Receipt>>,
    history: &Arc<Mutex<BTreeMap<u64, Result<HistoryPage, StorageError>>>>,
    runs: &Arc<Mutex<BTreeMap<u64, Result<RunsPage, StorageError>>>>,
    active_jobs: &Arc<Mutex<BTreeSet<u64>>>,
    barrier: Option<&WriterBarrier>,
    source: MonotonicSource,
) -> bool {
    let mut deferred: Option<Message> = None;
    loop {
        let message = if let Some(message) = deferred.take() {
            message
        } else {
            match receiver.try_recv() {
                Ok(message) => message,
                Err(mpsc::TryRecvError::Disconnected) => return false,
                Err(mpsc::TryRecvError::Empty) => {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(message) => message,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                    }
                }
            }
        };
        let message = match message {
            Message::History {
                job,
                filter,
                cursor,
                limit,
            } => {
                let result = store.read_history_measurements(&filter, cursor.as_ref(), limit);
                // Hold the short active-slot check through mailbox publication.
                // A cancellation cannot slip between the two and retain a late row.
                let active = active_jobs.lock().unwrap_or_else(|p| p.into_inner());
                if active.contains(&job) {
                    history
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(job, result);
                }
                continue;
            }
            Message::Runs { job, cursor, limit } => {
                let result = store.read_history_runs(cursor.as_ref(), limit);
                let active = active_jobs.lock().unwrap_or_else(|p| p.into_inner());
                if active.contains(&job) {
                    runs.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(job, result);
                }
                continue;
            }
            other => other,
        };
        if !matches!(
            message,
            Message::Finish(_, _, _) | Message::Activation(_, _)
        ) && (!matches!(message, Message::Start(_, _, _, _, _))
            || barrier.is_some_and(|barrier| barrier.0.hold_start))
            && !barrier.is_some_and(|barrier| barrier.0.hold_after_fact_commit)
            && (!barrier.is_some_and(|barrier| barrier.0.hold_terminal_operation)
                || matches!(message, Message::Operation(ref operation, _, _)
                    if operation.phase != "accepted"))
            && let Some(barrier) = barrier
        {
            barrier.await_release();
        }
        let result = match message {
            Message::Activation(entries, objects) => {
                store.commit_activation(&entries, &objects).map(|root| {
                    receipt
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .activation_root = Some(root);
                })
            }
            Message::Start(label, policy, submitted_at, boundary, first_record) => {
                TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    .and_then(|anchor| {
                        store.start_run_with_boundary_anchor_assigned(
                            &label,
                            policy,
                            &boundary,
                            &anchor,
                            first_record,
                        )
                    })
                    .and_then(|_| {
                        let health = store.storage_health()?;
                        let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                        status.state = RecordingState::Recording;
                        status.persisted = store.current_record_sequence();
                        status.confirmed_submission = Some(submitted_at);
                        status.run_no = store.current_run_no();
                        status.interval_no = store.current_interval_no();
                        status.storage = Some(health);
                        Ok(())
                    })
            }
            Message::Facts(facts, bytes, submitted_at, queued_at, first_record) => {
                if let Some(barrier) = barrier.filter(|barrier| barrier.0.panic_before_fact_sql) {
                    barrier.0.reached.store(true, Ordering::Release);
                    panic!("injected recorder worker panic before fact SQL");
                }
                let mut batch = vec![(facts, bytes, submitted_at, first_record)];
                let mut records = batch[0].0.len();
                let mut accounted_bytes = bytes;
                let deadline = queued_at + Duration::from_millis(100);
                while batch.len() < MAX_GROUPS && records < 256 && accounted_bytes < MAX_GROUP_BYTES
                {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match receiver.recv_timeout(remaining) {
                        Ok(Message::Facts(
                            next_facts,
                            next_bytes,
                            next_at,
                            next_queued,
                            next_first,
                        )) => {
                            if next_facts.len() > 256 - records
                                || next_bytes > MAX_GROUP_BYTES - accounted_bytes
                            {
                                deferred = Some(Message::Facts(
                                    next_facts,
                                    next_bytes,
                                    next_at,
                                    next_queued,
                                    next_first,
                                ));
                                break;
                            }
                            records += next_facts.len();
                            accounted_bytes += next_bytes;
                            batch.push((next_facts, next_bytes, next_at, next_first));
                        }
                        Ok(other) => {
                            deferred = Some(other);
                            break;
                        }
                        Err(
                            mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected,
                        ) => break,
                    }
                }
                let views: Vec<(&[RecordingFact], Duration)> = batch
                    .iter()
                    .map(|(facts, _, at, _)| (facts.as_slice(), *at))
                    .collect();
                let mut expected = batch[0].3;
                let aligned = batch.iter().all(|(facts, _, _, first)| {
                    if *first != expected {
                        return false;
                    }
                    let Some(next) = expected.checked_add(facts.len() as u64) else {
                        return false;
                    };
                    expected = next;
                    true
                });
                let last_submission = batch.last().expect("nonempty batch").2;
                let committed = if aligned {
                    store.append_fact_groups_assigned(&views, batch[0].3)
                } else {
                    Err(StorageError(
                        "noncontiguous owner record reservation".into(),
                    ))
                };
                committed.map(|sequence| {
                    if let Some(barrier) =
                        barrier.filter(|barrier| barrier.0.hold_after_fact_commit)
                    {
                        // The archive is already committed. A killed process
                        // can lose this receipt without losing the WAL record.
                        barrier.await_release();
                    }
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(last_submission);
                    status.released_records += records;
                    status.released_bytes += accounted_bytes;
                    status.released_groups += batch.len();
                })
            }
            Message::Operation(operation, bytes, assigned) => store
                .append_operation_assigned(&operation, assigned)
                .map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(operation.at);
                    status.released_records += 1;
                    status.released_bytes += bytes;
                    status.released_groups += 1;
                }),
            Message::Annotation(annotation, bytes, assigned) => store
                .append_annotation_assigned(&annotation, assigned)
                .map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(annotation.at);
                    status.released_records += 1;
                    status.released_bytes += bytes;
                    status.released_groups += 1;
                }),
            Message::GapSeal(gap, assigned) => {
                store.fail_run_assigned(&gap, assigned).map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.failure_persisted = true;
                    status.run_no = None;
                    status.interval_no = None;
                })
            }
            Message::Probe(submitted_at) => store.probe(submitted_at).map(|_| {
                receipt
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .confirmed_submission = Some(submitted_at);
            }),
            Message::ClockAnchor(assigned) => {
                TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    .and_then(|anchor| {
                        store.append_clock_anchor_assigned("periodic", &anchor, assigned)
                    })
                    .map(|_| {
                        receipt.lock().unwrap_or_else(|p| p.into_inner()).persisted =
                            store.current_record_sequence();
                    })
            }
            Message::History { .. } => unreachable!("history was handled before lifecycle match"),
            Message::Runs { .. } => unreachable!("runs were handled before lifecycle match"),
            Message::Stop(summary, requested_at, assigned) => {
                TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    .and_then(|anchor| {
                        store.stop_run_with_anchor_summary_assigned(
                            &anchor,
                            &summary,
                            requested_at,
                            assigned,
                        )
                    })
                    .map(|_| {
                        let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                        status.state = RecordingState::Idle;
                        status.persisted = store.current_record_sequence();
                        status.run_no = None;
                        status.interval_no = None;
                    })
            }
            Message::Finish(summary, at, assigned) => {
                if let Some(barrier) = barrier.filter(|barrier| barrier.0.hold_finish) {
                    barrier.await_release();
                }
                let anchor = TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()));
                let result = anchor.and_then(|anchor| {
                    store.finish_boot_with_summary_assigned(at, &summary, &anchor, assigned)
                });
                if let Err(error) = result {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.state = RecordingState::Failed;
                    status
                        .first_error
                        .get_or_insert_with(|| error.to_string().chars().take(512).collect());
                    return false;
                }
                {
                    receipt
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .terminal_seal_committed = true;
                }
                if let Err(error) = store.close() {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.state = RecordingState::Failed;
                    status
                        .first_error
                        .get_or_insert_with(|| error.to_string().chars().take(512).collect());
                    return false;
                }
                return true;
            }
        };
        let result = result.and_then(|()| {
            let health = store.storage_health()?;
            receipt
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .storage = Some(health);
            Ok(())
        });
        if let Err(error) = result {
            let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
            status.state = RecordingState::Failed;
            status
                .first_error
                .get_or_insert_with(|| error.to_string().chars().take(512).collect());
            return false;
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[test]
    fn contested_history_cancel_releases_slot_and_discards_late_result() {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("lab-m7-cancel-{suffix}.sqlite"));
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        let job = worker.request_runs(None, 1).unwrap();
        let slots = Arc::clone(&worker.active_history_jobs);
        let guard = slots.lock().unwrap();
        worker.cancel_history(job);
        drop(guard);
        worker.request_finish().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while worker.poll().state != RecordingState::Closed && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(worker.poll().state, RecordingState::Closed);
        assert_eq!(
            slots.lock().unwrap().len(),
            0,
            "cancellation cannot leak a slot"
        );
        assert!(
            worker.try_take_runs(job).is_none(),
            "canceled result cannot survive"
        );
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn eight_deferred_cancellations_never_expand_history_job_credit() {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("lab-m7-eight-cancel-{suffix}.sqlite"));
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        let jobs: Vec<_> = (0..MAX_HISTORY_JOBS)
            .map(|_| worker.request_runs(None, 1).unwrap())
            .collect();
        assert!(worker.request_runs(None, 1).is_err());
        let slots = Arc::clone(&worker.active_history_jobs);
        let guard = slots.lock().unwrap();
        for job in &jobs {
            worker.cancel_history(*job);
        }
        assert_eq!(worker.pending_cancellations.len(), MAX_HISTORY_JOBS);
        assert!(worker.request_runs(None, 1).is_err());
        drop(guard);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.pending_cancellations.is_empty() && Instant::now() < deadline {
            worker.poll();
            thread::yield_now();
        }
        assert!(worker.pending_cancellations.is_empty());
        assert!(slots.lock().unwrap().is_empty());
        let replacement = worker.request_runs(None, 1).unwrap();
        worker.cancel_history(replacement);
        worker.request_finish().unwrap();
        while worker.poll().state != RecordingState::Closed && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(worker.poll().state, RecordingState::Closed);
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn old_completed_job_ids_cannot_displace_a_live_cancellation() {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("lab-m7-old-cancel-{suffix}.sqlite"));
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut old_jobs = Vec::new();
        for _ in 0..MAX_HISTORY_JOBS {
            let job = worker.request_runs(None, 1).unwrap();
            while worker.try_take_runs(job).is_none() && Instant::now() < deadline {
                thread::yield_now();
            }
            old_jobs.push(job);
        }
        let live = worker.request_runs(None, 1).unwrap();
        let slots = Arc::clone(&worker.active_history_jobs);
        let guard = slots.lock().unwrap();
        for job in old_jobs {
            worker.cancel_history(job);
        }
        worker.cancel_history(live);
        drop(guard);
        worker.request_finish().unwrap();
        while worker.poll().state != RecordingState::Closed && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(worker.poll().state, RecordingState::Closed);
        assert!(slots.lock().unwrap().is_empty());
        assert!(worker.try_take_runs(live).is_none());
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod receipt_fence_tests {
    use super::*;

    fn temporary_database() -> std::path::PathBuf {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let name: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::temp_dir().join(format!("lab-m7-receipt-{name}.sqlite"))
    }

    fn remove_after_drop(path: &Path) {
        let by = Instant::now() + Duration::from_secs(2);
        while std::fs::remove_file(path).is_err() && Instant::now() < by {
            thread::yield_now();
        }
        assert!(!path.exists());
    }

    #[test]
    fn future_persisted_identity_cannot_confirm_an_owner_unreserved_record() {
        let path = temporary_database();
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        assert_eq!(worker.reserved_through, 0);
        worker.apply_receipt(Receipt {
            persisted: 1,
            ..Receipt::default()
        });
        assert_eq!(worker.cached.state, RecordingState::Failed);
        assert_eq!(worker.cached.persisted_through_sequence, 0);
        assert!(
            worker
                .cached
                .first_error
                .as_deref()
                .unwrap()
                .contains("future")
        );
        drop(worker);
        remove_after_drop(&path);
    }

    #[test]
    fn a_receipt_cannot_confirm_monotonic_time_never_submitted_by_owner() {
        let path = temporary_database();
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        worker
            .request_start_at("receipt time", Duration::ZERO)
            .unwrap();
        let by = Instant::now() + Duration::from_secs(2);
        while worker.poll().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            thread::yield_now();
        }
        let committed = worker.cached.persisted_through_sequence;
        worker.apply_receipt(Receipt {
            state: RecordingState::Recording,
            persisted: committed,
            confirmed_submission: Some(Duration::from_secs(1)),
            run_no: Some(1),
            interval_no: Some(1),
            ..Receipt::default()
        });
        assert_eq!(worker.cached.state, RecordingState::Failed);
        assert_eq!(worker.cached.persisted_through_sequence, committed);
        assert!(
            worker
                .cached
                .first_error
                .as_deref()
                .unwrap()
                .contains("future")
        );
        drop(worker);
        remove_after_drop(&path);
    }

    #[test]
    fn stale_probe_clone_cannot_roll_back_confirmed_required_progress() {
        let path = temporary_database();
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        worker
            .request_start_at("stale probe", Duration::ZERO)
            .unwrap();
        let by = Instant::now() + Duration::from_secs(2);
        while worker.poll().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            thread::yield_now();
        }
        let start_id = worker.poll().persisted_through_sequence;
        worker.request_probe_at(Duration::from_secs(1)).unwrap();
        while worker.poll().confirmed_submission != Some(Duration::from_secs(1)) {
            assert!(Instant::now() < by);
            thread::yield_now();
        }
        worker.apply_receipt(Receipt {
            state: RecordingState::Recording,
            persisted: start_id,
            confirmed_submission: Some(Duration::ZERO),
            run_no: Some(1),
            interval_no: Some(1),
            ..Receipt::default()
        });
        assert_eq!(worker.cached.state, RecordingState::Recording);
        assert_eq!(
            worker.cached.confirmed_submission,
            Some(Duration::from_secs(1))
        );
        assert_eq!(worker.cached.persisted_through_sequence, start_id);
        drop(worker);
        remove_after_drop(&path);
    }
}

#[cfg(test)]
mod close_receipt_tests {
    use super::*;

    #[test]
    fn closed_worker_rechecks_final_receipt_after_a_stale_owner_clone() {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("lab-m7-close-race-{suffix}.sqlite"));
        let mut worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
        worker.request_finish().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while worker.poll().state != RecordingState::Closed && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(worker.poll().state, RecordingState::Closed);
        let stale = Receipt {
            state: RecordingState::Idle,
            terminal_seal_committed: true,
            ..Receipt::default()
        };
        worker.cached.state = RecordingState::Stopping;
        let reconciled = worker.reconcile_receipt(Some(stale));
        assert_eq!(reconciled.state, RecordingState::Closed);
        assert!(reconciled.terminal_seal_committed);
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }
}
