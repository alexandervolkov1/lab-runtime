//! Explicit resource reconnect, retirement, probe, generation fencing and release.
//!
//! These methods advance the single ServiceHost owner declared in the parent
//! module; no reconnect, deployment, or shutdown state is copied into a manager.

use super::*;

impl ServiceHost {
    /// Explicitly retire and reopen one configured read-only COM resource.
    /// Port enumeration never calls this operation and no write retry is implied.
    pub fn reconnect_resource(
        &mut self,
        resource_id: u64,
        expected_binding_generation: u64,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        self.reconnect_resource_with_factory(
            resource_id,
            expected_binding_generation,
            ComTransport::open_windows_with_transient_retry,
        )
    }

    pub(super) fn reconnect_resource_with_factory(
        &mut self,
        resource_id: u64,
        expected_binding_generation: u64,
        factory: impl FnOnce(ComSettings, std::time::Instant) -> Result<ComTransport, SerialError>,
    ) -> Result<ReconnectResourceResult, LifecycleOperationError> {
        tracing::info!(
            event = "resource_reconnect_requested",
            resource_id,
            expected_binding_generation,
            "explicit resource reconnect requested"
        );
        self.reconnect_diagnostic = None;
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
        let instrument = active
            .effective()
            .dto
            .instruments
            .iter()
            .find_map(|instrument| match instrument {
                crate::configuration::InstrumentDto::Metakon {
                    id,
                    resource_id: bound,
                    ..
                } if *bound == resource_id => Some(*id),
                _ => None,
            })
            .ok_or(LifecycleOperationError::InvalidCandidate)?;
        let current = self
            .host
            .configured_binding_generation(instrument)
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
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
                    let _ =
                        self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
                    self.cancel_recorded_lifecycle(pending);
                    return Err(LifecycleOperationError::OwnerFailure);
                }
            }
            if std::time::Instant::now() >= deadline {
                self.reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized")
                    .stage = ReconnectStage::ProbeFailed;
                let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
                self.cancel_recorded_lifecycle(pending);
                return Err(LifecycleOperationError::OwnerFailure);
            }
            if self.host.service(&self.clock).is_err() {
                self.reconnect_diagnostic
                    .as_mut()
                    .expect("diagnostic initialized")
                    .stage = ReconnectStage::ProbeFailed;
                let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
                let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
            let _ = self.retire_failed_reconnect(resource_key, resource.recovery_timeout_ms);
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
            let _ = self.host.service(&self.clock);
            std::thread::yield_now();
        }
    }

    fn retire_failed_reconnect(
        &mut self,
        resource: ResourceId,
        timeout_ms: u64,
    ) -> Result<(), LifecycleOperationError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match self
                .host
                .retire_failed_configured_reconnect(resource, self.clock.now())
            {
                Ok(true) => return Ok(()),
                Ok(false) if std::time::Instant::now() < deadline => {
                    if self.host.service(&self.clock).is_err() {
                        return Err(LifecycleOperationError::OwnerFailure);
                    }
                    std::thread::yield_now();
                }
                _ => return Err(LifecycleOperationError::OwnerFailure),
            }
        }
    }
}
