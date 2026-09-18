//! One storage thread, finite ingress credit, and coalesced durable receipts.
//!
//! Only the storage worker can block on SQL. The owner uses try-send and brief
//! receipt inspection; shutdown requests never join a worker stuck in OS I/O.
//! [`RecorderWorker`] is the host-side boundary between semantic fact groups and
//! [`super::SqliteStore`]: ingress credit is charged on the owner lane, while one
//! worker exclusively owns SQLite writes, history reads, provenance, gaps and
//! sealing. `WriterBarrier` is a trusted fault-injection seam, not production
//! scheduling or experiment authority.
//!
//! The owning struct and channel protocol stay here. Focused implementations live
//! in `lifecycle` (state and receipts), `ingress` (finite credit), `history`
//! (bounded query jobs), and `storage_loop` (exclusive SQLite command execution).
//! Test-only blocking and storage-failure controls are isolated in
//! `fault_injection`.

use super::{
    AnnotationRecord, BoundarySnapshot, ConfigurationLifecycleRecord, HistoryCursor, HistoryFilter,
    HistoryPage, OperationRecord, ProvenanceEntry, ProvenanceObject, RecorderGap, RecordingPolicy,
    RunsCursor, RunsPage, SqliteStore, StorageError, StorageHealth, TimeAnchor,
};
use crate::host::{Clock, SystemClock};
use lab_core::{Value, recording::RecordingFact};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::RangeInclusive,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

mod fault_injection;
mod history;
mod ingress;
mod lifecycle;
mod storage_loop;

pub use fault_injection::WriterBarrier;
use storage_loop::worker_loop;

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
    /// Checked count of durable activation baselines committed in this boot.
    pub activation_generation: u64,
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
    activation_generation: u64,
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
            activation_generation: 0,
            failure_persisted: false,
            storage: None,
        }
    }
}

enum Message {
    Activation {
        entries: Vec<ProvenanceEntry>,
        objects: Vec<ProvenanceObject>,
        lifecycle: Option<(ConfigurationLifecycleRecord, usize, u64, u64)>,
    },
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

struct LiveActivationReservation {
    generation: u64,
    bytes: usize,
}

/// Host-owned bounded ingress and lifecycle handle for one SQLite worker.
///
/// No method executes disk I/O on the owner lane. Admission reserves record, byte
/// and causal-group credit before a nonblocking send; a durable worker receipt is
/// the only normal release of that credit. The storage thread exclusively owns
/// [`SqliteStore`], may block on SQLite, and reports bounded receipts/history pages
/// back to this handle. Dropping the handle never converts an unfinished write into
/// durability evidence and never waits indefinitely for an OS call.
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
    pending_activation_generation: Option<u64>,
    live_activation_reservation: Option<LiveActivationReservation>,
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
                    .and_then(|mut store| {
                        if let Some(threshold) =
                            barrier.as_ref().and_then(|fault| fault.0.low_wal_threshold)
                        {
                            store.lower_wal_threshold_for_testing(threshold)?;
                        }
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
                activation_generation: 0,
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
            pending_activation_generation: None,
            live_activation_reservation: None,
            last_owner_submission: None,
            last_accepted_fact: None,
            gap_scheduled: false,
            finish_requested: false,
        })
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    fn request_runs_after_transient_lock(worker: &mut RecorderWorker, deadline: Instant) -> u64 {
        loop {
            match worker.request_runs(None, 1) {
                Ok(job) => return job,
                Err(error) if error.0 == "history slots busy" => {
                    assert!(Instant::now() < deadline, "history slot lock stayed busy");
                    thread::yield_now();
                }
                Err(error) => panic!("unexpected history admission failure: {error}"),
            }
        }
    }

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
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut jobs = Vec::new();
        while jobs.len() < MAX_HISTORY_JOBS {
            jobs.push(request_runs_after_transient_lock(&mut worker, deadline));
        }
        assert!(worker.request_runs(None, 1).is_err());
        let slots = Arc::clone(&worker.active_history_jobs);
        let guard = slots.lock().unwrap();
        for job in &jobs {
            worker.cancel_history(*job);
        }
        assert_eq!(worker.pending_cancellations.len(), MAX_HISTORY_JOBS);
        assert!(worker.request_runs(None, 1).is_err());
        drop(guard);
        while !worker.pending_cancellations.is_empty() && Instant::now() < deadline {
            worker.poll();
            thread::yield_now();
        }
        assert!(worker.pending_cancellations.is_empty());
        assert!(slots.lock().unwrap().is_empty());
        let replacement =
            request_runs_after_transient_lock(&mut worker, Instant::now() + Duration::from_secs(2));
        worker.cancel_history(replacement);
        worker.request_finish().unwrap();
        let close_deadline = Instant::now() + Duration::from_secs(2);
        while worker.poll().state != RecordingState::Closed && Instant::now() < close_deadline {
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
