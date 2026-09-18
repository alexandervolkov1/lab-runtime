//! Staged deployment application and recorded configuration lifecycle orchestration.
//!
//! These methods advance the single ServiceHost owner declared in the parent
//! module; no reconnect, deployment, or shutdown state is copied into a manager.

use super::*;

impl ServiceHost {
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
