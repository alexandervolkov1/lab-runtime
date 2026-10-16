//! Bounded durable-history job admission, results, and cancellation.

use super::*;

impl RecorderWorker {
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

    pub(super) fn drain_history_cancellations(&mut self) {
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
}
