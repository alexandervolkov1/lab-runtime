//! Finite owner-side admission for semantic fact groups and audit records.

use super::*;

impl RecorderWorker {
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

    /// Check ordinary ingress credit for one already validated operation record.
    ///
    /// This owner-local check lets a safe-reducing terminal remain in the
    /// existing bounded operation tracker until credit is released. It neither
    /// reserves capacity nor permits an authority-increasing mutation to run.
    pub(crate) fn operation_credit_available(
        &mut self,
        operation: &OperationRecord,
    ) -> Result<bool, StorageError> {
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
        if bytes > MAX_GROUP_BYTES {
            return Err(StorageError(
                "operation record exceeds ingress bound".into(),
            ));
        }
        Ok(self.charged_groups < self.limits.groups
            && self.charged_records < self.limits.records
            && bytes <= self.limits.bytes.saturating_sub(self.charged_bytes))
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
}

fn fact_charge(fact: &RecordingFact) -> usize {
    match fact {
        RecordingFact::Output { .. } => 512,
        RecordingFact::Controller { .. } | RecordingFact::Reference { .. } => 512,
        RecordingFact::Measurement {
            sample, lineage, ..
        } => {
            // CapturedInput is inline in the transferred Vec, but SQLite also
            // builds a separately owned, at-most-512-byte lineage envelope.
            512 + usize::from(lineage.is_some()) * 512
                + match sample.value() {
                    Some(Value::Text(value) | Value::Enum(value)) => value
                        .capacity()
                        .saturating_add(value.len().saturating_mul(6)),
                    _ => 0,
                }
        }
    }
}
