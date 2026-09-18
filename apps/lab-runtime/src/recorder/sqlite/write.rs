//! Run/interval lifecycle, semantic fact persistence, gaps and sealing transactions.
//!
//! Transaction bodies and ordering are the accepted schema-v1 durability contract.

use super::*;

impl SqliteStore {
    /// Commit a new run and recording interval before accepting its facts.
    pub fn start_run(&mut self, label: &str) -> Result<(), StorageError> {
        self.start_run_with_policy(label, RecordingPolicy::Required)
    }

    /// Commit the host-selected immutable policy with the run's start barrier.
    pub fn start_run_with_policy(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
    ) -> Result<(), StorageError> {
        self.start_run_with_boundary(label, policy, &BoundarySnapshot::empty(Duration::ZERO))
    }

    /// Atomically bind a new run, its interval and the owner-frozen boundary.
    pub fn start_run_with_boundary(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
    ) -> Result<(), StorageError> {
        self.start_run_impl(label, policy, boundary, None)
    }

    /// Commit the actual interval-start UTC bracket in the same durable
    /// transaction as the frozen boundary and its checkpoint.
    pub fn start_run_with_boundary_anchor(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
        anchor: &TimeAnchor,
    ) -> Result<(), StorageError> {
        self.start_run_impl(label, policy, boundary, Some(anchor))
    }

    /// Start only at the exact two-record boundary/clock range reserved by the owner.
    pub fn start_run_with_boundary_anchor_assigned(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
        anchor: &TimeAnchor,
        first_record: u64,
    ) -> Result<(), StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(first_record) {
            return Err(StorageError("start record reservation mismatch".into()));
        }
        self.start_run_impl(label, policy, boundary, Some(anchor))
    }

    fn start_run_impl(
        &mut self,
        label: &str,
        policy: RecordingPolicy,
        boundary: &BoundarySnapshot,
        anchor: Option<&TimeAnchor>,
    ) -> Result<(), StorageError> {
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        if self.run_no.is_some()
            || label.len() > 128
            || label.trim().is_empty()
            || !boundary.valid()
        {
            return Err(StorageError("invalid or overlapping recording run".into()));
        }
        let run_no = self.next_run_no;
        let interval_no = self.next_interval_no;
        let following_run = run_no
            .checked_add(1)
            .ok_or_else(|| StorageError("run identity exhausted".into()))?;
        let following_interval = interval_no
            .checked_add(1)
            .ok_or_else(|| StorageError("interval identity exhausted".into()))?;
        let boundary_record = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("boundary record identity exhausted".into()))?;
        let anchor_record = anchor
            .map(|_| {
                boundary_record
                    .checked_add(1)
                    .ok_or_else(|| StorageError("start anchor record identity exhausted".into()))
            })
            .transpose()?;
        let final_record = anchor_record.unwrap_or(boundary_record);
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("start clock anchor identity exhausted".into()))
            })
            .transpose()?;
        let boundary_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("boundary commit identity exhausted".into()))?;
        let at = duration_blob(boundary.at)?;
        let boundary_wall = self.boot_anchor.estimate_us(boundary.at)?;
        let transaction = self.connection.transaction()?;
        let policy = match policy {
            RecordingPolicy::Required => "required",
            RecordingPolicy::BestEffort => "best-effort",
        };
        transaction.execute(
            "INSERT INTO runs(boot_id,run_no,label,policy,state,coverage,initial_activation_id,
             started_wall_us) VALUES(?1,?2,?3,?4,'recording','complete',?5,?6)",
            params![
                self.boot_id.as_slice(),
                u64_blob(run_no).as_slice(),
                label,
                policy,
                self.current_activation_no
                    .map(u64_blob)
                    .map(|id| id.to_vec()),
                anchor.and_then(TimeAnchor::wall_us),
            ],
        )?;
        transaction.execute(
            "INSERT INTO recording_intervals(boot_id,interval_no,run_no,state,coverage) \
             VALUES(?1,?2,?3,'recording','complete')",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                u64_blob(run_no).as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,
            kind,version,published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
            VALUES(?1,?2,?3,?4,'boundary_snapshot',1,?5,?5,?6,
            'boot_anchor','host_lifecycle',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_record).as_slice(),
                u64_blob(run_no).as_slice(),
                u64_blob(interval_no).as_slice(),
                at.as_slice(),
                boundary_wall,
                boundary.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,
            severity,code,data) VALUES(?1,?2,'boundary','info','run_start',?3)",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_record).as_slice(),
                boundary.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE recording_intervals SET boundary_seq=?3,start_seq=?3
            WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                u64_blob(boundary_record).as_slice()
            ],
        )?;
        if let (Some(anchor), Some(sequence)) = (anchor, anchor_record) {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                sequence,
                self.next_anchor_no,
                "interval_start",
                anchor,
                Some(run_no),
                Some(interval_no),
                &self.boot_anchor,
            )?;
        }
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,
            persisted_through_seq=?3,coverage='complete' WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(boundary_commit).as_slice(),
                u64_blob(final_record).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.run_no = Some(run_no);
        self.interval_no = Some(interval_no);
        self.next_run_no = following_run;
        self.next_interval_no = following_interval;
        self.next_record_sequence = final_record;
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.commit_no = boundary_commit;
        Ok(())
    }

    /// Atomically commit one bounded fact group and its cumulative checkpoint.
    /// Call only on the storage worker; the Runtime owner never waits here.
    pub fn append_facts(&mut self, facts: &[RecordingFact]) -> Result<u64, StorageError> {
        let capture = facts
            .last()
            .map(|fact| match fact {
                RecordingFact::Measurement { sample, .. } => sample.at(),
                RecordingFact::Output { at, .. }
                | RecordingFact::Controller { at, .. }
                | RecordingFact::Reference { at, .. } => *at,
            })
            .unwrap_or(Duration::ZERO);
        self.append_facts_with_capture(facts, capture)
    }

    /// Commit an owner group with its original host-admission time, independent
    /// of each observation's occurrence and publication timestamps.
    pub fn append_facts_with_capture(
        &mut self,
        facts: &[RecordingFact],
        captured_at: Duration,
    ) -> Result<u64, StorageError> {
        self.append_fact_groups(&[(facts, captured_at)])
    }

    /// Commit several causal owner groups without splitting any group. The
    /// worker retains their original capture times and one bounded batch.
    pub fn append_fact_groups(
        &mut self,
        groups: &[(&[RecordingFact], Duration)],
    ) -> Result<u64, StorageError> {
        let count = groups
            .iter()
            .try_fold(0usize, |total, (facts, _)| total.checked_add(facts.len()))
            .ok_or_else(|| StorageError("batch count arithmetic exhausted".into()))?;
        if groups.is_empty()
            || groups.len() > 4
            || count == 0
            || count > 256
            || groups.iter().any(|(facts, _)| facts.is_empty())
            || self.run_no.is_none()
        {
            return Err(StorageError("invalid or oversized recording batch".into()));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let mut next_sequence = self.next_record_sequence;
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("commit identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        for &(facts, captured_at) in groups {
            for fact in facts {
                validate_storage_fact(fact)?;
                next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("record identity exhausted".into()))?;
                let record_id = u64_blob(next_sequence);
                let fact_id = u64_blob(fact.sequence());
                let run_no = u64_blob(self.run_no.expect("run checked above"));
                let interval_no = u64_blob(self.interval_no.expect("run checked above"));
                let (kind, time) = match fact {
                    RecordingFact::Measurement { sample, .. } => ("measurement", sample.at()),
                    RecordingFact::Output { at, .. } => ("output", *at),
                    RecordingFact::Controller { at, .. } => ("controller", *at),
                    RecordingFact::Reference { at, .. } => ("reference", *at),
                };
                let at = duration_blob(time)?;
                let observed = match fact {
                    RecordingFact::Measurement { sample, .. } => sample.freshness_at(),
                    _ => time,
                };
                let observed = duration_blob(observed)?;
                let captured = duration_blob(captured_at)?;
                let wall_estimate = self.boot_anchor.estimate_us(time)?;
                transaction.execute(
                    "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
                 fact_seq,published_at,observed_at,captured_at,wall_estimate_us,wall_basis)
                 VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,'boot_anchor')",
                    params![
                        self.boot_id.as_slice(),
                        record_id.as_slice(),
                        run_no.as_slice(),
                        interval_no.as_slice(),
                        kind,
                        fact_id.as_slice(),
                        at.as_slice(),
                        observed.as_slice(),
                        captured.as_slice(),
                        wall_estimate
                    ],
                )?;
                match fact {
                    RecordingFact::Measurement {
                        sample,
                        generation,
                        revision,
                        state_revision,
                        lineage,
                        ..
                    } => {
                        let (value_kind, float_value, integer_value, bool_value, text_value) =
                            match sample.value() {
                                Some(Value::Float(value)) if value.is_finite() => {
                                    ("float", Some(*value), None, None, None)
                                }
                                Some(Value::Integer(value)) => {
                                    ("integer", None, Some(*value), None, None)
                                }
                                Some(Value::Boolean(value)) => {
                                    ("boolean", None, None, Some(i64::from(*value)), None)
                                }
                                Some(Value::Text(value)) => {
                                    ("text", None, None, None, Some(value.as_str()))
                                }
                                Some(Value::Enum(value)) => {
                                    ("enum", None, None, None, Some(value.as_str()))
                                }
                                Some(Value::Float(_)) => {
                                    return Err(StorageError("nonfinite measurement".into()));
                                }
                                None => ("none", None, None, None, None),
                            };
                        let quality = match sample.quality() {
                            SampleQuality::Good => "good",
                            SampleQuality::Unavailable => "unavailable",
                        };
                        let failure = sample.failure().map(|reason| format!("{reason:?}"));
                        let state_revision_blob = state_revision.map(u64_blob);
                        let lineage_blob = lineage.map(encode_lineage).transpose()?;
                        transaction.execute(
                        "INSERT INTO measurements(boot_id,record_seq,run_no,instrument_id,\
                         parameter_id,generation,revision,observed_at,published_at,unit_key,\
                         quality,failure,value_kind,float_value,integer_value,bool_value,text_value,\
                         state_revision,lineage) \
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                        params![self.boot_id.as_slice(), record_id.as_slice(), run_no.as_slice(),
                            u64_blob(sample.signal().instrument().get()).as_slice(),
                            u64_blob(sample.signal().parameter().get()).as_slice(),
                            u64_blob(*generation).as_slice(), u64_blob(*revision).as_slice(),
                            duration_blob(sample.freshness_at())?.as_slice(), at.as_slice(),
                            sample.unit().id(), quality, failure, value_kind,
                            float_value, integer_value, bool_value, text_value,
                            state_revision_blob.as_ref().map(|blob|blob.as_slice()),
                            lineage_blob.as_deref()],
                    )?;
                    }
                    RecordingFact::Output {
                        actuator,
                        attempt_id,
                        dispatch_id,
                        unit,
                        authority_epoch,
                        resource,
                        binding_generation,
                        mapping_revision,
                        stage,
                        value,
                        source,
                        ..
                    } => {
                        let stage = match stage {
                            OutputStage::SupersededBeforeSend => "superseded_before_send",
                            OutputStage::RejectedBeforeSend => "rejected_before_send",
                            OutputStage::ExpiredBeforeSend => "expired_before_send",
                            OutputStage::Requested => "requested",
                            OutputStage::Authorized => "authorized",
                            OutputStage::SendStarted => "send_started",
                            OutputStage::Acknowledged => "acknowledged",
                            OutputStage::ReadbackVerified => "readback_verified",
                            OutputStage::Failed => "failed",
                            OutputStage::Ambiguous => "ambiguous",
                            OutputStage::TransportUncertain => "transport_uncertain",
                            OutputStage::SafeRequested => "safe_requested",
                            OutputStage::SafeSendStarted => "safe_send_started",
                            OutputStage::SafeAcknowledged => "safe_acknowledged",
                            OutputStage::SafeReadbackVerified => "safe_readback_verified",
                            OutputStage::Revoked => "revoked",
                        };
                        let evidence_source = match source {
                            lab_core::recording::OutputEvidenceSource::None => "none",
                            lab_core::recording::OutputEvidenceSource::VirtualSimulation => {
                                "virtual_simulation"
                            }
                            lab_core::recording::OutputEvidenceSource::TransportProtocol => {
                                "transport_protocol"
                            }
                        };
                        let attempt_blob = attempt_id.map(u64_blob);
                        let epoch_blob = authority_epoch.map(u64_blob);
                        let resource_blob = resource.map(|id| u64_blob(id.get()));
                        let generation_blob = binding_generation.map(u64_blob);
                        let revision_blob = mapping_revision.map(u64_blob);
                        let dispatch_blob = dispatch_id.map(|id| {
                            let (instance, sequence) = id.diagnostic_parts();
                            let mut bytes = [0u8; 16];
                            bytes[..8].copy_from_slice(&instance.to_be_bytes());
                            bytes[8..].copy_from_slice(&sequence.to_be_bytes());
                            bytes
                        });
                        transaction.execute(
                            "INSERT INTO output_events(boot_id,record_seq,attempt_id,dispatch_id,\
                         resource_id,instrument_id,parameter_id,authority_epoch,generation,revision,\
                         stage,value,unit_key,evidence_source,evidence_basis) \
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'runtime')",
                            params![
                                self.boot_id.as_slice(),
                                record_id.as_slice(),
                                attempt_blob.as_ref().map(|blob| blob.as_slice()),
                                dispatch_blob.as_ref().map(|blob| blob.as_slice()),
                                resource_blob.as_ref().map(|blob| blob.as_slice()),
                                u64_blob(actuator.instrument().get()).as_slice(),
                                u64_blob(actuator.parameter().get()).as_slice(),
                                epoch_blob.as_ref().map(|blob| blob.as_slice()),
                                generation_blob.as_ref().map(|blob| blob.as_slice()),
                                revision_blob.as_ref().map(|blob| blob.as_slice()),
                                stage,
                                value,
                                unit.as_ref().map(lab_core::Unit::id),
                                evidence_source
                            ],
                        )?;
                    }
                    RecordingFact::Controller {
                        controller,
                        state,
                        config_revision,
                        pid,
                        ..
                    } => {
                        let state = format!("{state:?}").to_ascii_lowercase();
                        let settings = pid.map(|pid| {
                            serde_json::json!({
                                "kp":pid.kp,"ki":pid.ki,"kd":pid.kd,
                                "output_min":pid.output_min,"output_max":pid.output_max,
                            })
                            .to_string()
                        });
                        transaction.execute(
                        "INSERT INTO controller_events(boot_id,record_seq,controller_id,after_state,
                         event_kind,config_revision,diagnostics)
                         VALUES(?1,?2,?3,?4,?5,?6,?7)",
                        params![self.boot_id.as_slice(),record_id.as_slice(),
                            u64_blob(controller.get()).as_slice(),state,
                            if pid.is_some() {"configuration"} else {"lifecycle"},
                            u64_blob(*config_revision).as_slice(),settings],
                    )?;
                    }
                    RecordingFact::Reference {
                        reference,
                        revision,
                        value,
                        target,
                        rate,
                        unit,
                        at,
                        ..
                    } => {
                        transaction.execute(
                            "INSERT INTO reference_events(boot_id,record_seq,reference_id,revision,
                         event_kind,value,target,rate,unit_key,progress_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                            params![
                                self.boot_id.as_slice(),
                                record_id.as_slice(),
                                u64_blob(reference.get()).as_slice(),
                                u64_blob(*revision).as_slice(),
                                if target.is_some() { "ramp" } else { "fixed" },
                                value,
                                target,
                                rate,
                                unit.id(),
                                duration_blob(*at)?.as_slice()
                            ],
                        )?;
                    }
                }
            }
        }
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                u64_blob(next_sequence).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = next_sequence;
        self.commit_no = next_commit;
        Ok(next_sequence)
    }

    /// Persist a whole group/batch only at the range assigned before owner transfer.
    pub fn append_fact_groups_assigned(
        &mut self,
        groups: &[(&[RecordingFact], Duration)],
        first_record: u64,
    ) -> Result<u64, StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(first_record) {
            return Err(StorageError("fact record reservation mismatch".into()));
        }
        self.append_fact_groups(groups)
    }

    /// Atomically commit one application fact and its checkpoint on the worker.
    /// The transaction can preserve an accepted-only result after a process crash.
    pub fn append_operation(&mut self, operation: &OperationRecord) -> Result<u64, StorageError> {
        let lifecycle_terminal = self.run_no.is_none()
            && operation.command == "recording_stop"
            && matches!(operation.phase, "completed" | "failed");
        if !(self.run_no.is_some() || lifecycle_terminal) || !operation.valid() {
            return Err(StorageError("invalid operation recording fact".into()));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let sequence = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("operation record identity exhausted".into()))?;
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("operation commit identity exhausted".into()))?;
        let run = self.run_no.map(u64_blob);
        let interval = self.interval_no.map(u64_blob);
        let wall_estimate = self.boot_anchor.estimate_us(operation.at)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
             published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
             VALUES(?1,?2,?3,?4,'operation',1,?5,?5,?6,'boot_anchor',
             'application',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                run.as_ref().map(|value| value.as_slice()),
                interval.as_ref().map(|value| value.as_slice()),
                duration_blob(operation.at)?.as_slice(),
                wall_estimate,
                operation.data.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO operation_events(boot_id,record_seq,request_scope,request_seq,\
             phase,command,result,outcome_basis) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                self.boot_id.as_slice(),
                u64_blob(sequence).as_slice(),
                operation.scope,
                operation.request_seq.to_string(),
                operation.phase,
                operation.command,
                operation.data,
                operation.outcome_basis
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                u64_blob(sequence).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = sequence;
        self.commit_no = commit;
        Ok(sequence)
    }

    /// Commit an application fact only at its owner-reserved record identity.
    pub fn append_operation_assigned(
        &mut self,
        operation: &OperationRecord,
        record_seq: u64,
    ) -> Result<u64, StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(record_seq) {
            return Err(StorageError("operation record reservation mismatch".into()));
        }
        self.append_operation(operation)
    }

    /// Atomically commit one informational annotation at its owner-reserved ID.
    /// Caller data stays nested under `data` and never supplies record origin.
    pub fn append_annotation_assigned(
        &mut self,
        annotation: &AnnotationRecord,
        record_seq: u64,
    ) -> Result<u64, StorageError> {
        if self.run_no.is_none()
            || !annotation.valid()
            || self.next_record_sequence.checked_add(1) != Some(record_seq)
        {
            return Err(StorageError(
                "annotation reservation or interval invalid".into(),
            ));
        }
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        let data: serde_json::Value = serde_json::from_str(&annotation.data_json)
            .map_err(|_| StorageError("annotation data invalid".into()))?;
        let payload = serde_json::to_vec(&serde_json::json!({
            "name":annotation.name,
            "data":data,
            "actor_scope":annotation.scope,
            "request_seq":annotation.request_seq.to_string(),
        }))
        .map_err(|_| StorageError("annotation encoding failed".into()))?;
        if payload.len() > 4 * 1024 {
            return Err(StorageError("annotation envelope too large".into()));
        }
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("annotation commit identity exhausted".into()))?;
        let wall = self.boot_anchor.estimate_us(annotation.at)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
             published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)\
             VALUES(?1,?2,?3,?4,'annotation',1,?5,?5,?6,'boot_anchor',\
             'local_client',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(record_seq).as_slice(),
                u64_blob(self.run_no.expect("checked active run")).as_slice(),
                u64_blob(self.interval_no.expect("active run has interval")).as_slice(),
                duration_blob(annotation.at)?.as_slice(),
                wall,
                payload.as_slice(),
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)\
             VALUES(?1,?2,'annotation','info',?3,?4)",
            params![
                self.boot_id.as_slice(),
                u64_blob(record_seq).as_slice(),
                annotation.name,
                payload.as_slice()
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3\
             WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                u64_blob(record_seq).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = record_seq;
        self.commit_no = commit;
        Ok(record_seq)
    }

    /// Commit a real durable progress marker even during an otherwise quiet run.
    /// A queue heartbeat is not a substitute for this SQL transaction.
    pub fn probe(&mut self, submitted_at: Duration) -> Result<(), StorageError> {
        if self.run_no.is_none() {
            return Err(StorageError("progress probe outside recording".into()));
        }
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("commit identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,last_probe_at=?3 WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                duration_blob(submitted_at)?.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.commit_no = next_commit;
        Ok(())
    }

    /// Seal the current run in a transaction; no later facts enter that interval.
    pub fn stop_run(&mut self) -> Result<(), StorageError> {
        self.stop_run_impl(
            None,
            &serde_json::json!({"pending_operations":[]}),
            Duration::ZERO,
        )
    }

    /// Seal an interval with its actual UTC end bracket and final clock fact
    /// in the same transaction as the sealed coverage/checkpoint rows.
    pub fn stop_run_with_anchor(&mut self, anchor: &TimeAnchor) -> Result<(), StorageError> {
        self.stop_run_impl(
            Some(anchor),
            &serde_json::json!({"pending_operations":[]}),
            anchor.after(),
        )
    }

    /// Seal pending accepted identities and the final FIFO prefix atomically.
    pub fn stop_run_with_anchor_summary(
        &mut self,
        anchor: &TimeAnchor,
        summary: &serde_json::Value,
        requested_at: Duration,
    ) -> Result<(), StorageError> {
        self.stop_run_impl(Some(anchor), summary, requested_at)
    }

    /// Seal only at the owner's exact interval-end clock/seal range.
    pub fn stop_run_with_anchor_summary_assigned(
        &mut self,
        anchor: &TimeAnchor,
        summary: &serde_json::Value,
        requested_at: Duration,
        first_record: u64,
    ) -> Result<(), StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(first_record) {
            return Err(StorageError("stop record reservation mismatch".into()));
        }
        self.stop_run_impl(Some(anchor), summary, requested_at)
    }

    fn stop_run_impl(
        &mut self,
        anchor: Option<&TimeAnchor>,
        summary: &serde_json::Value,
        requested_at: Duration,
    ) -> Result<(), StorageError> {
        let Some(run_no) = self.run_no else {
            return Err(StorageError("no active recording run".into()));
        };
        let summary_bytes = serde_json::to_vec(summary)
            .map_err(|error| StorageError(format!("stop summary: {error}")))?;
        if summary_bytes.len() > 16 * 1024 - 512
            || !summary.is_object()
            || summary["pending_operations"]
                .as_array()
                .is_none_or(|items| items.len() > 64)
        {
            return Err(StorageError("invalid bounded stop summary".into()));
        }
        let interval_no = self.interval_no.expect("run exists");
        let anchor_record = anchor
            .map(|_| {
                self.next_record_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("end anchor record identity exhausted".into()))
            })
            .transpose()?;
        let seal_record = anchor_record
            .unwrap_or(self.next_record_sequence)
            .checked_add(1)
            .ok_or_else(|| StorageError("interval seal identity exhausted".into()))?;
        let seal_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("interval seal commit identity exhausted".into()))?;
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("end clock anchor identity exhausted".into()))
            })
            .transpose()?;
        let transaction = self.connection.transaction()?;
        if let (Some(anchor), Some(sequence)) = (anchor, anchor_record) {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                sequence,
                self.next_anchor_no,
                "interval_end",
                anchor,
                Some(run_no),
                Some(interval_no),
                &self.boot_anchor,
            )?;
        }
        let mut seal = summary.as_object().expect("validated stop object").clone();
        seal.insert(
            "accepted_prefix_through_seq".into(),
            serde_json::Value::String(self.next_record_sequence.to_string()),
        );
        seal.insert(
            "coverage".into(),
            serde_json::Value::String(if self.coverage_gap { "gap" } else { "complete" }.into()),
        );
        let seal = serde_json::Value::Object(seal).to_string();
        let stop_at = duration_blob(requested_at)?;
        let wall_estimate = self.boot_anchor.estimate_us(requested_at)?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,
             published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
             VALUES(?1,?2,?3,?4,'interval_seal',1,?5,?5,?6,'boot_anchor',
             'host_lifecycle',?7)",
            params![
                self.boot_id.as_slice(),
                u64_blob(seal_record).as_slice(),
                u64_blob(run_no).as_slice(),
                u64_blob(interval_no).as_slice(),
                stop_at.as_slice(),
                wall_estimate,
                seal.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
             VALUES(?1,?2,'recording','info','interval_sealed',?3)",
            params![
                self.boot_id.as_slice(),
                u64_blob(seal_record).as_slice(),
                seal.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE recording_intervals SET state='sealed',end_seq=?3
             WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval_no).as_slice(),
                u64_blob(seal_record).as_slice()
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state='sealed',ended_wall_us=?3 WHERE boot_id=?1 AND run_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(run_no).as_slice(),
                anchor.and_then(TimeAnchor::wall_us)
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3
             WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(seal_commit).as_slice(),
                u64_blob(seal_record).as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = seal_record;
        self.commit_no = seal_commit;
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.run_no = None;
        self.interval_no = None;
        Ok(())
    }

    /// Seal a writable failed interval after its already-admitted FIFO prefix.
    /// No missing fact is fabricated; the typed gap names the known source
    /// boundary and the checkpoint names only bytes this transaction committed.
    pub fn fail_run(&mut self, gap: &RecorderGap) -> Result<u64, StorageError> {
        if !gap.valid() {
            return Err(StorageError("invalid bounded recorder gap".into()));
        }
        let run = self
            .run_no
            .ok_or_else(|| StorageError("gap seal without active run".into()))?;
        let interval = self
            .interval_no
            .ok_or_else(|| StorageError("gap interval missing".into()))?;
        let sequence = self
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| StorageError("gap record identity exhausted".into()))?;
        let commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("gap commit identity exhausted".into()))?;
        let last_missing = gap.first_missing_fact.and_then(|first| {
            gap.known_missing_count
                .and_then(|count| first.checked_add(count - 1))
        });
        let summary = serde_json::json!({"reason":gap.reason,
            "first_missing_fact":gap.first_missing_fact.map(|id|id.to_string()),
            "known_missing_count":gap.known_missing_count.map(|count|count.to_string()),
            "last_accepted_fact":gap.last_accepted_fact.map(|id|id.to_string()),
            "cutoff_monotonic_ns":gap.at.as_nanos().to_string(),
            "last_confirmed_record_seq":self.next_record_sequence.to_string()})
        .to_string();
        let record = u64_blob(sequence);
        let cutoff = duration_blob(gap.at)?;
        let wall_estimate = self.boot_anchor.estimate_us(gap.at)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,run_no,interval_no,
            kind,version,published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
            VALUES(?1,?2,?3,?4,'recorder_gap',1,?5,?5,?6,
            'boot_anchor','host_lifecycle',?7)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                u64_blob(run).as_slice(),
                u64_blob(interval).as_slice(),
                cutoff.as_slice(),
                wall_estimate,
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "INSERT INTO gaps(boot_id,record_seq,interval_no,first_missing,
            last_missing,known_count,unknown_tail,reason,last_confirmed)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                u64_blob(interval).as_slice(),
                gap.first_missing_fact.map(u64_blob).map(|id| id.to_vec()),
                last_missing.map(u64_blob).map(|id| id.to_vec()),
                gap.known_missing_count.map(u64_blob).map(|id| id.to_vec()),
                i64::from(gap.known_missing_count.is_none()),
                gap.reason,
                u64_blob(self.next_record_sequence).as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,
            severity,code,data) VALUES(?1,?2,'recorder_gap','error','coverage_lost',?3)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE recording_intervals SET state='failed',coverage='gap',
            end_seq=?3,loss_summary=?4 WHERE boot_id=?1 AND interval_no=?2",
            params![
                self.boot_id.as_slice(),
                u64_blob(interval).as_slice(),
                record.as_slice(),
                summary
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state='failed',coverage='gap' WHERE boot_id=?1
            AND run_no=?2",
            params![self.boot_id.as_slice(), u64_blob(run).as_slice()],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,
            persisted_through_seq=?3,coverage='gap' WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(commit).as_slice(),
                record.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = sequence;
        self.commit_no = commit;
        self.run_no = None;
        self.interval_no = None;
        self.coverage_gap = true;
        Ok(sequence)
    }

    /// Persist a known loss marker only at its reserved owner identity.
    pub fn fail_run_assigned(
        &mut self,
        gap: &RecorderGap,
        record_seq: u64,
    ) -> Result<u64, StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(record_seq) {
            return Err(StorageError("gap record reservation mismatch".into()));
        }
        self.fail_run(gap)
    }

    /// Commit a terminal boot seal after every accepted run/fact barrier.
    /// Presence of this row proves SQL commit, independently of its later receipt.
    pub fn finish_boot(&mut self, at: Duration) -> Result<(), StorageError> {
        self.finish_boot_with_summary(
            at,
            &serde_json::json!({"safety_evidence":"unprovided"}),
            None,
        )
    }

    /// Atomically seal the frozen trusted shutdown observation and actual UTC anchor.
    pub fn finish_boot_with_summary(
        &mut self,
        at: Duration,
        evidence: &serde_json::Value,
        anchor: Option<&TimeAnchor>,
    ) -> Result<(), StorageError> {
        if self.run_no.is_some() || self.boot_sealed {
            return Err(StorageError(
                "boot finish requires a stopped, unsealed recorder".into(),
            ));
        }
        let evidence_bytes = serde_json::to_vec(evidence)
            .map_err(|error| StorageError(format!("shutdown evidence: {error}")))?;
        if evidence_bytes.len() > 16 * 1024 || !evidence.is_object() {
            return Err(StorageError("invalid bounded shutdown evidence".into()));
        }
        let anchor_sequence = anchor
            .map(|_| {
                self.next_record_sequence
                    .checked_add(1)
                    .ok_or_else(|| StorageError("terminal clock identity exhausted".into()))
            })
            .transpose()?;
        let anchor_no = anchor.map(|_| self.next_anchor_no);
        let next_anchor = anchor
            .map(|_| {
                self.next_anchor_no
                    .checked_add(1)
                    .ok_or_else(|| StorageError("terminal anchor identity exhausted".into()))
            })
            .transpose()?;
        let next_record = self
            .next_record_sequence
            .checked_add(if anchor.is_some() { 2 } else { 1 })
            .ok_or_else(|| StorageError("terminal record identity exhausted".into()))?;
        let next_commit = self
            .commit_no
            .checked_add(1)
            .ok_or_else(|| StorageError("terminal commit identity exhausted".into()))?;
        let record = u64_blob(next_record);
        let published = duration_blob(at)?;
        let mut summary = evidence.as_object().expect("validated object").clone();
        summary.insert(
            "accepted_prefix_through_seq".into(),
            serde_json::Value::String(self.next_record_sequence.to_string()),
        );
        summary.insert(
            "coverage".into(),
            serde_json::Value::String(if self.coverage_gap { "gap" } else { "complete" }.into()),
        );
        let summary = serde_json::Value::Object(summary).to_string();
        let transaction = self.connection.transaction()?;
        if let (Some(anchor), Some(anchor_sequence), Some(anchor_no)) =
            (anchor, anchor_sequence, anchor_no)
        {
            insert_clock_projection(
                &transaction,
                &self.boot_id,
                anchor_sequence,
                anchor_no,
                "boot_end",
                anchor,
                None,
                None,
                &self.boot_anchor,
            )?;
        }
        let changed = transaction.execute(
            "UPDATE runtime_boots SET state='sealed',exit_summary=?2,ended_wall_us=?3 \
             WHERE boot_id=?1 AND state='active'",
            params![
                self.boot_id.as_slice(),
                summary,
                anchor.and_then(TimeAnchor::wall_us)
            ],
        )?;
        if changed != 1 {
            return Err(StorageError("active boot seal target missing".into()));
        }
        transaction.execute(
            "INSERT INTO records(boot_id,record_seq,kind,version,published_at,
             captured_at,origin) VALUES(?1,?2,'shutdown',1,?3,?3,'host_lifecycle')",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                published.as_slice()
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
             VALUES(?1,?2,'shutdown','info','boot_sealed',?3)",
            params![
                self.boot_id.as_slice(),
                record.as_slice(),
                summary.as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3
             WHERE boot_id=?1",
            params![
                self.boot_id.as_slice(),
                u64_blob(next_commit).as_slice(),
                record.as_slice()
            ],
        )?;
        transaction.commit()?;
        self.next_record_sequence = next_record;
        self.commit_no = next_commit;
        if let Some(next_anchor) = next_anchor {
            self.next_anchor_no = next_anchor;
        }
        self.boot_sealed = true;
        Ok(())
    }

    /// Seal only at the two-record final clock/shutdown range reserved by owner.
    pub fn finish_boot_with_summary_assigned(
        &mut self,
        at: Duration,
        evidence: &serde_json::Value,
        anchor: &TimeAnchor,
        first_record: u64,
    ) -> Result<(), StorageError> {
        if self.next_record_sequence.checked_add(1) != Some(first_record) {
            return Err(StorageError("finish record reservation mismatch".into()));
        }
        self.finish_boot_with_summary(at, evidence, Some(anchor))
    }

    /// Explicit close reports a failure after a committed seal separately.
    pub fn close(self) -> Result<(), StorageError> {
        self.connection
            .close()
            .map_err(|(_connection, error)| StorageError(format!("sqlite close: {error}")))
    }
}
