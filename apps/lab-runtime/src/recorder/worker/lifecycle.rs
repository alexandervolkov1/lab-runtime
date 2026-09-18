//! Recorder lifecycle, activation, durable receipts, sealing, and failure state.

use super::*;

impl RecorderWorker {
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
        self.send_control(Message::Activation {
            entries,
            objects,
            lifecycle: None,
        })?;
        self.pending_activation_generation = Some(1);
        Ok(())
    }

    /// Reserve one of the unchanged four ordinary ingress groups before a
    /// configuration owner commit. `None` is bounded backpressure, not failure.
    pub fn try_reserve_live_activation(
        &mut self,
        lifecycle: &ConfigurationLifecycleRecord,
    ) -> Result<Option<u64>, StorageError> {
        let status = self.poll();
        if !matches!(
            status.state,
            RecordingState::Idle | RecordingState::Recording
        ) || status.activation_generation == 0
            || self.pending_activation_generation.is_some()
            || self.live_activation_reservation.is_some()
            || !lifecycle.valid()
        {
            return Err(StorageError("live activation reservation invalid".into()));
        }
        let bytes = lifecycle
            .charge()
            .ok_or_else(|| StorageError("live activation credit arithmetic exhausted".into()))?;
        if self.charged_groups >= self.limits.groups
            || self.charged_records >= self.limits.records
            || bytes > self.limits.bytes.saturating_sub(self.charged_bytes)
        {
            return Ok(None);
        }
        let generation = status
            .activation_generation
            .checked_add(1)
            .ok_or_else(|| StorageError("activation generation exhausted".into()))?;
        self.charged_records += 1;
        self.charged_bytes += bytes;
        self.charged_groups += 1;
        self.pending_activation_generation = Some(generation);
        self.live_activation_reservation = Some(LiveActivationReservation { generation, bytes });
        self.cached.outstanding_records = self.charged_records;
        self.cached.outstanding_bytes = self.charged_bytes;
        self.cached.outstanding_groups = self.charged_groups;
        Ok(Some(generation))
    }

    /// Assign and immediately enqueue the lifecycle identity after owner commit.
    ///
    /// Capacity, bytes and generation were reserved before side effects, but no
    /// record identity was. Ordinary facts may therefore remain FIFO-contiguous
    /// while a resource-scoped compatibility probe is in progress.
    pub fn commit_reserved_live_activation(
        &mut self,
        generation: u64,
        entries: Vec<ProvenanceEntry>,
        objects: Vec<ProvenanceObject>,
        lifecycle: ConfigurationLifecycleRecord,
    ) -> Result<(), StorageError> {
        let reservation = self
            .live_activation_reservation
            .as_ref()
            .ok_or_else(|| StorageError("live activation not reserved".into()))?;
        let charge = lifecycle
            .charge()
            .ok_or_else(|| StorageError("live activation credit arithmetic exhausted".into()))?;
        if generation != reservation.generation || charge > reservation.bytes || !lifecycle.valid()
        {
            return Err(StorageError("live activation reservation mismatch".into()));
        }
        let reserved_bytes = reservation.bytes;
        let reserved_generation = reservation.generation;
        let assigned = *self.planned_range(1)?.start();
        let submitted_at = lifecycle.at;
        let message = Message::Activation {
            entries,
            objects,
            lifecycle: Some((lifecycle, reserved_bytes, assigned, reserved_generation)),
        };
        match self.sender.try_send(message) {
            Ok(()) => {
                // Runtime-owner mutation is serialized. Advancing the tail only
                // after this exact message enters the single FIFO cannot leave a
                // future identity hole or interleave another owner admission.
                self.reserved_through = assigned;
                self.last_owner_submission = Some(
                    self.last_owner_submission
                        .map_or(submitted_at, |latest| latest.max(submitted_at)),
                );
                self.live_activation_reservation = None;
                Ok(())
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.fail("recorder lifecycle slot unavailable");
                Err(StorageError("recorder lifecycle slot unavailable".into()))
            }
        }
    }

    /// Release capacity after a precommit owner rejection without editing IDs.
    pub fn cancel_live_activation_reservation(
        &mut self,
        generation: u64,
    ) -> Result<(), StorageError> {
        let reservation = self
            .live_activation_reservation
            .take()
            .ok_or_else(|| StorageError("live activation not reserved".into()))?;
        if reservation.generation != generation {
            self.live_activation_reservation = Some(reservation);
            return Err(StorageError("live activation reservation mismatch".into()));
        }
        self.charged_records = self.charged_records.saturating_sub(1);
        self.charged_bytes = self.charged_bytes.saturating_sub(reservation.bytes);
        self.charged_groups = self.charged_groups.saturating_sub(1);
        self.pending_activation_generation = None;
        self.cached.outstanding_records = self.charged_records;
        self.cached.outstanding_bytes = self.charged_bytes;
        self.cached.outstanding_groups = self.charged_groups;
        Ok(())
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

    pub(super) fn reconcile_receipt(&mut self, fresh_receipt: Option<Receipt>) -> RecordingStatus {
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

    pub(super) fn apply_receipt(&mut self, receipt: Receipt) {
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
                self.cached.activation_generation = receipt.activation_generation;
                if self
                    .pending_activation_generation
                    .is_some_and(|generation| receipt.activation_generation >= generation)
                {
                    self.pending_activation_generation = None;
                }
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

    pub(super) fn planned_range(&self, count: usize) -> Result<RangeInclusive<u64>, StorageError> {
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
