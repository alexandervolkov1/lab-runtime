//! Staged deployment application and recorded configuration lifecycle orchestration.
//!
//! These methods advance the single ServiceHost owner declared in the parent
//! module; no reconnect, deployment, or shutdown state is copied into a manager.

use super::*;
use lab_core::control::ControllerId;

impl ServiceHost {
    pub(crate) fn quarantined_simple_resource_generation(
        &self,
        resource: ResourceId,
    ) -> Option<u64> {
        self.quarantined_simple_output
            .as_ref()
            .filter(|quarantine| quarantine.resource == resource)
            .map(|quarantine| quarantine.binding_generation)
    }

    pub(crate) fn simple_configuration_status(&self) -> serde_json::Value {
        let pending = self.pending_simple_apply.as_ref().map(|pending| {
            let phase = match pending.phase {
                SimpleApplyPhase::Prepare => "prepare",
                SimpleApplyPhase::ReserveRecording => "reserve_recording",
                SimpleApplyPhase::PrepareTopology => "prepare_topology",
                SimpleApplyPhase::EstablishSafe => "establish_safe",
                SimpleApplyPhase::Commit => "commit",
                SimpleApplyPhase::ConfirmRecording => "confirm_recording",
            };
            serde_json::json!({
                "candidate_id":pending.candidate.snapshot.id().to_string(),
                "phase":phase,
                "deadline_ns":pending.deadline.as_nanos().to_string(),
                "configuration_quiesced":pending.quiesced
            })
        });
        let quarantine = self.quarantined_simple_output.as_ref().map(|quarantine| {
            serde_json::json!({
                "candidate_id":quarantine.candidate_id.to_string(),
                "resource":quarantine.resource.get().to_string(),
                "actuator":{"instrument":quarantine.actuator.instrument().get().to_string(),
                    "parameter":quarantine.actuator.parameter().get().to_string()},
                "binding_generation":quarantine.binding_generation.to_string(),
                "mapping_revision":quarantine.mapping_revision.to_string(),
                "phase":"reconciliation_required",
                "send_started":true,
                "acknowledged":quarantine.acknowledged,
                "readback_verified":quarantine.readback_verified,
                "safe_resend_blocked":true
            })
        });
        serde_json::json!({
            "overlay_active":self.api_simple_overlay_active,
            "pending_apply":pending,
            "quarantine":quarantine
        })
    }
    pub(crate) fn is_staged_simple_device_candidate(&self, candidate_id: u64) -> bool {
        self.deployment
            .as_ref()
            .and_then(DeploymentLifecycle::staged)
            .is_some_and(|staged| staged.id() == candidate_id)
            && self
                .deployment
                .as_ref()
                .and_then(DeploymentLifecycle::staged_simple_metadata)
                .is_some()
    }
    /// Validate active cross-references and retain one complete process-local
    /// SimpleDevice overlay without publishing any candidate entity.
    pub(crate) fn stage_simple_device_candidate(
        &mut self,
        candidate: &SimpleDeviceCandidate,
        expected_revision: u64,
    ) -> Result<StagedSimpleDeviceResult, LifecycleOperationError> {
        if self.pending_simple_apply.is_some() {
            return Err(LifecycleOperationError::Capacity);
        }
        if self
            .quarantined_simple_output
            .as_ref()
            .is_some_and(|quarantine| {
                candidate
                    .instances
                    .iter()
                    .any(|instance| ResourceId::new(instance.resource_id) == quarantine.resource)
            })
        {
            return Err(LifecycleOperationError::Capacity);
        }
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        if lifecycle.revision() != expected_revision {
            return Err(LifecycleOperationError::Conflict);
        }
        for instance in &candidate.instances {
            if !lifecycle
                .active()
                .effective()
                .dto
                .resources
                .iter()
                .any(|resource| resource.id == instance.resource_id)
            {
                return Err(LifecycleOperationError::UnknownResource);
            }
        }
        let overlay = lifecycle
            .active()
            .with_simple_device_candidate(candidate)
            .map_err(|_| LifecycleOperationError::InvalidCandidate)?;
        let staged = lifecycle
            .stage_simple_device(overlay, candidate, self.clock.now())
            .map_err(|_| LifecycleOperationError::Capacity)?;
        Ok(StagedSimpleDeviceResult {
            staged,
            definition_id: candidate.definition.definition_id.clone(),
            definition_version: candidate.definition.definition_version,
            normalized_sha256: candidate.definition.canonical_sha256,
            instruments: candidate
                .instances
                .iter()
                .map(|instance| instance.instrument_id)
                .collect(),
        })
    }

    /// Consume one staged API candidate and hand its bounded incremental work to
    /// the serialized ServiceHost owner. This returns before physical progress.
    pub(crate) fn begin_simple_device_apply(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
    ) -> Result<(), LifecycleOperationError> {
        if self.pending_simple_apply.is_some() {
            return Err(LifecycleOperationError::Capacity);
        }
        if self.quarantined_simple_output.is_some() {
            let output_candidate = self
                .deployment
                .as_ref()
                .and_then(DeploymentLifecycle::staged_simple_metadata)
                .is_some_and(|metadata| metadata.output_capable);
            if output_candidate {
                return Err(LifecycleOperationError::Capacity);
            }
        }
        let now = self.clock.now();
        let deadline = now
            .checked_add(SIMPLE_APPLY_DEADLINE)
            .ok_or(LifecycleOperationError::OwnerFailure)?;
        let candidate = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .consume_simple_device(candidate_id, expected_revision, now)
            .map_err(|error| match error {
                ApplyError::UnknownCandidate | ApplyError::Conflict | ApplyError::Expired => {
                    LifecycleOperationError::Conflict
                }
                _ => LifecycleOperationError::OwnerFailure,
            })?;
        self.pending_simple_apply = Some(PendingSimpleConfigurationApply {
            candidate,
            phase: SimpleApplyPhase::Prepare,
            deadline,
            actuator: None,
            recording: None,
            quiesced: false,
            publication: None,
            committed_revision: None,
        });
        Ok(())
    }

    /// Advance at most one bounded phase of the process-wide SimpleDevice apply.
    pub(crate) fn poll_simple_device_apply(&mut self) -> Option<SimpleApplyCompletion> {
        let mut pending = self.pending_simple_apply.take()?;
        let now = self.clock.now();
        let candidate_id = pending.candidate.snapshot.id();
        if now >= pending.deadline {
            let error = self.pending_simple_deadline_error(&pending);
            return Some(self.fail_pending_simple_apply_with(pending, error));
        }
        let active = self.deployment.as_ref()?.active().clone();
        let step = match pending.phase {
            SimpleApplyPhase::Prepare => {
                if pending.candidate.metadata.output_capable {
                    match self.host.enter_configuration_safe_barrier(now) {
                        Ok(false) => Ok(false),
                        Err(_) => Err(LifecycleOperationError::OutputRejected),
                        Ok(true) => {
                            self.host.begin_configuration_quiesce();
                            pending.quiesced = true;
                            pending.phase = SimpleApplyPhase::ReserveRecording;
                            Ok(false)
                        }
                    }
                } else {
                    pending.phase = SimpleApplyPhase::ReserveRecording;
                    Ok(false)
                }
            }
            SimpleApplyPhase::EstablishSafe => {
                let actuator = pending.actuator.expect("output candidate has actuator");
                match self.host.prepared_simple_output(actuator) {
                    Some(snapshot) if snapshot.safe_confirmed => {
                        pending.phase = SimpleApplyPhase::Commit;
                        Ok(false)
                    }
                    Some(snapshot)
                        if snapshot.sent.is_none()
                            && snapshot.outcome
                                == Some(lab_core::output::DispatchOutcome::Failed) =>
                    {
                        Err(LifecycleOperationError::TransportUnavailable)
                    }
                    Some(snapshot)
                        if snapshot.fault_latched
                            || snapshot.state == lab_core::output::OutputState::FaultLatched =>
                    {
                        Err(LifecycleOperationError::OutputRejected)
                    }
                    Some(_) => Ok(false),
                    None => Err(LifecycleOperationError::OwnerFailure),
                }
            }
            SimpleApplyPhase::ReserveRecording => {
                if pending.recording.is_none() {
                    let base_revision = self.deployment.as_ref()?.revision();
                    let committed_revision = match base_revision.checked_add(1) {
                        Some(revision) => revision,
                        None => {
                            return Some(self.fail_pending_simple_apply_with(
                                pending,
                                LifecycleOperationError::OwnerFailure,
                            ));
                        }
                    };
                    let submitted_at = now;
                    let reopen_required = self.host.begin_configuration_recording_fence(now);
                    pending.recording = Some(PendingSimpleRecording {
                        record: ConfigurationLifecycleRecord {
                            operation_id: candidate_id,
                            operation_kind: "apply_configuration",
                            base_revision,
                            committed_revision,
                            toml_hash: pending.candidate.loaded.toml_hash(),
                            affected: configuration_affected(
                                &pending.candidate.loaded,
                                base_revision,
                                committed_revision,
                            ),
                            reason: None,
                            at: now,
                        },
                        generation: None,
                        reopen_required,
                        submitted_at,
                    });
                }
                let recording = pending.recording.as_mut().expect("created above");
                match self
                    .host
                    .try_reserve_configuration_activation(&recording.record, now)
                {
                    Ok(Some(generation)) => {
                        recording.generation = generation;
                        pending.phase = SimpleApplyPhase::PrepareTopology;
                        Ok(false)
                    }
                    Ok(None) => Ok(false),
                    Err(_) => Err(LifecycleOperationError::RecordingUnavailable),
                }
            }
            SimpleApplyPhase::PrepareTopology => match self.host.prepare_simple_device_overlay(
                &active,
                &pending.candidate.loaded,
                now,
            ) {
                Ok(actuator) if pending.candidate.metadata.output_capable => {
                    pending.actuator = actuator;
                    pending.phase = SimpleApplyPhase::EstablishSafe;
                    Ok(false)
                }
                Ok(None) if !pending.candidate.metadata.output_capable => {
                    pending.phase = SimpleApplyPhase::Commit;
                    Ok(false)
                }
                Ok(_) | Err(_) => Err(LifecycleOperationError::OwnerFailure),
            },
            SimpleApplyPhase::Commit => {
                let active_revision = self.deployment.as_ref().map(DeploymentLifecycle::revision);
                let revision = active_revision.and_then(|revision| revision.checked_add(1));
                let Some(revision) = revision else {
                    return Some(self.fail_pending_simple_apply_with(
                        pending,
                        LifecycleOperationError::OwnerFailure,
                    ));
                };
                let instruments: Vec<_> = pending
                    .candidate
                    .metadata
                    .instruments
                    .iter()
                    .copied()
                    .map(InstrumentId::new)
                    .collect();
                let controllers: Vec<_> = pending
                    .candidate
                    .loaded
                    .effective()
                    .dto
                    .controllers
                    .iter()
                    .filter(|controller| {
                        !active
                            .effective()
                            .dto
                            .controllers
                            .iter()
                            .any(|old| old.id == controller.id)
                    })
                    .map(|controller| ControllerId::new(controller.id))
                    .collect();
                if self
                    .host
                    .prepare_simple_device_publication(
                        &active,
                        &pending.candidate.loaded,
                        now,
                        revision,
                    )
                    .is_err()
                {
                    self.host.rollback_simple_device_publication(
                        &active,
                        &pending.candidate.loaded,
                        instruments,
                        active_revision.expect("revision checked above"),
                    );
                    Err(LifecycleOperationError::OwnerFailure)
                } else {
                    match self
                        .host
                        .prepare_simple_device_overlay_publication(instruments.clone(), controllers)
                    {
                        Ok(publication) => {
                            pending.publication = Some(publication);
                            pending.committed_revision = Some(revision);
                        }
                        Err(_) => {
                            self.host.rollback_simple_device_publication(
                                &active,
                                &pending.candidate.loaded,
                                instruments,
                                active_revision.expect("revision checked above"),
                            );
                            return Some(self.fail_pending_simple_apply_with(
                                pending,
                                LifecycleOperationError::OwnerFailure,
                            ));
                        }
                    }
                    let recording = pending.recording.as_ref().expect("recording reserved");
                    if self
                        .host
                        .commit_reserved_configuration_activation(
                            recording.generation,
                            recording.record.clone(),
                        )
                        .is_err()
                    {
                        self.host.configuration_recording_failed(now);
                        self.host.cancel_simple_device_overlay_publication(
                            pending.publication.take().expect("publication prepared"),
                        );
                        self.host.rollback_simple_device_publication(
                            &active,
                            &pending.candidate.loaded,
                            instruments,
                            active_revision.expect("revision checked above"),
                        );
                        pending.committed_revision = None;
                        Err(LifecycleOperationError::RecordingUnavailable)
                    } else {
                        pending.phase = SimpleApplyPhase::ConfirmRecording;
                        Ok(false)
                    }
                }
            }
            SimpleApplyPhase::ConfirmRecording => {
                let recording = pending.recording.as_ref().expect("recording committed");
                match self.host.live_activation_committed(
                    recording.generation,
                    recording.reopen_required,
                    recording.submitted_at,
                    now,
                ) {
                    Ok(true) => {
                        let publication = pending
                            .publication
                            .take()
                            .expect("durable activation owns prepared publication");
                        let revision = pending
                            .committed_revision
                            .take()
                            .expect("deployment revision was prevalidated");
                        self.host
                            .commit_simple_device_overlay_publication(publication, now);
                        self.deployment
                            .as_mut()
                            .expect("loaded configuration")
                            .commit_prevalidated_simple_device(pending.candidate.loaded, revision);
                        self.api_simple_overlay_active = true;
                        if pending.quiesced {
                            self.host.end_configuration_quiesce();
                        }
                        return Some(SimpleApplyCompletion {
                            candidate_id,
                            result: Ok(revision),
                        });
                    }
                    Ok(false) => Ok(false),
                    Err(_) => {
                        self.host.configuration_recording_failed(now);
                        Err(LifecycleOperationError::RecordingUnavailable)
                    }
                }
            }
        };
        match step {
            Ok(false) => {
                self.pending_simple_apply = Some(pending);
                None
            }
            Ok(true) => unreachable!(),
            Err(error) => Some(self.fail_pending_simple_apply_with(pending, error)),
        }
    }

    fn pending_simple_deadline_error(
        &self,
        pending: &PendingSimpleConfigurationApply,
    ) -> LifecycleOperationError {
        match pending.phase {
            SimpleApplyPhase::ReserveRecording | SimpleApplyPhase::ConfirmRecording => {
                LifecycleOperationError::RecordingUnavailable
            }
            SimpleApplyPhase::EstablishSafe => LifecycleOperationError::OutputRejected,
            SimpleApplyPhase::Prepare if pending.candidate.metadata.output_capable => {
                LifecycleOperationError::OutputRejected
            }
            SimpleApplyPhase::Prepare
            | SimpleApplyPhase::PrepareTopology
            | SimpleApplyPhase::Commit => LifecycleOperationError::OwnerFailure,
        }
    }

    fn fail_pending_simple_apply_with(
        &mut self,
        mut pending: PendingSimpleConfigurationApply,
        error: LifecycleOperationError,
    ) -> SimpleApplyCompletion {
        let candidate_id = pending.candidate.snapshot.id();
        if let Some(recording) = &pending.recording {
            let _ = self
                .host
                .cancel_configuration_activation(recording.generation);
        }
        let instruments: Vec<_> = pending
            .candidate
            .metadata
            .instruments
            .iter()
            .copied()
            .map(InstrumentId::new)
            .collect();
        let mut topology_rolled_back = false;
        if let Some(publication) = pending.publication.take() {
            self.host
                .cancel_simple_device_overlay_publication(publication);
            let active_revision = self
                .deployment
                .as_ref()
                .map(DeploymentLifecycle::revision)
                .unwrap_or(1);
            if let Some(active) = self
                .deployment
                .as_ref()
                .map(|lifecycle| lifecycle.active().clone())
            {
                self.host.rollback_simple_device_publication(
                    &active,
                    &pending.candidate.loaded,
                    instruments.clone(),
                    active_revision,
                );
                pending.committed_revision = None;
                topology_rolled_back = true;
            }
        }
        let mut ambiguous = false;
        if let Some(actuator) = pending.actuator
            && let Some(snapshot) = self.host.prepared_simple_output(actuator)
            && snapshot.sent.is_some()
            && !snapshot.safe_confirmed
        {
            ambiguous = true;
            if let Some(binding) = self.host.prepared_simple_binding(actuator.instrument()) {
                self.quarantined_simple_output = Some(QuarantinedSimpleOutput {
                    candidate_id,
                    resource: binding.resource,
                    actuator,
                    binding_generation: binding.binding_generation,
                    mapping_revision: binding.mapping_revision,
                    acknowledged: snapshot.acknowledged.is_some(),
                    readback_verified: snapshot.readback.is_some(),
                });
            }
        }
        if !ambiguous && !topology_rolled_back {
            self.host
                .discard_prepared_simple_overlay(instruments)
                .expect("prepared SimpleDevice failure cleanup invariant");
        }
        if pending.quiesced {
            self.host.end_configuration_quiesce();
        }
        SimpleApplyCompletion {
            candidate_id,
            result: Err(error),
        }
    }

    /// Reload, validate, stage and atomically commit a live-safe deployment diff.
    pub fn reload_configuration(
        &mut self,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        let staged = self.stage_configuration()?;
        self.apply_staged_configuration_kind(
            staged.id(),
            staged.base_revision(),
            "reload_configuration",
        )
    }

    /// Load, validate and retain exactly one immutable candidate without active mutation.
    pub fn stage_configuration(&mut self) -> Result<StagedConfiguration, LifecycleOperationError> {
        if self.api_simple_overlay_active {
            return Err(LifecycleOperationError::InvalidCandidate);
        }
        if self.pending_simple_apply.is_some() {
            return Err(LifecycleOperationError::Capacity);
        }
        let path = self
            .configuration_path
            .as_deref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let candidate =
            load_runtime_toml(path).map_err(|_| LifecycleOperationError::InvalidCandidate)?;
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let staged = lifecycle
            .stage(candidate, self.clock.now())
            .map_err(|error| match error {
                StageError::Busy | StageError::CounterExhausted | StageError::DeadlineOverflow => {
                    LifecycleOperationError::Conflict
                }
            })?;
        Ok(staged)
    }

    /// Validate and commit one supported process-local property overlay through
    /// the same staged deployment lifecycle used by trusted file reloads.
    pub(crate) fn configure_property(
        &mut self,
        target_kind: &str,
        target_id: u64,
        property: &str,
        value: PropertyValue,
        expected_revision: u64,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        if self.pending_simple_apply.is_some() {
            return Err(LifecycleOperationError::Capacity);
        }
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        if lifecycle.revision() != expected_revision {
            return Err(LifecycleOperationError::Conflict);
        }
        let candidate = lifecycle
            .active()
            .with_property_override(target_kind, target_id, property, value)
            .map_err(|_| LifecycleOperationError::InvalidCandidate)?;
        let staged = lifecycle
            .stage(candidate, self.clock.now())
            .map_err(|_| LifecycleOperationError::Conflict)?;
        self.apply_staged_configuration_kind(staged.id(), expected_revision, "property_configure")
    }

    /// Apply one retained candidate under its explicit identity/revision fence.
    pub fn apply_staged_configuration(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        self.apply_staged_configuration_kind(candidate_id, expected_revision, "apply_configuration")
    }

    fn apply_staged_configuration_kind(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
        operation_kind: &'static str,
    ) -> Result<ReloadConfigurationResult, LifecycleOperationError> {
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        let lifecycle = self
            .deployment
            .as_mut()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?;
        let apply_at = self.clock.now();
        let mut port = LiveApplyPort {
            host: &mut self.host,
            active: &active,
            at: apply_at,
            recording_fence: None,
            clock: self.clock,
            operation_id: candidate_id,
            operation_kind,
            base_revision: expected_revision,
            activation_generation: None,
            postcommit_recording_failure: false,
            prepared_bindings: BTreeMap::new(),
        };
        let applied = lifecycle.apply(candidate_id, expected_revision, self.clock.now(), &mut port);
        let recording_fence = port.recording_fence;
        let activation_generation = port.activation_generation;
        let postcommit_recording_failure = port.postcommit_recording_failure;
        match applied {
            Ok(ApplyResult::Applied { revision }) => {
                if postcommit_recording_failure {
                    return Err(LifecycleOperationError::OwnerFailure);
                }
                let (reopen_required, submitted_at) = recording_fence
                    .expect("successful apply crosses the recording fence before commit");
                let generation = activation_generation
                    .expect("successful apply reserves activation before commit");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match self.host.live_activation_committed(
                        generation,
                        reopen_required,
                        submitted_at,
                        self.clock.now(),
                    ) {
                        Ok(true) => {
                            self.host.end_configuration_quiesce();
                            break;
                        }
                        Ok(false) if std::time::Instant::now() < deadline => {
                            let _ = self.host.service(&self.clock);
                            std::thread::yield_now();
                        }
                        _ => {
                            self.host.configuration_recording_failed(self.clock.now());
                            self.host.end_configuration_quiesce();
                            return Err(LifecycleOperationError::OwnerFailure);
                        }
                    }
                }
                Ok(ReloadConfigurationResult { revision })
            }
            Ok(ApplyResult::FailedBeforeCommit) => {
                self.host.end_configuration_quiesce();
                Err(LifecycleOperationError::RequiresSafeBarrier)
            }
            Err(ApplyError::RestartRequired) => Err(LifecycleOperationError::InvalidCandidate),
            Err(ApplyError::OwnerFailure) => Err(LifecycleOperationError::OwnerFailure),
            Err(ApplyError::UnknownCandidate | ApplyError::Conflict | ApplyError::Expired) => {
                Err(LifecycleOperationError::Conflict)
            }
        }
    }

    fn begin_recorded_lifecycle(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        self.begin_recorded_lifecycle_with_scope(operation_kind, deployment, true, None)
    }

    pub(super) fn begin_resource_recorded_lifecycle(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        self.begin_recorded_lifecycle_with_scope(operation_kind, deployment, false, None)
    }

    fn begin_recorded_lifecycle_with_scope(
        &mut self,
        operation_kind: &'static str,
        deployment: &crate::configuration::FrozenDeployment,
        global_quiesced: bool,
        affected: Option<Vec<String>>,
    ) -> Result<PendingRecordedLifecycle, LifecycleOperationError> {
        let operation_id = self.next_lifecycle_operation;
        self.next_lifecycle_operation = operation_id
            .checked_add(1)
            .ok_or(LifecycleOperationError::Conflict)?;
        let revision = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .revision();
        let at = self.clock.now();
        let reopen_required = if global_quiesced {
            if !self
                .host
                .enter_configuration_safe_barrier(at)
                .map_err(|_| LifecycleOperationError::OwnerFailure)?
            {
                return Err(LifecycleOperationError::RequiresSafeBarrier);
            }
            let reopen_required = self.host.begin_configuration_recording_fence(at);
            self.host.begin_configuration_quiesce();
            reopen_required
        } else {
            false
        };
        let record = ConfigurationLifecycleRecord {
            operation_id,
            operation_kind,
            base_revision: revision,
            committed_revision: revision,
            toml_hash: deployment.toml_hash(),
            affected: affected
                .unwrap_or_else(|| configuration_affected(deployment, revision, revision)),
            reason: None,
            at,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let generation = loop {
            let reservation = self
                .host
                .try_reserve_configuration_activation(&record, self.clock.now())
                .map_err(|_| LifecycleOperationError::RecordingUnavailable);
            match reservation {
                Err(error) => {
                    if global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(error);
                }
                Ok(Some(generation)) => break generation,
                Ok(None) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        if global_quiesced {
                            self.host.end_configuration_quiesce();
                        }
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                Ok(None) => {
                    if global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
        };
        Ok(PendingRecordedLifecycle {
            record,
            generation,
            reopen_required,
            submitted_at: at,
            global_quiesced,
        })
    }

    pub(super) fn cancel_recorded_lifecycle(&mut self, pending: PendingRecordedLifecycle) {
        let _ = self
            .host
            .cancel_configuration_activation(pending.generation);
        if pending.global_quiesced {
            self.host.end_configuration_quiesce();
        }
    }

    fn finish_recorded_lifecycle(
        &mut self,
        pending: PendingRecordedLifecycle,
    ) -> Result<(), LifecycleOperationError> {
        self.finish_recorded_lifecycle_detailed(pending)
            .map_err(|_| LifecycleOperationError::RecordingUnavailable)
    }

    pub(super) fn finish_recorded_lifecycle_detailed(
        &mut self,
        pending: PendingRecordedLifecycle,
    ) -> Result<(), RecordedLifecycleFailureStage> {
        if self
            .host
            .commit_reserved_configuration_activation(pending.generation, pending.record)
            .is_err()
        {
            self.host.configuration_recording_failed(self.clock.now());
            if pending.global_quiesced {
                self.host.end_configuration_quiesce();
            }
            return Err(RecordedLifecycleFailureStage::Commit);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match self.host.live_activation_committed(
                pending.generation,
                pending.reopen_required,
                pending.submitted_at,
                self.clock.now(),
            ) {
                Ok(true) => {
                    if pending.global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Ok(());
                }
                Ok(false) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        self.host.configuration_recording_failed(self.clock.now());
                        if pending.global_quiesced {
                            self.host.end_configuration_quiesce();
                        }
                        return Err(RecordedLifecycleFailureStage::Durability);
                    }
                    std::thread::yield_now();
                }
                _ => {
                    self.host.configuration_recording_failed(self.clock.now());
                    if pending.global_quiesced {
                        self.host.end_configuration_quiesce();
                    }
                    return Err(RecordedLifecycleFailureStage::Durability);
                }
            }
        }
    }

    /// Restart only Runtime-owned native virtual models, never managed components or resources.
    pub fn restart_virtual_models(
        &mut self,
    ) -> Result<RestartModelsResult, LifecycleOperationError> {
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        if self.host.virtual_model_count() == 0 {
            return Err(LifecycleOperationError::InvalidCandidate);
        }
        let pending = self.begin_recorded_lifecycle("virtual_models_restart", &active)?;
        let (models, generation) = match self
            .host
            .restart_configured_virtual_models(&active, self.clock.now())
        {
            Ok(result) => result,
            Err(_) => {
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
        };
        self.finish_recorded_lifecycle(pending)?;
        Ok(RestartModelsResult { models, generation })
    }
}
