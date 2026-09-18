//! Synchronous command/query routing for the authoritative Runtime owner.
//!
//! Commands mutate the parent `Runtime`; queries clone bounded committed projections.
//! This module owns no state and performs no host, storage, wire, or OS work.

use super::*;

impl Runtime {
    /// Configuration/registration validation failures are atomic.
    /// MeasurementUnavailable commits a failed observation with the attempt time.
    /// Output commands first advance the explicit watchdog: expired authority may
    /// be revoked even when the requested producer action is then rejected.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        match command {
            Command::StageComponent {
                definition,
                replaces,
                at,
            } => self.stage_component(definition, replaces, at, false),
            Command::PrepareComponentReplacement {
                definition,
                replaces,
                at,
            } => self.stage_component(definition, Some(replaces), at, true),
            Command::CommitPreparedComponents { components, at } => {
                self.commit_prepared_components(&components, at)
            }
            Command::DiscardPreparedComponents => {
                self.prepared.clear();
                Ok(CommandResult::PreparedComponentsDiscarded)
            }
            Command::InvokeComponent { component, at } => self.invoke_component(component, at),
            Command::PollComponents { at } => {
                self.poll_components(at)?;
                Ok(CommandResult::ComponentsPolled)
            }
            Command::QuiesceManaged { at } => {
                self.check_output_time(at)?;
                if !self.managed_quiesced {
                    self.managed_quiesced = true;
                    if let Some(staged) = self.staged.take()
                        && let Some(executor) = self.executor.as_mut()
                    {
                        executor.try_cancel(staged.correlation);
                    }
                    self.prepared.clear();
                    let identities: Vec<_> = self.managed.keys().copied().collect();
                    for id in identities {
                        self.fail_component(id, at);
                    }
                    if let Some(executor) = self.executor.as_mut() {
                        executor.begin_shutdown();
                    }
                }
                Ok(CommandResult::ManagedQuiesced)
            }
            Command::Output {
                actuator,
                command,
                at,
            } => {
                self.service_required_recording_deadline(at);
                let ordinary = matches!(
                    command,
                    OutputCommand::Acquire { .. }
                        | OutputCommand::Propose(_)
                        | OutputCommand::AcknowledgeFault
                ) || matches!(command, OutputCommand::BeginDispatch)
                    && self.outputs.get(&actuator).is_some_and(|authority| {
                        authority.snapshot().state != crate::output::OutputState::SafePending
                    });
                if ordinary {
                    self.require_recording_open(at)?;
                }
                if !self.outputs.contains_key(&actuator) {
                    return Err(OutputError::UnknownActuator.into());
                }
                if at < self.output_time {
                    return Err(OutputError::InvalidTime.into());
                }
                // Output commands carry trusted Runtime time. Watchdog transitions
                // occur even if the requested producer action subsequently fails.
                // A client therefore cannot keep another expired owner alive by
                // submitting invalid proposals instead of explicit Tick commands.
                self.output_time = at;
                self.tick_output_watchdogs(at)?;
                if self.warming_on(actuator) {
                    match command {
                        OutputCommand::RequestSafe => {
                            self.cancel_warming(actuator, ControllerState::Paused);
                        }
                        OutputCommand::Trip => {
                            self.cancel_warming(actuator, ControllerState::Failed);
                        }
                        OutputCommand::Acquire { .. } => return Err(OutputError::Busy.into()),
                        OutputCommand::BindProfile(_) => {
                            return Err(OutputError::InvalidState.into());
                        }
                        _ => {}
                    }
                }
                let issued = command.clone();
                let proposed_attempt = match &issued {
                    OutputCommand::Propose(_) => Some(self.allocate_output_attempt()?),
                    OutputCommand::RequestSafe
                    | OutputCommand::Release(_)
                    | OutputCommand::Trip => self.allocate_safe_output_attempt(),
                    _ => None,
                };
                let original_dispatch = self
                    .outputs
                    .get(&actuator)
                    .and_then(|authority| authority.snapshot().in_flight);
                let was_safe_completion =
                    original_dispatch.is_some_and(|dispatch| dispatch.is_safe());
                let requested_before = self
                    .outputs
                    .get(&actuator)
                    .and_then(|authority| authority.snapshot().requested);
                let pending_before = self
                    .outputs
                    .get(&actuator)
                    .is_some_and(|authority| authority.snapshot().pending);
                let prior_attempt = self.output_attempts.get(&actuator).copied();
                let prior_epoch = self
                    .outputs
                    .get(&actuator)
                    .map(|authority| authority.snapshot().epoch);
                let result = self
                    .outputs
                    .get_mut(&actuator)
                    .ok_or(OutputError::UnknownActuator)?
                    .command(command, at);
                self.sync_recording_output_context(actuator);
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        if let (Some(attempt_id), OutputCommand::Propose(proposal)) =
                            (proposed_attempt, &issued)
                        {
                            self.recording_facts.output_proposal(
                                actuator,
                                crate::recording::OutputStage::RejectedBeforeSend,
                                match &proposal.value {
                                    Value::Float(value) if value.is_finite() => Some(*value),
                                    _ => None,
                                },
                                proposal.unit,
                                at,
                                Some(attempt_id),
                            );
                        }
                        if matches!(issued, OutputCommand::BeginDispatch)
                            && matches!(error, Error::Output(OutputError::Expired))
                        {
                            self.recording_facts.output_correlated(
                                actuator,
                                crate::recording::OutputStage::ExpiredBeforeSend,
                                requested_before,
                                at,
                                crate::recording::OutputEvidenceSource::None,
                                self.output_attempts.get(&actuator).copied(),
                                None,
                            );
                        }
                        return Err(error);
                    }
                };
                if let Some(attempt_id) = proposed_attempt {
                    self.output_attempts.insert(actuator, attempt_id);
                }
                let attempt_id = self.output_attempts.get(&actuator).copied();
                if let OutputResult::Dispatched(dispatch) = &result
                    && let Some(attempt_id) = attempt_id
                {
                    self.output_dispatch_attempts
                        .insert(actuator, (dispatch.id(), attempt_id));
                }
                match (&issued, &result) {
                    (
                        OutputCommand::RequestSafe
                        | OutputCommand::Release(_)
                        | OutputCommand::Trip,
                        OutputResult::Updated,
                    ) => {
                        if pending_before {
                            self.recording_facts.output_displaced(
                                actuator,
                                crate::recording::OutputStage::SupersededBeforeSend,
                                requested_before,
                                at,
                                prior_attempt,
                                prior_epoch.expect("accepted output has an authority epoch"),
                            );
                        }
                        self.recording_facts.output_correlated(
                            actuator,
                            crate::recording::OutputStage::Revoked,
                            None,
                            at,
                            crate::recording::OutputEvidenceSource::None,
                            attempt_id,
                            None,
                        );
                        self.recording_facts.output_correlated(
                            actuator,
                            crate::recording::OutputStage::SafeRequested,
                            None,
                            at,
                            crate::recording::OutputEvidenceSource::None,
                            attempt_id,
                            None,
                        );
                    }
                    (OutputCommand::Propose(proposal), OutputResult::Queued) => {
                        self.recording_facts.output_proposal(
                            actuator,
                            crate::recording::OutputStage::Requested,
                            self.outputs
                                .get(&actuator)
                                .and_then(|authority| authority.snapshot().requested),
                            proposal.unit,
                            at,
                            attempt_id,
                        );
                    }
                    (OutputCommand::BeginDispatch, OutputResult::Dispatched(dispatch))
                        if !dispatch.is_safe() =>
                    {
                        self.recording_facts.output_correlated(
                            actuator,
                            crate::recording::OutputStage::Authorized,
                            Some(dispatch.value()),
                            at,
                            crate::recording::OutputEvidenceSource::None,
                            self.output_dispatch_attempts.get(&actuator).and_then(
                                |(id, attempt)| (*id == dispatch.id()).then_some(*attempt),
                            ),
                            Some(dispatch.id()),
                        );
                        self.recording_facts.output_correlated(
                            actuator,
                            crate::recording::OutputStage::SendStarted,
                            Some(dispatch.value()),
                            at,
                            crate::recording::OutputEvidenceSource::VirtualSimulation,
                            self.output_dispatch_attempts.get(&actuator).and_then(
                                |(id, attempt)| (*id == dispatch.id()).then_some(*attempt),
                            ),
                            Some(dispatch.id()),
                        );
                    }
                    (OutputCommand::BeginDispatch, OutputResult::Dispatched(dispatch)) => {
                        self.recording_facts.output_correlated(
                            actuator,
                            crate::recording::OutputStage::SafeSendStarted,
                            Some(dispatch.value()),
                            at,
                            crate::recording::OutputEvidenceSource::VirtualSimulation,
                            attempt_id,
                            Some(dispatch.id()),
                        );
                    }
                    (OutputCommand::Complete { outcome, .. }, OutputResult::Completed) => {
                        let stage = match (was_safe_completion, outcome) {
                            (true, DispatchOutcome::Acknowledged) => {
                                crate::recording::OutputStage::SafeAcknowledged
                            }
                            (true, DispatchOutcome::ReadbackVerified) => {
                                crate::recording::OutputStage::SafeReadbackVerified
                            }
                            (false, DispatchOutcome::Acknowledged) => {
                                crate::recording::OutputStage::Acknowledged
                            }
                            (false, DispatchOutcome::ReadbackVerified) => {
                                crate::recording::OutputStage::ReadbackVerified
                            }
                            (_, DispatchOutcome::Failed) => crate::recording::OutputStage::Failed,
                            (_, DispatchOutcome::Ambiguous) => {
                                crate::recording::OutputStage::Ambiguous
                            }
                        };
                        let dispatch = original_dispatch
                            .expect("successful completion had an in-flight dispatch");
                        self.recording_facts.output_completion(
                            actuator,
                            stage,
                            Some(dispatch.value()),
                            at,
                            crate::recording::OutputEvidenceSource::VirtualSimulation,
                            match &issued {
                                OutputCommand::Complete { dispatch_id, .. } => {
                                    self.output_dispatch_attempts.get(&actuator).and_then(
                                        |(id, attempt)| (*id == *dispatch_id).then_some(*attempt),
                                    )
                                }
                                _ => None,
                            },
                            dispatch.id(),
                            dispatch.epoch(),
                        );
                    }
                    _ => {}
                }
                if let OutputResult::Dispatched(dispatch) = result {
                    self.apply_virtual_dispatch(actuator, dispatch.value())?;
                    Ok(CommandResult::Output(OutputResult::Dispatched(dispatch)))
                } else {
                    Ok(CommandResult::Output(result))
                }
            }
            Command::RegisterVirtual(config) => {
                let id = config.id;
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = VirtualInstrument::new(config)?;
                let actuator = ActuatorId::new(id, crate::HEATER_POWER);
                let parameter = instrument.parameter(crate::HEATER_POWER)?;
                if parameter.role != crate::ParameterRole::Actuator
                    || parameter.write_effect != crate::WriteEffect::OutputAffecting
                {
                    return Err(OutputError::UnknownActuator.into());
                }
                let authority =
                    OutputAuthority::new(actuator, parameter.value_spec.clone(), parameter.unit)?;
                self.instruments.insert(id, instrument);
                self.outputs.insert(actuator, authority);
                self.sync_recording_output_context(actuator);
                Ok(CommandResult::Registered(id))
            }
            Command::PublishVirtualMeasurement {
                instrument,
                parameter,
                value,
                expected_generation,
                at,
            } => {
                if expected_generation != 1 {
                    return Err(Error::InvalidConfiguration("stale virtual generation"));
                }
                let instance = self
                    .instruments
                    .get_mut(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                let sample = instance.publish(parameter, at, value)?;
                self.recording_facts.measurement(sample.clone(), 1, 1);
                Ok(CommandResult::VirtualMeasurementPublished(sample))
            }
            Command::RegisterThermalPlant(config) => {
                let id = config.id;
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = ThermalPlantInstrument::new(config)?;
                let actuator = ActuatorId::new(id, crate::HEATER_POWER);
                let parameter = instrument
                    .descriptor
                    .parameter(crate::HEATER_POWER)
                    .ok_or(OutputError::UnknownActuator)?;
                let authority =
                    OutputAuthority::new(actuator, parameter.value_spec.clone(), parameter.unit)?;
                self.thermal_plants.insert(id, instrument);
                self.outputs.insert(actuator, authority);
                self.sync_recording_output_context(actuator);
                Ok(CommandResult::Registered(id))
            }
            Command::RestartThermalPlant {
                instrument,
                config,
                expected_generation,
                at: _,
            } => {
                if config.id != instrument {
                    return Err(Error::InvalidConfiguration(
                        "model restart must retain instrument identity",
                    ));
                }
                let current = self
                    .thermal_plants
                    .get(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                if current.generation != expected_generation {
                    return Err(Error::InvalidConfiguration("stale model generation"));
                }
                if self.controllers.values().any(|controller| {
                    (controller.config.input.instrument() == instrument
                        || controller.config.output.instrument() == instrument)
                        && matches!(
                            controller.state,
                            ControllerState::Warming | ControllerState::Running
                        )
                }) {
                    return Err(Error::InvalidConfiguration(
                        "dependent controller must be stopped before model restart",
                    ));
                }
                let actuator = ActuatorId::new(instrument, crate::HEATER_POWER);
                if self.outputs.get(&actuator).is_some_and(|output| {
                    let snapshot = output.snapshot();
                    snapshot.lease.is_some() || snapshot.pending || snapshot.in_flight.is_some()
                }) {
                    return Err(Error::InvalidConfiguration(
                        "model restart requires revoked output authority",
                    ));
                }
                let generation = expected_generation
                    .checked_add(1)
                    .ok_or(Error::InvalidConfiguration("model generation exhausted"))?;
                let mut replacement = ThermalPlantInstrument::new(config)?;
                replacement.generation = generation;
                self.thermal_plants.insert(instrument, replacement);
                Ok(CommandResult::ModelRestarted {
                    instrument,
                    generation,
                })
            }
            Command::RegisterReference(config) => {
                let id = config.id();
                if self.references.contains_key(&id) {
                    return Err(ControllerError::DuplicateReference.into());
                }
                if self.references.len() >= MAX_REFERENCES {
                    return Err(ControllerError::InvalidConfiguration.into());
                }
                let reference = RuntimeReference::new(config)
                    .map_err(|_| ControllerError::InvalidConfiguration)?;
                self.references.insert(id, reference);
                self.capture_reference_fact(
                    id,
                    match config {
                        ReferenceConfig::Fixed { .. } => Duration::ZERO,
                        ReferenceConfig::Ramp { at, .. } => at,
                    },
                );
                Ok(CommandResult::ReferenceRegistered(id))
            }
            Command::EvaluateReference { reference, at } => {
                let result = self
                    .references
                    .get_mut(&reference)
                    .ok_or(ControllerError::UnknownReference.into())
                    .and_then(|item| {
                        item.value_at(at)
                            .map(CommandResult::ReferenceEvaluated)
                            .map_err(map_reference_error)
                            .map_err(Into::into)
                    });
                if result.is_ok() {
                    self.capture_reference_fact(reference, at);
                }
                result
            }
            Command::RetuneRampReference {
                reference,
                target,
                rate,
                expected_revision,
                at,
            } => {
                let result = self
                    .references
                    .get_mut(&reference)
                    .ok_or(ControllerError::UnknownReference.into())
                    .and_then(|item| {
                        item.retune(target, rate, expected_revision, at)
                            .map(CommandResult::ReferenceRetuned)
                            .map_err(map_reference_error)
                            .map_err(Into::into)
                    });
                if result.is_ok() {
                    self.capture_reference_fact(reference, at);
                }
                result
            }
            Command::ReconfigureReference {
                reference,
                config,
                expected_revision,
            } => {
                let current = self
                    .references
                    .get(&reference)
                    .ok_or(ControllerError::UnknownReference)?;
                let candidate_unit = match config {
                    ReferenceConfig::Fixed { unit, .. } | ReferenceConfig::Ramp { unit, .. } => {
                        unit
                    }
                };
                if current.unit() != candidate_unit
                    || self.controllers.values().any(|controller| {
                        controller.config.reference == reference
                            && matches!(
                                controller.state,
                                ControllerState::Warming | ControllerState::Running
                            )
                    })
                {
                    return Err(ControllerError::InvalidConfiguration.into());
                }
                let snapshot = self
                    .references
                    .get_mut(&reference)
                    .expect("checked above")
                    .reconfigure(config, expected_revision)
                    .map_err(map_reference_error)?;
                self.capture_reference_fact(reference, self.output_time);
                Ok(CommandResult::ReferenceConfigured(snapshot))
            }
            Command::RegisterController(config) => {
                let id = config.id;
                if self.controllers.contains_key(&id) {
                    return Err(ControllerError::DuplicateController.into());
                }
                if self.controllers.len() >= MAX_CONTROLLERS {
                    return Err(ControllerError::InvalidConfiguration.into());
                }
                let controller = NativeController::new(config)?;
                let snapshot = controller.snapshot();
                self.controllers.insert(id, controller);
                self.recording_facts.controller(
                    id,
                    snapshot.state,
                    snapshot.config_revision,
                    Some(config.pid),
                    self.output_time,
                );
                Ok(CommandResult::ControllerUpdated(snapshot))
            }
            Command::ConfigureControllerPid {
                controller,
                pid,
                expected_revision,
            } => self.configure_controller_pid(controller, pid, expected_revision),
            Command::ReconfigureController {
                controller,
                config,
                expected_revision,
            } => self.reconfigure_controller(controller, config, expected_revision),
            Command::PrepareController(id) => {
                let result = self.prepare_controller(id);
                if result.is_ok() {
                    self.capture_controller_fact(id, self.output_time);
                }
                result
            }
            Command::StartController { controller, at } => {
                self.require_recording_open(at)?;
                let result =
                    self.start_or_resume_controller(controller, at, ControllerState::Ready);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::TickController { controller, at } => {
                self.require_recording_open(at)?;
                let result = self.tick_controller(controller, at);
                self.capture_controller_fact(controller, at);
                result
            }
            Command::PauseController { controller, at } => {
                let result = self.pause_controller(controller, at);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::ResumeController { controller, at } => {
                self.require_recording_open(at)?;
                let result =
                    self.start_or_resume_controller(controller, at, ControllerState::Paused);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::ResetFailedController { controller, at } => {
                let result = self.reset_failed_controller(controller, at);
                if result.is_ok() {
                    self.capture_controller_fact(controller, at);
                }
                result
            }
            Command::InjectPlantMeasurementFailure { instrument, at } => {
                let plant = self
                    .thermal_plants
                    .get_mut(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                let result = plant.inject_failure(at);
                if result.is_ok()
                    && let Some(sample) = plant.signal.latest().cloned()
                {
                    self.recording_facts
                        .measurement(sample, plant.generation, 1);
                }
                result.map(CommandResult::MeasurementFailed)
            }
            Command::RegisterMetakon(config) => {
                let id = config.definition.id;
                if self.contains_instrument(id) {
                    return Err(Error::DuplicateInstrument(id));
                }
                if self.instrument_count() >= MAX_INSTRUMENTS {
                    return Err(Error::InvalidConfiguration("instrument limit reached (64)"));
                }
                let instrument = MetakonInstrument::new(config)?;
                let mut authorities = Vec::new();
                for parameter in &instrument.descriptor.parameters {
                    if parameter.role == crate::ParameterRole::Actuator {
                        let actuator = ActuatorId::new(id, parameter.id);
                        authorities.push((
                            actuator,
                            OutputAuthority::new(
                                actuator,
                                parameter.value_spec.clone(),
                                parameter.unit,
                            )?,
                        ));
                    }
                }
                self.metakon_instruments.insert(id, instrument);
                let actuators: Vec<_> = authorities.iter().map(|(actuator, _)| *actuator).collect();
                self.outputs.extend(authorities);
                for actuator in actuators {
                    self.sync_recording_output_context(actuator);
                }
                Ok(CommandResult::Registered(id))
            }
            Command::QueueMetakonRead {
                instrument,
                parameter,
                at,
                queue_ttl,
                timeout,
            } => {
                self.check_transport_time(at)?;
                let instance = self
                    .metakon_instruments
                    .get(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                let definition = instance.definition.parameter_definition(parameter).ok_or(
                    Error::UnknownParameter {
                        instrument,
                        parameter,
                    },
                )?;
                if definition.access == crate::AccessMode::WriteOnly {
                    return Err(Error::OperationNotAllowed(parameter));
                }
                let binding = instance.binding;
                let address = Address::new(
                    binding.device,
                    binding.channel,
                    definition.operation.register(),
                );
                let (value_type, writable) = match definition.operation {
                    KnownOperation::ChannelType => (MetakonType::U8, false),
                    KnownOperation::Temperature => (MetakonType::I16, false),
                    KnownOperation::Output => (MetakonType::I8, true),
                };
                let expected = ExpectedRead::new(address, value_type, true, writable);
                let deadline = at
                    .checked_add(queue_ttl)
                    .ok_or(TransportError::InvalidTransaction)?;
                let request = encode_read(address)?;
                let executor = self
                    .resources
                    .get_mut(&binding.resource)
                    .ok_or(TransportError::UnknownResource)?;
                let transaction = executor.enqueue_read(
                    request.as_bytes(),
                    expected.frame_len(),
                    at,
                    deadline,
                    timeout,
                    true,
                    binding.binding_generation,
                    binding.mapping_revision,
                )?;
                self.pending_reads.insert(
                    (binding.resource, transaction),
                    PendingRead {
                        instrument,
                        parameter,
                        expected,
                        operation: definition.operation,
                        scale: definition.scale,
                        binding_generation: binding.binding_generation,
                        mapping_revision: binding.mapping_revision,
                        failure_published: false,
                    },
                );
                Ok(CommandResult::TransportQueued(transaction))
            }
            Command::QueueMetakonOutput {
                actuator,
                at,
                queue_ttl,
                timeout,
            } => self
                .queue_metakon_output(actuator, at, queue_ttl, timeout)
                .map(CommandResult::TransportQueued),
            Command::PollTransports { at } => {
                self.poll_transports(at)?;
                Ok(CommandResult::TransportsPolled)
            }
            Command::ServiceSafety { at } => {
                self.service_safety(at)?;
                Ok(CommandResult::SafetyServiced)
            }
            Command::RebindMetakon {
                instrument,
                binding,
                at,
            } => {
                if self.controllers.values().any(|controller| {
                    matches!(
                        controller.state,
                        ControllerState::Warming | ControllerState::Running
                    ) && controller.config.output.instrument() == instrument
                }) {
                    return Err(OutputError::Busy.into());
                }
                self.check_output_time(at)?;
                let instance = self
                    .metakon_instruments
                    .get(&instrument)
                    .ok_or(Error::UnknownInstrument(instrument))?;
                if binding.binding_generation == 0
                    || binding.mapping_revision == 0
                    || !self.resources.contains_key(&binding.resource)
                {
                    return Err(Error::InvalidConfiguration("invalid replacement binding"));
                }
                let outputs: Vec<_> = instance
                    .descriptor
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.role == crate::ParameterRole::Actuator)
                    .collect();
                match (outputs.as_slice(), binding.expected_output_unit) {
                    ([], None)
                        if binding.output_queue_ttl.is_none()
                            && binding.output_timeout.is_none() => {}
                    ([output], Some(unit))
                        if output.unit == unit
                            && binding.output_queue_ttl.is_some()
                            && binding.output_timeout.is_some() => {}
                    _ => {
                        return Err(Error::InvalidConfiguration(
                            "output binding unit does not match descriptor",
                        ));
                    }
                }
                let mut replacements = Vec::new();
                for parameter in outputs {
                    let actuator = ActuatorId::new(instrument, parameter.id);
                    replacements.push((
                        actuator,
                        OutputAuthority::new(
                            actuator,
                            parameter.value_spec.clone(),
                            parameter.unit,
                        )?,
                    ));
                }
                self.metakon_instruments
                    .get_mut(&instrument)
                    .expect("validated above")
                    .binding = binding;
                let invalidated = {
                    let instance = self
                        .metakon_instruments
                        .get_mut(&instrument)
                        .expect("validated above");
                    let descriptors: Vec<_> = instance
                        .descriptor
                        .parameters
                        .iter()
                        .filter_map(|parameter| {
                            parameter.signal.map(|signal| (signal, parameter.unit))
                        })
                        .collect();
                    let mut samples = Vec::with_capacity(descriptors.len());
                    for (signal, unit) in descriptors {
                        let sample = Sample::unavailable(
                            signal,
                            unit,
                            at,
                            crate::MeasurementFailure::Transport,
                        );
                        instance
                            .signals
                            .get_mut(&signal)
                            .expect("validated signal")
                            .push(sample.clone())?;
                        samples.push(sample);
                    }
                    samples
                };
                for sample in invalidated {
                    self.recording_facts.measurement(
                        sample,
                        binding.binding_generation,
                        binding.mapping_revision,
                    );
                }
                self.pending_output_writes
                    .retain(|(resource, _), _| *resource != binding.resource);
                self.pending_output_readbacks
                    .retain(|_, pending| pending.intent.actuator.instrument() != instrument);
                self.outputs.extend(replacements);
                Ok(CommandResult::Registered(instrument))
            }
            Command::ReconfigureMetakon {
                config,
                expected_binding_generation,
                expected_mapping_revision,
                at,
            } => {
                let id = config.definition.id;
                self.check_output_time(at)?;
                let current = self
                    .metakon_instruments
                    .get(&id)
                    .ok_or(Error::UnknownInstrument(id))?;
                if current.binding.binding_generation != expected_binding_generation
                    || current.binding.mapping_revision != expected_mapping_revision
                    || config.binding.binding_generation
                        != expected_binding_generation.checked_add(1).unwrap_or(0)
                    || config.binding.mapping_revision
                        != expected_mapping_revision.checked_add(1).unwrap_or(0)
                    || !self.resources.contains_key(&config.binding.resource)
                {
                    return Err(Error::InvalidConfiguration(
                        "stale or invalid Metakon replacement",
                    ));
                }
                if self.controllers.values().any(|controller| {
                    matches!(
                        controller.state,
                        ControllerState::Warming | ControllerState::Running
                    ) && (controller.config.input.instrument() == id
                        || controller.config.output.instrument() == id)
                }) {
                    return Err(OutputError::Busy.into());
                }
                let mut candidate = MetakonInstrument::new(config)?;
                let replacements: Vec<_> = candidate
                    .descriptor
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.role == crate::ParameterRole::Actuator)
                    .map(|parameter| {
                        let actuator = ActuatorId::new(id, parameter.id);
                        Ok((
                            actuator,
                            OutputAuthority::new(
                                actuator,
                                parameter.value_spec.clone(),
                                parameter.unit,
                            )?,
                        ))
                    })
                    .collect::<Result<_, Error>>()?;
                let binding = candidate.binding;
                let descriptors: Vec<_> = candidate
                    .descriptor
                    .parameters
                    .iter()
                    .filter_map(|parameter| parameter.signal.map(|signal| (signal, parameter.unit)))
                    .collect();
                let mut invalidated = Vec::with_capacity(descriptors.len());
                for (signal, unit) in descriptors {
                    let sample =
                        Sample::unavailable(signal, unit, at, crate::MeasurementFailure::Transport);
                    candidate
                        .signals
                        .get_mut(&signal)
                        .expect("validated signal")
                        .push(sample.clone())?;
                    invalidated.push(sample);
                }
                self.pending_reads
                    .retain(|_, pending| pending.instrument != id);
                self.pending_output_writes
                    .retain(|(resource, _), _| *resource != binding.resource);
                self.pending_output_readbacks
                    .retain(|_, pending| pending.intent.actuator.instrument() != id);
                self.metakon_instruments.insert(id, candidate);
                self.outputs.extend(replacements);
                for sample in invalidated {
                    self.recording_facts.measurement(
                        sample,
                        binding.binding_generation,
                        binding.mapping_revision,
                    );
                }
                Ok(CommandResult::Registered(id))
            }
            Command::RenameInstrument { instrument, name } => {
                validate_name(&name)?;
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    instance.descriptor.name = name;
                } else if let Some(instance) = self.metakon_instruments.get_mut(&instrument) {
                    instance.descriptor.name = name;
                } else if let Some(instance) = self.thermal_plants.get_mut(&instrument) {
                    instance.descriptor.name = name;
                } else {
                    return Err(Error::UnknownInstrument(instrument));
                }
                Ok(CommandResult::Renamed(instrument))
            }
            Command::ConfigureParameter {
                instrument,
                parameter,
                value,
            } => {
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    instance.configure(parameter, value)?;
                } else if self.metakon_instruments.contains_key(&instrument)
                    || self.thermal_plants.contains_key(&instrument)
                {
                    return Err(Error::OperationNotAllowed(parameter));
                } else {
                    return Err(Error::UnknownInstrument(instrument));
                }
                Ok(CommandResult::Configured {
                    instrument,
                    parameter,
                })
            }
            Command::RefreshMeasurement {
                instrument,
                parameter,
                at,
            } => {
                if let Some(instance) = self.instruments.get_mut(&instrument) {
                    let result = instance.refresh(parameter, at);
                    if parameter == TEMPERATURE
                        && let Some(sample) = instance.signal.latest().cloned()
                        && sample.at() == at
                    {
                        self.recording_facts.measurement(sample, 1, 1);
                    }
                    result.map(CommandResult::MeasurementRefreshed)
                } else if let Some(instance) = self.thermal_plants.get_mut(&instrument) {
                    if parameter != TEMPERATURE {
                        return Err(Error::OperationNotAllowed(parameter));
                    }
                    let result = instance.refresh(at);
                    if let Some(sample) = instance.signal.latest().cloned()
                        && sample.at() == at
                    {
                        self.recording_facts
                            .measurement(sample, instance.generation, 1);
                    }
                    result.map(CommandResult::MeasurementRefreshed)
                } else if self.metakon_instruments.contains_key(&instrument) {
                    Err(Error::OperationNotAllowed(parameter))
                } else {
                    Err(Error::UnknownInstrument(instrument))
                }
            }
        }
    }

    /// Reads owned snapshots only. Never refreshes, reads a clock or advances simulation.
    pub fn query(&self, query: Query) -> Result<QueryResult, Error> {
        match query {
            Query::Component(id) => self
                .managed
                .get(&id)
                .map(|component| QueryResult::Component(component.snapshot()))
                .ok_or(ComponentError::Unknown.into()),
            Query::Output(id) => self
                .outputs
                .get(&id)
                .map(|authority| QueryResult::Output(authority.snapshot()))
                .ok_or(OutputError::UnknownActuator.into()),
            Query::Transport(id) => self
                .resources
                .get(&id)
                .map(|executor| QueryResult::Transport(executor.snapshot()))
                .ok_or(TransportError::UnknownResource.into()),
            Query::Controller(id) => self
                .controllers
                .get(&id)
                .map(|controller| QueryResult::Controller(controller.snapshot()))
                .ok_or(ControllerError::UnknownController.into()),
            Query::ControllerConfig(id) => self
                .controllers
                .get(&id)
                .map(|controller| QueryResult::ControllerConfig(controller.config))
                .ok_or(ControllerError::UnknownController.into()),
            Query::Reference(id) => self
                .references
                .get(&id)
                .map(|reference| QueryResult::Reference(reference.snapshot()))
                .ok_or(ControllerError::UnknownReference.into()),
            Query::Discover => {
                let mut descriptors: Vec<_> = self
                    .instruments
                    .values()
                    .map(|instrument| instrument.descriptor.clone())
                    .chain(
                        self.metakon_instruments
                            .values()
                            .map(|instrument| instrument.descriptor.clone()),
                    )
                    .chain(
                        self.thermal_plants
                            .values()
                            .map(|instrument| instrument.descriptor.clone()),
                    )
                    .chain(
                        self.managed
                            .values()
                            .map(|component| component.descriptor.clone()),
                    )
                    .collect();
                descriptors.sort_by_key(|descriptor| descriptor.id);
                Ok(QueryResult::Instruments(descriptors))
            }
            Query::DescribeInstrument(id) => {
                Ok(QueryResult::Descriptor(self.descriptor(id)?.clone()))
            }
            Query::GetInstrumentState(id) => {
                if let Some(instrument) = self.instruments.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        generation: 1,
                        configured: instrument.configured(),
                        observations: vec![ParameterObservation {
                            parameter: TEMPERATURE,
                            signal: SignalId::new(id, TEMPERATURE),
                            latest: instrument.signal.latest().cloned(),
                        }],
                    }))
                } else if let Some(instrument) = self.metakon_instruments.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        generation: instrument.binding.binding_generation,
                        configured: instrument.configured(),
                        observations: instrument
                            .descriptor
                            .parameters
                            .iter()
                            .filter_map(|parameter| {
                                parameter.signal.map(|signal| ParameterObservation {
                                    parameter: parameter.id,
                                    signal,
                                    latest: instrument
                                        .signals
                                        .get(&signal)
                                        .and_then(SignalBuffer::latest)
                                        .cloned(),
                                })
                            })
                            .collect(),
                    }))
                } else if let Some(instrument) = self.thermal_plants.get(&id) {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        generation: instrument.generation,
                        configured: instrument.configured(),
                        observations: vec![ParameterObservation {
                            parameter: TEMPERATURE,
                            signal: SignalId::new(id, TEMPERATURE),
                            latest: instrument.signal.latest().cloned(),
                        }],
                    }))
                } else if let Some(component) = self
                    .managed
                    .values()
                    .find(|item| item.definition.manifest.instrument == id)
                {
                    Ok(QueryResult::State(InstrumentState {
                        instrument: id,
                        generation: component.generation,
                        configured: vec![],
                        observations: vec![ParameterObservation {
                            parameter: component.definition.manifest.parameter,
                            signal: SignalId::new(id, component.definition.manifest.parameter),
                            latest: component.signal.latest().cloned(),
                        }],
                    }))
                } else {
                    Err(Error::UnknownInstrument(id))
                }
            }
            Query::GetLatestSignal(id) => {
                Ok(QueryResult::Latest(self.signal(id)?.latest().cloned()))
            }
            Query::GetSignalWindow(id) => Ok(QueryResult::Window(self.signal(id)?.window())),
        }
    }
}
