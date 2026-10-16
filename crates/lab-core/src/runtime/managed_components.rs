//! Managed-component validation, invocation, bounded completion and commit progression.
//!
//! Worker completions are plain non-authoritative results until this parent `Runtime`
//! validates generation, revision, shape and signal data and commits them.

use super::*;

impl Runtime {
    /// Validate the manifest and topology before sending disposable init work to a worker.
    fn validate_component_definition(
        &self,
        definition: &ComponentDefinition,
        replaces: Option<ComponentId>,
    ) -> Result<(), Error> {
        let manifest = &definition.manifest;
        validate_name(&manifest.name)?;
        definition.config.validate()?;
        if manifest.schema_version != 1
            || !manifest.min.is_finite()
            || !manifest.max.is_finite()
            || manifest.min >= manifest.max
            || !(1..=64).contains(&manifest.warmup_samples)
            || manifest.max_input_age.is_zero()
        {
            return Err(ComponentError::InvalidConfiguration.into());
        }
        SignalBuffer::new(
            SignalId::new(manifest.instrument, manifest.parameter),
            manifest.history_capacity,
        )?;
        match replaces {
            None => {
                if self.managed.contains_key(&manifest.id)
                    || self.contains_instrument(manifest.instrument)
                    || self.managed.len() >= MAX_COMPONENTS
                    || self.instrument_count() >= MAX_INSTRUMENTS
                {
                    return Err(ComponentError::InvalidConfiguration.into());
                }
            }
            Some(id) => {
                let old = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
                let previous = &old.definition.manifest;
                if id != manifest.id
                    || previous.instrument != manifest.instrument
                    || previous.parameter != manifest.parameter
                    || previous.kind != manifest.kind
                    || previous.unit != manifest.unit
                    || previous.min != manifest.min
                    || previous.max != manifest.max
                    || previous.name != manifest.name
                    || previous.warmup_samples != manifest.warmup_samples
                    || previous.max_input_age != manifest.max_input_age
                    || previous.history_capacity != manifest.history_capacity
                {
                    return Err(ComponentError::InvalidConfiguration.into());
                }
                old.generation
                    .checked_add(1)
                    .ok_or(ComponentError::IdentityExhausted)?;
            }
        }
        if let ComponentKind::Transform { input } = manifest.kind {
            if input.instrument() == manifest.instrument {
                return Err(ComponentError::InvalidConfiguration.into());
            }
            if self.managed.values().any(|item| {
                item.definition.manifest.instrument == input.instrument()
                    && matches!(
                        item.definition.manifest.kind,
                        ComponentKind::Transform { .. }
                    )
            }) {
                return Err(ComponentError::InvalidConfiguration.into());
            }
            let parameter = self
                .descriptor(input.instrument())?
                .parameter(input.parameter())
                .ok_or(ComponentError::InvalidConfiguration)?;
            if parameter.signal != Some(input)
                || parameter.role != ParameterRole::Measurement
                || !matches!(parameter.value_spec, ValueSpec::Float { .. })
                || parameter.unit != manifest.unit
            {
                return Err(ComponentError::InvalidConfiguration.into());
            }
        }
        Ok(())
    }

    pub(super) fn stage_component(
        &mut self,
        definition: ComponentDefinition,
        replaces: Option<ComponentId>,
        at: Duration,
        prepare_only: bool,
    ) -> Result<CommandResult, Error> {
        if self.managed_quiesced {
            return Err(ComponentError::Executor.into());
        }
        if self.staged.is_some() {
            return Err(ComponentError::Busy.into());
        }
        if prepare_only
            && (self.prepared.len() >= MAX_COMPONENTS
                || self.prepared.contains_key(&definition.manifest.id))
        {
            return Err(ComponentError::Busy.into());
        }
        self.validate_component_definition(&definition, replaces)?;
        self.check_output_time(at)?;
        let generation = replaces.map_or(Ok(1), |id| {
            self.managed
                .get(&id)
                .expect("validated above")
                .generation
                .checked_add(1)
                .ok_or(ComponentError::IdentityExhausted)
        })?;
        let revision = replaces.map_or(0, |id| {
            self.managed.get(&id).expect("validated above").revision
        });
        let correlation = Correlation {
            runtime: self.component_runtime,
            component: definition.manifest.id,
            generation,
            attempt: 0,
            revision,
        };
        let job = Invocation {
            correlation,
            phase: InvocationPhase::Init,
            definition: definition.clone(),
            state: PlainData::default(),
            at,
            dt: Duration::ZERO,
            input: None,
        };
        self.executor
            .as_mut()
            .ok_or(ComponentError::Executor)?
            .try_submit(job)?;
        let id = definition.manifest.id;
        self.staged = Some(StagedComponent {
            definition,
            replaces,
            correlation,
            prepare_only,
        });
        Ok(CommandResult::ComponentStaged(id))
    }

    fn managed_descriptor(manifest: &ComponentManifest) -> Result<InstrumentDescriptor, Error> {
        let signal = SignalId::new(manifest.instrument, manifest.parameter);
        let parameter = crate::ParameterDescriptor {
            id: manifest.parameter,
            name: "value".into(),
            value_spec: ValueSpec::Float {
                min: manifest.min,
                max: manifest.max,
            },
            unit: manifest.unit,
            access: crate::AccessMode::ReadOnly,
            role: ParameterRole::Measurement,
            write_effect: crate::WriteEffect::None,
            signal: Some(signal),
        };
        parameter.validate_definition()?;
        Ok(InstrumentDescriptor {
            id: manifest.instrument,
            name: manifest.name.clone(),
            parameters: vec![parameter],
        })
    }

    fn validate_managed_result(
        result: &crate::managed::ComponentResult,
        manifest: &ComponentManifest,
        phase: InvocationPhase,
    ) -> Result<(), ComponentError> {
        if result.unit != manifest.unit
            || result.state.validate().is_err()
            || result.diagnostics.len() > 4
            || result.diagnostics.iter().any(|line| line.len() > 256)
            || result.diagnostics.iter().map(String::len).sum::<usize>() > 1024
        {
            return Err(ComponentError::InvalidResult);
        }
        match (phase, result.status, result.value) {
            (InvocationPhase::Init, ComponentStatus::Init, None) => Ok(()),
            (InvocationPhase::Step, ComponentStatus::Warming, None)
            | (InvocationPhase::Step, ComponentStatus::Unavailable, None) => Ok(()),
            (InvocationPhase::Step, ComponentStatus::Ready, Some(value))
                if value.is_finite() && (manifest.min..=manifest.max).contains(&value) =>
            {
                Ok(())
            }
            _ => Err(ComponentError::InvalidResult),
        }
    }

    /// Init is validated before quiescing dependents or replacing the committed definition.
    fn commit_staged(
        &mut self,
        staged: StagedComponent,
        result: crate::managed::ComponentResult,
        at: Duration,
    ) -> Result<(), Error> {
        Self::validate_managed_result(&result, &staged.definition.manifest, InvocationPhase::Init)?;
        let mut invalidated_dependents = Vec::new();
        if let Some(id) = staged.replaces {
            let old = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
            let pending_to_cancel = old.pending;
            if old.generation.checked_add(1) != Some(staged.correlation.generation)
                || old.revision != staged.correlation.revision
                || old.signal.latest().is_some_and(|last| last.at() > at)
            {
                return Err(ComponentError::InvalidResult.into());
            }
            let signal = SignalId::new(
                old.definition.manifest.instrument,
                old.definition.manifest.parameter,
            );
            let dependent_ids: Vec<_> = self
                .managed
                .iter()
                .filter(|(other, component)| {
                    **other != id
                        && component.definition.manifest.kind
                            == ComponentKind::Transform { input: signal }
                })
                .map(|(other, _)| *other)
                .collect();
            let mut affected_signals = vec![signal];
            for dependent in &dependent_ids {
                if let Some(component) = self.managed.get(dependent) {
                    if component.signal.latest().is_some_and(|last| last.at() > at) {
                        return Err(ComponentError::InvalidResult.into());
                    }
                    affected_signals.push(SignalId::new(
                        component.definition.manifest.instrument,
                        component.definition.manifest.parameter,
                    ));
                }
            }
            let affected: Vec<_> = self
                .controllers
                .iter()
                .filter(|(_, controller)| {
                    affected_signals.contains(&controller.config.input)
                        && matches!(
                            controller.state,
                            ControllerState::Running | ControllerState::Warming
                        )
                })
                .map(|(controller, _)| *controller)
                .collect();
            for controller in affected {
                self.pause_controller(controller, at)?;
            }
            if let Some(correlation) = pending_to_cancel
                && let Some(executor) = self.executor.as_mut()
            {
                executor.try_cancel(correlation);
            }
            for dependent in dependent_ids {
                if let Some(other) = self.managed.get_mut(&dependent) {
                    if let Some(correlation) = other.pending
                        && let Some(executor) = self.executor.as_mut()
                    {
                        executor.try_cancel(correlation);
                    }
                    other.pending = None;
                    other.pending_input = None;
                    other.pending_at = None;
                    other.state = ComponentState::Failed;
                    if other.signal.latest().is_some() {
                        other
                            .signal
                            .invalidate(at, crate::MeasurementFailure::ComponentFailure)?;
                        if let Some(sample) = other.signal.latest().cloned() {
                            invalidated_dependents.push((sample, other.generation, other.revision));
                        }
                    }
                }
            }
        }
        let manifest = &staged.definition.manifest;
        let descriptor = Self::managed_descriptor(manifest)?;
        let signal_id = SignalId::new(manifest.instrument, manifest.parameter);
        let mut replacement_failure = None;
        let signal = if let Some(old) = staged.replaces {
            let mut retained = self
                .managed
                .remove(&old)
                .ok_or(ComponentError::Unknown)?
                .signal;
            if retained.latest().is_some() {
                retained.invalidate(at, crate::MeasurementFailure::ComponentFailure)?;
                replacement_failure = retained.latest().cloned();
            }
            retained
        } else {
            SignalBuffer::new(signal_id, manifest.history_capacity)?
        };
        self.managed.insert(
            manifest.id,
            ManagedInstance {
                definition: staged.definition,
                descriptor,
                signal,
                generation: staged.correlation.generation,
                revision: 0,
                attempt: 0,
                state: ComponentState::Warming,
                committed: result.state,
                good_steps: 0,
                last_service: None,
                last_observation: None,
                pending: None,
                pending_input: None,
                pending_at: None,
                diagnostics: result.diagnostics,
            },
        );
        for (sample, generation, revision) in invalidated_dependents {
            self.recording_facts
                .managed_measurement(sample, generation, revision, None);
        }
        if let Some(sample) = replacement_failure {
            self.recording_facts.managed_measurement(
                sample,
                staged.correlation.generation,
                0,
                None,
            );
        }
        Ok(())
    }

    /// Validate a fresh typed input before any executor work is admitted.
    fn capture_component_input(
        &self,
        manifest: &ComponentManifest,
        at: Duration,
    ) -> Result<Option<CapturedInput>, ComponentError> {
        let ComponentKind::Transform { input } = manifest.kind else {
            return Ok(None);
        };
        let sample = self
            .signal(input)
            .ok()
            .and_then(SignalBuffer::latest)
            .ok_or(ComponentError::InputUnavailable)?;
        if sample.quality() != SampleQuality::Good
            || sample.unit() != manifest.unit
            || at < sample.freshness_at()
            || at - sample.freshness_at() >= manifest.max_input_age
        {
            return Err(ComponentError::InputUnavailable);
        }
        let Some(Value::Float(value)) = sample.value() else {
            return Err(ComponentError::InputUnavailable);
        };
        if !value.is_finite() {
            return Err(ComponentError::InputUnavailable);
        }
        // Freeze the producer identity with the input value before the worker
        // starts. A later managed replacement or M3 rebind cannot relabel it.
        let (source_generation, source_revision, source_state_revision) = self
            .managed
            .values()
            .find(|component| {
                SignalId::new(
                    component.definition.manifest.instrument,
                    component.definition.manifest.parameter,
                ) == input
            })
            .map(|component| (component.generation, 1, Some(component.revision)))
            .or_else(|| {
                self.metakon_instruments
                    .get(&input.instrument())
                    .map(|instrument| {
                        (
                            instrument.binding.binding_generation,
                            instrument.binding.mapping_revision,
                            None,
                        )
                    })
            })
            .unwrap_or((1, 1, None));
        Ok(Some(CapturedInput {
            signal: input,
            value: *value,
            unit: sample.unit(),
            at: sample.at(),
            freshness_at: sample.freshness_at(),
            source_generation,
            source_revision,
            source_state_revision,
        }))
    }

    pub(super) fn invoke_component(
        &mut self,
        id: ComponentId,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        self.check_output_time(at)?;
        if self.managed_quiesced {
            return Err(ComponentError::Executor.into());
        }
        let component = self.managed.get(&id).ok_or(ComponentError::Unknown)?;
        if component.state == ComponentState::Failed {
            return Err(ComponentError::InvalidResult.into());
        }
        if component.pending.is_some() {
            return Err(ComponentError::Busy.into());
        }
        let captured = self.capture_component_input(&component.definition.manifest, at);
        let input = match captured {
            Ok(input) => input,
            Err(error) => {
                self.fail_component(id, at);
                return Err(error.into());
            }
        };
        let observation = input.map_or(at, |reading| reading.freshness_at);
        if component
            .last_observation
            .is_some_and(|last| observation <= last)
            || component.last_service.is_some_and(|last| at <= last)
        {
            return Err(ComponentError::InputUnavailable.into());
        }
        let dt = component
            .last_observation
            .map_or(Duration::ZERO, |last| observation - last);
        let attempt = component
            .attempt
            .checked_add(1)
            .ok_or(ComponentError::IdentityExhausted)?;
        let correlation = Correlation {
            runtime: self.component_runtime,
            component: id,
            generation: component.generation,
            attempt,
            revision: component.revision,
        };
        let job = Invocation {
            correlation,
            phase: InvocationPhase::Step,
            definition: component.definition.clone(),
            state: component.committed.clone(),
            at,
            dt,
            input,
        };
        self.executor
            .as_mut()
            .ok_or(ComponentError::Executor)?
            .try_submit(job)?;
        let component = self.managed.get_mut(&id).expect("checked above");
        component.attempt = attempt;
        component.pending = Some(correlation);
        component.pending_input = input;
        component.pending_at = Some(at);
        Ok(CommandResult::ComponentInvoked(correlation))
    }

    /// Invalidate a failed branch and its fixed M5 dependents before further producer work.
    pub(super) fn fail_component(&mut self, id: ComponentId, at: Duration) {
        let Some(component) = self.managed.get_mut(&id) else {
            return;
        };
        if let Some(pending) = component.pending
            && let Some(executor) = self.executor.as_mut()
        {
            executor.try_cancel(pending);
        }
        let signal = SignalId::new(
            component.definition.manifest.instrument,
            component.definition.manifest.parameter,
        );
        component.state = ComponentState::Failed;
        component.pending = None;
        component.pending_input = None;
        component.pending_at = None;
        if component.signal.latest().is_some() {
            let _ = component
                .signal
                .invalidate(at, crate::MeasurementFailure::ComponentFailure);
        } else {
            let _ = component.signal.push(Sample::unavailable(
                signal,
                component.definition.manifest.unit,
                at,
                crate::MeasurementFailure::ComponentFailure,
            ));
        }
        if let Some(sample) = component.signal.latest().cloned()
            && sample.at() == at
        {
            self.recording_facts.managed_measurement(
                sample,
                component.generation,
                component.revision,
                None,
            );
        }
        let dependents: Vec<_> = self
            .managed
            .iter()
            .filter(|(other, instance)| {
                **other != id
                    && instance.definition.manifest.kind
                        == ComponentKind::Transform { input: signal }
            })
            .map(|(other, _)| *other)
            .collect();
        for dependent in &dependents {
            if let Some(other) = self.managed.get_mut(dependent) {
                if let Some(pending) = other.pending
                    && let Some(executor) = self.executor.as_mut()
                {
                    executor.try_cancel(pending);
                }
                other.state = ComponentState::Failed;
                other.pending = None;
                other.pending_input = None;
                other.pending_at = None;
                let downstream = SignalId::new(
                    other.definition.manifest.instrument,
                    other.definition.manifest.parameter,
                );
                if other.signal.latest().is_some() {
                    let _ = other
                        .signal
                        .invalidate(at, crate::MeasurementFailure::ComponentFailure);
                } else {
                    let _ = other.signal.push(Sample::unavailable(
                        downstream,
                        other.definition.manifest.unit,
                        at,
                        crate::MeasurementFailure::ComponentFailure,
                    ));
                }
                if let Some(sample) = other.signal.latest().cloned()
                    && sample.at() == at
                {
                    self.recording_facts.managed_measurement(
                        sample,
                        other.generation,
                        other.revision,
                        None,
                    );
                }
            }
        }
        let affected_signals: Vec<_> = std::iter::once(signal)
            .chain(dependents.iter().filter_map(|dependent| {
                self.managed.get(dependent).map(|instance| {
                    SignalId::new(
                        instance.definition.manifest.instrument,
                        instance.definition.manifest.parameter,
                    )
                })
            }))
            .collect();
        let affected: Vec<_> = self
            .controllers
            .iter()
            .filter(|(_, native)| {
                affected_signals.contains(&native.config.input)
                    && matches!(
                        native.state,
                        ControllerState::Running | ControllerState::Warming
                    )
            })
            .map(|(native, _)| *native)
            .collect();
        for native in affected {
            let mut controller = self.controllers.remove(&native).expect("listed above");
            if controller.state == ControllerState::Warming {
                controller.state = ControllerState::Failed;
            } else {
                let _ = self.fail_controller(&mut controller, at);
            }
            self.controllers.insert(native, controller);
        }
    }

    fn commit_step(&mut self, completion: ComponentCompletion, at: Duration) {
        let id = completion.correlation.component;
        let Some(component) = self.managed.get(&id) else {
            return;
        };
        if component.pending != Some(completion.correlation)
            || component.generation != completion.correlation.generation
            || component.revision != completion.correlation.revision
            || completion.correlation.runtime != self.component_runtime
            || component.state == ComponentState::Failed
        {
            return;
        }
        let captured = component.pending_input;
        let scheduled = component.pending_at;
        let manifest = component.definition.manifest.clone();
        if !completion.timely
            || at < scheduled.unwrap_or(Duration::ZERO)
            || captured.is_some_and(|input| {
                at < input.freshness_at
                    || at - input.freshness_at >= manifest.max_input_age
                    || self
                        .signal(input.signal)
                        .ok()
                        .and_then(SignalBuffer::latest)
                        .is_none_or(|latest| {
                            latest.quality() != SampleQuality::Good
                                || latest.freshness_at() < input.freshness_at
                        })
            })
        {
            self.fail_component(id, at);
            return;
        }
        let Ok(result) = completion.outcome else {
            self.fail_component(id, at);
            return;
        };
        if Self::validate_managed_result(&result, &manifest, InvocationPhase::Step).is_err() {
            self.fail_component(id, at);
            return;
        }
        let component = self.managed.get_mut(&id).expect("checked above");
        let Some(next_revision) = component.revision.checked_add(1) else {
            self.fail_component(id, at);
            return;
        };
        let count = component
            .good_steps
            .saturating_add(1)
            .min(manifest.warmup_samples);
        if result.status == ComponentStatus::Unavailable
            || (result.status == ComponentStatus::Ready && count < manifest.warmup_samples)
            || (component.state == ComponentState::Ready && result.status != ComponentStatus::Ready)
        {
            self.fail_component(id, at);
            return;
        }
        let signal = SignalId::new(manifest.instrument, manifest.parameter);
        let source_at = captured.map_or(
            scheduled.expect("pending job has scheduled time"),
            |input| input.freshness_at,
        );
        let published = if let Some(value) = result.value {
            Sample::derived_good(signal, manifest.unit, at, source_at, Value::Float(value))
        } else {
            Ok(Sample::unavailable(
                signal,
                manifest.unit,
                at,
                crate::MeasurementFailure::ProcessingWarmup,
            ))
        };
        let Ok(sample) = published else {
            self.fail_component(id, at);
            return;
        };
        if component.signal.push(sample.clone()).is_err() {
            self.fail_component(id, at);
            return;
        }
        component.committed = result.state;
        component.revision = next_revision;
        component.good_steps = count;
        component.state = if result.status == ComponentStatus::Ready {
            ComponentState::Ready
        } else {
            ComponentState::Warming
        };
        component.pending = None;
        component.pending_input = None;
        component.pending_at = None;
        component.diagnostics = result.diagnostics;
        component.last_service = Some(at);
        component.last_observation = Some(source_at);
        self.recording_facts.managed_measurement(
            sample,
            component.generation,
            component.revision,
            captured,
        );
    }

    pub(super) fn poll_components(&mut self, at: Duration) -> Result<(), Error> {
        self.service_safety(at)?;
        if let Some(staged) = &self.staged
            && self
                .executor
                .as_mut()
                .is_some_and(|port| port.try_expire(staged.correlation))
        {
            let staged = self.staged.take().expect("checked above");
            self.executor
                .as_mut()
                .expect("checked above")
                .try_cancel(staged.correlation);
        }
        let expired: Vec<_> = self
            .managed
            .iter()
            .filter_map(|(id, instance)| {
                instance
                    .pending
                    .filter(|correlation| {
                        self.executor
                            .as_mut()
                            .is_some_and(|port| port.try_expire(*correlation))
                    })
                    .map(|_| *id)
            })
            .collect();
        for id in expired {
            self.fail_component(id, at);
        }
        for _ in 0..2 {
            let Some(completion) = self.executor.as_mut().and_then(|port| port.try_poll()) else {
                break;
            };
            if self
                .staged
                .as_ref()
                .is_some_and(|candidate| candidate.correlation == completion.correlation)
            {
                let staged = self.staged.take().expect("checked above");
                if completion.timely
                    && let Ok(result) = completion.outcome
                {
                    if staged.prepare_only {
                        if Self::validate_managed_result(
                            &result,
                            &staged.definition.manifest,
                            InvocationPhase::Init,
                        )
                        .is_ok()
                        {
                            self.prepared.insert(
                                staged.definition.manifest.id,
                                PreparedComponent { staged, result },
                            );
                        }
                    } else {
                        let _ = self.commit_staged(staged, result, at);
                    }
                }
            } else {
                self.commit_step(completion, at);
            }
        }
        Ok(())
    }

    pub(super) fn commit_prepared_components(
        &mut self,
        components: &[ComponentId],
        at: Duration,
    ) -> Result<CommandResult, Error> {
        if components.is_empty() || components.len() > MAX_COMPONENTS {
            return Err(ComponentError::InvalidConfiguration.into());
        }
        let mut unique = std::collections::BTreeSet::new();
        let mut batch = Vec::with_capacity(components.len());
        for id in components {
            if !unique.insert(*id) {
                return Err(ComponentError::InvalidConfiguration.into());
            }
            let mut prepared = self
                .prepared
                .get(id)
                .ok_or(ComponentError::Unknown)?
                .clone();
            self.validate_component_definition(
                &prepared.staged.definition,
                prepared.staged.replaces,
            )?;
            Self::validate_managed_result(
                &prepared.result,
                &prepared.staged.definition.manifest,
                InvocationPhase::Init,
            )?;
            let old = self.managed.get(id).ok_or(ComponentError::Unknown)?;
            if old.generation.checked_add(1) != Some(prepared.staged.correlation.generation)
                || old.signal.latest().is_some_and(|sample| sample.at() > at)
            {
                return Err(ComponentError::InvalidResult.into());
            }
            // Ordinary old-generation steps may commit while candidate init runs.
            // The replacement deliberately resets plain state, so fence by
            // generation and refresh the internal state revision at atomic commit.
            prepared.staged.correlation.revision = old.revision;
            batch.push(prepared);
        }
        // Every candidate and generation is checked before the first committed
        // replacement. Any safety pause performed by commit remains fail-closed;
        // it can never rearm authority if a later invariant unexpectedly fails.
        for prepared in batch {
            self.commit_staged(prepared.staged, prepared.result, at)?;
        }
        for id in components {
            self.prepared.remove(id);
        }
        Ok(CommandResult::ComponentsCommitted(components.len()))
    }

    /// Whether a validated replacement init is retained outside the active graph.
    pub fn component_prepared(&self, id: ComponentId) -> bool {
        self.prepared.contains_key(&id)
    }

    /// Copy one committed bounded definition for trusted host composition/provenance.
    /// This is not an Application query and never executes implementation code.
    pub fn component_definition(&self, id: ComponentId) -> Option<ComponentDefinition> {
        self.managed
            .get(&id)
            .map(|component| component.definition.clone())
    }

    /// Whether the single managed init staging slot is currently occupied.
    pub fn component_prepare_pending(&self) -> bool {
        self.staged.is_some()
    }
}
