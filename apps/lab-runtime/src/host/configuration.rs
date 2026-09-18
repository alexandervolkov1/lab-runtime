//! Configured resource, probe, rebind, deployment-apply and component-staging orchestration.
//!
//! The methods here remain implementations of the single HostCore owner declared
//! in the parent module; this module introduces no additional mutable state owner.

use super::*;

impl HostCore {
    /// Queue each configured read-only compatibility probe exactly once.
    pub fn begin_configured_probes(&mut self, at: Duration) -> Result<(), Error> {
        for probe in &mut self.configured_probes {
            if probe.queued {
                continue;
            }
            probe.baseline = match self.runtime.query(Query::GetLatestSignal(SignalId::new(
                probe.instrument,
                probe.parameter,
            )))? {
                QueryResult::Latest(sample) => sample,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "configured probe query mismatch",
                    ));
                }
            };
            self.runtime.command(Command::QueueMetakonRead {
                instrument: probe.instrument,
                parameter: probe.parameter,
                at,
                queue_ttl: probe.queue_ttl,
                timeout: probe.timeout,
            })?;
            probe.queued = true;
        }
        Ok(())
    }

    /// Quiesce ordinary acquisition for one explicit reconnect without pausing
    /// unrelated resources, Recorder progress, recovery or native safety work.
    pub fn begin_configured_resource_reconnect(
        &mut self,
        resource: ResourceId,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        self.reconnect_quiesced_resources.insert(resource);
        Ok(())
    }

    /// Report the resource-scoped reconnect gate for diagnostics and acceptance.
    pub fn configured_resource_reconnect_quiesced(&self, resource: ResourceId) -> bool {
        self.reconnect_quiesced_resources.contains(&resource)
    }

    /// Queue only the trusted compatibility probes bound to one replacement.
    pub fn begin_configured_probes_for_resource(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<(), Error> {
        let mut found = false;
        for probe in &mut self.configured_probes {
            let bound = self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource);
            if !bound {
                continue;
            }
            found = true;
            if probe.queued {
                continue;
            }
            probe.baseline = match self.runtime.query(Query::GetLatestSignal(SignalId::new(
                probe.instrument,
                probe.parameter,
            )))? {
                QueryResult::Latest(sample) => sample,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "configured probe query mismatch",
                    ));
                }
            };
            self.runtime.command(Command::QueueMetakonRead {
                instrument: probe.instrument,
                parameter: probe.parameter,
                at,
                queue_ttl: probe.queue_ttl,
                timeout: probe.timeout,
            })?;
            probe.queued = true;
        }
        if !found {
            return Err(Error::InvalidConfiguration(
                "configured resource has no compatibility probe",
            ));
        }
        Ok(())
    }

    /// Replace one read-only adapter at an explicit closed boundary and advance
    /// every dependent physical binding generation. No port enumeration or
    /// automatic selection occurs here.
    pub fn rebind_configured_transport(
        &mut self,
        resource: ResourceId,
        adapter: Box<dyn ByteTransport>,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        let instruments: Vec<_> = self
            .plan
            .metakon_reads
            .iter()
            .filter_map(|read| {
                self.runtime
                    .metakon_binding(read.instrument)
                    .filter(|binding| binding.resource == resource)
                    .map(|binding| (read.instrument, binding))
            })
            .collect();
        if instruments.is_empty() {
            return Err(Error::InvalidConfiguration(
                "configured resource has no instrument",
            ));
        }
        let replacements: Vec<_> = instruments
            .iter()
            .map(|(instrument, binding)| {
                Ok((
                    *instrument,
                    MetakonBinding {
                        binding_generation: binding
                            .binding_generation
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("binding generation exhausted"))?,
                        mapping_revision: binding
                            .mapping_revision
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("mapping revision exhausted"))?,
                        ..*binding
                    },
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        self.runtime.replace_transport(resource, adapter)?;
        for (instrument, binding) in replacements {
            self.runtime.command(Command::RebindMetakon {
                instrument,
                binding,
                at,
            })?;
            for (actuator, profile) in self
                .active_safety_profiles
                .iter()
                .filter(|(actuator, _)| actuator.instrument() == instrument)
            {
                self.runtime.command(Command::Output {
                    actuator: *actuator,
                    at,
                    command: OutputCommand::BindProfile(profile.clone()),
                })?;
            }
        }
        for probe in &mut self.configured_probes {
            if self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource)
            {
                probe.queued = false;
                probe.baseline = None;
            }
        }
        self.closed_resources.remove(&resource);
        self.observe(at, None)
    }

    /// Replace a closed resource and every candidate definition/address bound
    /// to it in one serialized owner turn. The logical IDs remain stable while
    /// both physical binding generation and mapping revision advance once.
    pub fn rebind_configured_transport_from_configuration(
        &mut self,
        deployment: &FrozenDeployment,
        resource: ResourceId,
        adapter: Box<dyn ByteTransport>,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.resources.contains(&resource) {
            return Err(Error::InvalidConfiguration("configured resource missing"));
        }
        let mut replacements = Vec::new();
        for instrument in &deployment.effective().dto.instruments {
            let InstrumentDto::Metakon {
                id,
                definition,
                resource_id,
                address,
                queue_timeout_ms,
                transaction_timeout_ms,
                ..
            } = instrument
            else {
                continue;
            };
            if *resource_id != resource.get() {
                continue;
            }
            let old = self
                .runtime
                .metakon_binding(InstrumentId::new(*id))
                .ok_or(Error::InvalidConfiguration("configured binding missing"))?;
            let bytes = deployment
                .artifact_bytes(definition)
                .ok_or(Error::InvalidConfiguration("frozen definition missing"))?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::InvalidConfiguration("definition is not UTF-8"))?;
            let definition = parse_definition_json(text)
                .map_err(|_| Error::InvalidConfiguration("invalid frozen definition"))?;
            if definition.id != InstrumentId::new(*id) {
                return Err(Error::InvalidConfiguration(
                    "definition and deployment instrument IDs differ",
                ));
            }
            let temperature = definition
                .parameters
                .iter()
                .find(|parameter| parameter.operation == KnownOperation::Temperature)
                .ok_or(Error::InvalidConfiguration(
                    "read-only Metakon definition lacks temperature",
                ))?
                .id;
            let channel_type = definition
                .parameters
                .iter()
                .find(|parameter| parameter.operation == KnownOperation::ChannelType)
                .ok_or(Error::InvalidConfiguration(
                    "read-only Metakon definition lacks compatibility probe",
                ))?
                .id;
            let output_unit = definition
                .parameters
                .iter()
                .find(|parameter| parameter.operation == KnownOperation::Output)
                .map(|parameter| parameter.unit);
            replacements.push((
                InstrumentId::new(*id),
                MetakonInstrumentConfig {
                    definition,
                    binding: MetakonBinding {
                        resource,
                        device: *address,
                        channel: 0,
                        binding_generation: old
                            .binding_generation
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("binding generation exhausted"))?,
                        mapping_revision: old
                            .mapping_revision
                            .checked_add(1)
                            .ok_or(Error::InvalidConfiguration("mapping revision exhausted"))?,
                        expected_output_unit: output_unit,
                        output_queue_ttl: output_unit
                            .map(|_| Duration::from_millis(*queue_timeout_ms)),
                        output_timeout: output_unit
                            .map(|_| Duration::from_millis(*transaction_timeout_ms)),
                    },
                    history_capacity: 64,
                },
                old,
                temperature,
                channel_type,
            ));
        }
        if replacements.is_empty() {
            return Err(Error::InvalidConfiguration(
                "candidate resource has no instrument",
            ));
        }
        let recovery_timeout = deployment
            .effective()
            .dto
            .resources
            .iter()
            .find(|candidate| candidate.id == resource.get())
            .ok_or(Error::InvalidConfiguration("candidate resource missing"))?
            .recovery_timeout_ms;
        self.runtime.replace_transport_with_recovery_timeout(
            resource,
            adapter,
            Duration::from_millis(recovery_timeout),
        )?;
        for (instrument, config, old, temperature, channel_type) in replacements {
            self.runtime.command(Command::ReconfigureMetakon {
                config,
                expected_binding_generation: old.binding_generation,
                expected_mapping_revision: old.mapping_revision,
                at,
            })?;
            for (actuator, profile) in self
                .active_safety_profiles
                .iter()
                .filter(|(actuator, _)| actuator.instrument() == instrument)
            {
                self.runtime.command(Command::Output {
                    actuator: *actuator,
                    at,
                    command: OutputCommand::BindProfile(profile.clone()),
                })?;
            }
            let read = self
                .plan
                .metakon_reads
                .iter_mut()
                .find(|read| read.instrument == instrument)
                .ok_or(Error::InvalidConfiguration(
                    "configured read schedule missing",
                ))?;
            read.parameter = temperature;
            let probe = self
                .configured_probes
                .iter_mut()
                .find(|probe| probe.instrument == instrument)
                .ok_or(Error::InvalidConfiguration("configured probe missing"))?;
            probe.parameter = channel_type;
            probe.queued = false;
            probe.baseline = None;
        }
        self.closed_resources.remove(&resource);
        self.observe(at, None)
    }

    /// Retire the current adapter without waiting; true means a replacement can
    /// be installed without creating a second handle owner.
    pub fn prepare_configured_transport_replacement(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<bool, Error> {
        Ok(self.runtime.shutdown_transport(resource, at)?
            == lab_core::transport::TransportShutdown::Complete)
    }

    /// Copy the current physical binding generation for diagnostics/tests.
    pub fn configured_binding_generation(&self, instrument: u64) -> Option<u64> {
        self.runtime
            .metakon_binding(InstrumentId::new(instrument))
            .map(|binding| binding.binding_generation)
    }

    /// Highest active binding generation using one stable logical resource.
    pub(crate) fn configured_resource_generation(&self, resource: ResourceId) -> Option<u64> {
        self.plan
            .metakon_reads
            .iter()
            .filter_map(|read| self.runtime.metakon_binding(read.instrument))
            .filter(|binding| binding.resource == resource)
            .map(|binding| binding.binding_generation)
            .max()
    }

    /// Copy the authoritative executor state for bounded reconnect diagnostics.
    pub(crate) fn configured_resource_executor_state(
        &self,
        resource: ResourceId,
    ) -> Option<ExecutorState> {
        match self.runtime.query(Query::Transport(resource)).ok()? {
            QueryResult::Transport(snapshot) => Some(snapshot.state),
            _ => None,
        }
    }

    /// Check frozen channel-type observations without polling or hidden I/O.
    pub fn configured_probes_ready(&self) -> Result<bool, Error> {
        for probe in &self.configured_probes {
            let QueryResult::Latest(Some(sample)) = self.runtime.query(Query::GetLatestSignal(
                SignalId::new(probe.instrument, probe.parameter),
            ))?
            else {
                return Ok(false);
            };
            if probe.baseline.as_ref() == Some(&sample) {
                return Ok(false);
            }
            if sample.quality() != SampleQuality::Good
                || sample.value() != Some(&lab_core::Value::Integer(3))
            {
                return Err(Error::InvalidConfiguration(
                    "Metakon channel compatibility probe failed",
                ));
            }
        }
        Ok(true)
    }

    /// Request the configured physical safe procedure only after compatibility
    /// probes have established the intended Metakon profile. The next bounded
    /// safety turns use the normal OutputAuthority/write/ACK/readback path.
    pub fn request_configured_physical_safe(&mut self, at: Duration) -> Result<(), Error> {
        if !self.configured_probes_ready()? {
            return Err(Error::InvalidConfiguration(
                "physical output compatibility probe is not ready",
            ));
        }
        for actuator in self.outputs.clone() {
            if self
                .runtime
                .metakon_binding(actuator.instrument())
                .is_none()
            {
                continue;
            }
            let QueryResult::Output(snapshot) = self.runtime.query(Query::Output(actuator))? else {
                unreachable!()
            };
            if snapshot.state == lab_core::output::OutputState::Unverified {
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::RequestSafe,
                })?;
            }
        }
        self.observe(at, None)
    }

    /// True only when every configured physical output has matching readback of
    /// its safe value and no lease remains.
    pub fn configured_physical_outputs_safe(&self) -> Result<bool, Error> {
        for actuator in &self.outputs {
            if self
                .runtime
                .metakon_binding(actuator.instrument())
                .is_none()
            {
                continue;
            }
            let QueryResult::Output(snapshot) = self.runtime.query(Query::Output(*actuator))?
            else {
                unreachable!()
            };
            if !snapshot.safe_confirmed || snapshot.lease.is_some() || snapshot.in_flight.is_some()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Prepare deferred physical controllers only after their output has trusted
    /// safe readback. Registration itself never acquires a lease.
    pub fn prepare_configured_physical_controllers(&mut self) -> Result<(), Error> {
        for (controller, _, _) in &self.plan.controllers {
            let QueryResult::Controller(snapshot) =
                self.runtime.query(Query::Controller(*controller))?
            else {
                unreachable!()
            };
            if snapshot.state != ControllerState::Created {
                continue;
            }
            let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(*controller))?
            else {
                unreachable!()
            };
            if self
                .runtime
                .metakon_binding(config.output.instrument())
                .is_some()
            {
                self.runtime
                    .command(Command::PrepareController(*controller))?;
            }
        }
        Ok(())
    }

    /// Inspect only the compatibility probes bound to one reconnecting resource.
    /// The rebind-generated Unavailable baseline is pending, not probe failure.
    pub fn configured_probes_ready_for_resource(
        &self,
        resource: ResourceId,
    ) -> Result<bool, Error> {
        let mut found = false;
        for probe in &self.configured_probes {
            let bound = self
                .runtime
                .metakon_binding(probe.instrument)
                .is_some_and(|binding| binding.resource == resource);
            if !bound {
                continue;
            }
            found = true;
            if !probe.queued {
                return Ok(false);
            }
            let QueryResult::Latest(Some(sample)) = self.runtime.query(Query::GetLatestSignal(
                SignalId::new(probe.instrument, probe.parameter),
            ))?
            else {
                return Ok(false);
            };
            if probe.baseline.as_ref() == Some(&sample) {
                return Ok(false);
            }
            if sample.quality() != SampleQuality::Good
                || sample.value() != Some(&lab_core::Value::Integer(3))
            {
                return Err(Error::InvalidConfiguration(
                    "Metakon channel compatibility probe failed",
                ));
            }
        }
        if !found {
            return Err(Error::InvalidConfiguration(
                "configured resource has no compatibility probe",
            ));
        }
        Ok(true)
    }

    /// Release one replacement only after its trusted probe and durable lifecycle
    /// have succeeded. The fresh deadline prevents a reconnect catch-up burst.
    pub fn activate_configured_resource_after_reconnect(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<(), Error> {
        if !self.reconnect_quiesced_resources.contains(&resource)
            || !self.configured_probes_ready_for_resource(resource)?
        {
            return Err(Error::InvalidConfiguration(
                "configured resource reconnect is not ready",
            ));
        }
        for read in &mut self.plan.metakon_reads {
            if self
                .runtime
                .metakon_binding(read.instrument)
                .is_some_and(|binding| binding.resource == resource)
            {
                read.slot.next_due = at
                    .checked_add(read.slot.period)
                    .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
            }
        }
        self.reconnect_quiesced_resources.remove(&resource);
        Ok(())
    }

    /// Retire a replacement that crossed the generation fence but failed its
    /// compatibility gate. The new generation remains authoritative and gated.
    pub fn retire_failed_configured_reconnect(
        &mut self,
        resource: ResourceId,
        at: Duration,
    ) -> Result<bool, Error> {
        if !self.reconnect_quiesced_resources.contains(&resource) {
            return Err(Error::InvalidConfiguration(
                "configured resource reconnect is not quiesced",
            ));
        }
        let complete = self.runtime.shutdown_transport(resource, at)?
            == lab_core::transport::TransportShutdown::Complete;
        self.observe(at, None)?;
        if complete {
            self.closed_resources.insert(resource);
        }
        Ok(complete)
    }

    /// Apply display/cadence-only fields at one owner commit without replacing
    /// instruments, observations, authority or generations.
    pub(crate) fn apply_live_configuration(
        &mut self,
        candidate: &FrozenDeployment,
    ) -> Result<(), Error> {
        for instrument in &candidate.effective().dto.instruments {
            match instrument {
                InstrumentDto::VirtualMeasurement {
                    id,
                    display_name,
                    poll_period_ms,
                    ..
                }
                | InstrumentDto::ThermalPlant {
                    id,
                    display_name,
                    poll_period_ms,
                    ..
                } => {
                    self.runtime.command(Command::RenameInstrument {
                        instrument: InstrumentId::new(*id),
                        name: display_name.clone(),
                    })?;
                    let (_, slot) = self
                        .plan
                        .plants
                        .iter_mut()
                        .find(|(instrument, _)| instrument.get() == *id)
                        .ok_or(Error::InvalidConfiguration("configured schedule missing"))?;
                    slot.period = Duration::from_millis(*poll_period_ms);
                    slot.next_due = self
                        .last_now
                        .checked_add(slot.period)
                        .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
                }
                InstrumentDto::Metakon {
                    id,
                    poll_period_ms,
                    queue_timeout_ms,
                    transaction_timeout_ms,
                    ..
                } => {
                    let read = self
                        .plan
                        .metakon_reads
                        .iter_mut()
                        .find(|read| read.instrument.get() == *id)
                        .ok_or(Error::InvalidConfiguration(
                            "configured read schedule missing",
                        ))?;
                    read.slot.period = Duration::from_millis(*poll_period_ms);
                    read.slot.next_due = self
                        .last_now
                        .checked_add(read.slot.period)
                        .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
                    read.queue_ttl = Duration::from_millis(*queue_timeout_ms);
                    read.timeout = Duration::from_millis(*transaction_timeout_ms);
                }
            }
        }
        self.deployment_provenance = candidate
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(())
    }

    /// Revoke every configured authority and prove the existing virtual safe
    /// policy before configuration replacement. This method exposes no rearm.
    pub(crate) fn enter_configuration_safe_barrier(&mut self, at: Duration) -> Result<bool, Error> {
        let controllers: Vec<_> = self.plan.controllers.iter().map(|(id, _, _)| *id).collect();
        for controller in controllers {
            let QueryResult::Controller(snapshot) =
                self.runtime.query(Query::Controller(controller))?
            else {
                return Err(Error::InvalidConfiguration("configured controller missing"));
            };
            if matches!(
                snapshot.state,
                ControllerState::Warming | ControllerState::Running
            ) {
                self.runtime
                    .command(Command::PauseController { controller, at })?;
            }
        }
        for actuator in self.outputs.clone() {
            let physical = self
                .runtime
                .metakon_binding(actuator.instrument())
                .is_some();
            let QueryResult::Output(before) = self.runtime.query(Query::Output(actuator))? else {
                unreachable!()
            };
            if !before.safe_confirmed && before.state != lab_core::output::OutputState::SafePending
            {
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::RequestSafe,
                })?;
            }
            if physical {
                continue;
            }
            if before.safe_confirmed {
                continue;
            }
            let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::BeginDispatch,
                })?
            else {
                return Ok(false);
            };
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
            })?;
        }
        self.observe(at, None)?;
        Ok(self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed && snapshot.lease.is_none())
        }))
    }

    /// Commit a fixed-topology candidate after its safe barrier. All mutations
    /// occur in this serialized owner turn; controllers remain Ready/Paused and
    /// no output authority is acquired.
    pub(crate) fn apply_configuration(
        &mut self,
        active: &FrozenDeployment,
        candidate: &FrozenDeployment,
        at: Duration,
    ) -> Result<(), Error> {
        let old = &active.effective().dto;
        let new = &candidate.effective().dto;

        if old
            .instruments
            .iter()
            .zip(&new.instruments)
            .any(|(old, new)| thermal_configuration_changed(old, new))
        {
            self.restart_configured_virtual_models(candidate, at)?;
        }

        for (old_reference, reference) in old.references.iter().zip(&new.references) {
            if old_reference == reference {
                continue;
            }
            let id = ReferenceId::new(reference.id);
            let revision = match self.runtime.query(Query::Reference(id))? {
                QueryResult::Reference(ReferenceSnapshot::Fixed { revision, .. })
                | QueryResult::Reference(ReferenceSnapshot::Ramp { revision, .. }) => revision,
                _ => return Err(Error::InvalidConfiguration("configured Reference missing")),
            };
            let unit = Unit::new(&reference.unit_id, &reference.unit_symbol)?;
            let config = match reference.kind {
                ReferenceKindDto::Fixed => ReferenceConfig::Fixed {
                    id,
                    value: reference.value,
                    unit,
                },
                ReferenceKindDto::Ramp => ReferenceConfig::Ramp {
                    id,
                    start: reference.value,
                    target: reference
                        .target
                        .ok_or(Error::InvalidConfiguration("ramp target missing"))?,
                    rate: reference
                        .rate
                        .ok_or(Error::InvalidConfiguration("ramp rate missing"))?,
                    unit,
                    at,
                },
            };
            self.runtime.command(Command::ReconfigureReference {
                reference: id,
                config,
                expected_revision: revision,
            })?;
        }

        for (old_safe, safe) in old.safe_profiles.iter().zip(&new.safe_profiles) {
            if old_safe == safe {
                continue;
            }
            let actuator = ActuatorId::new(
                InstrumentId::new(safe.instrument_id),
                lab_core::ParameterId::new(safe.parameter_id),
            );
            let profile = SafeProfile {
                min: safe.min,
                max: safe.max,
                safe_value: safe.safe_value,
                max_lease: Duration::from_millis(safe.max_lease_ms),
                max_proposal_ttl: Duration::from_millis(safe.max_proposal_ttl_ms),
                required_evidence: match safe.required_evidence {
                    EvidenceDto::Ack => EvidenceLevel::Acknowledgement,
                    EvidenceDto::Readback => EvidenceLevel::Readback,
                },
            };
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::BindProfile(profile.clone()),
            })?;
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::RequestSafe,
            })?;
            let CommandResult::Output(OutputResult::Dispatched(dispatch)) =
                self.runtime.command(Command::Output {
                    actuator,
                    at,
                    command: OutputCommand::BeginDispatch,
                })?
            else {
                return Err(Error::InvalidConfiguration("new safe dispatch absent"));
            };
            self.runtime.command(Command::Output {
                actuator,
                at,
                command: OutputCommand::Complete {
                    dispatch_id: dispatch.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
            })?;
            if let Some((_, current)) = self
                .active_safety_profiles
                .iter_mut()
                .find(|(current, _)| *current == actuator)
            {
                *current = profile;
            }
        }

        for (old_controller, controller) in old.controllers.iter().zip(&new.controllers) {
            let id = ControllerId::new(controller.id);
            if old_controller != controller {
                let QueryResult::Controller(snapshot) =
                    self.runtime.query(Query::Controller(id))?
                else {
                    return Err(Error::InvalidConfiguration("configured controller missing"));
                };
                let reference = ReferenceId::new(controller.reference_id);
                let unit = match self.runtime.query(Query::Reference(reference))? {
                    QueryResult::Reference(ReferenceSnapshot::Fixed { unit, .. }) => unit,
                    QueryResult::Reference(ReferenceSnapshot::Ramp { state, .. }) => state.unit,
                    _ => {
                        return Err(Error::InvalidConfiguration("configured Reference missing"));
                    }
                };
                self.runtime.command(Command::ReconfigureController {
                    controller: id,
                    config: NativeControllerConfig {
                        id,
                        input: SignalId::new(
                            InstrumentId::new(controller.input_instrument_id),
                            lab_core::ParameterId::new(controller.input_parameter_id),
                        ),
                        output: ActuatorId::new(
                            InstrumentId::new(controller.output_instrument_id),
                            lab_core::ParameterId::new(controller.output_parameter_id),
                        ),
                        reference,
                        ema: EmaConfig {
                            time_constant: Duration::from_millis(controller.ema_time_constant_ms),
                            warmup_samples: controller.ema_warmup_samples,
                            unit,
                        },
                        pid: PidConfig {
                            kp: controller.kp,
                            ki: controller.ki,
                            kd: controller.kd,
                            output_min: controller.output_min,
                            output_max: controller.output_max,
                        },
                        max_input_age: Duration::from_millis(controller.max_input_age_ms),
                        max_tick_gap: Duration::from_millis(controller.max_tick_gap_ms),
                        lease_lifetime: Duration::from_millis(controller.lease_lifetime_ms),
                        proposal_ttl: Duration::from_millis(controller.proposal_ttl_ms),
                    },
                    expected_revision: snapshot.config_revision,
                })?;
            }
            let (_, _, slot) = self
                .plan
                .controllers
                .iter_mut()
                .find(|(current, _, _)| *current == id)
                .ok_or(Error::InvalidConfiguration("controller schedule missing"))?;
            slot.period = Duration::from_millis(controller.period_ms);
            slot.next_due = at
                .checked_add(slot.period)
                .ok_or(Error::InvalidConfiguration("scheduler time exhausted"))?;
        }
        self.apply_live_configuration(candidate)?;
        self.observe(at, None)
    }

    /// Restart every configured native thermal model under its generation fence.
    pub(crate) fn restart_configured_virtual_models(
        &mut self,
        deployment: &FrozenDeployment,
        at: Duration,
    ) -> Result<(usize, u64), Error> {
        let mut count = 0usize;
        let mut latest_generation = 0u64;
        for instrument in &deployment.effective().dto.instruments {
            if let InstrumentDto::ThermalPlant {
                id,
                display_name,
                history_capacity,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                ..
            } = instrument
            {
                let id = InstrumentId::new(*id);
                let expected = *self
                    .virtual_model_generations
                    .get(&id)
                    .ok_or(Error::InvalidConfiguration("model generation missing"))?;
                let CommandResult::ModelRestarted { generation, .. } =
                    self.runtime.command(Command::RestartThermalPlant {
                        instrument: id,
                        config: ThermalPlantConfig {
                            id,
                            name: display_name.clone(),
                            history_capacity: *history_capacity,
                            ambient_temperature: *ambient_temperature,
                            initial_temperature: *initial_temperature,
                            gain_per_percent: *gain_per_percent,
                            time_constant: Duration::from_millis(*time_constant_ms),
                        },
                        expected_generation: expected,
                        at,
                    })?
                else {
                    return Err(Error::InvalidConfiguration("unexpected restart result"));
                };
                self.virtual_model_generations.insert(id, generation);
                latest_generation = generation;
                count += 1;
            }
        }
        if count == 0 {
            return Err(Error::InvalidConfiguration("no configured native models"));
        }
        self.observe(at, None)?;
        Ok((count, latest_generation))
    }

    /// Stage one configured managed component. Startup and deployment replacement
    /// call this serially so Core's accepted one-stage/two-worker bound is unchanged.
    pub(crate) fn stage_configured_component(
        &mut self,
        deployment: &FrozenDeployment,
        index: usize,
        replaces: bool,
        at: Duration,
    ) -> Result<ComponentId, Error> {
        let component = deployment
            .effective()
            .dto
            .managed_components
            .get(index)
            .ok_or(Error::InvalidConfiguration("managed component index"))?;
        let id = ComponentId::new(component.id);
        let definition = build_component_definition(
            &component.implementation,
            NativeComponentDefinition {
                id,
                instrument: InstrumentId::new(component.instrument_id),
                name: component.display_name.clone(),
                input: component
                    .input_instrument_id
                    .map(|input| SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE)),
                config: component
                    .plain_config()
                    .map_err(|_| Error::InvalidConfiguration("managed PlainData config"))?,
            },
        )?;
        self.runtime.command(if replaces {
            Command::PrepareComponentReplacement {
                definition,
                replaces: id,
                at,
            }
        } else {
            Command::StageComponent {
                definition,
                replaces: None,
                at,
            }
        })?;
        if !replaces {
            self.events.track_component(id);
            self.components.push((
                id,
                if component.input_instrument_id.is_some() {
                    "transform"
                } else {
                    "source"
                },
            ));
        }
        Ok(id)
    }

    /// Whether one configured init result committed and no callback remains.
    pub(crate) fn component_initialized(&self, id: ComponentId) -> bool {
        matches!(self.runtime.query(Query::Component(id)),Ok(QueryResult::Component(snapshot))
            if matches!(snapshot.state,ComponentState::Warming|ComponentState::Ready)
                && snapshot.pending.is_none())
    }

    /// Start configured component cadences only after every init committed.
    pub(crate) fn activate_configured_components(
        &mut self,
        deployment: &FrozenDeployment,
        at: Duration,
    ) -> Result<(), Error> {
        self.plan.sources.clear();
        self.plan.transforms.clear();
        for component in &deployment.effective().dto.managed_components {
            let id = ComponentId::new(component.id);
            if !self.component_initialized(id) {
                return Err(Error::InvalidConfiguration("managed init incomplete"));
            }
            if let Some(input) = component.input_instrument_id {
                self.plan.transforms.push((
                    id,
                    SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE),
                ));
            } else {
                let mut slot = Periodic::new(Duration::from_millis(component.period_ms));
                slot.next_due = at;
                self.plan.sources.push((id, slot));
            }
        }
        self.deployment_provenance = deployment
            .provenance_entries()
            .into_iter()
            .map(|(kind, encoding, content)| ProvenanceEntry {
                kind,
                encoding,
                content,
            })
            .collect();
        Ok(())
    }
}
