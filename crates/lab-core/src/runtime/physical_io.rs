//! Physical acquisition, output transaction correlation and transport retirement.
//!
//! READ decode commits observations only through the parent `Runtime`. WRITE progression
//! remains `OutputProposal -> OutputAuthority -> ResourceExecutor -> final authority and
//! generation recheck -> Metakon WRITE -> ACK -> separate readback`. Requested,
//! authorized, send-started, ACK, readback and physical effect are distinct; ambiguous
//! started writes are not blindly retried.

use super::*;

impl Runtime {
    /// Fence resource work and make one nonblocking retirement attempt.
    pub fn shutdown_transport(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<TransportShutdown, Error> {
        self.check_transport_time(at)?;
        let mut executor = self
            .resources
            .remove(&resource)
            .ok_or(TransportError::UnknownResource)?;
        let status = executor.try_shutdown();
        let mut events = Vec::with_capacity(MAX_QUEUED_TRANSACTIONS + 2);
        while let Some(event) = executor.take_event() {
            events.push(event);
        }
        self.resources.insert(resource, executor);
        for event in events {
            self.handle_transport_event(resource, event, at)?;
        }
        Ok(status)
    }

    pub(super) fn deliver_physical(
        &mut self,
        actuator: ActuatorId,
        proposal: OutputProposal,
        at: Duration,
    ) -> Result<(), Error> {
        let attempt_id = self.allocate_output_attempt()?;
        self.outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .command(OutputCommand::Propose(proposal), at)?;
        self.output_attempts.insert(actuator, attempt_id);
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::Requested,
            self.outputs
                .get(&actuator)
                .and_then(|output| output.snapshot().requested),
            at,
            crate::recording::OutputEvidenceSource::None,
            Some(attempt_id),
            None,
        );
        self.require_recording_open(at)?;
        let binding = self
            .metakon_instruments
            .get(&actuator.instrument())
            .ok_or(OutputError::UnknownActuator)?
            .binding;
        self.queue_metakon_output(
            actuator,
            at,
            binding.output_queue_ttl.ok_or(Error::InvalidConfiguration(
                "physical output timing missing",
            ))?,
            binding.output_timeout.ok_or(Error::InvalidConfiguration(
                "physical output timing missing",
            ))?,
        )?;
        Ok(())
    }

    pub(super) fn deliver_simulated(
        &mut self,
        actuator: ActuatorId,
        proposal: OutputProposal,
        at: Duration,
    ) -> Result<(), Error> {
        let attempt_id = self.allocate_output_attempt()?;
        self.outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .command(OutputCommand::Propose(proposal), at)?;
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::Requested,
            self.outputs
                .get(&actuator)
                .and_then(|output| output.snapshot().requested),
            at,
            crate::recording::OutputEvidenceSource::None,
            Some(attempt_id),
            None,
        );
        // A one-call PID can propose and send in the same owner unit. Recheck
        // capture capacity before that unit reaches its ordinary send boundary.
        self.require_recording_open(at)?;
        let dispatch = match self
            .outputs
            .get_mut(&actuator)
            .expect("authority exists after proposal")
            .command(OutputCommand::BeginDispatch, at)?
        {
            OutputResult::Dispatched(dispatch) => dispatch,
            _ => unreachable!("BeginDispatch has one successful result kind"),
        };
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::Authorized,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::None,
            Some(attempt_id),
            Some(dispatch.id()),
        );
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::SendStarted,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
            Some(attempt_id),
            Some(dispatch.id()),
        );
        if let Err(error) = self.apply_virtual_dispatch(actuator, dispatch.value()) {
            self.outputs
                .get_mut(&actuator)
                .expect("authority exists after dispatch")
                .command(
                    OutputCommand::Complete {
                        dispatch_id: dispatch.id(),
                        outcome: DispatchOutcome::Failed,
                    },
                    at,
                )?;
            self.recording_facts.output_correlated(
                actuator,
                crate::recording::OutputStage::Failed,
                Some(dispatch.value()),
                at,
                crate::recording::OutputEvidenceSource::VirtualSimulation,
                Some(attempt_id),
                Some(dispatch.id()),
            );
            return Err(error);
        }
        self.outputs
            .get_mut(&actuator)
            .expect("authority exists after dispatch")
            .command(
                OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
                at,
            )?;
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::ReadbackVerified,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
            Some(attempt_id),
            Some(dispatch.id()),
        );
        Ok(())
    }

    pub(super) fn allocate_output_attempt(&mut self) -> Result<u64, Error> {
        self.next_output_attempt = self
            .next_output_attempt
            .checked_add(1)
            .ok_or(OutputError::CounterExhausted)?;
        Ok(self.next_output_attempt)
    }

    // Safe control is allowed to proceed when audit correlation identities
    // exhaust; the outbox records an honest first loss and Required fails closed.
    pub(super) fn allocate_safe_output_attempt(&mut self) -> Option<u64> {
        match self.allocate_output_attempt() {
            Ok(attempt) => Some(attempt),
            Err(_) => {
                self.recording_facts.lose_correlation();
                None
            }
        }
    }

    pub(super) fn complete_simulated_safe(
        &mut self,
        actuator: ActuatorId,
        at: Duration,
    ) -> Result<(), Error> {
        let dispatch = match self
            .outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .command(OutputCommand::BeginDispatch, at)?
        {
            OutputResult::Dispatched(dispatch) => dispatch,
            _ => unreachable!("BeginDispatch has one successful result kind"),
        };
        let attempt_id = self.output_attempts.get(&actuator).copied();
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::SafeSendStarted,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
            attempt_id,
            Some(dispatch.id()),
        );
        self.apply_virtual_dispatch(actuator, dispatch.value())?;
        self.outputs
            .get_mut(&actuator)
            .expect("authority exists after safe dispatch")
            .command(
                OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
                at,
            )?;
        self.recording_facts.output_correlated(
            actuator,
            crate::recording::OutputStage::SafeReadbackVerified,
            Some(dispatch.value()),
            at,
            crate::recording::OutputEvidenceSource::VirtualSimulation,
            attempt_id,
            Some(dispatch.id()),
        );
        Ok(())
    }

    pub(super) fn apply_virtual_dispatch(
        &mut self,
        actuator: ActuatorId,
        value: f64,
    ) -> Result<(), Error> {
        if let Some(plant) = self.thermal_plants.get_mut(&actuator.instrument())
            && actuator.parameter() == crate::HEATER_POWER
        {
            plant.apply_heater(value)?;
        }
        Ok(())
    }

    pub(super) fn descriptor(&self, id: InstrumentId) -> Result<&InstrumentDescriptor, Error> {
        self.instruments
            .get(&id)
            .map(|instrument| &instrument.descriptor)
            .or_else(|| {
                self.metakon_instruments
                    .get(&id)
                    .map(|instrument| &instrument.descriptor)
            })
            .or_else(|| {
                self.thermal_plants
                    .get(&id)
                    .map(|instrument| &instrument.descriptor)
            })
            .or_else(|| {
                self.managed
                    .values()
                    .find(|item| item.definition.manifest.instrument == id)
                    .map(|item| &item.descriptor)
            })
            .ok_or(Error::UnknownInstrument(id))
    }

    pub(super) fn check_output_time(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.output_time {
            return Err(OutputError::InvalidTime.into());
        }
        self.output_time = at;
        self.tick_output_watchdogs(at)
    }

    // Capture each watchdog transition immediately after its authoritative
    // state mutation. A later failed command must not erase an earlier expiry.
    pub(super) fn tick_output_watchdogs(&mut self, at: Duration) -> Result<(), Error> {
        let actuators: Vec<_> = self.outputs.keys().copied().collect();
        for actuator in actuators {
            let before = self
                .outputs
                .get(&actuator)
                .expect("enumerated output")
                .snapshot();
            self.outputs
                .get_mut(&actuator)
                .expect("enumerated output")
                .tick(at)?;
            let after_epoch = self
                .outputs
                .get(&actuator)
                .expect("enumerated output")
                .snapshot()
                .epoch;
            if after_epoch == before.epoch {
                continue;
            }
            let prior_attempt = self.output_attempts.get(&actuator).copied();
            self.sync_recording_output_context(actuator);
            if before.pending {
                self.recording_facts.output_displaced(
                    actuator,
                    crate::recording::OutputStage::ExpiredBeforeSend,
                    before.requested,
                    at,
                    prior_attempt,
                    before.epoch,
                );
            }
            let safe_attempt = self.allocate_safe_output_attempt();
            if let Some(attempt) = safe_attempt {
                self.output_attempts.insert(actuator, attempt);
            }
            self.recording_facts.output_correlated(
                actuator,
                crate::recording::OutputStage::Revoked,
                None,
                at,
                crate::recording::OutputEvidenceSource::None,
                safe_attempt,
                None,
            );
            self.recording_facts.output_correlated(
                actuator,
                crate::recording::OutputStage::SafeRequested,
                None,
                at,
                crate::recording::OutputEvidenceSource::None,
                safe_attempt,
                None,
            );
        }
        Ok(())
    }

    /// This explicit service needs host scheduling; a finite lease alone cannot send safe bytes.
    /// It does not run EMA/PID or renew a native owner and never waits on component work.
    pub(super) fn service_safety(&mut self, at: Duration) -> Result<(), Error> {
        self.service_required_recording_deadline(at);
        self.check_output_time(at)?;
        let identities: Vec<_> = self.controllers.keys().copied().collect();
        for id in identities {
            let mut controller = self.controllers.remove(&id).expect("listed above");
            let before_state = controller.state;
            let live = matches!(
                controller.state,
                ControllerState::Warming | ControllerState::Running
            );
            if live {
                let timed_out = controller
                    .last_tick
                    .is_some_and(|last| at > last && at - last > controller.config.max_tick_gap);
                let input_failed = self
                    .control_input(controller.config.input, at, controller.config.max_input_age)
                    .is_err();
                let ownership_lost = controller.state == ControllerState::Running
                    && controller.lease.is_some_and(|lease| {
                        self.outputs
                            .get(&controller.config.output)
                            .is_none_or(|authority| authority.snapshot().lease != Some(lease))
                    });
                if timed_out || input_failed || ownership_lost {
                    if controller.state == ControllerState::Warming {
                        controller.state = ControllerState::Failed;
                    } else {
                        let _ = self.fail_controller(&mut controller, at);
                    }
                }
            }
            if controller.state != before_state {
                self.recording_facts.controller(
                    id,
                    controller.state,
                    controller.config_revision,
                    None,
                    at,
                );
            }
            self.controllers.insert(id, controller);
        }
        // Only the known Rust virtual plant may report simulated readback here.
        // A physical Metakon safe request still needs its trusted transport/evidence path.
        let virtual_safe: Vec<_> = self
            .outputs
            .iter()
            .filter(|(actuator, authority)| {
                self.thermal_plants.contains_key(&actuator.instrument())
                    && authority.snapshot().state == crate::output::OutputState::SafePending
                    && authority.snapshot().in_flight.is_none()
            })
            .map(|(actuator, _)| *actuator)
            .collect();
        for actuator in virtual_safe {
            let _ = self.complete_simulated_safe(actuator, at);
        }
        let physical_safe: Vec<_> = self
            .outputs
            .iter()
            .filter(|(actuator, authority)| {
                self.metakon_instruments
                    .contains_key(&actuator.instrument())
                    && authority.snapshot().state == crate::output::OutputState::SafePending
                    && authority.snapshot().in_flight.is_none()
            })
            .filter_map(|(actuator, _)| {
                self.metakon_instruments
                    .get(&actuator.instrument())
                    .and_then(|instrument| {
                        Some((
                            *actuator,
                            instrument.binding.output_queue_ttl?,
                            instrument.binding.output_timeout?,
                        ))
                    })
            })
            .collect();
        for (actuator, queue_ttl, timeout) in physical_safe {
            // A previously admitted safe transaction owns the authority's single
            // reservation and returns Busy here. Before send-start, other bounded
            // admission failures leave the safe request available for a later turn.
            let _ = self.queue_metakon_output(actuator, at, queue_ttl, timeout);
        }
        // Every M3 resource gets one bounded recovery/dispatch opportunity.
        self.poll_transports(at)
    }

    /// Warming is a bounded preparation guard, not an output owner or lease.
    pub(super) fn warming_on(&self, actuator: ActuatorId) -> bool {
        self.controllers.values().any(|controller| {
            controller.state == ControllerState::Warming && controller.config.output == actuator
        })
    }

    pub(super) fn cancel_warming(&mut self, actuator: ActuatorId, state: ControllerState) {
        for controller in self.controllers.values_mut() {
            if controller.state == ControllerState::Warming && controller.config.output == actuator
            {
                controller.state = state;
            }
        }
    }

    pub(super) fn check_transport_time(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.transport_time {
            return Err(TransportError::InvalidTime.into());
        }
        self.transport_time = at;
        Ok(())
    }

    pub(super) fn queue_metakon_output(
        &mut self,
        actuator: ActuatorId,
        at: Duration,
        queue_ttl: Duration,
        timeout: Duration,
    ) -> Result<TransactionId, Error> {
        self.check_transport_time(at)?;
        let deadline = at
            .checked_add(queue_ttl)
            .ok_or(TransportError::InvalidTransaction)?;
        let instrument = self
            .metakon_instruments
            .get(&actuator.instrument())
            .ok_or(OutputError::UnknownActuator)?;
        let scale = instrument
            .definition
            .parameter_definition(actuator.parameter())
            .filter(|definition| definition.operation == KnownOperation::Output)
            .ok_or(OutputError::UnknownActuator)?
            .scale;
        let binding = instrument.binding;
        let intent = self
            .outputs
            .get_mut(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .reserve_transport(
                at,
                deadline,
                self.output_attempts.get(&actuator).copied(),
                binding.binding_generation,
                binding.mapping_revision,
            )?;
        let raw = match encode_scaled_i8(&Value::Float(intent.value), scale) {
            Ok(raw) => raw,
            Err(error) => {
                self.outputs
                    .get_mut(&actuator)
                    .expect("authority reserved the intent")
                    .abort_transport(intent);
                return Err(error.into());
            }
        };
        let address = Address::new(binding.device, binding.channel, 6);
        let request = match encode_write(address, MetakonValue::I8(raw)) {
            Ok(request) => request,
            Err(error) => {
                self.outputs
                    .get_mut(&actuator)
                    .expect("authority reserved the intent")
                    .abort_transport(intent);
                return Err(error.into());
            }
        };
        let enqueue = self
            .resources
            .get_mut(&binding.resource)
            .ok_or(TransportError::UnknownResource)
            .and_then(|executor| {
                executor.enqueue_output(request.as_bytes(), 5, at, deadline, timeout, intent)
            });
        match enqueue {
            Ok(transaction) => {
                self.pending_output_writes
                    .insert((binding.resource, transaction), timeout);
                Ok(transaction)
            }
            Err(error) => {
                self.outputs
                    .get_mut(&actuator)
                    .expect("authority reserved the intent")
                    .abort_transport(intent);
                Err(error.into())
            }
        }
    }

    pub(super) fn poll_transports(&mut self, at: Duration) -> Result<(), Error> {
        self.service_required_recording_deadline(at);
        self.check_transport_time(at)?;
        self.check_output_time(at)?;
        let bound_outputs: Vec<_> = self
            .outputs
            .keys()
            .filter(|actuator| {
                self.metakon_instruments
                    .contains_key(&actuator.instrument())
            })
            .copied()
            .collect();
        for actuator in bound_outputs {
            self.sync_recording_output_context(actuator);
        }
        let resources: Vec<_> = self.resources.keys().copied().collect();
        for resource in resources {
            self.service_required_recording_deadline(at);
            let required_open = !self.required_recording.enabled || self.required_recording.open;
            // Temporarily taking the executor lets the coordinator borrow the
            // authority registry during the serialized final-check/write call.
            let mut executor = self
                .resources
                .remove(&resource)
                .ok_or(TransportError::UnknownResource)?;
            let instruments = &self.metakon_instruments;
            let outputs = &mut self.outputs;
            let recording_facts = &mut self.recording_facts;
            let event = executor.poll_authorized(at, &mut |intent, step| {
                if !intent.safe && !required_open {
                    return Err(());
                }
                let instrument = instruments.get(&intent.actuator.instrument()).ok_or(())?;
                let binding = instrument.binding;
                if binding.resource != resource
                    || binding.binding_generation != intent.binding_generation
                    || binding.mapping_revision != intent.mapping_revision
                {
                    return Err(());
                }
                let authority = outputs.get_mut(&intent.actuator).ok_or(())?;
                match step {
                    AuthorizationStep::Validate => {
                        authority.validate_transport(intent, at).map_err(|_| ())?;
                        Ok(None)
                    }
                    AuthorizationStep::Started => {
                        let dispatch = authority.begin_transport(intent, at).map_err(|_| ())?;
                        if intent.safe {
                            recording_facts.output_transport(
                                intent,
                                resource,
                                crate::recording::OutputStage::SafeSendStarted,
                                at,
                                crate::recording::OutputEvidenceSource::TransportProtocol,
                                Some(dispatch),
                            );
                        } else {
                            recording_facts.output_transport(
                                intent,
                                resource,
                                crate::recording::OutputStage::Authorized,
                                at,
                                crate::recording::OutputEvidenceSource::None,
                                Some(dispatch),
                            );
                            recording_facts.output_transport(
                                intent,
                                resource,
                                crate::recording::OutputStage::SendStarted,
                                at,
                                crate::recording::OutputEvidenceSource::TransportProtocol,
                                Some(dispatch),
                            );
                        }
                        Ok(Some(dispatch))
                    }
                }
            })?;
            self.resources.insert(resource, executor);
            if let Some(event) = event {
                self.handle_transport_event(resource, event, at)?;
            }
        }
        Ok(())
    }

    fn handle_transport_event(
        &mut self,
        resource: ResourceId,
        event: TransportEvent,
        at: Duration,
    ) -> Result<(), Error> {
        match event {
            TransportEvent::ReadUnavailable {
                id,
                binding_generation,
                mapping_revision,
            } => {
                let Some(pending) = self.pending_reads.get(&(resource, id)).cloned() else {
                    return Ok(());
                };
                let generation_matches = pending.binding_generation == binding_generation
                    && pending.mapping_revision == mapping_revision
                    && self
                        .metakon_instruments
                        .get(&pending.instrument)
                        .is_some_and(|instrument| {
                            instrument.binding.resource == resource
                                && instrument.binding.binding_generation == binding_generation
                                && instrument.binding.mapping_revision == mapping_revision
                        });
                if generation_matches {
                    self.push_transport_failure(&pending, at)?;
                    self.record_pending_sample_at(&pending, at);
                    if let Some(current) = self.pending_reads.get_mut(&(resource, id)) {
                        current.failure_published = true;
                    }
                }
            }
            TransportEvent::ReadFenced { id } => {
                self.pending_reads.remove(&(resource, id));
                if let Some(pending) = self.pending_output_readbacks.remove(&(resource, id)) {
                    self.fail_output_readback(resource, pending, at)?;
                }
            }
            TransportEvent::ReadTerminal { record, response } => {
                if let Some(pending) = self.pending_output_readbacks.remove(&(resource, record.id))
                {
                    self.handle_output_readback(resource, pending, record, response, at)?;
                    return Ok(());
                }
                let Some(pending) = self.pending_reads.remove(&(resource, record.id)) else {
                    return Ok(());
                };
                let current = self.metakon_instruments.get(&pending.instrument);
                let generation_matches = current.is_some_and(|instrument| {
                    instrument.binding.resource == resource
                        && instrument.binding.binding_generation == pending.binding_generation
                        && instrument.binding.mapping_revision == pending.mapping_revision
                });
                if !generation_matches {
                    return Ok(());
                }
                if record.outcome == TransactionOutcome::Completed {
                    let decoded = response
                        .as_deref()
                        .ok_or(crate::metakon::CodecError::WrongLength)
                        .and_then(|bytes| decode_read(bytes, pending.expected));
                    match decoded {
                        Ok(value) => self.apply_metakon_value(&pending, value, at)?,
                        Err(_) => {
                            self.push_transport_failure(&pending, at)?;
                            self.resources
                                .get_mut(&resource)
                                .expect("executor reinserted before event handling")
                                .protocol_failure()?;
                        }
                    }
                } else if !pending.failure_published {
                    self.push_transport_failure(&pending, at)?;
                }
                self.record_pending_sample_at(&pending, at);
            }
            TransportEvent::OutputUncertain {
                id,
                intent,
                dispatch,
            } => {
                self.pending_output_writes.remove(&(resource, id));
                if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                    authority.transport_uncertain(dispatch)?;
                    self.recording_facts.output_transport(
                        intent,
                        resource,
                        crate::recording::OutputStage::TransportUncertain,
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                        Some(dispatch),
                    );
                }
            }
            TransportEvent::OutputTerminal {
                intent,
                dispatch,
                record,
                response,
            } => {
                let readback_timeout = self.pending_output_writes.remove(&(resource, record.id));
                let Some(dispatch) = dispatch else {
                    if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                        authority.abort_transport(intent);
                    }
                    return Ok(());
                };
                let current_binding = self
                    .metakon_instruments
                    .get(&intent.actuator.instrument())
                    .map(|instrument| instrument.binding);
                if record.outcome == TransactionOutcome::Completed
                    && current_binding.is_some_and(|binding| {
                        binding.resource == resource
                            && binding.binding_generation == intent.binding_generation
                            && binding.mapping_revision == intent.mapping_revision
                    })
                {
                    let binding = current_binding.expect("checked above");
                    let address = Address::new(binding.device, binding.channel, 6);
                    if response
                        .as_deref()
                        .is_some_and(|bytes| decode_ack(bytes, address).is_ok())
                    {
                        if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                            let requires_readback = authority.requires_readback()?;
                            if requires_readback {
                                authority.acknowledge_transport(dispatch, at)?;
                            } else {
                                authority.complete_transport(
                                    dispatch,
                                    crate::output::DispatchOutcome::Acknowledged,
                                    at,
                                )?;
                            }
                            self.recording_facts.output_transport(
                                intent,
                                resource,
                                if intent.safe {
                                    crate::recording::OutputStage::SafeAcknowledged
                                } else {
                                    crate::recording::OutputStage::Acknowledged
                                },
                                at,
                                crate::recording::OutputEvidenceSource::TransportProtocol,
                                Some(dispatch),
                            );
                            if requires_readback {
                                let timeout =
                                    readback_timeout.ok_or(Error::InvalidConfiguration(
                                        "physical output readback timeout missing",
                                    ))?;
                                let expected =
                                    ExpectedRead::new(address, MetakonType::I8, true, true);
                                let request = encode_read(address)?;
                                let deadline = at
                                    .checked_add(timeout)
                                    .ok_or(TransportError::InvalidTransaction)?;
                                let transaction = self
                                    .resources
                                    .get_mut(&resource)
                                    .expect("executor reinserted before event handling")
                                    .enqueue_read(
                                        request.as_bytes(),
                                        expected.frame_len(),
                                        at,
                                        deadline,
                                        timeout,
                                        false,
                                        intent.binding_generation,
                                        intent.mapping_revision,
                                    );
                                match transaction {
                                    Ok(transaction) => {
                                        let scale = self
                                            .metakon_instruments
                                            .get(&intent.actuator.instrument())
                                            .and_then(|instrument| {
                                                instrument.definition.parameter_definition(
                                                    intent.actuator.parameter(),
                                                )
                                            })
                                            .map(|definition| definition.scale)
                                            .ok_or(OutputError::UnknownActuator)?;
                                        self.pending_output_readbacks.insert(
                                            (resource, transaction),
                                            PendingOutputReadback {
                                                intent,
                                                dispatch,
                                                expected,
                                                scale,
                                            },
                                        );
                                    }
                                    Err(_) => {
                                        let pending = PendingOutputReadback {
                                            intent,
                                            dispatch,
                                            expected,
                                            scale: 1.0,
                                        };
                                        self.fail_output_readback(resource, pending, at)?;
                                    }
                                }
                            }
                        }
                    } else if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                        authority.transport_uncertain(dispatch)?;
                        self.recording_facts.output_transport(
                            intent,
                            resource,
                            crate::recording::OutputStage::TransportUncertain,
                            at,
                            crate::recording::OutputEvidenceSource::TransportProtocol,
                            Some(dispatch),
                        );
                        self.unsettled_outputs.insert(resource, (intent, dispatch));
                        self.resources
                            .get_mut(&resource)
                            .expect("executor reinserted before event handling")
                            .protocol_failure()?;
                    }
                } else if let Some(authority) = self.outputs.get_mut(&intent.actuator) {
                    authority.complete_transport(
                        dispatch,
                        crate::output::DispatchOutcome::Ambiguous,
                        at,
                    )?;
                    self.recording_facts.output_transport(
                        intent,
                        resource,
                        crate::recording::OutputStage::Ambiguous,
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                        Some(dispatch),
                    );
                }
            }
            TransportEvent::BoundaryRecovered => {
                if let Some((intent, dispatch)) = self.unsettled_outputs.remove(&resource)
                    && let Some(authority) = self.outputs.get_mut(&intent.actuator)
                {
                    authority.complete_transport(
                        dispatch,
                        crate::output::DispatchOutcome::Ambiguous,
                        at,
                    )?;
                    self.recording_facts.output_transport(
                        intent,
                        resource,
                        crate::recording::OutputStage::Ambiguous,
                        at,
                        crate::recording::OutputEvidenceSource::TransportProtocol,
                        Some(dispatch),
                    );
                }
            }
            TransportEvent::BoundaryFailed => {}
        }
        Ok(())
    }

    fn handle_output_readback(
        &mut self,
        resource: ResourceId,
        pending: PendingOutputReadback,
        record: crate::transport::TransactionRecord,
        response: Option<Vec<u8>>,
        at: Duration,
    ) -> Result<(), Error> {
        let current_binding = self
            .metakon_instruments
            .get(&pending.intent.actuator.instrument())
            .map(|instrument| instrument.binding);
        let current = current_binding.is_some_and(|binding| {
            binding.resource == resource
                && binding.binding_generation == pending.intent.binding_generation
                && binding.mapping_revision == pending.intent.mapping_revision
        });
        if !current {
            return Ok(());
        }
        if record.outcome != TransactionOutcome::Completed {
            return self.fail_output_readback(resource, pending, at);
        }
        let decoded = response
            .as_deref()
            .ok_or(crate::metakon::CodecError::WrongLength)
            .and_then(|bytes| decode_read(bytes, pending.expected));
        let MetakonValue::I8(raw) = (match decoded {
            Ok(value) => value,
            Err(_) => {
                self.fail_output_readback(resource, pending, at)?;
                self.resources
                    .get_mut(&resource)
                    .expect("executor reinserted before event handling")
                    .protocol_failure()?;
                return Ok(());
            }
        }) else {
            self.fail_output_readback(resource, pending, at)?;
            return Ok(());
        };
        let reported = f64::from(raw) * pending.scale;
        let matched = self
            .outputs
            .get_mut(&pending.intent.actuator)
            .ok_or(OutputError::UnknownActuator)?
            .complete_transport_readback(pending.dispatch, reported, at)?;
        self.recording_facts.output_transport_value(
            pending.intent,
            resource,
            if matched {
                if pending.intent.safe {
                    crate::recording::OutputStage::SafeReadbackVerified
                } else {
                    crate::recording::OutputStage::ReadbackVerified
                }
            } else {
                crate::recording::OutputStage::Failed
            },
            at,
            crate::recording::OutputEvidenceSource::TransportProtocol,
            Some(pending.dispatch),
            reported,
        );
        if !pending.intent.safe {
            self.settle_controller_after_physical_readback(pending.intent.actuator, matched, at)?;
        }
        Ok(())
    }

    fn fail_output_readback(
        &mut self,
        resource: ResourceId,
        pending: PendingOutputReadback,
        at: Duration,
    ) -> Result<(), Error> {
        if let Some(authority) = self.outputs.get_mut(&pending.intent.actuator) {
            authority.fail_transport_readback(pending.dispatch, at)?;
            self.recording_facts.output_transport(
                pending.intent,
                resource,
                crate::recording::OutputStage::Ambiguous,
                at,
                crate::recording::OutputEvidenceSource::TransportProtocol,
                Some(pending.dispatch),
            );
        }
        if !pending.intent.safe {
            self.settle_controller_after_physical_readback(pending.intent.actuator, false, at)?;
        }
        Ok(())
    }

    fn settle_controller_after_physical_readback(
        &mut self,
        actuator: ActuatorId,
        matched: bool,
        at: Duration,
    ) -> Result<(), Error> {
        let controller = self.controllers.iter().find_map(|(id, controller)| {
            (controller.state == ControllerState::Running
                && controller.config.output == actuator
                && controller.lease.is_some())
            .then_some(*id)
        });
        let Some(id) = controller else {
            return Ok(());
        };
        let mut controller = self.controllers.remove(&id).expect("controller was found");
        let renewal = if matched {
            let lease = controller
                .lease
                .expect("running physical controller owns lease");
            self.outputs
                .get_mut(&actuator)
                .ok_or(OutputError::UnknownActuator)?
                .renew_native(lease, controller.config.lease_lifetime, at)
        } else {
            Err(OutputError::StaleLease.into())
        };
        if let Ok(replacement) = renewal {
            controller.lease = Some(replacement);
            controller.latest_output = controller.pid.snapshot().latest;
        } else {
            self.fail_controller(&mut controller, at)?;
            self.recording_facts.controller(
                id,
                controller.state,
                controller.config_revision,
                None,
                at,
            );
        }
        self.controllers.insert(id, controller);
        Ok(())
    }

    fn apply_metakon_value(
        &mut self,
        pending: &PendingRead,
        value: MetakonValue,
        at: Duration,
    ) -> Result<(), Error> {
        let instrument = self
            .metakon_instruments
            .get_mut(&pending.instrument)
            .ok_or(Error::UnknownInstrument(pending.instrument))?;
        let descriptor =
            instrument
                .descriptor
                .parameter(pending.parameter)
                .ok_or(Error::UnknownParameter {
                    instrument: pending.instrument,
                    parameter: pending.parameter,
                })?;
        let Some(signal) = descriptor.signal else {
            return Ok(());
        };
        let domain_value = match pending.operation {
            KnownOperation::ChannelType => match value {
                MetakonValue::U8(3) => Value::Integer(3),
                _ => {
                    instrument
                        .signals
                        .get_mut(&signal)
                        .expect("validated signal")
                        .push(Sample::unavailable(
                            signal,
                            descriptor.unit,
                            at,
                            crate::MeasurementFailure::Transport,
                        ))?;
                    return Ok(());
                }
            },
            KnownOperation::Temperature => match scale_temperature(value, pending.scale)? {
                TemperatureReading::Value(value) => Value::Float(value),
                TemperatureReading::SensorFault => {
                    instrument
                        .signals
                        .get_mut(&signal)
                        .expect("validated signal")
                        .push(Sample::unavailable(
                            signal,
                            descriptor.unit,
                            at,
                            crate::MeasurementFailure::SensorFault,
                        ))?;
                    return Ok(());
                }
            },
            KnownOperation::Output => return Ok(()),
        };
        descriptor.value_spec.validate(&domain_value)?;
        instrument
            .signals
            .get_mut(&signal)
            .expect("validated signal")
            .push(Sample::good(signal, descriptor.unit, at, domain_value))
    }

    fn push_transport_failure(&mut self, pending: &PendingRead, at: Duration) -> Result<(), Error> {
        let instrument = self
            .metakon_instruments
            .get_mut(&pending.instrument)
            .ok_or(Error::UnknownInstrument(pending.instrument))?;
        let descriptor =
            instrument
                .descriptor
                .parameter(pending.parameter)
                .ok_or(Error::UnknownParameter {
                    instrument: pending.instrument,
                    parameter: pending.parameter,
                })?;
        if let Some(signal) = descriptor.signal {
            instrument
                .signals
                .get_mut(&signal)
                .expect("validated signal")
                .push(Sample::unavailable(
                    signal,
                    descriptor.unit,
                    at,
                    crate::MeasurementFailure::Transport,
                ))?;
        }
        Ok(())
    }

    fn record_pending_sample_at(&mut self, pending: &PendingRead, at: Duration) {
        if let Some(instrument) = self.metakon_instruments.get(&pending.instrument)
            && let Some(descriptor) = instrument.descriptor.parameter(pending.parameter)
            && let Some(signal) = descriptor.signal
            && let Some(sample) = instrument
                .signals
                .get(&signal)
                .and_then(|buffer| buffer.latest())
                .cloned()
            && sample.at() == at
        {
            self.recording_facts.measurement(
                sample,
                instrument.binding.binding_generation,
                instrument.binding.mapping_revision,
            );
        }
    }
}
