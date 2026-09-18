//! Recorder lifecycle, bounded semantic-fact admission and durable-history orchestration.
//!
//! The methods here remain implementations of the single HostCore owner declared
//! in the parent module; this module introduces no additional mutable state owner.

use super::*;

impl HostCore {
    /// Attach one already-open worker under trusted host composition.
    /// Required control starts closed and opens only on a committed start receipt.
    pub fn attach_recorder(
        &mut self,
        mut worker: RecorderWorker,
        policy: RecordingPolicy,
        at: Duration,
    ) -> Result<(), Error> {
        if self.recorder.is_some() || self.stopping || at < self.last_now {
            return Err(Error::InvalidConfiguration("recorder attach state"));
        }
        let (entries, objects) = self.frozen_activation_entries()?;
        worker
            .configure_policy(policy)
            .map_err(|_| Error::InvalidConfiguration("recorder policy configuration"))?;
        worker
            .request_activation(entries, objects)
            .map_err(|_| Error::InvalidConfiguration("recorder activation admission"))?;
        if policy == RecordingPolicy::Required {
            self.runtime.require_recording(at);
        }
        self.recording_status = Some(worker.poll());
        self.recording_policy = Some(policy);
        self.recorder = Some(worker);
        Ok(())
    }

    /// Read the latest owned receipt/status without SQL or hidden worker polling.
    pub fn recording_status(&self) -> Option<&RecordingStatus> {
        self.recording_status.as_ref()
    }

    /// Observe the startup activation receipt without SQL or owner blocking.
    pub fn recording_activation_committed(&mut self) -> Result<bool, Error> {
        let Some(worker) = self.recorder.as_mut() else {
            return Ok(true);
        };
        let status = worker.poll();
        let committed = status.activation_root.is_some();
        let failed = status.state == RecordingState::Failed;
        self.recording_status = Some(status);
        if failed {
            return Err(Error::InvalidConfiguration("recorder activation failed"));
        }
        Ok(committed)
    }

    /// Close Required ordinary admission before a configuration commit whose
    /// durable activation receipt has not yet arrived.
    pub(crate) fn begin_configuration_recording_fence(&mut self, at: Duration) -> bool {
        let was_open = self.recording_policy == Some(RecordingPolicy::Required)
            && self.runtime.required_recording_open();
        if was_open {
            self.runtime.require_recording(at);
        }
        was_open
    }

    /// Suspend new acquisition/control production while safety and already
    /// admitted transport/Recorder work continue to make bounded progress.
    pub(crate) fn begin_configuration_quiesce(&mut self) {
        self.configuration_quiesced = true;
    }

    /// Resume only acquisition scheduling after a confirmed activation outcome.
    /// Controllers and output authority remain in their post-barrier state.
    pub(crate) fn end_configuration_quiesce(&mut self) {
        self.configuration_quiesced = false;
    }

    /// Drain Core facts and reserve one of the existing Recorder ingress groups.
    /// Outer `None` means backpressure; inner `None` means recording is disabled.
    pub(crate) fn try_reserve_configuration_activation(
        &mut self,
        lifecycle: &ConfigurationLifecycleRecord,
        at: Duration,
    ) -> Result<Option<Option<u64>>, Error> {
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        let Some(worker) = self.recorder.as_mut() else {
            return Ok(Some(None));
        };
        worker
            .try_reserve_live_activation(lifecycle)
            .map(|reservation| reservation.map(Some))
            .map_err(|_| Error::InvalidConfiguration("live activation reservation rejected"))
    }

    /// Fill the reserved activation only after the owner commit has succeeded.
    pub(crate) fn commit_reserved_configuration_activation(
        &mut self,
        generation: Option<u64>,
        lifecycle: ConfigurationLifecycleRecord,
    ) -> Result<(), Error> {
        let Some(generation) = generation else {
            return Ok(());
        };
        let (entries, objects) = self.frozen_activation_entries()?;
        self.recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder reservation lost"))?
            .commit_reserved_live_activation(generation, entries, objects, lifecycle)
            .map_err(|_| Error::InvalidConfiguration("live activation rejected"))
    }

    pub(crate) fn cancel_configuration_activation(
        &mut self,
        generation: Option<u64>,
    ) -> Result<(), Error> {
        let Some(generation) = generation else {
            return Ok(());
        };
        self.recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder reservation lost"))?
            .cancel_live_activation_reservation(generation)
            .map_err(|_| Error::InvalidConfiguration("live activation cancellation failed"))
    }

    pub(crate) fn configuration_recording_failed(&mut self, at: Duration) {
        if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(at);
        }
    }

    /// Poll a specific live activation receipt and reopen Required admission only
    /// from that durable generation.
    pub(crate) fn live_activation_committed(
        &mut self,
        generation: Option<u64>,
        reopen_required: bool,
        submitted_at: Duration,
        now: Duration,
    ) -> Result<bool, Error> {
        let Some(generation) = generation else {
            return Ok(true);
        };
        self.poll_recorder(now);
        let status = self
            .recording_status
            .as_ref()
            .ok_or(Error::InvalidConfiguration("recorder status missing"))?;
        if status.state == RecordingState::Failed {
            self.runtime.recording_failure(now);
            return Err(Error::RecordingUnavailable);
        }
        if status.activation_generation < generation {
            return Ok(false);
        }
        if reopen_required {
            self.runtime.confirm_recording_start(submitted_at, now)?;
        }
        Ok(true)
    }

    // Capture the actual currently committed scalar composition before any
    // worker hashing. All source bytes come from the fixed loaded component
    // definitions, never from a pathname reread at recording time.
    fn frozen_activation_entries(
        &self,
    ) -> Result<(Vec<ProvenanceEntry>, Vec<ProvenanceObject>), Error> {
        let mut entries = self.deployment_provenance.clone();
        entries.reserve(8);
        let push =
            |entries: &mut Vec<ProvenanceEntry>, kind: &str, encoding: &str, content: Vec<u8>| {
                entries.push(ProvenanceEntry {
                    kind: kind.into(),
                    encoding: encoding.into(),
                    content,
                })
            };
        push(
            &mut entries,
            "rust_build",
            "utf8",
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
        );
        let mut controllers = Vec::with_capacity(self.plan.controllers.len());
        for (id, _, _) in &self.plan.controllers {
            let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(*id))?
            else {
                return Err(Error::InvalidConfiguration("controller config unavailable"));
            };
            let QueryResult::Controller(state) = self.runtime.query(Query::Controller(*id))? else {
                return Err(Error::InvalidConfiguration("controller state unavailable"));
            };
            controllers.push(serde_json::json!({
                "id":config.id.get().to_string(),"revision":state.config_revision.to_string(),
                "input":{"instrument":config.input.instrument().get().to_string(),
                    "parameter":config.input.parameter().get().to_string()},
                "output":{"instrument":config.output.instrument().get().to_string(),
                    "parameter":config.output.parameter().get().to_string()},
                "reference":config.reference.get().to_string(),
                "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
                    "output_min":config.pid.output_min,"output_max":config.pid.output_max},
                "ema":{"time_constant_ns":config.ema.time_constant.as_nanos().to_string(),
                    "warmup_samples":config.ema.warmup_samples,"unit":config.ema.unit.id()},
                "max_input_age_ns":config.max_input_age.as_nanos().to_string(),
                "max_tick_gap_ns":config.max_tick_gap.as_nanos().to_string(),
                "lease_lifetime_ns":config.lease_lifetime.as_nanos().to_string(),
                "proposal_ttl_ns":config.proposal_ttl.as_nanos().to_string(),
            }));
        }
        let mut references = Vec::with_capacity(self.plan.references.len());
        for (id, _) in &self.plan.references {
            let QueryResult::Reference(reference) = self.runtime.query(Query::Reference(*id))?
            else {
                return Err(Error::InvalidConfiguration("reference unavailable"));
            };
            references.push(match reference {
                ReferenceSnapshot::Ramp {
                    id,
                    state,
                    revision,
                } => serde_json::json!({
                    "kind":"ramp","id":id.get().to_string(),"revision":revision.to_string(),
                    "current":state.current,"target":state.target,"rate":state.rate,
                    "unit":state.unit.id(),"progress_at_ns":state.last_at.as_nanos().to_string()}),
                ReferenceSnapshot::Fixed {
                    id,
                    value,
                    unit,
                    revision,
                    ..
                } => serde_json::json!({
                    "kind":"fixed","id":id.get().to_string(),"revision":revision.to_string(),
                    "value":value,"unit":unit.id()}),
            });
        }
        let mut outputs = Vec::with_capacity(self.active_safety_profiles.len());
        for (actuator, profile) in &self.active_safety_profiles {
            let unit = match self
                .runtime
                .query(Query::DescribeInstrument(actuator.instrument()))?
            {
                QueryResult::Descriptor(descriptor) => descriptor
                    .parameter(actuator.parameter())
                    .map(|parameter| parameter.unit.id().to_owned())
                    .ok_or(Error::InvalidConfiguration("safety output unit missing"))?,
                _ => {
                    return Err(Error::InvalidConfiguration(
                        "safety output descriptor missing",
                    ));
                }
            };
            outputs.push(serde_json::json!({
                "instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"unit":unit,
                "min":profile.min,"max":profile.max,"safe_value":profile.safe_value,
                "max_lease_ns":profile.max_lease.as_nanos().to_string(),
                "max_proposal_ttl_ns":profile.max_proposal_ttl.as_nanos().to_string(),
                "required_evidence":match profile.required_evidence {
                    EvidenceLevel::Acknowledgement=>"ack",EvidenceLevel::Readback=>"readback"},
            }));
        }
        push(
            &mut entries,
            "native_composition",
            "json_v1",
            serde_json::json!({"profile":"virtual-demo","controllers":&controllers,
                "references":&references,"safety_outputs":&outputs})
            .to_string()
            .into_bytes(),
        );
        let mut managed_definition_indices = BTreeMap::new();
        let mut cached_binary_hash = None;
        for (id, _) in &self.components {
            let definition = self
                .runtime
                .component_definition(*id)
                .ok_or(Error::InvalidConfiguration("managed definition missing"))?;
            managed_definition_indices.insert(*id, entries.len());
            let mut implementation = serde_json::json!({
                "component":id.get().to_string(),
                "implementation":definition.implementation.id().as_str(),
                "artifact":"built_in",
                "package_version":env!("CARGO_PKG_VERSION"),
                "config":plain_data_activation_json(&definition.config),
            });
            let binary_hash = match cached_binary_hash {
                Some(hash) => hash,
                None => {
                    let hash = runtime_binary_sha256().map_err(|_| {
                        Error::InvalidConfiguration("runtime binary identity unavailable")
                    })?;
                    cached_binary_hash = Some(hash);
                    hash
                }
            };
            implementation["runtime_binary_sha256"] = hex_sha256(binary_hash).into();
            push(
                &mut entries,
                "managed_component_implementation",
                "json_v1",
                implementation.to_string().into_bytes(),
            );
        }
        push(
            &mut entries,
            "deployment_config",
            "utf8",
            b"not_present".to_vec(),
        );
        let mut objects = Vec::new();
        let QueryResult::Instruments(instruments) = self.runtime.query(Query::Discover)? else {
            return Err(Error::InvalidConfiguration(
                "activation discovery unavailable",
            ));
        };
        for instrument in instruments {
            let component = self
                .components
                .iter()
                .find(|(id, _)| id.get() == instrument.id.get())
                .map(|(id, _)| *id);
            let (generation, source_index) = if let Some(id) = component {
                let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(id))?
                else {
                    return Err(Error::InvalidConfiguration(
                        "component baseline unavailable",
                    ));
                };
                (
                    snapshot.generation,
                    *managed_definition_indices
                        .get(&id)
                        .ok_or(Error::InvalidConfiguration("component baseline missing"))?,
                )
            } else {
                (1, 1)
            };
            let parameters: Vec<_> = instrument
                .parameters
                .iter()
                .map(parameter_activation_json)
                .collect();
            let binding = self.runtime.metakon_binding(instrument.id).map(|binding| {
                serde_json::json!({
                    "resource":binding.resource.get().to_string(),
                    "device":binding.device,
                    "channel":binding.channel,
                    "binding_generation":binding.binding_generation.to_string(),
                    "mapping_revision":binding.mapping_revision.to_string(),
                    "expected_output_unit":binding.expected_output_unit.map(|unit|unit.id().to_owned()),
                })
                .to_string()
            });
            objects.push(ProvenanceObject {
                kind: "instrument",
                id: instrument.id.get().to_be_bytes().to_vec(),
                logical_key: format!("instrument:{}", instrument.id.get()),
                label: instrument.name,
                descriptor: serde_json::json!({"parameters":parameters}).to_string(),
                unit_key: None,
                generation: Some(generation),
                binding,
                definition_entry_index: source_index,
                source_content_sha256: None,
            });
        }
        for controller in controllers {
            let id = controller["id"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation controller id"))?;
            objects.push(ProvenanceObject {
                kind: "controller",
                id: id.to_be_bytes().to_vec(),
                logical_key: format!("controller:{id}"),
                label: format!("Native PID {id}"),
                descriptor: controller.to_string(),
                unit_key: None,
                generation: Some(1),
                binding: Some(controller["output"].to_string()),
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for reference in references {
            let id = reference["id"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation reference id"))?;
            objects.push(ProvenanceObject {
                kind: "reference",
                id: id.to_be_bytes().to_vec(),
                logical_key: format!("reference:{id}"),
                label: format!("Reference {id}"),
                unit_key: reference["unit"].as_str().map(str::to_owned),
                descriptor: reference.to_string(),
                generation: Some(1),
                binding: None,
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for output in outputs {
            let instrument = output["instrument"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation output instrument"))?;
            let parameter = output["parameter"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .ok_or(Error::InvalidConfiguration("activation output parameter"))?;
            let mut id = Vec::with_capacity(16);
            id.extend_from_slice(&instrument.to_be_bytes());
            id.extend_from_slice(&parameter.to_be_bytes());
            objects.push(ProvenanceObject {
                kind: "actuator",
                id,
                logical_key: format!("actuator:{instrument}:{parameter}"),
                label: format!("Actuator {instrument}:{parameter}"),
                unit_key: output["unit"].as_str().map(str::to_owned),
                descriptor: output.to_string(),
                generation: Some(1),
                binding: None,
                definition_entry_index: 1,
                source_content_sha256: None,
            });
        }
        for (id, _) in &self.components {
            let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "component snapshot unavailable",
                ));
            };
            objects.push(ProvenanceObject {kind:"managed_component",
                id:id.get().to_be_bytes().to_vec(),logical_key:format!("component:{}",id.get()),
                label:format!("Managed component {}",id.get()),
                descriptor:serde_json::json!({"instrument":snapshot.instrument.get().to_string(),
                    "implementation":snapshot.implementation.as_str(),
                    "generation":snapshot.generation.to_string(),"state_revision":snapshot.revision.to_string(),
                    "committed_state":plain_data_activation_json(&snapshot.committed_state)})
                    .to_string(),unit_key:None,generation:Some(snapshot.generation),binding:None,
                definition_entry_index:*managed_definition_indices.get(id)
                    .ok_or(Error::InvalidConfiguration("component definition index missing"))?,
                source_content_sha256:None});
        }
        Ok((entries, objects))
    }

    /// Stable database identity of this host's durable history, if enabled.
    pub fn recording_database_id(&self) -> Option<&str> {
        self.recorder.as_ref().map(RecorderWorker::database_id)
    }

    /// Boot identity fixed when an already-open worker is attached by trusted
    /// composition, so its archive keys and public serving boot cannot diverge.
    pub fn recording_boot_id(&self) -> Option<&str> {
        self.recorder.as_ref().map(RecorderWorker::boot_id)
    }

    /// Recording policy fixed by trusted startup composition.
    pub fn recording_policy(&self) -> Option<RecordingPolicy> {
        self.recording_policy
    }

    /// Emit one typed application fact through Recorder credit without changing
    /// the command's truthful domain result or waiting for SQLite.
    pub fn record_operation(&mut self, operation: OperationRecord) {
        let at = operation.at;
        let key = (operation.scope.clone(), operation.request_seq);
        let accepted = operation.phase == "accepted";
        let state = self.recording_status.as_ref().map(|status| status.state);
        if accepted {
            if self.pending_operations.len() >= 64 && !self.pending_operations.contains_key(&key) {
                if let Some(worker) = self.recorder.as_mut() {
                    worker.fail_with_gap(RecorderGap {
                        reason: "pending operation identity credit exhausted".into(),
                        at,
                        first_missing_fact: None,
                        known_missing_count: Some(1),
                        last_accepted_fact: None,
                    });
                }
                if self.recording_policy == Some(RecordingPolicy::Required) {
                    self.runtime.recording_failure(at);
                }
                self.poll_recorder(at);
                return;
            }
            let defer =
                state == Some(RecordingState::Idle) && operation.command == "recording_start";
            self.pending_operations.insert(
                key.clone(),
                PendingOperation {
                    command: operation.command,
                    accepted_at: at,
                    accepted_recorded: false,
                    deferred_start: defer.then(|| operation.clone()),
                    deferred_safe_terminal: None,
                },
            );
            if defer {
                self.poll_recorder(at);
                return;
            }
        }
        let in_interval = matches!(
            state,
            Some(RecordingState::Starting | RecordingState::Recording)
        );
        let lifecycle_terminal = !accepted
            && state == Some(RecordingState::Idle)
            && operation.command == "recording_stop";
        if in_interval || lifecycle_terminal {
            let defer_safe_terminal = !accepted
                && operation.command == "controller_pause"
                && self.recorder.as_mut().is_some_and(|worker| {
                    matches!(worker.operation_credit_available(&operation), Ok(false))
                });
            if defer_safe_terminal {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.deferred_safe_terminal = Some(operation);
                }
                self.poll_recorder(at);
                return;
            }
            let admitted = self
                .recorder
                .as_mut()
                .is_some_and(|worker| worker.try_admit_operation(operation).is_ok());
            if admitted && accepted {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.accepted_recorded = true;
                }
            } else if !admitted && self.recording_policy == Some(RecordingPolicy::Required) {
                self.runtime.recording_failure(at);
            }
        }
        if !accepted {
            self.pending_operations.remove(&key);
        }
        self.poll_recorder(at);
    }

    /// Admit one informational annotation while the current interval is active.
    /// The returned identity proves FIFO admission only; SQL remains pending.
    pub fn annotate(&mut self, annotation: AnnotationRecord) -> Result<u64, Error> {
        let at = annotation.at;
        if self.recording_status.as_ref().map(|status| status.state)
            != Some(RecordingState::Recording)
        {
            return Err(Error::RecordingUnavailable);
        }
        let result = self
            .recorder
            .as_mut()
            .ok_or(Error::RecordingUnavailable)?
            .try_admit_annotation(annotation)
            .map_err(|_| Error::RecordingUnavailable);
        if result.is_err() && self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(at);
        }
        self.poll_recorder(at);
        result
    }

    /// Schedule indexed archive work on the storage worker, never on this owner.
    pub fn request_history(
        &mut self,
        filter: HistoryFilter,
        cursor: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.recorder
            .as_mut()
            .ok_or_else(StorageError::disabled)?
            .request_history(filter, cursor, limit)
    }

    /// Take an already-completed bounded worker page without disk I/O.
    pub fn try_take_history(&mut self, job: u64) -> Option<Result<HistoryPage, StorageError>> {
        self.recorder.as_mut()?.try_take_history(job)
    }

    /// Schedule indexed archive-run discovery off the Runtime owner lane.
    pub fn request_runs(
        &mut self,
        cursor: Option<RunsCursor>,
        limit: usize,
    ) -> Result<u64, StorageError> {
        self.recorder
            .as_mut()
            .ok_or_else(StorageError::disabled)?
            .request_runs(cursor, limit)
    }

    /// Take an already-completed bounded run listing without SQL.
    pub fn try_take_runs(&mut self, job: u64) -> Option<Result<RunsPage, StorageError>> {
        self.recorder.as_mut()?.try_take_runs(job)
    }

    /// Fence a timed-out or disconnected history job before its late result.
    pub fn cancel_history(&mut self, job: u64) {
        if let Some(worker) = self.recorder.as_mut() {
            worker.cancel_history(job);
        }
    }

    /// Submit the start barrier; this does not start a controller or claim durability.
    pub fn start_recording(&mut self, label: &str, at: Duration) -> Result<(), Error> {
        if self.stopping || at < self.last_now {
            return Err(Error::InvalidConfiguration("recording start time/state"));
        }
        let boundary = self.frozen_boundary_snapshot(at)?;
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_start_with_boundary(label, at, boundary)
            .map_err(|_| Error::InvalidConfiguration("recording start rejected"))?;
        // The owner boundary is fixed before returning. Facts produced while
        // SQLite is still committing Start enter FIFO after that barrier, with
        // no Required control authority until its confirmed receipt.
        self.runtime.enable_recording_facts();
        // Inspect the committed successor against the previous owner state.
        // A fast worker may receipt Start before this method returns; assigning
        // its new status first would skip the Required gate and fact-capture
        // transition entirely.
        self.poll_recorder(at);
        Ok(())
    }

    // Copy authoritative committed snapshots while the owner is serialized.
    // Their original timestamps remain inside the boundary, so the archive
    // cannot mistake an old attempt for a new post-start measurement.
    fn frozen_boundary_snapshot(&self, at: Duration) -> Result<BoundarySnapshot, Error> {
        let QueryResult::Instruments(instruments) = self.runtime.query(Query::Discover)? else {
            return Err(Error::InvalidConfiguration(
                "boundary discovery unavailable",
            ));
        };
        let mut latest_samples = Vec::new();
        for instrument in instruments {
            for parameter in instrument.parameters {
                let Some(signal) = parameter.signal else {
                    continue;
                };
                let QueryResult::Latest(latest) =
                    self.runtime.query(Query::GetLatestSignal(signal))?
                else {
                    return Err(Error::InvalidConfiguration("boundary latest unavailable"));
                };
                if let Some(sample) = latest {
                    latest_samples.push(sample_boundary_json(&sample));
                }
            }
        }
        let mut controller_revisions = Vec::new();
        let mut controller_configurations = Vec::new();
        for (id, _, _) in &self.plan.controllers {
            let QueryResult::Controller(snapshot) = self.runtime.query(Query::Controller(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary controller unavailable",
                ));
            };
            controller_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "revision":snapshot.config_revision.to_string()}));
            let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary controller config unavailable",
                ));
            };
            controller_configurations.push(serde_json::json!({
                "id":id.get().to_string(),"revision":snapshot.config_revision.to_string(),
                "input":{"instrument":config.input.instrument().get().to_string(),
                    "parameter":config.input.parameter().get().to_string()},
                "output":{"instrument":config.output.instrument().get().to_string(),
                    "parameter":config.output.parameter().get().to_string()},
                "reference":config.reference.get().to_string(),
                "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
                    "output_min":config.pid.output_min,"output_max":config.pid.output_max},
                "ema":{"time_constant_ns":config.ema.time_constant.as_nanos().to_string(),
                    "warmup_samples":config.ema.warmup_samples,
                    "unit":config.ema.unit.id()},
                "max_input_age_ns":config.max_input_age.as_nanos().to_string(),
                "max_tick_gap_ns":config.max_tick_gap.as_nanos().to_string(),
                "lease_lifetime_ns":config.lease_lifetime.as_nanos().to_string(),
                "proposal_ttl_ns":config.proposal_ttl.as_nanos().to_string(),
            }));
        }
        let mut reference_revisions = Vec::new();
        let mut reference_configurations = Vec::new();
        for (id, _) in &self.plan.references {
            let QueryResult::Reference(snapshot) = self.runtime.query(Query::Reference(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary reference unavailable",
                ));
            };
            let revision = match snapshot {
                ReferenceSnapshot::Fixed { revision, .. }
                | ReferenceSnapshot::Ramp { revision, .. } => revision,
            };
            reference_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "revision":revision.to_string()}));
            reference_configurations.push(match snapshot {
                ReferenceSnapshot::Fixed {
                    value,
                    unit,
                    last_at,
                    ..
                } => serde_json::json!({
                    "kind":"fixed","id":id.get().to_string(),
                    "revision":revision.to_string(),"value":value,"unit":unit.id(),
                    "last_at_ns":last_at.map(|at|at.as_nanos().to_string())}),
                ReferenceSnapshot::Ramp { state, .. } => serde_json::json!({
                    "kind":"ramp","id":id.get().to_string(),
                    "revision":revision.to_string(),"current":state.current,
                    "target":state.target,"rate":state.rate,"unit":state.unit.id(),
                    "progress_at_ns":state.last_at.as_nanos().to_string()}),
            });
        }
        let mut managed_revisions = Vec::new();
        for (id, _) in &self.components {
            let QueryResult::Component(snapshot) = self.runtime.query(Query::Component(*id))?
            else {
                return Err(Error::InvalidConfiguration(
                    "boundary component unavailable",
                ));
            };
            managed_revisions.push(serde_json::json!({"id":id.get().to_string(),
                "generation":snapshot.generation.to_string(),
                "revision":snapshot.revision.to_string()}));
        }
        let data = serde_json::json!({"captured_at_ns":at.as_nanos().to_string(),
            "latest_samples":latest_samples,"controller_revisions":controller_revisions,
            "controller_configurations":controller_configurations,
            "reference_revisions":reference_revisions,
            "reference_configurations":reference_configurations,
            "managed_revisions":managed_revisions,
            "pending_operations":self.pending_operations.iter().map(|((scope,seq),pending)|
                serde_json::json!({"scope":scope,"request_seq":seq.to_string(),
                    "command":pending.command,"accepted_at_ns":pending.accepted_at.as_nanos().to_string(),
                    "accepted_recorded":pending.accepted_recorded})).collect::<Vec<_>>()})
        .to_string();
        if data.len() > 64 * 1024 {
            return Err(Error::InvalidConfiguration(
                "boundary snapshot credit exhausted",
            ));
        }
        Ok(BoundarySnapshot { at, data })
    }

    /// Submit the stop barrier only after the tracked Required controllers pause.
    pub fn stop_recording(&mut self) -> Result<(), Error> {
        self.stop_recording_at(self.last_now)
    }

    /// Seal at the actual owner admission time supplied by the public operation.
    pub fn stop_recording_at(&mut self, at: Duration) -> Result<(), Error> {
        if at < self.last_now {
            return Err(Error::InvalidConfiguration("recording stop time regressed"));
        }
        // Move every already-committed Core fact into the Recorder FIFO before
        // reserving the seal. Otherwise a fact can remain behind the public stop
        // boundary merely because owner service has not reached its next drain.
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        if self.recording_policy == Some(RecordingPolicy::Required)
            && (self.controllers_active() || !self.outputs_safe_for_required_stop())
        {
            return Err(Error::InvalidConfiguration(
                "required recording safety unresolved",
            ));
        }
        let summary = self.frozen_stop_summary(at);
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_stop_with_summary(summary, at)
            .map_err(|_| Error::InvalidConfiguration("recording stop rejected"))?;
        if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.close_required_recording();
        }
        self.runtime.disable_recording_facts();
        self.recording_status = Some(worker.poll());
        Ok(())
    }

    /// Request a finite worker close after its already-accepted work.
    pub fn finish_recorder(&mut self) -> Result<(), Error> {
        let worker = self
            .recorder
            .as_mut()
            .ok_or(Error::InvalidConfiguration("recorder disabled"))?;
        worker
            .request_finish()
            .map_err(|_| Error::InvalidConfiguration("recording finish rejected"))?;
        self.recorder_finish_requested = true;
        Ok(())
    }

    /// Progress shutdown stop/close barriers without a worker join or SQL call.
    pub fn shutdown_recorder_step(&mut self, now: Duration) {
        self.poll_recorder(now);
        let Some(status) = self.recording_status.as_ref() else {
            return;
        };
        match status.state {
            RecordingState::Recording => {
                let summary = self.frozen_stop_summary(now);
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_stop_with_summary(summary, now).is_ok()
                {
                    self.runtime.close_required_recording();
                    self.runtime.disable_recording_facts();
                }
            }
            RecordingState::Idle if !self.recorder_finish_requested => {
                let summary = self.frozen_shutdown_evidence();
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_finish_with_summary(summary, now).is_ok()
                {
                    self.recorder_finish_requested = true;
                }
            }
            RecordingState::Idle => {}
            RecordingState::Failed
                if status.failure_persisted && !self.recorder_finish_requested =>
            {
                let summary = self.frozen_shutdown_evidence();
                if let Some(worker) = self.recorder.as_mut()
                    && worker.request_finish_with_summary(summary, now).is_ok()
                {
                    self.recorder_finish_requested = true;
                }
            }
            RecordingState::Starting
            | RecordingState::Stopping
            | RecordingState::Failed
            | RecordingState::Closed => {}
        }
        self.poll_recorder(now);
    }

    fn controllers_active(&self) -> bool {
        self.plan.controllers.iter().any(|(id, _, _)| {
            matches!(self.runtime.query(Query::Controller(*id)),
                Ok(QueryResult::Controller(snapshot)) if matches!(
                    snapshot.state, ControllerState::Warming | ControllerState::Running))
        })
    }

    // Required coverage can end only after every tracked authority has no
    // lease/pending send and Rust has verified the selected safe evidence.
    fn outputs_safe_for_required_stop(&self) -> bool {
        self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed
                    && snapshot.lease.is_none() && !snapshot.pending
                    && snapshot.in_flight.is_none())
        }) && self.resources.iter().all(|resource| {
            matches!(self.runtime.query(Query::Transport(*resource)),
                Ok(QueryResult::Transport(snapshot)) if snapshot.queue_len == 0
                    && snapshot.active.is_none())
        })
    }

    pub(super) fn outputs_safe_for_transport_close(&self) -> bool {
        self.outputs.iter().all(|actuator| {
            matches!(self.runtime.query(Query::Output(*actuator)),
                Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed
                    && snapshot.lease.is_none() && !snapshot.pending
                    && snapshot.in_flight.is_none())
        })
    }

    pub(super) fn poll_recorder(&mut self, now: Duration) {
        let Some(worker) = self.recorder.as_mut() else {
            return;
        };
        let previous = self.recording_status.clone();
        let prior = previous.as_ref().map(|s| s.state);
        let status = worker.poll();
        let activated =
            prior != Some(RecordingState::Recording) && status.state == RecordingState::Recording;
        if activated {
            if self.recording_policy == Some(RecordingPolicy::Required)
                && let Some(submitted) = status.confirmed_submission
            {
                let _ = self.runtime.confirm_recording_start(submitted, now);
            }
        } else if status.state == RecordingState::Recording
            && self.recording_policy == Some(RecordingPolicy::Required)
            && let Some(submitted) = status.confirmed_submission
            && self
                .last_recording_submission
                .is_some_and(|old| submitted > old)
        {
            let _ = self.runtime.confirm_recording_progress(submitted, now);
        }
        if let Some(submitted) = status.confirmed_submission {
            self.last_recording_submission = Some(submitted);
        }
        if status.state == RecordingState::Failed
            && self.recording_policy == Some(RecordingPolicy::Required)
        {
            self.runtime.recording_failure(now);
        }
        if status.state == RecordingState::Failed {
            self.runtime.disable_recording_facts();
        }
        let recording = status.state == RecordingState::Recording;
        let publish_lifecycle = crate::recorder_api::lifecycle_changed(previous.as_ref(), &status);
        self.recording_status = Some(status);
        if publish_lifecycle {
            let event_boot_id = self.event_log().boot_id().to_owned();
            let data = crate::recorder_api::status_json(
                self.recording_status.as_ref(),
                self.recording_database_id(),
                self.recording_policy,
                &event_boot_id,
            );
            let archive_id = self.recording_database_id().map(str::to_owned);
            let _ = self
                .event_log_mut()
                .recorder_state(now, archive_id.as_deref(), data);
        }
        if recording {
            self.flush_deferred_safe_terminals(now);
            if let Some(worker) = self.recorder.as_mut() {
                let _ = worker.request_probe_at(now);
            }
        }
        if activated {
            self.flush_deferred_start_acceptance(now);
        }
    }

    fn flush_deferred_safe_terminals(&mut self, now: Duration) {
        let deferred = self.pending_operations.iter().find_map(|(key, pending)| {
            pending
                .deferred_safe_terminal
                .as_ref()
                .map(|terminal| (key.clone(), terminal.clone()))
        });
        let Some((key, terminal)) = deferred else {
            return;
        };
        let credit = self
            .recorder
            .as_mut()
            .and_then(|worker| worker.operation_credit_available(&terminal).ok())
            .unwrap_or(false);
        if !credit {
            return;
        }
        let admitted = self
            .recorder
            .as_mut()
            .is_some_and(|worker| worker.try_admit_operation(terminal).is_ok());
        if admitted {
            self.pending_operations.remove(&key);
        } else if self.recording_policy == Some(RecordingPolicy::Required) {
            self.runtime.recording_failure(now);
        }
    }

    // Start acceptance predates the new interval. The boundary records it as
    // pending; its small original intent enters the FIFO after committed Start.
    fn flush_deferred_start_acceptance(&mut self, now: Duration) {
        let deferred: Vec<_> = self
            .pending_operations
            .iter()
            .filter_map(|(key, pending)| {
                pending
                    .deferred_start
                    .as_ref()
                    .map(|operation| (key.clone(), operation.clone()))
            })
            .collect();
        for (key, operation) in deferred {
            let admitted = self
                .recorder
                .as_mut()
                .is_some_and(|worker| worker.try_admit_operation(operation).is_ok());
            if admitted {
                if let Some(pending) = self.pending_operations.get_mut(&key) {
                    pending.deferred_start = None;
                    pending.accepted_recorded = true;
                }
            } else if self.recording_policy == Some(RecordingPolicy::Required) {
                self.runtime.recording_failure(now);
            }
        }
    }

    pub(super) fn admit_recording_facts(&mut self, now: Duration) {
        if self.recorder.is_none() {
            return;
        }
        let facts = self.runtime.take_recording_facts();
        let overflow = self.runtime.recording_facts_overflowed();
        let first_missing = self.runtime.recording_first_lost_fact();
        let suppressed = self.runtime.recording_suppressed_fact_count();
        let last_accepted = facts
            .last()
            .map(lab_core::recording::RecordingFact::sequence);
        if let Some(worker) = self.recorder.as_mut() {
            if !facts.is_empty()
                && worker.try_admit_at(facts, now).is_err()
                && self.recording_policy == Some(RecordingPolicy::Required)
            {
                self.runtime.recording_failure(now);
            }
            if overflow {
                worker.fail_with_gap(RecorderGap {
                    reason: "core fact outbox overflow".into(),
                    at: now,
                    first_missing_fact: first_missing,
                    known_missing_count: suppressed,
                    last_accepted_fact: last_accepted,
                });
                self.runtime.disable_recording_facts();
                if self.recording_policy == Some(RecordingPolicy::Required) {
                    self.runtime.recording_failure(now);
                }
            }
        }
        // Publish the just-latched gap before the next client Query or snapshot.
        // BestEffort may keep control running, but coverage must be truthful now.
        self.poll_recorder(now);
    }
}
