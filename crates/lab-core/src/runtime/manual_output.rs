//! Scope-owned manual control reusing the normal authority and delivery paths.

use super::*;
use crate::output::ManualOutputCommand;

impl Runtime {
    pub(super) fn output_snapshot(&self, actuator: ActuatorId) -> Result<OutputSnapshot, Error> {
        let mut snapshot = self
            .outputs
            .get(&actuator)
            .filter(|_| !self.instrument_is_prepared(actuator.instrument()))
            .ok_or(OutputError::UnknownActuator)?
            .snapshot();
        snapshot.manual_supported = self
            .metakon_instruments
            .contains_key(&actuator.instrument())
            || self
                .simple_device_instruments
                .contains_key(&actuator.instrument())
            || self.thermal_plants.contains_key(&actuator.instrument());
        Ok(snapshot)
    }
    pub(super) fn manual_output(
        &mut self,
        actuator: ActuatorId,
        owner: u64,
        expected_instance: u64,
        expected_epoch: u64,
        command: ManualOutputCommand,
        at: Duration,
    ) -> Result<CommandResult, Error> {
        // Metadata-only virtual instruments have no executable output adapter.
        let physical = self
            .metakon_instruments
            .contains_key(&actuator.instrument())
            || self
                .simple_device_instruments
                .contains_key(&actuator.instrument());
        if self.instrument_is_prepared(actuator.instrument())
            || !(physical || self.thermal_plants.contains_key(&actuator.instrument()))
        {
            return Err(OutputError::UnknownActuator.into());
        }
        self.command(Command::Output {
            actuator,
            command: OutputCommand::Tick,
            at,
        })?;
        let snapshot = self
            .outputs
            .get(&actuator)
            .ok_or(OutputError::UnknownActuator)?
            .snapshot();
        if snapshot.instance != expected_instance || snapshot.epoch != expected_epoch {
            return Err(OutputError::StaleLease.into());
        }
        if self.warming_on(actuator)
            || snapshot
                .lease
                .is_some_and(|lease| lease.owner() != OutputOwner::Manual(owner))
        {
            return Err(OutputError::Busy.into());
        }
        let current_lease = || snapshot.lease.ok_or(Error::Output(OutputError::StaleLease));
        let authority_command = match command {
            ManualOutputCommand::Acquire { lifetime } => OutputCommand::Acquire {
                owner: OutputOwner::Manual(owner),
                lifetime,
            },
            ManualOutputCommand::Renew { lifetime } => OutputCommand::RenewManual {
                lease: current_lease()?,
                lifetime,
            },
            ManualOutputCommand::Safe => OutputCommand::RequestSafe,
            ManualOutputCommand::Write { value, ttl } => {
                if snapshot.pending
                    || snapshot.in_flight.is_some()
                    || snapshot
                        .manual_attempt
                        .as_ref()
                        .is_some_and(|attempt| !attempt.terminal)
                {
                    return Err(OutputError::Busy.into());
                }
                self.require_recording_open(at)?;
                let unit = self
                    .descriptor(actuator.instrument())?
                    .parameter(actuator.parameter())
                    .ok_or(OutputError::UnknownActuator)?
                    .unit;
                let proposal = OutputProposal {
                    lease: current_lease()?,
                    value,
                    unit,
                    ttl,
                };
                let prior_attempt = snapshot.manual_attempt.map(|attempt| attempt.id);
                let result = if physical {
                    self.deliver_physical(actuator, proposal, at)
                } else {
                    self.deliver_simulated(actuator, proposal, at)
                };
                if let Err(error) = result {
                    // No ordinary proposal survives a failed admission/queue step.
                    // A started operation still settles through its original dispatch.
                    if self.outputs[&actuator]
                        .snapshot()
                        .manual_attempt
                        .as_ref()
                        .map(|attempt| attempt.id)
                        != prior_attempt
                    {
                        self.outputs
                            .get_mut(&actuator)
                            .expect("validated actuator")
                            .fail_manual_before_send();
                        self.command(Command::Output {
                            actuator,
                            command: OutputCommand::RequestSafe,
                            at,
                        })?;
                    }
                    return Err(error);
                }
                return self
                    .output_snapshot(actuator)
                    .map(CommandResult::ManualOutput);
            }
        };
        self.command(Command::Output {
            actuator,
            command: authority_command,
            at,
        })?;
        self.output_snapshot(actuator)
            .map(CommandResult::ManualOutput)
    }
}
