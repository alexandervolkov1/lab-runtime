//! Explicit resource reconnect, retirement, probe, generation fencing and release.
//!
//! These methods advance the single ServiceHost owner declared in the parent
//! module; no reconnect, deployment, or shutdown state is copied into a manager.

use super::*;

impl ServiceHost {
    /// Explicitly retire and reopen one configured COM resource.
    /// Port enumeration never calls this operation, writable authorities are
    /// re-established safe without controller rearm, and no write retry is implied.
    pub fn reconnect_resource(
        &mut self,
        resource_id: u64,
        expected_binding_generation: u64,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        self.reconnect_resource_with_factory(
            resource_id,
            expected_binding_generation,
            ComTransport::open_serial_with_transient_retry,
        )
    }

    pub(super) fn reconnect_resource_with_factory(
        &mut self,
        resource_id: u64,
        expected_binding_generation: u64,
        factory: impl FnOnce(ComSettings, std::time::Instant) -> Result<ComTransport, SerialError>,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        if self.pending_simple_apply.is_some() {
            return Err(LifecycleOperationError::Capacity);
        }
        tracing::info!(
            event = "resource_reconnect_requested",
            resource_id,
            expected_binding_generation,
            "explicit resource reconnect requested"
        );
        let active = self
            .deployment
            .as_ref()
            .ok_or(LifecycleOperationError::ConfigurationDisabled)?
            .active()
            .clone();
        let resource = active
            .effective()
            .dto
            .resources
            .iter()
            .find(|resource| resource.id == resource_id)
            .ok_or(LifecycleOperationError::InvalidCandidate)?
            .clone();
        if self
            .quarantined_simple_output
            .as_ref()
            .is_some_and(|quarantine| quarantine.resource.get() == resource_id)
        {
            return self.reconcile_quarantined_simple_output(
                active,
                resource,
                expected_binding_generation,
                factory,
            );
        }
        self.reconnect_diagnostic = None;
        let current = self
            .host
            .configured_resource_generation(ResourceId::new(resource_id))
            .ok_or(LifecycleOperationError::Conflict)?;
        if current != expected_binding_generation {
            return Err(LifecycleOperationError::Conflict);
        }
        let generation = current
            .checked_add(1)
            .ok_or(LifecycleOperationError::Conflict)?;
        let resource_key = ResourceId::new(resource_id);
        self.reconnect_diagnostic = Some(ReconnectDiagnostic {
            resource_id,
            current_generation: current,
            target_generation: generation,
            stage: ReconnectStage::SafeBarrier,
            com_state: self
                .host
                .configured_resource_executor_state(resource_key)
                .map(executor_com_state),
            serial_error: None,
            old_worker_finished: false,
            replacement_worker_spawned: false,
            os_port_open_confirmed: false,
            open_attempts: 0,
            core_rebind_crossed: false,
            cleanup_failure: None,
        });
        if let Some((quarantined_resource, candidate)) =
            self.quarantined_reconnect_candidate.as_mut()
        {
            if candidate.try_shutdown() == lab_core::transport::TransportShutdown::Complete {
                self.quarantined_reconnect_candidate = None;
            } else {
                if let Some(diagnostic) = self.reconnect_diagnostic.as_mut() {
                    let snapshot = candidate.snapshot();
                    diagnostic.stage = ReconnectStage::ReplacementPortOpenFailed;
                    diagnostic.com_state = Some(snapshot.state);
                    diagnostic.serial_error = snapshot.last_open_error.or(snapshot.last_error);
                    diagnostic.replacement_worker_spawned = true;
                    diagnostic.open_attempts = snapshot.open_attempts;
                }
                let _ = quarantined_resource;
                return Err(LifecycleOperationError::TransportUnavailable);
            }
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::RecorderReservation;
        let pending = self.begin_resource_recorded_lifecycle("reconnect_resource", &active)?;
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::RetireOldBegin;
        if self
            .host
            .begin_configured_resource_reconnect(resource_key)
            .is_err()
        {
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(resource.recovery_timeout_ms);
        loop {
            match self
                .host
                .prepare_configured_transport_replacement(resource_key, self.clock.now())
            {
                Ok(true) => {
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.old_worker_finished = true;
                    diagnostic.com_state = Some(ComState::Closed);
                    break;
                }
                Ok(false) if std::time::Instant::now() < deadline => {
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.stage = ReconnectStage::RetireOldPending;
                    diagnostic.com_state = Some(ComState::Closing);
                    if self.host.service(&self.clock).is_err() {
                        self.reconnect_diagnostic
                            .as_mut()
                            .expect("diagnostic initialized")
                            .stage = ReconnectStage::RetireOldFailed;
                        self.cancel_recorded_lifecycle(pending);
                        return Err(LifecycleOperationError::TransportUnavailable);
                    }
                    std::thread::yield_now();
                }
                Ok(false) => {
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.stage = ReconnectStage::RetireOldTimeout;
                    diagnostic.com_state = Some(ComState::Closing);
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
                Err(_) => {
                    self.reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized")
                        .stage = ReconnectStage::RetireOldFailed;
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
            }
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::ReplacementSettings;
        let settings = match com_settings(&resource, generation) {
            Ok(settings) => settings,
            Err(error) => {
                self.reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized")
                    .serial_error = Some(SerialError::InvalidSettings);
                self.cancel_recorded_lifecycle(pending);
                return Err(error);
            }
        };
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::ReplacementWorkerSpawn;
        let open_deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(resource.open_timeout_ms);
        let mut adapter = match factory(settings, open_deadline) {
            Ok(adapter) => adapter,
            Err(error) => {
                let diagnostic = self
                    .reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized");
                diagnostic.serial_error = Some(error);
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::TransportUnavailable);
            }
        };
        {
            let snapshot = adapter.snapshot();
            let diagnostic = self
                .reconnect_diagnostic
                .as_mut()
                .expect("diagnostic initialized");
            diagnostic.stage = ReconnectStage::ActualPortOpening;
            diagnostic.com_state = Some(snapshot.state);
            diagnostic.replacement_worker_spawned = snapshot.worker_spawned;
            diagnostic.open_attempts = snapshot.open_attempts;
        }
        loop {
            match adapter.open_status() {
                ComOpenStatus::Ready => {
                    let snapshot = adapter.snapshot();
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.stage = ReconnectStage::ReplacementPortReady;
                    diagnostic.com_state = Some(snapshot.state);
                    diagnostic.os_port_open_confirmed = snapshot.os_port_open_confirmed;
                    diagnostic.open_attempts = snapshot.open_attempts;
                    diagnostic.serial_error = snapshot.last_open_error;
                    tracing::info!(
                        event = "resource_port_ready",
                        resource_id,
                        generation,
                        attempts = snapshot.open_attempts,
                        "replacement COM resource is ready"
                    );
                    break;
                }
                ComOpenStatus::Failed(error) => {
                    let snapshot = adapter.snapshot();
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.stage = ReconnectStage::ReplacementPortOpenFailed;
                    diagnostic.com_state = Some(snapshot.state);
                    diagnostic.serial_error = Some(error);
                    diagnostic.open_attempts = snapshot.open_attempts;
                    tracing::warn!(
                        event = "resource_reconnect_open_failed",
                        resource_id,
                        generation,
                        error = ?error,
                        attempts = snapshot.open_attempts,
                        "replacement COM resource failed to open"
                    );
                    self.cancel_recorded_lifecycle(pending);
                    self.retire_uninstalled_reconnect_candidate(
                        resource_key,
                        adapter,
                        resource.recovery_timeout_ms,
                    );
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
                ComOpenStatus::Opening if std::time::Instant::now() < open_deadline => {
                    let snapshot = adapter.snapshot();
                    if let Some(diagnostic) = self.reconnect_diagnostic.as_mut() {
                        diagnostic.open_attempts = snapshot.open_attempts;
                        diagnostic.serial_error = snapshot.last_open_error;
                    }
                    if self.host.service(&self.clock).is_err() {
                        self.cancel_recorded_lifecycle(pending);
                        self.retire_uninstalled_reconnect_candidate(
                            resource_key,
                            adapter,
                            resource.recovery_timeout_ms,
                        );
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                ComOpenStatus::Opening => {
                    let snapshot = adapter.snapshot();
                    let diagnostic = self
                        .reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized");
                    diagnostic.stage = ReconnectStage::ReplacementPortOpenFailed;
                    diagnostic.com_state = Some(snapshot.state);
                    diagnostic.serial_error =
                        snapshot.last_open_error.or(Some(SerialError::Timeout));
                    diagnostic.open_attempts = snapshot.open_attempts;
                    self.cancel_recorded_lifecycle(pending);
                    self.retire_uninstalled_reconnect_candidate(
                        resource_key,
                        adapter,
                        resource.recovery_timeout_ms,
                    );
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
            }
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::ReplacementInstall;
        if self
            .host
            .rebind_configured_transport(resource_key, Box::new(adapter), self.clock.now())
            .is_err()
        {
            self.retire_failed_reconnect_best_effort(resource_key, resource.recovery_timeout_ms);
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        {
            let diagnostic = self
                .reconnect_diagnostic
                .as_mut()
                .expect("diagnostic initialized");
            diagnostic.stage = ReconnectStage::CoreRebind;
            diagnostic.core_rebind_crossed = true;
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::CompatibilityProbeEnqueue;
        if self
            .host
            .begin_configured_probes_for_resource(resource_key, self.clock.now())
            .is_err()
        {
            self.retire_failed_reconnect_best_effort(resource_key, resource.recovery_timeout_ms);
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(resource.recovery_timeout_ms);
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::ProbeWaiting;
        loop {
            match self.host.configured_probes_ready_for_resource(resource_key) {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) => {
                    self.reconnect_diagnostic
                        .as_mut()
                        .expect("diagnostic initialized")
                        .stage = ReconnectStage::ProbeFailed;
                    self.retire_failed_reconnect_best_effort(
                        resource_key,
                        resource.recovery_timeout_ms,
                    );
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
            if std::time::Instant::now() >= deadline {
                self.reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized")
                    .stage = ReconnectStage::ProbeFailed;
                self.retire_failed_reconnect_best_effort(
                    resource_key,
                    resource.recovery_timeout_ms,
                );
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            if self.host.service(&self.clock).is_err() {
                self.reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized")
                    .stage = ReconnectStage::ProbeFailed;
                self.retire_failed_reconnect_best_effort(
                    resource_key,
                    resource.recovery_timeout_ms,
                );
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            std::thread::yield_now();
        }
        if self
            .host
            .request_configured_physical_safe(self.clock.now())
            .is_err()
        {
            self.retire_failed_reconnect_best_effort(resource_key, resource.recovery_timeout_ms);
            self.cancel_recorded_lifecycle(pending);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let output_wait_ms = active
            .effective()
            .dto
            .instruments
            .iter()
            .filter_map(|instrument| match instrument {
                crate::configuration::InstrumentDto::Metakon {
                    resource_id: bound,
                    queue_timeout_ms,
                    transaction_timeout_ms,
                    ..
                } if *bound == resource_id => Some(
                    queue_timeout_ms
                        .saturating_add(transaction_timeout_ms.saturating_mul(2))
                        .saturating_add(500),
                ),
                crate::configuration::InstrumentDto::SimpleDevice {
                    resource_id: bound,
                    queue_timeout_ms,
                    transaction_timeout_ms,
                    ..
                } if *bound == resource_id => Some(
                    queue_timeout_ms
                        .saturating_add(transaction_timeout_ms.saturating_mul(2))
                        .saturating_add(500),
                ),
                _ => None,
            })
            .max()
            .unwrap_or(1);
        let output_deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(output_wait_ms);
        while !self
            .host
            .configured_physical_outputs_safe()
            .map_err(|_| LifecycleOperationError::OwnerFailure)?
        {
            if std::time::Instant::now() >= output_deadline
                || self.host.service(&self.clock).is_err()
            {
                self.retire_failed_reconnect_best_effort(
                    resource_key,
                    resource.recovery_timeout_ms,
                );
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            std::thread::yield_now();
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::LifecycleRecorderCommit;
        if let Err(stage) = self.finish_recorded_lifecycle_detailed(pending) {
            self.reconnect_diagnostic
                .as_mut()
                .expect("diagnostic initialized")
                .stage = match stage {
                RecordedLifecycleFailureStage::Commit => ReconnectStage::LifecycleRecorderCommit,
                RecordedLifecycleFailureStage::Durability => ReconnectStage::LifecycleDurability,
            };
            self.retire_failed_reconnect_best_effort(resource_key, resource.recovery_timeout_ms);
            return Err(LifecycleOperationError::RecordingUnavailable);
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::LifecycleDurability;
        if self
            .host
            .activate_configured_resource_after_reconnect(resource_key, self.clock.now())
            .is_err()
        {
            self.retire_failed_reconnect_best_effort(resource_key, resource.recovery_timeout_ms);
            return Err(LifecycleOperationError::OwnerFailure);
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("diagnostic initialized")
            .stage = ReconnectStage::Complete;
        tracing::info!(
            event = "resource_reconnect_complete",
            resource_id,
            binding_generation = generation,
            "resource reconnect completed without controller rearm"
        );
        Ok(ReconnectResourceResult {
            resource_id,
            binding_generation: generation,
        })
    }

    fn reconcile_quarantined_simple_output(
        &mut self,
        active: crate::configuration::FrozenDeployment,
        resource: crate::configuration::ResourceDto,
        expected_binding_generation: u64,
        factory: impl FnOnce(ComSettings, std::time::Instant) -> Result<ComTransport, SerialError>,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        let quarantine = self
            .quarantined_simple_output
            .as_ref()
            .ok_or(LifecycleOperationError::InvalidCandidate)?;
        if quarantine.binding_generation != expected_binding_generation {
            return Err(LifecycleOperationError::Conflict);
        }
        let target_generation = expected_binding_generation
            .checked_add(1)
            .ok_or(LifecycleOperationError::Conflict)?;
        let resource_key = quarantine.resource;
        let instrument = quarantine.actuator.instrument();
        self.reconnect_diagnostic = Some(ReconnectDiagnostic {
            resource_id: resource.id,
            current_generation: expected_binding_generation,
            target_generation,
            stage: ReconnectStage::RecorderReservation,
            com_state: self
                .host
                .configured_resource_executor_state(resource_key)
                .map(executor_com_state),
            serial_error: None,
            old_worker_finished: false,
            replacement_worker_spawned: false,
            os_port_open_confirmed: false,
            open_attempts: 0,
            core_rebind_crossed: false,
            cleanup_failure: None,
        });
        let pending = self.begin_resource_recorded_lifecycle("reconnect_resource", &active)?;
        self.host
            .begin_configured_resource_reconnect(resource_key)
            .map_err(|_| LifecycleOperationError::OwnerFailure)?;

        let retirement_deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(resource.recovery_timeout_ms);
        loop {
            match self
                .host
                .prepare_configured_transport_replacement(resource_key, self.clock.now())
            {
                Ok(true) => {
                    self.reconnect_diagnostic
                        .as_mut()
                        .expect("reconciliation diagnostic")
                        .old_worker_finished = true;
                    break;
                }
                Ok(false) if std::time::Instant::now() < retirement_deadline => {
                    self.reconnect_diagnostic
                        .as_mut()
                        .expect("reconciliation diagnostic")
                        .stage = ReconnectStage::RetireOldPending;
                    self.host
                        .service(&self.clock)
                        .map_err(|_| LifecycleOperationError::TransportUnavailable)?;
                    std::thread::yield_now();
                }
                _ => {
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
            }
        }

        let settings = match com_settings(&resource, target_generation) {
            Ok(settings) => settings,
            Err(error) => {
                self.cancel_recorded_lifecycle(pending);
                return Err(error);
            }
        };
        let open_deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(resource.open_timeout_ms);
        let mut adapter = match factory(settings, open_deadline) {
            Ok(adapter) => adapter,
            Err(_) => {
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::TransportUnavailable);
            }
        };
        loop {
            match adapter.open_status() {
                ComOpenStatus::Ready => break,
                ComOpenStatus::Failed(error) => {
                    self.reconnect_diagnostic
                        .as_mut()
                        .expect("reconciliation diagnostic")
                        .serial_error = Some(error);
                    self.cancel_recorded_lifecycle(pending);
                    self.retire_uninstalled_reconnect_candidate(
                        resource_key,
                        adapter,
                        resource.recovery_timeout_ms,
                    );
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
                ComOpenStatus::Opening if std::time::Instant::now() < open_deadline => {
                    self.host
                        .service(&self.clock)
                        .map_err(|_| LifecycleOperationError::OwnerFailure)?;
                    std::thread::yield_now();
                }
                ComOpenStatus::Opening => {
                    self.cancel_recorded_lifecycle(pending);
                    self.retire_uninstalled_reconnect_candidate(
                        resource_key,
                        adapter,
                        resource.recovery_timeout_ms,
                    );
                    return Err(LifecycleOperationError::TransportUnavailable);
                }
            }
        }
        // Drain the closed executor's bounded terminal evidence before the
        // explicit reconciliation discards the hidden provisional authority.
        // The ready replacement is still uninstalled, so a failure here can
        // retire it while preserving the quarantine evidence.
        loop {
            match self.host.discard_prepared_simple_overlay(vec![instrument]) {
                Ok(()) => break,
                Err(_) if std::time::Instant::now() < retirement_deadline => {
                    self.host
                        .service(&self.clock)
                        .map_err(|_| LifecycleOperationError::OwnerFailure)?;
                    std::thread::yield_now();
                }
                Err(_) => {
                    self.cancel_recorded_lifecycle(pending);
                    self.retire_uninstalled_reconnect_candidate(
                        resource_key,
                        adapter,
                        resource.recovery_timeout_ms,
                    );
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
        }
        self.host
            .install_reconciled_simple_transport(resource_key, Box::new(adapter), self.clock.now())
            .map_err(|_| LifecycleOperationError::OwnerFailure)?;
        self.reconnect_diagnostic
            .as_mut()
            .expect("reconciliation diagnostic")
            .core_rebind_crossed = true;
        self.quarantined_simple_output = None;
        if self.finish_recorded_lifecycle_detailed(pending).is_err() {
            return Err(LifecycleOperationError::RecordingUnavailable);
        }
        self.reconnect_diagnostic
            .as_mut()
            .expect("reconciliation diagnostic")
            .stage = ReconnectStage::Complete;
        Ok(ReconnectResourceResult {
            resource_id: resource.id,
            binding_generation: target_generation,
        })
    }

    fn retire_uninstalled_reconnect_candidate(
        &mut self,
        resource: ResourceId,
        mut candidate: ComTransport,
        timeout_ms: u64,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if candidate.try_shutdown() == lab_core::transport::TransportShutdown::Complete {
                if let Some(diagnostic) = self.reconnect_diagnostic.as_mut() {
                    diagnostic.com_state = Some(ComState::Closed);
                }
                return;
            }
            if std::time::Instant::now() >= deadline {
                if let Some(diagnostic) = self.reconnect_diagnostic.as_mut() {
                    diagnostic.com_state = Some(candidate.snapshot().state);
                }
                self.quarantined_reconnect_candidate = Some((resource, candidate));
                return;
            }
            if self.host.service(&self.clock).is_err() {
                self.record_reconnect_cleanup_issue("candidate_retirement_service_failure");
                tracing::error!("failed uninstalled reconnect candidate cleanup progress");
            }
            std::thread::yield_now();
        }
    }

    fn retire_failed_reconnect_best_effort(&mut self, resource: ResourceId, timeout_ms: u64) {
        if let Err(error) = self.retire_failed_reconnect(resource, timeout_ms) {
            self.record_reconnect_cleanup_failure(error);
        }
    }

    fn record_reconnect_cleanup_failure(&mut self, error: LifecycleOperationError) {
        let reason = match error {
            LifecycleOperationError::OwnerFailure => "retirement_owner_failure",
            LifecycleOperationError::TransportUnavailable => "retirement_transport_failure",
            _ => "retirement_failure",
        };
        self.record_reconnect_cleanup_issue(reason);
        tracing::error!(?error, "failed reconnect cleanup did not complete");
    }

    fn record_reconnect_cleanup_issue(&mut self, reason: &'static str) {
        if let Some(diagnostic) = self.reconnect_diagnostic.as_mut() {
            diagnostic.cleanup_failure.get_or_insert(reason);
        }
    }

    fn retire_failed_reconnect(
        &mut self,
        resource: ResourceId,
        timeout_ms: u64,
    ) -> Result<(), LifecycleOperationError> {
        #[cfg(test)]
        if self.reconnect_cleanup_retire_failure {
            self.record_reconnect_cleanup_issue("retirement_injected_failure");
            return Err(LifecycleOperationError::OwnerFailure);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match self
                .host
                .retire_failed_configured_reconnect(resource, self.clock.now())
            {
                Ok(true) => return Ok(()),
                Ok(false) if std::time::Instant::now() < deadline => {
                    #[cfg(test)]
                    let service_failed = self.reconnect_cleanup_service_failure;
                    #[cfg(not(test))]
                    let service_failed = false;
                    if service_failed || self.host.service(&self.clock).is_err() {
                        self.record_reconnect_cleanup_issue("retirement_service_failure");
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                _ => return Err(LifecycleOperationError::OwnerFailure),
            }
        }
    }
}
