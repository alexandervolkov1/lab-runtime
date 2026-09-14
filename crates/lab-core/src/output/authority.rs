//! Private authority state machine owned exclusively by Runtime.
//!
//! A reserved safe slot cannot be crowded out by normal proposals. Revocation
//! fences queued work immediately, but an in-flight send must settle before the
//! safe procedure can run. This explicit split makes race tests deterministic.

use super::*;
use crate::ValueSpec;
use std::sync::atomic::{AtomicU64, Ordering};

/// Allocate process-local instance fences, not timestamps or persistent identities.
/// Relaxed ordering suffices: only uniqueness is shared, never mutable device state.
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
struct Pending {
    lease: OutputLease,
    value: f64,
    expires: Duration,
}

/// One registered actuator's authority, independent of client or algorithm lifetime.
pub(crate) struct OutputAuthority {
    actuator: ActuatorId,
    instance: u64,
    limits: ValueSpec,
    unit: Unit,
    profile: Option<SafeProfile>,
    snapshot: OutputSnapshot,
    pending: Option<Pending>,
    safe_needed: bool,
    next_dispatch: u64,
}

impl OutputAuthority {
    pub(crate) fn new(actuator: ActuatorId, limits: ValueSpec, unit: Unit) -> Result<Self, Error> {
        let instance = NEXT_INSTANCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| OutputError::CounterExhausted)?;
        Ok(Self {
            actuator,
            instance,
            limits,
            unit,
            profile: None,
            snapshot: OutputSnapshot {
                state: OutputState::Unverified,
                lease: None,
                epoch: 0,
                fault_latched: false,
                safe_confirmed: false,
                pending: false,
                in_flight: None,
                requested: None,
                sent: None,
                acknowledged: None,
                readback: None,
                outcome: None,
            },
            pending: None,
            safe_needed: false,
            next_dispatch: 1,
        })
    }

    pub(crate) fn snapshot(&self) -> OutputSnapshot {
        self.snapshot.clone()
    }

    /// The enclosing owner drives this watchdog even when a producer sends nothing.
    pub(crate) fn tick(&mut self, at: Duration) -> Result<(), Error> {
        if let Some(lease) = self.snapshot.lease
            && at >= lease.expires
        {
            self.request_safe(false)?;
        }
        Ok(())
    }

    pub(crate) fn command(
        &mut self,
        command: OutputCommand,
        at: Duration,
    ) -> Result<OutputResult, Error> {
        match command {
            OutputCommand::BindProfile(profile) => self.bind(profile),
            OutputCommand::RequestSafe => {
                self.profile()?;
                self.request_safe(false)?;
                Ok(OutputResult::Updated)
            }
            OutputCommand::Acquire { owner, lifetime } => self.acquire(owner, lifetime, at),
            OutputCommand::Release(lease) => {
                self.check_lease(lease, at)?;
                self.request_safe(false)?;
                Ok(OutputResult::Updated)
            }
            OutputCommand::Trip => {
                self.profile()?;
                self.request_safe(true)?;
                Ok(OutputResult::Updated)
            }
            OutputCommand::AcknowledgeFault => {
                if self.snapshot.state != OutputState::FaultLatched
                    || !self.snapshot.safe_confirmed
                    || self.snapshot.in_flight.is_some()
                    || self.safe_needed
                {
                    return Err(OutputError::InvalidState.into());
                }
                // Recovery evidence is necessary, but acknowledging it never grants a lease.
                self.snapshot.fault_latched = false;
                self.snapshot.state = OutputState::Disarmed;
                Ok(OutputResult::Updated)
            }
            OutputCommand::Propose(proposal) => self.propose(proposal, at),
            OutputCommand::BeginDispatch => self.begin(at),
            OutputCommand::Complete {
                dispatch_id,
                outcome,
            } => self.complete(dispatch_id, outcome, at),
            OutputCommand::Tick => Ok(OutputResult::Updated),
        }
    }

    fn profile(&self) -> Result<&SafeProfile, Error> {
        self.profile
            .as_ref()
            .ok_or(OutputError::InvalidProfile.into())
    }

    fn bind(&mut self, profile: SafeProfile) -> Result<OutputResult, Error> {
        if !matches!(
            self.snapshot.state,
            OutputState::Unverified | OutputState::Disarmed
        ) || self.snapshot.in_flight.is_some()
        {
            return Err(OutputError::InvalidState.into());
        }
        profile.validate()?;
        // Installation limits may be stricter than device limits, never wider.
        self.limits.validate(&Value::Float(profile.min))?;
        self.limits.validate(&Value::Float(profile.max))?;
        self.bump_epoch()?;
        self.profile = Some(profile);
        self.snapshot.state = OutputState::Unverified;
        self.snapshot.safe_confirmed = false;
        self.clear_pending();
        Ok(OutputResult::Updated)
    }

    fn acquire(
        &mut self,
        owner: OutputOwner,
        lifetime: Duration,
        at: Duration,
    ) -> Result<OutputResult, Error> {
        if self.snapshot.state != OutputState::Disarmed || !self.snapshot.safe_confirmed {
            return Err(OutputError::InvalidState.into());
        }
        if lifetime.is_zero() || lifetime > self.profile()?.max_lease {
            return Err(OutputError::InvalidProfile.into());
        }
        let expires = at.checked_add(lifetime).ok_or(OutputError::InvalidTime)?;
        self.bump_epoch()?;
        let lease = OutputLease {
            actuator: self.actuator,
            instance: self.instance,
            owner,
            epoch: self.snapshot.epoch,
            expires,
        };
        self.snapshot.lease = Some(lease);
        self.snapshot.safe_confirmed = false;
        self.snapshot.state = match owner {
            OutputOwner::Manual(_) => OutputState::ArmedManual,
            OutputOwner::Automatic(_) => OutputState::ArmedAuto,
        };
        Ok(OutputResult::Lease(lease))
    }

    fn check_lease(&self, lease: OutputLease, at: Duration) -> Result<(), Error> {
        if at >= lease.expires {
            return Err(OutputError::Expired.into());
        }
        if self.snapshot.lease != Some(lease)
            || lease.actuator != self.actuator
            || lease.instance != self.instance
            || lease.epoch != self.snapshot.epoch
            || !matches!(
                self.snapshot.state,
                OutputState::ArmedManual | OutputState::ArmedAuto
            )
        {
            return Err(OutputError::StaleLease.into());
        }
        Ok(())
    }

    fn propose(&mut self, proposal: OutputProposal, at: Duration) -> Result<OutputResult, Error> {
        self.check_lease(proposal.lease, at)?;
        if proposal.unit != self.unit {
            return Err(OutputError::WrongUnit.into());
        }
        let profile = self.profile()?;
        ValueSpec::Float {
            min: profile.min,
            max: profile.max,
        }
        .validate(&proposal.value)?;
        if proposal.ttl.is_zero() || proposal.ttl > profile.max_proposal_ttl {
            return Err(OutputError::InvalidProfile.into());
        }
        let deadline = at
            .checked_add(proposal.ttl)
            .ok_or(OutputError::InvalidTime)?;
        if self.pending.is_some() {
            return Err(OutputError::Busy.into());
        }
        let Value::Float(value) = proposal.value else {
            unreachable!("ValueSpec validated the exact scalar kind")
        };
        self.pending = Some(Pending {
            lease: proposal.lease,
            value,
            expires: deadline.min(proposal.lease.expires),
        });
        self.snapshot.pending = true;
        self.snapshot.requested = Some(value);
        Ok(OutputResult::Queued)
    }

    fn begin(&mut self, at: Duration) -> Result<OutputResult, Error> {
        if self.snapshot.in_flight.is_some() {
            return Err(OutputError::Busy.into());
        }

        let (value, safe) = if self.safe_needed {
            // Safe output is a Rust-owned reserved operation, not a producer proposal.
            // Revocation already invalidated any normal queued work.
            (self.profile()?.safe_value, true)
        } else {
            let candidate = self.pending.ok_or(OutputError::NothingPending)?;
            // This is the final simulated-send boundary. Enqueue-time validation
            // cannot protect against a revoke or deadline reached while waiting.
            let validation = self.check_lease(candidate.lease, at);
            if let Err(error) = validation {
                self.clear_pending();
                return Err(error);
            }
            if at >= candidate.expires {
                self.clear_pending();
                return Err(OutputError::Expired.into());
            }
            (candidate.value, false)
        };
        let following_id = self
            .next_dispatch
            .checked_add(1)
            .ok_or(OutputError::CounterExhausted)?;
        let dispatch = Dispatch {
            id: DispatchId {
                instance: self.instance,
                sequence: self.next_dispatch,
            },
            value,
            safe,
            epoch: self.snapshot.epoch,
        };
        self.next_dispatch = following_id;
        self.clear_pending();
        self.safe_needed = false;
        self.snapshot.safe_confirmed = false;
        self.snapshot.in_flight = Some(dispatch);
        self.snapshot.sent = Some(OutputObservation { value, at });
        // Old readback must not masquerade as evidence for this newer send.
        self.snapshot.acknowledged = None;
        self.snapshot.readback = None;
        self.snapshot.outcome = None;
        Ok(OutputResult::Dispatched(dispatch))
    }

    fn complete(
        &mut self,
        id: DispatchId,
        outcome: DispatchOutcome,
        at: Duration,
    ) -> Result<OutputResult, Error> {
        let dispatch = self
            .snapshot
            .in_flight
            .ok_or(OutputError::UnknownDispatch)?;
        if dispatch.id != id {
            return Err(OutputError::UnknownDispatch.into());
        }
        self.snapshot.in_flight = None;
        self.snapshot.outcome = Some(outcome);
        let observation = OutputObservation {
            value: dispatch.value,
            at,
        };
        if matches!(
            outcome,
            DispatchOutcome::Acknowledged | DispatchOutcome::ReadbackVerified
        ) {
            self.snapshot.acknowledged = Some(observation);
        }
        if outcome == DispatchOutcome::ReadbackVerified {
            self.snapshot.readback = Some(observation);
        }

        if matches!(
            outcome,
            DispatchOutcome::Failed | DispatchOutcome::Ambiguous
        ) {
            let recovery_was_requested = self.safe_needed;
            self.request_safe(true)?;
            if dispatch.safe && !recovery_was_requested {
                // Do not spin retrying a failed safe procedure. Retain the fault
                // and wait for an explicit recovery request, without claiming safe.
                // A newer explicit request made during this send is not lost.
                self.safe_needed = false;
                self.snapshot.state = OutputState::FaultLatched;
            }
        } else if dispatch.safe && dispatch.epoch == self.snapshot.epoch {
            let sufficient = outcome == DispatchOutcome::ReadbackVerified
                || self.profile()?.required_evidence == EvidenceLevel::Acknowledgement;
            self.snapshot.safe_confirmed = sufficient;
            if !sufficient {
                self.snapshot.fault_latched = true;
            }
            self.snapshot.state = if self.snapshot.fault_latched {
                OutputState::FaultLatched
            } else {
                OutputState::Disarmed
            };
        }
        // A normal completion, or an obsolete safe completion, never grants
        // ownership and never confirms a newer safe request.
        Ok(OutputResult::Completed)
    }

    fn request_safe(&mut self, fault: bool) -> Result<(), Error> {
        self.bump_epoch()?;
        self.snapshot.lease = None;
        self.snapshot.fault_latched |= fault;
        self.snapshot.safe_confirmed = false;
        self.snapshot.state = OutputState::SafePending;
        self.safe_needed = true;
        // Keep the slot bounded and make revocation immediately visible. The
        // final send check remains mandatory for deadline/owner validation.
        self.clear_pending();
        Ok(())
    }

    fn clear_pending(&mut self) {
        self.pending = None;
        self.snapshot.pending = false;
    }

    fn bump_epoch(&mut self) -> Result<(), Error> {
        let Some(epoch) = self.snapshot.epoch.checked_add(1) else {
            // Never wrap into a previously valid lease epoch, even at exhaustion.
            self.snapshot.lease = None;
            self.snapshot.fault_latched = true;
            self.snapshot.safe_confirmed = false;
            self.snapshot.state = OutputState::FaultLatched;
            self.clear_pending();
            return Err(OutputError::CounterExhausted.into());
        };
        self.snapshot.epoch = epoch;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed() -> (OutputAuthority, OutputLease) {
        let id = ActuatorId::new(InstrumentId::new(1), crate::HEATER_POWER);
        let mut authority = OutputAuthority::new(
            id,
            ValueSpec::Float {
                min: 0.0,
                max: 100.0,
            },
            Unit::PERCENT,
        )
        .unwrap();
        authority
            .command(
                OutputCommand::BindProfile(SafeProfile {
                    min: 0.0,
                    max: 100.0,
                    safe_value: 0.0,
                    max_lease: Duration::from_secs(1),
                    max_proposal_ttl: Duration::from_millis(100),
                    required_evidence: EvidenceLevel::Readback,
                }),
                Duration::ZERO,
            )
            .unwrap();
        authority
            .command(OutputCommand::RequestSafe, Duration::ZERO)
            .unwrap();
        let OutputResult::Dispatched(safe) = authority.begin(Duration::ZERO).unwrap() else {
            panic!()
        };
        authority
            .complete(safe.id(), DispatchOutcome::ReadbackVerified, Duration::ZERO)
            .unwrap();
        let OutputResult::Lease(lease) = authority
            .acquire(
                OutputOwner::Automatic(1),
                Duration::from_secs(1),
                Duration::ZERO,
            )
            .unwrap()
        else {
            panic!()
        };
        authority
            .propose(
                OutputProposal {
                    lease,
                    value: Value::Float(90.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(100),
                },
                Duration::ZERO,
            )
            .unwrap();
        (authority, lease)
    }

    #[test]
    fn final_epoch_check_protects_even_if_eager_queue_cleanup_is_missed() {
        let (mut authority, _) = armed();
        // Normal revocation eagerly removes this slot. Deliberately retain it
        // here to prove that final authorization is independent defense, not
        // merely an assumption that queue cleanup always succeeded.
        authority.bump_epoch().unwrap();
        let previous_send = authority.snapshot.sent;
        assert_eq!(
            authority.begin(Duration::ZERO),
            Err(OutputError::StaleLease.into())
        );
        assert_eq!(authority.snapshot.sent, previous_send);
        assert!(!authority.snapshot.pending);
    }

    #[test]
    fn epoch_exhaustion_never_wraps_to_an_old_owner() {
        let (mut authority, _) = armed();
        authority.snapshot.epoch = u64::MAX;
        assert_eq!(
            authority.request_safe(false),
            Err(OutputError::CounterExhausted.into())
        );
        assert_eq!(authority.snapshot.state, OutputState::FaultLatched);
        assert!(authority.snapshot.lease.is_none());
        assert!(!authority.snapshot.safe_confirmed);
        assert!(!authority.snapshot.pending);
    }
}
