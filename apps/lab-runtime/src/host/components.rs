//! Native managed-component scheduling composition and virtual-instrument projections.
//!
//! The methods here remain implementations of the single HostCore owner declared
//! in the parent module; this module introduces no additional mutable state owner.

use super::*;

impl HostCore {
    /// Install one trusted nonblocking component port before managed startup.
    pub fn install_component_executor(
        &mut self,
        executor: Box<dyn ComponentExecutor>,
    ) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        self.runtime.install_component_executor(executor)
    }

    /// Stage the fixed native transform used by the unconfigured demonstration profile.
    pub fn stage_standard_components(&mut self, at: Duration) -> Result<(), Error> {
        if !self.components.is_empty() {
            return Err(Error::InvalidConfiguration(
                "managed profile already staged",
            ));
        }
        let definition = build_component_definition(
            MOVING_MEAN_IMPLEMENTATION,
            NativeComponentDefinition {
                id: MANAGED_FILTER,
                instrument: InstrumentId::new(MANAGED_FILTER.get()),
                name: "Native moving mean".into(),
                input: Some(SignalId::new(PLANT, lab_core::TEMPERATURE)),
                config: PlainData {
                    fields: BTreeMap::from([("window".into(), PlainValue::Number(3.0))]),
                },
            },
        )?;
        self.runtime.command(Command::StageComponent {
            definition,
            replaces: None,
            at,
        })?;
        self.events.track_component(MANAGED_FILTER);
        self.components.push((MANAGED_FILTER, "transform"));
        Ok(())
    }

    /// True only after the native initialization callback committed and no job waits.
    pub fn standard_components_initialized(&self) -> bool {
        matches!(self.runtime.query(Query::Component(MANAGED_FILTER)),
            Ok(QueryResult::Component(snapshot)) if matches!(snapshot.state,ComponentState::Warming|ComponentState::Ready) && snapshot.pending.is_none())
    }

    /// Start trusted managed cadence after bounded startup init completes.
    pub fn activate_standard_components(&mut self, _at: Duration) -> Result<(), Error> {
        if !self.standard_components_initialized() {
            return Err(Error::InvalidConfiguration("managed init incomplete"));
        }
        self.plan
            .transforms
            .push((MANAGED_FILTER, SignalId::new(PLANT, lab_core::TEMPERATURE)));
        Ok(())
    }

    /// Fixed trusted component identities/kinds for discovery and typed status.
    pub fn component_catalog(&self) -> &[(ComponentId, &'static str)] {
        &self.components
    }

    /// Public semantic class for discovery; never exposes a Rust implementation type.
    pub fn instrument_kind(&self, id: InstrumentId) -> &'static str {
        if self.physical_instruments.contains(&id) {
            "physical"
        } else if self
            .components
            .iter()
            .any(|(component, _)| component.get() == id.get())
        {
            "managed"
        } else if self.virtual_model_generations.contains_key(&id) {
            "emulated"
        } else {
            "virtual"
        }
    }

    /// Current replacement fence used consistently by discovery and measurement DTOs.
    pub fn signal_generation(&self, signal: SignalId) -> u64 {
        if let Some((component, _)) = self
            .components
            .iter()
            .find(|(component, _)| component.get() == signal.instrument().get())
            && let Ok(QueryResult::Component(snapshot)) =
                self.runtime.query(Query::Component(*component))
        {
            snapshot.generation
        } else if let Some(binding) = self.runtime.metakon_binding(signal.instrument()) {
            binding.binding_generation
        } else {
            self.virtual_model_generations
                .get(&signal.instrument())
                .copied()
                .unwrap_or(1)
        }
    }

    /// Whether this exact virtual signal is deployment-authorized for API publication.
    pub fn emulator_writable(&self, signal: SignalId) -> bool {
        self.emulator_targets.contains(&signal)
    }

    /// Number of explicitly configured external-emulator targets.
    pub fn emulator_target_count(&self) -> usize {
        self.emulator_targets.len()
    }

    /// Number of Runtime-owned native virtual models with generation-fenced restart.
    pub fn virtual_model_count(&self) -> usize {
        self.virtual_model_generations.len()
    }

    /// Commit one external virtual observation through the ordinary owner path.
    pub fn publish_emulator_measurement(
        &mut self,
        signal: SignalId,
        value: Option<Value>,
        expected_generation: u64,
        at: Duration,
        cause: Option<(String, u64)>,
    ) -> Result<Sample, Error> {
        if !self.emulator_targets.contains(&signal) {
            return Err(Error::OperationNotAllowed(signal.parameter()));
        }
        self.command_with_cause(
            Command::PublishVirtualMeasurement {
                instrument: signal.instrument(),
                parameter: signal.parameter(),
                value,
                expected_generation,
                at,
            },
            cause,
        )?;
        match self.runtime.query(Query::GetLatestSignal(signal))? {
            QueryResult::Latest(Some(sample)) if sample.at() == at => Ok(sample),
            _ => Err(Error::InvalidConfiguration(
                "virtual publication did not commit",
            )),
        }
    }

    /// Add one trusted managed-input controller fixture after Transform init.
    /// It has a separate safe output; the independent native plant remains bound.
    pub fn add_managed_dependent_fixture(&mut self) -> Result<(), Error> {
        if !self.standard_components_initialized() {
            return Err(Error::InvalidConfiguration(
                "managed fixture requires committed Transform",
            ));
        }
        if self.outputs.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 output limit"));
        }
        self.runtime
            .command(Command::RegisterThermalPlant(ThermalPlantConfig {
                id: DEPENDENT_PLANT,
                name: "Managed-input thermal plant".into(),
                history_capacity: 64,
                ambient_temperature: 20.0,
                initial_temperature: 20.0,
                gain_per_percent: 0.8,
                time_constant: Duration::from_secs(8),
            }))?;
        let actuator = ActuatorId::new(DEPENDENT_PLANT, lab_core::HEATER_POWER);
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(2),
                max_proposal_ttl: Duration::from_millis(200),
                required_evidence: EvidenceLevel::Readback,
            }),
        })?;
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })?;
        let CommandResult::Output(OutputResult::Dispatched(safe)) =
            self.runtime.command(Command::Output {
                actuator,
                at: Duration::ZERO,
                command: OutputCommand::BeginDispatch,
            })?
        else {
            return Err(Error::InvalidConfiguration(
                "managed fixture safe dispatch absent",
            ));
        };
        self.runtime.command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::Complete {
                dispatch_id: safe.id(),
                outcome: DispatchOutcome::ReadbackVerified,
            },
        })?;
        self.outputs.push(actuator);
        self.runtime
            .command(Command::RegisterReference(ReferenceConfig::Fixed {
                id: DEPENDENT_REFERENCE,
                value: 60.0,
                unit: Unit::CELSIUS,
            }))?;
        self.events.track_reference(DEPENDENT_REFERENCE);
        self.runtime
            .command(Command::RegisterController(NativeControllerConfig {
                id: DEPENDENT_CONTROLLER,
                input: SignalId::new(
                    InstrumentId::new(MANAGED_FILTER.get()),
                    lab_core::TEMPERATURE,
                ),
                output: actuator,
                reference: DEPENDENT_REFERENCE,
                ema: EmaConfig {
                    time_constant: Duration::from_millis(200),
                    warmup_samples: 1,
                    unit: Unit::CELSIUS,
                },
                pid: PidConfig {
                    kp: 3.0,
                    ki: 0.4,
                    kd: 0.2,
                    output_min: 0.0,
                    output_max: 100.0,
                },
                max_input_age: Duration::from_millis(750),
                max_tick_gap: Duration::from_millis(750),
                lease_lifetime: Duration::from_secs(2),
                proposal_ttl: Duration::from_millis(200),
            }))?;
        self.runtime
            .command(Command::PrepareController(DEPENDENT_CONTROLLER))?;
        self.events.track_controller(DEPENDENT_CONTROLLER);
        self.plan
            .plants
            .push((DEPENDENT_PLANT, Periodic::new(Duration::from_millis(100))));
        self.plan.references.push((
            DEPENDENT_REFERENCE,
            Periodic::new(Duration::from_millis(100)),
        ));
        self.plan.controllers.push((
            DEPENDENT_CONTROLLER,
            SignalId::new(
                InstrumentId::new(MANAGED_FILTER.get()),
                lab_core::TEMPERATURE,
            ),
            Periodic::new(Duration::from_millis(200)),
        ));
        self.observe(self.last_now, None)?;
        Ok(())
    }
}
