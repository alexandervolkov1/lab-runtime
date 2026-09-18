//! Host command/event boundary, output and resource evidence, and shutdown coordination.
//!
//! The methods here remain implementations of the single HostCore owner declared
//! in the parent module; this module introduces no additional mutable state owner.

use super::*;

impl HostCore {
    /// Number of configured safe outputs in this trusted bounded host.
    pub fn configured_output_count(&self) -> usize {
        self.outputs.len()
    }
    /// Include a trusted registered output in the owner's bounded shutdown
    /// evidence set. This local composition seam grants no wire authority;
    /// registration and the safe profile must precede active service use.
    pub fn track_trusted_output(&mut self, actuator: ActuatorId) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        if self.outputs.contains(&actuator) {
            return Ok(());
        }
        if self.outputs.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 output limit"));
        }
        let QueryResult::Output(_) = self.runtime.query(Query::Output(actuator))? else {
            return Err(Error::InvalidConfiguration(
                "trusted output is not registered",
            ));
        };
        self.outputs.push(actuator);
        Ok(())
    }
    /// Install one trusted bounded M3 byte resource without raw TCP I/O.
    pub fn register_transport(
        &mut self,
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
    ) -> Result<(), Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        if self.resources.len() >= 8 {
            return Err(Error::InvalidConfiguration("M6 resource limit"));
        }
        self.runtime.register_transport(id, adapter)?;
        self.resources.push(id);
        Ok(())
    }
    /// Freeze bounded M3 resource status alongside facts without polling it.
    pub fn resource_records(&self) -> Vec<serde_json::Value> {
        self.resources.iter().filter_map(|id|match self.runtime.query(Query::Transport(*id)){
            Ok(QueryResult::Transport(s))=>Some(serde_json::json!({"kind":"resource","target":{"id":id.get().to_string()},
                "data":{"state":match s.state{ExecutorState::Idle=>"idle",ExecutorState::InFlight=>"in_flight",
                    ExecutorState::Recovering=>"recovering",ExecutorState::Offline=>"offline"},
                    "queue_len":s.queue_len,"generation":s.generation.to_string(),"active":s.active.map(|a|a.get().to_string()),
                    "latest":s.latest.map(|r|serde_json::json!({"id":r.id.get().to_string(),"outcome":match r.outcome{
                        TransactionOutcome::Completed=>"completed",TransactionOutcome::QueueExpired=>"queue_expired",TransactionOutcome::Failed=>"failed"},
                        "started":r.started,"generation":r.generation.to_string()}))}})),_=>None}).collect()
    }
    /// Retain per-output Rust authority/evidence facts through safe shutdown.
    pub fn output_safe_records(&self) -> Vec<serde_json::Value> {
        self.outputs.iter().map(|actuator|match self.runtime.query(Query::Output(*actuator)){
            Ok(QueryResult::Output(s))=>serde_json::json!({"instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"safe_confirmed":s.safe_confirmed,
                "lease_present":s.lease.is_some(),"fault_latched":s.fault_latched,
                "readback":s.readback.map(|r|serde_json::json!({"value":r.value,"at":r.at.as_nanos().to_string()})),
                "state":match s.state{lab_core::output::OutputState::Unverified=>"unverified",
                    lab_core::output::OutputState::SafePending=>"safe_pending",lab_core::output::OutputState::Disarmed=>"disarmed",
                    lab_core::output::OutputState::ArmedManual=>"armed_manual",lab_core::output::OutputState::ArmedAuto=>"armed_auto",
                    lab_core::output::OutputState::FaultLatched=>"fault_latched"}}),
            _=>serde_json::json!({"instrument":actuator.instrument().get().to_string(),
                "parameter":actuator.parameter().get().to_string(),"safe_confirmed":false,"state":"unknown"})}).collect()
    }

    // This snapshot is taken only after the safety grace has resolved; it
    // reports unknown evidence as unknown and cannot confirm physical safety.
    pub(super) fn frozen_shutdown_evidence(&self) -> serde_json::Value {
        let controllers: Vec<_> = self
            .plan
            .controllers
            .iter()
            .map(
                |(id, _, _)| match self.runtime.query(Query::Controller(*id)) {
                    Ok(QueryResult::Controller(snapshot)) => serde_json::json!({
                        "id":id.get().to_string(),
                        "state":format!("{:?}",snapshot.state),
                    }),
                    _ => serde_json::json!({"id":id.get().to_string(),"state":"unknown"}),
                },
            )
            .collect();
        serde_json::json!({
            "outputs":self.output_safe_records(),
            "resources":self.resource_records(),
            "controllers":controllers,
            "unfinished_managed_workers":self.runtime.unfinished_component_workers(),
        })
    }

    // Pending identities are already accepted facts, never predictions of
    // their terminal domain or physical outcome.
    pub(super) fn frozen_stop_summary(&self, at: Duration) -> serde_json::Value {
        let pending:Vec<_>=self.pending_operations.iter().map(|((scope,seq),operation)|
            serde_json::json!({"scope":scope,"request_seq":seq.to_string(),
                "command":operation.command,"accepted_at_ns":operation.accepted_at.as_nanos().to_string(),
                "accepted_recorded":operation.accepted_recorded})).collect();
        serde_json::json!({"requested_stop_at_ns":at.as_nanos().to_string(),
            "pending_operations":pending})
    }

    /// Fence producers before Rust begins a new required safe procedure.
    pub fn begin_shutdown(&mut self, clock: &impl Clock) -> Result<(), Error> {
        if self.stopping {
            return Ok(());
        }
        self.stopping = true;
        self.runtime
            .command(Command::QuiesceManaged { at: clock.now() })?;
        self.observe(clock.now(), None)?;
        let identities: Vec<_> = self.plan.controllers.iter().map(|(id, _, _)| *id).collect();
        for id in identities {
            let QueryResult::Controller(snapshot) = self.runtime.query(Query::Controller(id))?
            else {
                unreachable!()
            };
            if matches!(
                snapshot.state,
                ControllerState::Warming | ControllerState::Running
            ) {
                self.runtime.command(Command::PauseController {
                    controller: id,
                    at: clock.now(),
                })?;
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
            }
        }
        for actuator in self.outputs.clone() {
            self.runtime.command(Command::Output {
                actuator,
                command: OutputCommand::RequestSafe,
                at: clock.now(),
            })?;
            self.observe(clock.now(), None)?;
        }
        self.plan.safety.next_due = clock.now();
        Ok(())
    }

    /// Copy output evidence and worker cleanup status without progressing either.
    pub fn shutdown_status(&self) -> ShutdownStatus {
        let safe_confirmed=self.outputs.iter().all(|actuator|matches!(self.runtime.query(Query::Output(*actuator)),
            Ok(QueryResult::Output(snapshot)) if snapshot.safe_confirmed && snapshot.lease.is_none()));
        let unfinished_workers = self.runtime.unfinished_component_workers();
        let unfinished_transports = self
            .resources
            .len()
            .saturating_sub(self.closed_resources.len());
        let transports_closed = unfinished_transports == 0;
        let recorder_state = self.recording_status.as_ref().map(|status| status.state);
        let recorder_flushed = self.recording_status.as_ref().is_none_or(|status| {
            status.state == RecordingState::Closed
                && status.terminal_seal_committed
                && status.first_error.is_none()
                && status.outstanding_records == 0
        });
        let recorder_error = recorder_state == Some(RecordingState::Failed);
        let recorder_unfinished = self
            .recording_status
            .as_ref()
            .is_some_and(|status| !status.worker_closed);
        ShutdownStatus {
            safe_confirmed,
            unfinished_workers,
            unfinished_transports,
            transports_closed,
            fatal_error: false,
            recorder_flushed,
            recorder_unfinished,
            recorder_error,
            exit_success: safe_confirmed
                && unfinished_workers == 0
                && transports_closed
                && recorder_flushed
                && !recorder_error,
        }
    }

    /// Replace a schedule only through a trusted local composition decision.
    /// Network requests cannot access this seam or choose controller cadence.
    pub fn replace_plan(&mut self, plan: SchedulePlan) -> Result<(), Error> {
        if plan.plants.len() > 8
            || plan.metakon_reads.len() > 64
            || plan.references.len() > 8
            || plan.controllers.len() > 8
            || plan.sources.len() + plan.transforms.len() > 8
        {
            return Err(Error::InvalidConfiguration("M6 host schedule limit"));
        }
        self.plan = plan;
        Ok(())
    }

    /// Serialize a local domain Command on this owner; snapshots are separate.
    pub fn command(&mut self, command: Command) -> Result<CommandResult, Error> {
        self.command_with_cause(command, None)
    }

    /// Serialize one command and publish resulting facts with optional causing ID.
    pub fn command_with_cause(
        &mut self,
        command: Command,
        cause: Option<(String, u64)>,
    ) -> Result<CommandResult, Error> {
        if self.stopping {
            return Err(Error::InvalidConfiguration("host is stopping"));
        }
        self.poll_recorder(self.last_now);
        let start = match &command {
            Command::StartController { controller, .. }
            | Command::ResumeController { controller, .. } => Some(*controller),
            _ => None,
        };
        let profile_binding = match &command {
            Command::Output {
                actuator,
                command: OutputCommand::BindProfile(profile),
                ..
            } => Some((*actuator, profile.clone())),
            _ => None,
        };
        let outcome = self.runtime.command(command);
        if outcome.is_ok()
            && let Some((actuator, profile)) = profile_binding
        {
            if let Some((_, active)) = self
                .active_safety_profiles
                .iter_mut()
                .find(|(id, _)| *id == actuator)
            {
                *active = profile;
            } else {
                self.active_safety_profiles.push((actuator, profile));
                self.outputs.push(actuator);
            }
        }
        let at = self.last_now;
        self.observe(at, cause.as_ref().map(|(s, n)| (s.as_str(), *n)))?;
        self.admit_recording_facts(at);
        self.poll_recorder(at);
        let outcome = outcome?;
        if let Some(controller) = start
            && let QueryResult::ControllerConfig(config) =
                self.runtime.query(Query::ControllerConfig(controller))?
        {
            let QueryResult::Latest(sample) =
                self.runtime.query(Query::GetLatestSignal(config.input))?
            else {
                unreachable!()
            };
            self.consumed.insert(controller, sample);
        }
        Ok(outcome)
    }

    /// Borrow the owner-local bounded semantic publication ring.
    pub const fn event_log(&self) -> &EventLog {
        &self.events
    }
    /// Borrow the ring mutably only from the serialized service owner.
    pub fn event_log_mut(&mut self) -> &mut EventLog {
        &mut self.events
    }
    /// Install the OS process identity before listener readiness.
    pub fn set_boot_id(&mut self, boot: &str) {
        self.events.set_boot_id(boot);
    }
    pub(super) fn observe(
        &mut self,
        at: Duration,
        cause: Option<(&str, u64)>,
    ) -> Result<(), Error> {
        self.events
            .observe(&self.runtime, at, cause)
            .map_err(event_domain_error)
    }

    /// Return an owned pure domain snapshot without polling clocks or workers.
    pub fn query(&self, query: Query) -> Result<QueryResult, Error> {
        self.runtime.query(query)
    }
}
