//! Native Reference/controller lifecycle and controller-to-output proposal progression.
//!
//! Controller state and OutputAuthority remain fields of the parent `Runtime`. The
//! orchestration here never grants physical send authority by itself.

use super::*;

impl Runtime {
    /// Validate all authority/descriptor constraints before touching controller memory.
    pub(super) fn configure_controller_pid(
        &mut self,
        id: ControllerId,
        pid: PidConfig,
        expected_revision: u64,
    ) -> Result<CommandResult, Error> {
        let controller = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?;
        if controller.config_revision != expected_revision {
            return Err(ControllerError::RevisionConflict.into());
        }
        if !matches!(
            controller.state,
            ControllerState::Ready | ControllerState::Paused
        ) || controller.lease.is_some()
            || self.warming_on(controller.config.output)
        {
            return Err(ControllerError::InvalidState.into());
        }
        let next = controller
            .config_revision
            .checked_add(1)
            .ok_or(ControllerError::RevisionExhausted)?;
        let new_pid = Pid::new(pid).map_err(|_| ControllerError::InvalidConfiguration)?;
        let authority = self
            .outputs
            .get(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if !authority.accepts_pid_limits(pid.output_min, pid.output_max) {
            return Err(ControllerError::Output.into());
        }
        let controller = self.controllers.get_mut(&id).expect("checked above");
        controller.pid = new_pid;
        controller.config.pid = pid;
        controller.reset_algorithms();
        controller.last_tick = None;
        controller.config_revision = next;
        self.recording_facts.controller(
            id,
            controller.state,
            controller.config_revision,
            Some(pid),
            self.output_time,
        );
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    /// Validate all replacement state before exchanging the stopped controller.
    /// The graph bindings remain fixed in M8 and no lease can be transferred.
    pub(super) fn reconfigure_controller(
        &mut self,
        id: ControllerId,
        config: NativeControllerConfig,
        expected_revision: u64,
    ) -> Result<CommandResult, Error> {
        let current = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?;
        if current.config_revision != expected_revision {
            return Err(ControllerError::RevisionConflict.into());
        }
        if config.id != id
            || config.input != current.config.input
            || config.output != current.config.output
            || config.reference != current.config.reference
            || !matches!(
                current.state,
                ControllerState::Ready | ControllerState::Paused
            )
            || current.lease.is_some()
            || self.warming_on(current.config.output)
        {
            return Err(ControllerError::InvalidState.into());
        }
        let next = current
            .config_revision
            .checked_add(1)
            .ok_or(ControllerError::RevisionExhausted)?;
        let state = current.state;
        let mut candidate = NativeController::new(config)?;
        self.validate_controller_config(config)?;
        candidate.state = state;
        candidate.config_revision = next;
        let snapshot = candidate.snapshot();
        self.controllers.insert(id, candidate);
        self.recording_facts.controller(
            id,
            snapshot.state,
            snapshot.config_revision,
            Some(config.pid),
            self.output_time,
        );
        Ok(CommandResult::ControllerUpdated(snapshot))
    }

    pub(super) fn prepare_controller(&mut self, id: ControllerId) -> Result<CommandResult, Error> {
        let config = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?
            .config;
        if self.controllers.get(&id).expect("looked up above").state != ControllerState::Created {
            return Err(ControllerError::InvalidState.into());
        }

        self.validate_controller_config(config)?;

        let controller = self.controllers.get_mut(&id).expect("validated above");
        controller.state = ControllerState::Ready;
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    fn validate_controller_config(&self, config: NativeControllerConfig) -> Result<(), Error> {
        let input = self
            .descriptor(config.input.instrument())?
            .parameter(config.input.parameter())
            .filter(|parameter| {
                parameter.signal == Some(config.input)
                    && parameter.role == ParameterRole::Measurement
            })
            .ok_or(ControllerError::InvalidConfiguration)?;
        if !matches!(input.value_spec, ValueSpec::Float { .. }) || input.unit != config.ema.unit {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        let reference = self
            .references
            .get(&config.reference)
            .ok_or(ControllerError::UnknownReference)?;
        if reference.unit() != input.unit {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        let output = self
            .descriptor(config.output.instrument())?
            .parameter(config.output.parameter())
            .filter(|parameter| parameter.role == ParameterRole::Actuator)
            .ok_or(ControllerError::InvalidConfiguration)?;
        let ValueSpec::Float { min, max } = output.value_spec else {
            return Err(ControllerError::InvalidConfiguration.into());
        };
        if config.pid.output_min < min
            || config.pid.output_max > max
            || !self.outputs.contains_key(&config.output)
            || (!self
                .thermal_plants
                .contains_key(&config.output.instrument())
                && !self
                    .metakon_instruments
                    .contains_key(&config.output.instrument()))
        {
            return Err(ControllerError::InvalidConfiguration.into());
        }
        if !self
            .outputs
            .get(&config.output)
            .expect("output checked above")
            .valid_native_duration(config.max_tick_gap, config.lease_lifetime)
        {
            return Err(ControllerError::InvalidConfiguration.into());
        }

        Ok(())
    }

    pub(super) fn start_or_resume_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
        expected: ControllerState,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        let result = (|| {
            if controller.state != expected {
                return Err(ControllerError::InvalidState.into());
            }
            if self.warming_on(controller.config.output) {
                return Err(OutputError::Busy.into());
            }
            if !self
                .outputs
                .get(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?
                .can_prepare()
            {
                return Err(ControllerError::Output.into());
            }
            if !self
                .outputs
                .get(&controller.config.output)
                .expect("checked above")
                .valid_native_duration(
                    controller.config.max_tick_gap,
                    controller.config.lease_lifetime,
                )
            {
                return Err(ControllerError::InvalidConfiguration.into());
            }
            let (measurement, unit, sample_at) =
                self.control_input(controller.config.input, at, controller.config.max_input_age)?;
            if unit != controller.config.ema.unit {
                return Err(ControllerError::InvalidConfiguration.into());
            }
            let reference = self
                .references
                .get_mut(&controller.config.reference)
                .ok_or(ControllerError::UnknownReference)?
                .value_at(at)
                .map_err(map_reference_error)?;
            if reference.unit != unit {
                return Err(ControllerError::InvalidConfiguration.into());
            }

            controller.reset_algorithms();
            let update = controller
                .ema
                .update(Some(measurement), SampleQuality::Good, unit, sample_at)
                .map_err(|_| ControllerError::Algorithm)?;
            controller.last_tick = Some(at);
            controller.state = ControllerState::Warming;
            if update.status == EmaStatus::Ready {
                self.activate_controller(&mut controller, at)?;
            }
            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
        })();
        self.controllers.insert(id, controller);
        result
    }

    pub(super) fn tick_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        if !matches!(
            controller.state,
            ControllerState::Running | ControllerState::Warming
        ) {
            self.controllers.insert(id, controller);
            return Err(ControllerError::InvalidState.into());
        }
        if controller.last_tick.is_some_and(|previous| at <= previous) {
            self.controllers.insert(id, controller);
            return Err(ControllerError::InvalidTickTime.into());
        }
        if controller.state == ControllerState::Warming {
            let result = self.advance_warming(&mut controller, at);
            self.controllers.insert(id, controller);
            return result;
        }

        let update = self.calculate_controller_update(&mut controller, at);
        let result = match update {
            Ok((pid, unit, ttl)) => {
                let lease = controller
                    .lease
                    .expect("Running controller owns its last token");
                let output = controller.config.output;
                let proposal = OutputProposal {
                    lease,
                    value: Value::Float(pid.output),
                    unit,
                    ttl,
                };
                let physical = self.metakon_instruments.contains_key(&output.instrument());
                let delivery = if physical {
                    self.deliver_physical(output, proposal, at)
                } else {
                    self.deliver_simulated(output, proposal, at)
                };
                match delivery {
                    Ok(()) => {
                        let renewal = if physical {
                            Ok(lease)
                        } else {
                            self.outputs
                                .get_mut(&output)
                                .ok_or(Error::from(OutputError::UnknownActuator))
                                .and_then(|authority| {
                                    authority.renew_native(
                                        lease,
                                        controller.config.lease_lifetime,
                                        at,
                                    )
                                })
                        };
                        if let Ok(current_or_replacement) = renewal {
                            controller.lease = Some(current_or_replacement);
                            controller.last_tick = Some(at);
                            if !physical {
                                controller.latest_output = Some(pid);
                            }
                            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
                        } else {
                            let _ = self.fail_controller(&mut controller, at);
                            Err(ControllerError::Output.into())
                        }
                    }
                    Err(_) => {
                        let _ = self.fail_controller(&mut controller, at);
                        Err(ControllerError::Output.into())
                    }
                }
            }
            Err(error) => {
                let _ = self.fail_controller(&mut controller, at);
                Err(error.into())
            }
        };
        self.controllers.insert(id, controller);
        result
    }

    /// Serially acquire the actuator only after all warm-up observations are committed.
    fn activate_controller(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(), Error> {
        let authority = self
            .outputs
            .get_mut(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if !authority.can_prepare() {
            controller.state = ControllerState::Failed;
            return Err(ControllerError::Output.into());
        }
        let acquisition = authority.command(
            OutputCommand::Acquire {
                owner: OutputOwner::Automatic(controller.config.id.get()),
                lifetime: controller.config.lease_lifetime,
            },
            at,
        );
        match acquisition {
            Ok(OutputResult::Lease(lease)) => {
                controller.lease = Some(lease);
                controller.state = ControllerState::Running;
                Ok(())
            }
            Ok(_) => unreachable!("Acquire has one successful result kind"),
            Err(error) => {
                controller.state = ControllerState::Failed;
                Err(error)
            }
        }
    }

    /// A Warming service may observe the same Good attempt once without recounting it.
    fn advance_warming(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        let attempt = (|| {
            let previous = controller.last_tick.ok_or(ControllerError::InvalidState)?;
            if at - previous > controller.config.max_tick_gap {
                return Err(ControllerError::InvalidTickTime);
            }
            let (value, unit, sample_at) =
                self.control_input(controller.config.input, at, controller.config.max_input_age)?;
            if unit != controller.config.ema.unit {
                return Err(ControllerError::InvalidConfiguration);
            }
            let reference = self
                .references
                .get_mut(&controller.config.reference)
                .ok_or(ControllerError::UnknownReference)?
                .value_at(at)
                .map_err(map_reference_error)?;
            if reference.unit != unit {
                return Err(ControllerError::InvalidConfiguration);
            }
            if controller
                .ema
                .snapshot()
                .last_at
                .is_some_and(|last| sample_at <= last)
            {
                controller.last_tick = Some(at);
                return Ok(());
            }
            let updated = controller
                .ema
                .update(Some(value), SampleQuality::Good, unit, sample_at)
                .map_err(|_| ControllerError::Algorithm)?;
            controller.last_tick = Some(at);
            if updated.status == EmaStatus::Ready {
                self.activate_controller(controller, at)
                    .map_err(|_| ControllerError::Output)?;
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => Ok(CommandResult::ControllerUpdated(controller.snapshot())),
            Err(error) => {
                // Preparation never owned the actuator, so its algorithm fault needs no Trip.
                controller.state = ControllerState::Failed;
                controller.lease = None;
                Err(error.into())
            }
        }
    }

    fn calculate_controller_update(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(crate::control::PidUpdate, Unit, Duration), ControllerError> {
        let previous = controller.last_tick.ok_or(ControllerError::InvalidState)?;
        if at - previous > controller.config.max_tick_gap {
            return Err(ControllerError::InvalidTickTime);
        }
        let lease = controller.lease.ok_or(ControllerError::Output)?;
        if at >= lease.expires()
            || self
                .outputs
                .get(&controller.config.output)
                .ok_or(ControllerError::Output)?
                .snapshot()
                .lease
                != Some(lease)
        {
            return Err(ControllerError::Output);
        }
        let (measurement, unit, sample_at) =
            self.control_input(controller.config.input, at, controller.config.max_input_age)?;
        let reference = self
            .references
            .get_mut(&controller.config.reference)
            .ok_or(ControllerError::UnknownReference)?
            .value_at(at)
            .map_err(map_reference_error)?;
        if reference.unit != unit {
            return Err(ControllerError::InvalidConfiguration);
        }
        let filtered = controller
            .ema
            .update(Some(measurement), SampleQuality::Good, unit, sample_at)
            .map_err(|_| ControllerError::Algorithm)?;
        if filtered.status != EmaStatus::Ready {
            return Err(ControllerError::Algorithm);
        }
        let pid = controller
            .pid
            .update(
                filtered.value.ok_or(ControllerError::Algorithm)?,
                reference.value,
                at,
            )
            .map_err(|_| ControllerError::Algorithm)?;
        let remaining_freshness = controller.config.max_input_age - (at - sample_at);
        let ttl = controller
            .config
            .proposal_ttl
            .min(remaining_freshness)
            .min(lease.expires() - at);
        if ttl.is_zero() {
            return Err(ControllerError::StaleInput);
        }
        Ok((pid, self.output_unit(controller.config.output)?, ttl))
    }

    pub(super) fn pause_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let mut controller = self
            .controllers
            .remove(&id)
            .ok_or(ControllerError::UnknownController)?;
        let result = (|| {
            if controller.state == ControllerState::Warming {
                controller.state = ControllerState::Paused;
                return Ok(CommandResult::ControllerUpdated(controller.snapshot()));
            }
            if controller.state != ControllerState::Running {
                return Err(ControllerError::InvalidState.into());
            }
            let lease = controller
                .lease
                .take()
                .ok_or(ControllerError::InvalidState)?;
            let authority = self
                .outputs
                .get_mut(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?;
            controller.state = ControllerState::Paused;
            if authority.snapshot().lease == Some(lease) {
                authority.command(OutputCommand::Release(lease), at)?;
            } else if authority.snapshot().lease.is_none()
                && authority.snapshot().state == crate::output::OutputState::SafePending
            {
                // The Runtime watchdog may already have revoked an expired lease.
                // Pausing must still settle the reserved safe action and must not
                // leave the controller claiming authority it no longer owns.
                authority.command(OutputCommand::RequestSafe, at)?;
            } else {
                // A previously revoked controller cannot mutate a subsequent owner.
                return Ok(CommandResult::ControllerUpdated(controller.snapshot()));
            }
            if self
                .thermal_plants
                .contains_key(&controller.config.output.instrument())
            {
                self.complete_simulated_safe(controller.config.output, at)?;
            }
            Ok(CommandResult::ControllerUpdated(controller.snapshot()))
        })();
        self.controllers.insert(id, controller);
        result
    }

    /// A validated new component generation never restarts Failed control implicitly.
    /// Recovery requires prior Rust virtual safe readback and deliberate acknowledgement.
    pub(super) fn reset_failed_controller(
        &mut self,
        id: ControllerId,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        let controller = self
            .controllers
            .get(&id)
            .ok_or(ControllerError::UnknownController)?;
        if controller.state != ControllerState::Failed || controller.lease.is_some() {
            return Err(ControllerError::InvalidState.into());
        }
        if self.warming_on(controller.config.output)
            || !self
                .outputs
                .get(&controller.config.output)
                .ok_or(OutputError::UnknownActuator)?
                .can_prepare()
        {
            return Err(ControllerError::Output.into());
        }
        let controller = self.controllers.get_mut(&id).expect("validated above");
        controller.reset_algorithms();
        controller.last_tick = None;
        controller.state = ControllerState::Paused;
        Ok(CommandResult::ControllerUpdated(controller.snapshot()))
    }

    pub(super) fn fail_controller(
        &mut self,
        controller: &mut NativeController,
        at: Duration,
    ) -> Result<(), Error> {
        controller.state = ControllerState::Failed;
        let lease = controller.lease.take();
        let authority = self
            .outputs
            .get_mut(&controller.config.output)
            .ok_or(OutputError::UnknownActuator)?;
        if authority.snapshot().lease == lease && lease.is_some() {
            authority.command(OutputCommand::Trip, at)?;
        } else if authority.snapshot().lease.is_none()
            && authority.snapshot().state == crate::output::OutputState::SafePending
        {
            // A failed loop latches its own fault even if watchdog already requested safe.
            authority.command(OutputCommand::Trip, at)?;
        } else {
            return Ok(());
        }
        if self
            .thermal_plants
            .contains_key(&controller.config.output.instrument())
        {
            self.complete_simulated_safe(controller.config.output, at)
        } else {
            Ok(())
        }
    }

    pub(super) fn control_input(
        &self,
        signal: SignalId,
        at: Duration,
        max_age: Duration,
    ) -> Result<(f64, Unit, Duration), ControllerError> {
        let sample = self
            .signal(signal)
            .ok()
            .and_then(SignalBuffer::latest)
            .ok_or(ControllerError::InputUnavailable)?;
        if sample.quality() != SampleQuality::Good {
            return Err(ControllerError::InputUnavailable);
        }
        if at < sample.freshness_at() {
            return Err(ControllerError::InvalidTickTime);
        }
        if at - sample.freshness_at() >= max_age {
            return Err(ControllerError::StaleInput);
        }
        let Some(Value::Float(value)) = sample.value() else {
            return Err(ControllerError::InputUnavailable);
        };
        if !value.is_finite() {
            return Err(ControllerError::InputUnavailable);
        }
        Ok((*value, sample.unit(), sample.freshness_at()))
    }

    fn output_unit(&self, actuator: ActuatorId) -> Result<Unit, ControllerError> {
        self.descriptor(actuator.instrument())
            .ok()
            .and_then(|descriptor| descriptor.parameter(actuator.parameter()))
            .map(|parameter| parameter.unit)
            .ok_or(ControllerError::InvalidConfiguration)
    }

    // Freeze the current accepted binding and authority identity at Core fact
    // capture. A later rebind or revoke cannot rewrite an earlier output row.
    pub(super) fn sync_recording_output_context(&mut self, actuator: ActuatorId) {
        let Some(authority) = self.outputs.get(&actuator) else {
            return;
        };
        let binding = self.metakon_instruments.get(&actuator.instrument());
        self.recording_facts.output_context(
            actuator,
            crate::recording::OutputContext {
                unit: self.output_unit(actuator).ok(),
                authority_epoch: Some(authority.snapshot().epoch),
                resource: binding.map(|instrument| instrument.binding.resource),
                binding_generation: binding.map(|instrument| instrument.binding.binding_generation),
                mapping_revision: binding.map(|instrument| instrument.binding.mapping_revision),
            },
        );
    }

    pub(super) fn capture_reference_fact(&mut self, id: ReferenceId, at: Duration) {
        let Some(snapshot) = self.references.get(&id).map(RuntimeReference::snapshot) else {
            return;
        };
        match snapshot {
            crate::reference::ReferenceSnapshot::Fixed {
                value,
                unit,
                revision,
                ..
            } => self
                .recording_facts
                .reference(crate::recording::ReferenceDetails {
                    reference: id,
                    revision,
                    value,
                    target: None,
                    rate: None,
                    unit,
                    at,
                }),
            crate::reference::ReferenceSnapshot::Ramp {
                state, revision, ..
            } => self
                .recording_facts
                .reference(crate::recording::ReferenceDetails {
                    reference: id,
                    revision,
                    value: state.current,
                    target: Some(state.target),
                    rate: Some(state.rate),
                    unit: state.unit,
                    at,
                }),
        }
    }

    pub(super) fn capture_controller_fact(&mut self, id: ControllerId, at: Duration) {
        if let Some(controller) = self.controllers.get(&id) {
            self.recording_facts.controller(
                id,
                controller.state,
                controller.config_revision,
                None,
                at,
            );
        }
    }
}
