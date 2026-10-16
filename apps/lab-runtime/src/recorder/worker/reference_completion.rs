//! Atomic pre-admission credit for a synchronous Reference intent and completion.
use super::*;
const COMPLETION_RECORDS: usize = 2;
// A compact <=16 KiB terminal payload, bounded scope/metadata, one inline
// Reference fact (no heap payload), and encoding/owner-token scratch. Accepted
// and terminal allocations are compacted before transfer; surplus capacity may
// not silently escape this charge. At most one untransferred token exists.
const COMPLETION_BYTES: usize = 16 * 1024 + 4 * 64 + 1024 + std::mem::size_of::<RecordingFact>();
const ACCEPTED_BYTES: usize = 16 * 1024 + 64 + 256;
// Two independent clients may each retain accepted and completion evidence.
pub(super) const PROTECTED: Credit = Credit {
    groups: 4,
    records: 6,
    bytes: 2 * (ACCEPTED_BYTES + COMPLETION_BYTES),
};
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Credit {
    pub groups: usize,
    pub records: usize,
    pub bytes: usize,
}
impl Credit {
    pub fn contains(self, other: Self) -> bool {
        other.groups <= self.groups && other.records <= self.records && other.bytes <= self.bytes
    }
    pub fn delta(self, earlier: Self) -> Option<Self> {
        Some(Self {
            groups: self.groups.checked_sub(earlier.groups)?,
            records: self.records.checked_sub(earlier.records)?,
            bytes: self.bytes.checked_sub(earlier.bytes)?,
        })
    }
    pub fn add(&mut self, credit: Self) {
        self.groups += credit.groups;
        self.records += credit.records;
        self.bytes += credit.bytes;
    }
}
pub(super) struct Reservation {
    scope: String,
    seq: u64,
    command: &'static str,
    accepted: Option<OperationRecord>,
    accepted_bytes: usize,
    generation: u64,
    run: Option<u64>,
    interval: Option<u64>,
    fact: Option<RecordingFact>,
    captured_at: Duration,
}
impl RecorderWorker {
    // Smaller trusted fault profiles preserve their deliberately shared budget.
    // Production protects Reference headroom in all three resource dimensions.
    pub(super) fn ordinary_capacity_available(&self, records: usize, bytes: usize) -> bool {
        let protected = if self.limits == RecorderLimits::default() {
            PROTECTED
                .delta(self.reference_credit)
                .expect("bounded Reference credit")
        } else {
            Credit::default()
        };
        self.charged_groups < self.limits.groups.saturating_sub(protected.groups)
            && records
                <= self
                    .limits
                    .records
                    .saturating_sub(protected.records)
                    .saturating_sub(self.charged_records)
            && bytes
                <= self
                    .limits
                    .bytes
                    .saturating_sub(protected.bytes)
                    .saturating_sub(self.charged_bytes)
    }
    pub(crate) fn reserve_reference_operation(
        &mut self,
        mut accepted: OperationRecord,
    ) -> Result<bool, StorageError> {
        self.poll();
        if self.reference_completion.is_some()
            || !matches!(accepted.command, "reference_retune" | "reference_configure")
            || accepted.phase != "accepted"
            || accepted.outcome_basis != "application_admission"
            || !accepted.valid()
            || self.cached.state != RecordingState::Recording
        {
            return Err(StorageError(
                "Reference admission reservation invalid".into(),
            ));
        }
        if self.pending_activation_generation.is_some() {
            return Ok(false);
        }
        accepted.scope = accepted.scope.into_boxed_str().into_string();
        accepted.data = accepted.data.into_boxed_str().into_string();
        let accepted_bytes = accepted
            .charge()
            .ok_or_else(|| StorageError("Reference credit arithmetic exhausted".into()))?;
        let credit = Credit {
            groups: 2,
            records: 3,
            bytes: accepted_bytes + COMPLETION_BYTES,
        };
        if credit.groups > self.limits.groups.saturating_sub(self.charged_groups)
            || credit.records > self.limits.records.saturating_sub(self.charged_records)
            || credit.bytes > self.limits.bytes.saturating_sub(self.charged_bytes)
            || (self.limits == RecorderLimits::default()
                && !PROTECTED
                    .delta(self.reference_credit)
                    .is_some_and(|free| free.contains(credit)))
        {
            return Ok(false);
        }
        // One atomic owner-local charge; no FIFO IDs or SessionStore mutation.
        self.charged_groups += credit.groups;
        self.charged_records += credit.records;
        self.charged_bytes += credit.bytes;
        self.reference_credit.add(credit);
        self.reference_completion = Some(Reservation {
            scope: accepted.scope.clone(),
            seq: accepted.request_seq,
            command: accepted.command,
            accepted_bytes,
            generation: self.cached.activation_generation,
            run: self.cached.run_no,
            interval: self.cached.interval_no,
            fact: None,
            captured_at: accepted.at,
            accepted: Some(accepted),
        });
        Ok(true)
    }
    pub(crate) fn has_reference_completion(&self) -> bool {
        self.reference_completion.is_some()
    }
    pub(super) fn submitted_reference_credit(&self) -> Credit {
        let unused = self
            .reference_completion
            .as_ref()
            .map_or(Credit::default(), |r| {
                let accepted = usize::from(r.accepted.is_some());
                Credit {
                    groups: 1 + accepted,
                    records: COMPLETION_RECORDS + accepted,
                    bytes: COMPLETION_BYTES + accepted * r.accepted_bytes,
                }
            });
        self.reference_credit
            .delta(unused)
            .expect("owned Reference credit")
    }
    fn reference_generation_current(&self, r: &Reservation) -> bool {
        self.cached.state == RecordingState::Recording
            && self.pending_activation_generation.is_none()
            && self.cached.activation_generation == r.generation
            && self.cached.run_no == r.run
            && self.cached.interval_no == r.interval
    }
    pub(crate) fn reference_completion_ready(
        &mut self,
        scope: &str,
        seq: u64,
        command: &'static str,
    ) -> bool {
        self.poll();
        self.reference_completion.as_ref().is_some_and(|r| {
            r.scope == scope
                && r.seq == seq
                && r.command == command
                && r.accepted.is_none()
                && self.reference_generation_current(r)
        })
    }
    // Only never-transferred capacity is cancellable. SQL failure cannot release
    // submitted credit; the cumulative post-commit receipt remains its authority.
    pub(crate) fn cancel_reference_reservation(&mut self) {
        if let Some(r) = self.reference_completion.take() {
            let accepted = usize::from(r.accepted.is_some());
            self.release_unused_reference(Credit {
                groups: 1 + accepted,
                records: COMPLETION_RECORDS + accepted,
                bytes: COMPLETION_BYTES + accepted * r.accepted_bytes,
            });
        }
    }
    fn release_unused_reference(&mut self, credit: Credit) {
        self.reference_credit = self
            .reference_credit
            .delta(credit)
            .expect("owned Reference reservation");
        self.charged_groups -= credit.groups;
        self.charged_records -= credit.records;
        self.charged_bytes -= credit.bytes;
    }
    pub(crate) fn commit_reference_acceptance(
        &mut self,
        operation: OperationRecord,
    ) -> Result<(), StorageError> {
        self.poll();
        let valid = self.reference_completion.as_ref().is_some_and(|r| {
            self.reference_generation_current(r)
                && r.accepted.as_ref().is_some_and(|a| {
                    a.scope == operation.scope
                        && a.request_seq == operation.request_seq
                        && a.command == operation.command
                        && a.phase == operation.phase
                        && a.data == operation.data
                        && a.at == operation.at
                        && a.outcome_basis == operation.outcome_basis
                })
        });
        if !valid {
            self.cancel_reference_reservation();
            self.fail("Reference acceptance identity or generation mismatch");
            return Err(StorageError("Reference acceptance unavailable".into()));
        }
        let assigned = match self.planned_range(1) {
            Ok(range) => *range.start(),
            Err(error) => {
                self.cancel_reference_reservation();
                return Err(error);
            }
        };
        let r = self.reference_completion.as_mut().unwrap();
        let accepted = r.accepted.take().unwrap();
        let at = accepted.at;
        let bytes = r.accepted_bytes;
        match self
            .sender
            .try_send(Message::ReferenceAccepted(accepted, bytes, assigned))
        {
            Ok(()) => {
                self.reserved_through = assigned;
                self.last_owner_submission =
                    Some(self.last_owner_submission.map_or(at, |old| old.max(at)));
                Ok(())
            }
            Err(_) => {
                self.release_unused_reference(Credit {
                    groups: 1,
                    records: 1,
                    bytes,
                });
                self.cancel_reference_reservation();
                self.fail("Reference acceptance FIFO unavailable");
                Err(StorageError("Reference acceptance FIFO unavailable".into()))
            }
        }
    }
    pub(crate) fn capture_reference_completion(
        &mut self,
        mut facts: Vec<RecordingFact>,
        at: Duration,
    ) -> Result<(), StorageError> {
        let r = self
            .reference_completion
            .as_mut()
            .ok_or_else(|| StorageError("Reference completion not reserved".into()))?;
        if facts.is_empty() {
            return Ok(());
        }
        if facts.len() != 1
            || r.fact.is_some()
            || !matches!(facts[0], RecordingFact::Reference { .. })
        {
            self.fail_with_gap(RecorderGap {
                reason: "Reference completion fact bound violated".into(),
                at,
                first_missing_fact: facts.first().map(RecordingFact::sequence),
                known_missing_count: u64::try_from(facts.len()).ok(),
                last_accepted_fact: self.last_accepted_fact,
            });
            return Err(StorageError(
                "Reference completion fact bound violated".into(),
            ));
        }
        r.fact = facts.pop();
        r.captured_at = at;
        Ok(())
    }
    pub(crate) fn commit_reference_completion(
        &mut self,
        mut terminal: OperationRecord,
    ) -> Result<(), StorageError> {
        self.poll();
        let valid = self.reference_completion.as_ref().is_some_and(|r| {
            self.reference_generation_current(r)
                && r.accepted.is_none()
                && terminal.scope == r.scope
                && terminal.request_seq == r.seq
                && terminal.command == r.command
                && terminal.valid()
                && matches!(terminal.phase, "completed" | "failed")
                && terminal.outcome_basis == "domain_result"
        });
        if !valid {
            self.cancel_reference_reservation();
            self.fail("Reference completion identity, generation or storage failure");
            return Err(StorageError("Reference completion unavailable".into()));
        }
        terminal.scope = terminal.scope.into_boxed_str().into_string();
        terminal.data = terminal.data.into_boxed_str().into_string();
        let r = self.reference_completion.as_ref().unwrap();
        let records = 1 + usize::from(r.fact.is_some());
        let assigned = match self.planned_range(records) {
            Ok(range) => range,
            Err(error) => {
                self.cancel_reference_reservation();
                return Err(error);
            }
        };
        let r = self.reference_completion.take().unwrap();
        let submitted_at = terminal.at.max(r.captured_at);
        let last_fact = r.fact.as_ref().map(RecordingFact::sequence);
        let message = Message::ReferenceCompletion {
            fact: r.fact.map(Box::new),
            captured_at: r.captured_at,
            terminal,
            bytes: COMPLETION_BYTES,
            first_record: *assigned.start(),
        };
        match self.sender.try_send(message) {
            Ok(()) => {
                self.reserved_through = *assigned.end();
                self.last_owner_submission = Some(
                    self.last_owner_submission
                        .map_or(submitted_at, |old| old.max(submitted_at)),
                );
                self.release_unused_reference(Credit {
                    records: COMPLETION_RECORDS - records,
                    ..Credit::default()
                });
                if last_fact.is_some() {
                    self.last_accepted_fact = last_fact;
                }
                Ok(())
            }
            Err(_) => {
                self.release_unused_reference(Credit {
                    groups: 1,
                    records: COMPLETION_RECORDS,
                    bytes: COMPLETION_BYTES,
                });
                self.fail_with_gap(RecorderGap {
                    reason: "Reference completion FIFO unavailable".into(),
                    at: submitted_at,
                    first_missing_fact: last_fact,
                    known_missing_count: Some(records as u64),
                    last_accepted_fact: self.last_accepted_fact,
                });
                Err(StorageError("Reference completion FIFO unavailable".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Release(WriterBarrier);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    fn wait(worker: &mut RecorderWorker, predicate: impl Fn(&RecordingStatus) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = worker.poll();
            if predicate(&status) {
                return;
            }
            assert!(Instant::now() < deadline, "{status:?}");
            thread::yield_now();
        }
    }
    fn fixture() -> (RecorderWorker, Release) {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let id: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("reference-credit-{id}.sqlite"));
        let barrier = WriterBarrier::held();
        let mut worker =
            RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
                .unwrap();
        worker.set_periodic_time_for_test(Duration::ZERO);
        worker
            .request_activation(
                vec![ProvenanceEntry {
                    kind: "runtime_toml".into(),
                    encoding: "toml_utf8_v1".into(),
                    content: b"old=true".to_vec(),
                }],
                vec![],
            )
            .unwrap();
        wait(&mut worker, |s| s.activation_generation == 1);
        worker.request_start_at("credit", Duration::ZERO).unwrap();
        wait(&mut worker, |s| s.state == RecordingState::Recording);
        (worker, Release(barrier))
    }
    fn accepted() -> OperationRecord {
        OperationRecord {
            scope: "scope".into(),
            request_seq: 1,
            command: "reference_retune",
            phase: "accepted",
            data: "{}".into(),
            outcome_basis: "application_admission",
            at: Duration::from_millis(1),
        }
    }
    #[test]
    fn unused_atomic_reservation_cancels_once_without_ids_and_shutdown_drains() {
        let (mut worker, release) = fixture();
        let before = worker.poll();
        assert!(worker.reserve_reference_operation(accepted()).unwrap());
        assert_eq!(worker.charged_groups, 2);
        assert_eq!(worker.charged_records, 3);
        assert_eq!(worker.reserved_through, before.persisted_through_sequence);
        assert!(worker.request_stop().is_err());
        worker.cancel_reference_reservation();
        worker.cancel_reference_reservation();
        assert_eq!(worker.charged_groups, 0);
        assert_eq!(worker.charged_records, 0);
        assert_eq!(worker.charged_bytes, 0);
        assert_eq!(worker.reference_credit.groups, 0);
        worker.request_stop().unwrap();
        release.0.release();
        wait(&mut worker, |s| s.state == RecordingState::Idle);
        worker.request_finish().unwrap();
        wait(&mut worker, |s| s.worker_closed);
        assert!(worker.poll().terminal_seal_committed);
    }
    #[test]
    fn generation_fence_blocks_admission_and_stale_reservation_cannot_publish_acceptance() {
        let (mut worker, release) = fixture();
        worker.pending_activation_generation = Some(2);
        assert!(!worker.reserve_reference_operation(accepted()).unwrap());
        assert_eq!(worker.charged_groups, 0);
        worker.pending_activation_generation = None;
        assert!(worker.reserve_reference_operation(accepted()).unwrap());
        worker.reference_completion.as_mut().unwrap().generation += 1;
        assert!(worker.commit_reference_acceptance(accepted()).is_err());
        assert_eq!(worker.charged_groups, 0);
        assert_eq!(worker.reference_credit.groups, 0);
        assert_eq!(worker.reserved_through, 2);
        assert_eq!(worker.poll().state, RecordingState::Failed);
        release.0.release();
        worker.request_finish().unwrap();
        wait(&mut worker, |s| s.worker_closed);
    }
    #[test]
    fn reference_receipt_cannot_release_untransferred_or_extra_credit() {
        let (mut worker, release) = fixture();
        assert!(worker.reserve_reference_operation(accepted()).unwrap());
        let mut receipt = worker.receipt.lock().unwrap().clone();
        receipt.reference_released = Credit {
            groups: 1,
            records: 1,
            bytes: 1,
        };
        // Even internally consistent cumulative totals may not release a token
        // which has never entered the FIFO or received a record identity.
        receipt.released_groups = 1;
        receipt.released_records = 1;
        receipt.released_bytes = 1;
        worker.apply_receipt(receipt);
        assert_eq!(worker.cached.state, RecordingState::Failed);
        assert_eq!(worker.charged_groups, 2);
        assert_eq!(worker.reference_credit.groups, 2);
        assert_eq!(worker.cached.persisted_through_sequence, 2);
        worker.cancel_reference_reservation();
        release.0.release();
        worker.request_finish().unwrap();
        wait(&mut worker, |s| s.worker_closed);
    }

    #[test]
    fn full_acquisition_rebind_probe_and_two_reference_budget_drains_exactly() {
        use lab_core::{InstrumentId, Sample, SignalId, Unit, reference::ReferenceId};
        let (mut worker, release) = fixture();
        let measurements = |first: u64| -> Vec<RecordingFact> {
            (first..first + 256)
                .map(|sequence| RecordingFact::Measurement {
                    sequence,
                    sample: Sample::validated_good(
                        SignalId::new(InstrumentId::new(7), lab_core::TEMPERATURE),
                        Unit::CELSIUS,
                        Duration::from_millis(sequence),
                        Value::Float(20.0),
                    )
                    .unwrap(),
                    generation: 2,
                    revision: 3,
                    state_revision: None,
                    lineage: None,
                })
                .collect()
        };
        for first in [1, 257, 513, 769] {
            worker
                .try_admit_at(measurements(first), Duration::from_millis(first))
                .unwrap();
        }
        for seq in 1..=2 {
            let mut audit = accepted();
            audit.request_seq = seq;
            assert!(worker.reserve_reference_operation(audit.clone()).unwrap());
            worker.commit_reference_acceptance(audit.clone()).unwrap();
            worker
                .capture_reference_completion(
                    vec![RecordingFact::Reference {
                        sequence: 1024 + seq,
                        reference: ReferenceId::new(1),
                        revision: seq,
                        value: 20.0,
                        target: Some(21.0),
                        rate: Some(1.0),
                        unit: Unit::CELSIUS,
                        at: Duration::from_millis(1024 + seq),
                    }],
                    Duration::from_millis(1024 + seq),
                )
                .unwrap();
            audit.phase = "completed";
            audit.outcome_basis = "domain_result";
            worker.commit_reference_completion(audit).unwrap();
        }
        // The two full envelopes remain charged before any replacement/probe
        // fact exists. Their conversion must not borrow Reference headroom.
        let baseline = worker.reserve_fact_group().unwrap().unwrap();
        let probe = worker.reserve_fact_group().unwrap().unwrap();
        for phase in ["accepted", "completed"] {
            let mut audit = accepted();
            audit.command = "reconnect_resource";
            audit.data.reserve(MAX_GROUP_BYTES);
            audit.phase = phase;
            if phase == "completed" {
                audit.outcome_basis = "domain_result";
            }
            worker.try_admit_operation(audit).unwrap();
        }
        let lifecycle = ConfigurationLifecycleRecord {
            operation_id: 1,
            operation_kind: "reconnect_resource",
            base_revision: 1,
            committed_revision: 1,
            toml_hash: [0x5a; 32],
            affected: vec!["resource:7:binding_generation:2".into()],
            reason: None,
            at: Duration::from_millis(1600),
        };
        let generation = worker
            .try_reserve_live_activation(&lifecycle)
            .unwrap()
            .unwrap();
        let held = worker.poll();
        assert_eq!(held.outstanding_groups, 13);
        assert_eq!(held.outstanding_records, 1545);
        assert_eq!(held.persisted_through_sequence, 2);
        assert_eq!(held.activation_generation, 1);
        // Worst-case allocation envelopes, including each full 512 KiB fact
        // group and 64 KiB lifecycle, fit without increasing the 4 MiB cap.
        let worst_bytes = 6 * MAX_GROUP_BYTES + 64 * 1024 + 2 * ACCEPTED_BYTES + PROTECTED.bytes;
        assert!(worst_bytes <= held.limits.bytes);
        assert!(held.outstanding_bytes <= worst_bytes);
        assert!(worker.reserve_fact_group().unwrap().is_none());
        assert_eq!(worker.poll().state, RecordingState::Recording);
        worker
            .admit_reserved_facts(baseline, measurements(1027), Duration::from_millis(1282))
            .unwrap();
        worker
            .admit_reserved_facts(probe, measurements(1283), Duration::from_millis(1538))
            .unwrap();
        worker
            .commit_reserved_live_activation(
                generation,
                vec![ProvenanceEntry {
                    kind: "runtime_toml".into(),
                    encoding: "toml_utf8_v1".into(),
                    content: b"new=true".to_vec(),
                }],
                vec![],
                lifecycle,
            )
            .unwrap();
        assert_eq!(worker.poll().outstanding_records, 1545);
        release.0.release();
        wait(&mut worker, |s| s.outstanding_groups == 0);
        let drained = worker.poll();
        assert_eq!(drained.state, RecordingState::Recording);
        assert_eq!(drained.outstanding_records, 0);
        assert_eq!(drained.outstanding_bytes, 0);
        assert_eq!(drained.persisted_through_sequence, 1547);
        assert_eq!(drained.activation_generation, 2);
        assert_eq!(worker.reference_credit.groups, 0);
        worker.request_stop().unwrap();
        wait(&mut worker, |s| s.state == RecordingState::Idle);
        worker.request_finish().unwrap();
        wait(&mut worker, |s| s.worker_closed);
        assert!(worker.poll().terminal_seal_committed);
    }
}
