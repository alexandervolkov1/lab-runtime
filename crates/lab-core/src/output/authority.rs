//! Private output-permission state machine owned exclusively by Runtime.
//!
//! The normal path is `OutputProposal -> pending proposal -> transport reservation
//! -> final validation -> send started -> ACK -> separate readback`. A reserved safe
//! slot cannot be crowded out by normal proposals. Revocation fences queued work
//! immediately, but accepted bytes cannot be recalled.
//!
//! `safe_obligation_pending` records that safe state is still required. It does not
//! by itself authorize another WRITE: `ambiguous_safe_resend_blocked` separately
//! records that a started safe command has an unknown physical outcome. Keeping both
//! facts is what preserves the safety obligation without blindly retrying that
//! command or allowing normal output.

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

/// One registered actuator's permission state, independent of client or algorithm lifetime.
///
/// Proposals carry requested values but no permission to send. Permission is bound
/// to a finite lease, authority instance, revocation epoch and current physical
/// binding/mapping generation. `ResourceExecutor` asks this owner to validate those
/// facts again immediately before the first possible byte and then reports the
/// distinct send-started transition.
///
/// `requested != authorized != send_started != ACK != readback != physical_effect`.
/// ACK and readback are separate evidence stages, and neither proves independent
/// physical effect. If a started write settles ambiguously, normal authority stays
/// revoked. For a safe write, the required safe obligation remains recorded while
/// resend permission is separately blocked: obligation is not authorization to
/// repeat an operation whose physical outcome is unknown.
pub(crate) struct OutputAuthority {
    actuator: ActuatorId,
    instance: u64,
    limits: ValueSpec,
    unit: Unit,
    profile: Option<SafeProfile>,
    snapshot: OutputSnapshot,
    pending: Option<Pending>,
    transport_reserved: Option<OutputIntent>,
    safe_obligation_pending: bool,
    // An already-started safe write with an unknown outcome does not discharge
    // the safe obligation, but it also cannot authorize another write. Only a
    // fresh authority instance/rebind or explicit future reconciliation can
    // resolve this fail-closed state.
    ambiguous_safe_resend_blocked: bool,
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
                reported_readback: None,
                readback_failure: None,
                outcome: None,
            },
            pending: None,
            transport_reserved: None,
            safe_obligation_pending: false,
            ambiguous_safe_resend_blocked: false,
            next_dispatch: 1,
        })
    }

    pub(crate) fn snapshot(&self) -> OutputSnapshot {
        self.snapshot.clone()
    }

    /// Admission for a controller preparing input without acquiring an output lease.
    pub(crate) fn can_prepare(&self) -> bool {
        self.snapshot.state == OutputState::Disarmed
            && self.snapshot.safe_confirmed
            && self.snapshot.lease.is_none()
            && self.pending.is_none()
            && self.transport_reserved.is_none()
            && self.snapshot.in_flight.is_none()
            && !self.safe_obligation_pending
    }

    /// Require a finite controller cadence strictly shorter than its configured lease.
    pub(crate) fn valid_native_duration(&self, gap: Duration, lifetime: Duration) -> bool {
        !gap.is_zero()
            && gap < lifetime
            && self
                .profile
                .as_ref()
                .is_some_and(|profile| lifetime <= profile.max_lease)
    }

    /// New native PID limits must fit both actuator descriptor and bound safe profile.
    pub(crate) fn accepts_pid_limits(&self, min: f64, max: f64) -> bool {
        self.can_prepare()
            && matches!(self.limits, ValueSpec::Float { min: low, max: high } if min >= low && max <= high)
            && self
                .profile
                .as_ref()
                .is_some_and(|profile| min >= profile.min && max <= profile.max)
    }

    /// Replace only the exact current native token after trusted successful delivery.
    /// An ordinary client has no command exposing this operation.
    pub(crate) fn renew_native(
        &mut self,
        lease: OutputLease,
        lifetime: Duration,
        at: Duration,
    ) -> Result<OutputLease, Error> {
        self.check_lease(lease, at)?;
        if self.snapshot.state != OutputState::ArmedAuto
            || self.snapshot.fault_latched
            || self.safe_obligation_pending
        {
            return Err(OutputError::InvalidState.into());
        }
        if self.pending.is_some()
            || self.transport_reserved.is_some()
            || self.snapshot.in_flight.is_some()
        {
            return Err(OutputError::Busy.into());
        }
        if lifetime.is_zero() || lifetime > self.profile()?.max_lease {
            return Err(OutputError::InvalidProfile.into());
        }
        let expires = at.checked_add(lifetime).ok_or(OutputError::InvalidTime)?;
        if expires <= lease.expires {
            return Err(OutputError::InvalidTime.into());
        }
        let replacement = OutputLease { expires, ..lease };
        self.snapshot.lease = Some(replacement);
        Ok(replacement)
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
                    || self.safe_obligation_pending
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

    pub(crate) fn requires_readback(&self) -> Result<bool, Error> {
        Ok(self.profile()?.required_evidence == EvidenceLevel::Readback)
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
        if self.snapshot.in_flight.is_some() || self.transport_reserved.is_some() {
            return Err(OutputError::Busy.into());
        }

        let (value, safe) = if self.safe_obligation_pending {
            if self.ambiguous_safe_resend_blocked {
                return Err(OutputError::InvalidState.into());
            }
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
        self.safe_obligation_pending = false;
        self.snapshot.safe_confirmed = false;
        self.snapshot.in_flight = Some(dispatch);
        self.snapshot.sent = Some(OutputObservation { value, at });
        // Old readback must not masquerade as evidence for this newer send.
        self.snapshot.acknowledged = None;
        self.snapshot.readback = None;
        self.snapshot.reported_readback = None;
        self.snapshot.readback_failure = None;
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
            self.request_safe(true)?;
            if dispatch.safe {
                // The safe target remains required, but a started write with an
                // unknown result is not permission to emit that command again.
                // Keep the obligation while preventing both automatic safety
                // service and direct trusted queue admission from resending it.
                self.ambiguous_safe_resend_blocked = true;
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
        self.snapshot.state = if self.ambiguous_safe_resend_blocked {
            OutputState::FaultLatched
        } else {
            OutputState::SafePending
        };
        self.safe_obligation_pending = true;
        self.transport_reserved = None;
        // Keep the slot bounded and make revocation immediately visible. The
        // final send check remains mandatory for deadline/owner validation.
        self.clear_pending();
        Ok(())
    }

    fn clear_pending(&mut self) {
        self.pending = None;
        self.snapshot.pending = false;
    }

    /// Reserve the current safe request or normal proposal for a trusted transport queue.
    pub(crate) fn reserve_transport(
        &mut self,
        at: Duration,
        queue_deadline: Duration,
        attempt_id: Option<u64>,
        binding_generation: u64,
        mapping_revision: u64,
    ) -> Result<OutputIntent, Error> {
        if self.snapshot.in_flight.is_some() || self.transport_reserved.is_some() {
            return Err(OutputError::Busy.into());
        }
        if at >= queue_deadline || binding_generation == 0 || mapping_revision == 0 {
            return Err(OutputError::InvalidTime.into());
        }
        let intent = if self.safe_obligation_pending {
            if self.ambiguous_safe_resend_blocked {
                return Err(OutputError::InvalidState.into());
            }
            OutputIntent {
                actuator: self.actuator,
                attempt_id,
                instance: self.instance,
                lease: None,
                epoch: self.snapshot.epoch,
                value: self.profile()?.safe_value,
                unit: self.unit,
                expires: queue_deadline,
                safe: true,
                binding_generation,
                mapping_revision,
            }
        } else {
            let pending = self.pending.ok_or(OutputError::NothingPending)?;
            self.check_lease(pending.lease, at)?;
            if at >= pending.expires {
                self.clear_pending();
                return Err(OutputError::Expired.into());
            }
            OutputIntent {
                actuator: self.actuator,
                attempt_id,
                instance: self.instance,
                lease: Some(pending.lease),
                epoch: self.snapshot.epoch,
                value: pending.value,
                unit: self.unit,
                expires: pending.expires.min(queue_deadline),
                safe: false,
                binding_generation,
                mapping_revision,
            }
        };
        self.transport_reserved = Some(intent);
        Ok(intent)
    }

    /// Revalidate an intent immediately before a first-byte attempt.
    pub(crate) fn validate_transport(
        &self,
        intent: OutputIntent,
        at: Duration,
    ) -> Result<(), Error> {
        if self.transport_reserved != Some(intent)
            || intent.actuator != self.actuator
            || intent.instance != self.instance
            || intent.epoch != self.snapshot.epoch
            || intent.unit != self.unit
            || at >= intent.expires
            || self.snapshot.in_flight.is_some()
        {
            return Err(OutputError::StaleLease.into());
        }
        self.limits.validate(&Value::Float(intent.value))?;
        let profile = self.profile()?;
        ValueSpec::Float {
            min: profile.min,
            max: profile.max,
        }
        .validate(&Value::Float(intent.value))?;
        if intent.safe {
            if !self.safe_obligation_pending || intent.lease.is_some() {
                return Err(OutputError::StaleLease.into());
            }
        } else {
            self.check_lease(intent.lease.ok_or(OutputError::StaleLease)?, at)?;
        }
        Ok(())
    }

    /// Record the first accepted byte after the final validation performed by the same owner.
    pub(crate) fn begin_transport(
        &mut self,
        intent: OutputIntent,
        at: Duration,
    ) -> Result<DispatchId, Error> {
        self.validate_transport(intent, at)?;
        let following_id = self
            .next_dispatch
            .checked_add(1)
            .ok_or(OutputError::CounterExhausted)?;
        let dispatch = Dispatch {
            id: DispatchId {
                instance: self.instance,
                sequence: self.next_dispatch,
            },
            value: intent.value,
            safe: intent.safe,
            epoch: intent.epoch,
        };
        self.next_dispatch = following_id;
        self.transport_reserved = None;
        if intent.safe {
            self.safe_obligation_pending = false;
        } else {
            self.clear_pending();
        }
        self.snapshot.safe_confirmed = false;
        self.snapshot.in_flight = Some(dispatch);
        self.snapshot.sent = Some(OutputObservation {
            value: intent.value,
            at,
        });
        self.snapshot.acknowledged = None;
        self.snapshot.readback = None;
        self.snapshot.reported_readback = None;
        self.snapshot.readback_failure = None;
        self.snapshot.outcome = None;
        Ok(dispatch.id)
    }

    /// Release an unstarted queue reservation without claiming that bytes were sent.
    pub(crate) fn abort_transport(&mut self, intent: OutputIntent) {
        if self.transport_reserved == Some(intent) {
            self.transport_reserved = None;
            if !intent.safe {
                self.clear_pending();
            }
        }
    }

    /// Revoke ordinary authority as soon as a started physical write becomes uncertain.
    pub(crate) fn transport_uncertain(&mut self, id: DispatchId) -> Result<(), Error> {
        let dispatch = self
            .snapshot
            .in_flight
            .filter(|dispatch| dispatch.id() == id)
            .ok_or(OutputError::UnknownDispatch)?;
        self.request_safe(true)?;
        if dispatch.is_safe() {
            self.ambiguous_safe_resend_blocked = true;
            self.snapshot.state = OutputState::FaultLatched;
        }
        Ok(())
    }

    /// Record a strict protocol ACK while retaining the dispatch for the distinct
    /// physical register-readback phase.
    pub(crate) fn acknowledge_transport(
        &mut self,
        id: DispatchId,
        at: Duration,
    ) -> Result<(), Error> {
        let dispatch = self
            .snapshot
            .in_flight
            .ok_or(OutputError::UnknownDispatch)?;
        if dispatch.id != id {
            return Err(OutputError::UnknownDispatch.into());
        }
        self.snapshot.acknowledged = Some(OutputObservation {
            value: dispatch.value,
            at,
        });
        self.snapshot.outcome = Some(DispatchOutcome::Acknowledged);
        Ok(())
    }

    /// Complete physical output only when the separately read register matches
    /// the immutable authorized value. A mismatch remains visible and fails safe.
    pub(crate) fn complete_transport_readback(
        &mut self,
        id: DispatchId,
        reported: f64,
        at: Duration,
    ) -> Result<bool, Error> {
        let dispatch = self
            .snapshot
            .in_flight
            .ok_or(OutputError::UnknownDispatch)?;
        if dispatch.id != id || !reported.is_finite() {
            return Err(OutputError::UnknownDispatch.into());
        }
        self.snapshot.reported_readback = Some(OutputObservation {
            value: reported,
            at,
        });
        if reported == dispatch.value {
            self.complete(id, DispatchOutcome::ReadbackVerified, at)?;
            Ok(true)
        } else {
            self.snapshot.readback_failure = Some(OutputReadbackFailure::Mismatch);
            self.complete(id, DispatchOutcome::Failed, at)?;
            Ok(false)
        }
    }

    /// Settle a bounded readback failure without forgetting the preceding ACK.
    pub(crate) fn fail_transport_readback(
        &mut self,
        id: DispatchId,
        at: Duration,
    ) -> Result<(), Error> {
        self.snapshot.readback_failure = Some(OutputReadbackFailure::Unavailable);
        self.complete(id, DispatchOutcome::Ambiguous, at)
            .map(|_| ())
    }

    /// Apply a settled transport result through the same M2 evidence state machine.
    pub(crate) fn complete_transport(
        &mut self,
        id: DispatchId,
        outcome: DispatchOutcome,
        at: Duration,
    ) -> Result<(), Error> {
        self.complete(id, outcome, at).map(|_| ())
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

    #[test]
    fn native_replacement_requires_empty_authority_work_and_exact_current_token() {
        let (mut authority, initial) = armed();
        let now = Duration::from_millis(100);
        let lifetime = Duration::from_secs(1);
        assert_eq!(
            authority.renew_native(initial, lifetime, now),
            Err(OutputError::Busy.into())
        );
        assert_eq!(authority.snapshot.lease, Some(initial));
        authority.clear_pending();
        let replacement = authority.renew_native(initial, lifetime, now).unwrap();
        assert_eq!(replacement.epoch(), initial.epoch());
        assert_eq!(replacement.owner(), initial.owner());
        assert_eq!(replacement.expires(), now + lifetime);
        assert_eq!(
            authority.renew_native(initial, lifetime, now),
            Err(OutputError::StaleLease.into())
        );

        authority
            .propose(
                OutputProposal {
                    lease: replacement,
                    value: Value::Float(5.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(50),
                },
                now,
            )
            .unwrap();
        let intent = authority
            .reserve_transport(now, now + Duration::from_millis(50), None, 1, 1)
            .unwrap();
        assert_eq!(
            authority.renew_native(replacement, lifetime, now),
            Err(OutputError::Busy.into())
        );
        authority.abort_transport(intent);
        authority
            .propose(
                OutputProposal {
                    lease: replacement,
                    value: Value::Float(5.0),
                    unit: Unit::PERCENT,
                    ttl: Duration::from_millis(50),
                },
                now,
            )
            .unwrap();
        let OutputResult::Dispatched(dispatch) = authority.begin(now).unwrap() else {
            panic!()
        };
        assert_eq!(
            authority.renew_native(replacement, lifetime, now),
            Err(OutputError::Busy.into())
        );
        authority
            .complete(dispatch.id(), DispatchOutcome::ReadbackVerified, now)
            .unwrap();
        assert_eq!(
            authority.renew_native(replacement, Duration::ZERO, now),
            Err(OutputError::InvalidProfile.into())
        );
        assert_eq!(
            authority.renew_native(replacement, lifetime + Duration::from_nanos(1), now),
            Err(OutputError::InvalidProfile.into())
        );
        assert_eq!(
            authority.renew_native(replacement, lifetime, replacement.expires()),
            Err(OutputError::Expired.into())
        );
        assert_eq!(authority.snapshot.lease, Some(replacement));
    }

    #[test]
    fn native_duration_admission_and_revoke_never_resurrect_old_authority() {
        let (mut authority, initial) = armed();
        authority.clear_pending();
        assert!(
            authority.valid_native_duration(Duration::from_millis(100), Duration::from_secs(1))
        );
        assert!(!authority.valid_native_duration(Duration::from_secs(1), Duration::from_secs(1)));
        assert!(!authority.valid_native_duration(Duration::ZERO, Duration::from_secs(1)));
        assert!(
            !authority.valid_native_duration(Duration::from_millis(100), Duration::from_secs(2))
        );
        authority
            .command(OutputCommand::Trip, Duration::ZERO)
            .unwrap();
        assert_eq!(
            authority.renew_native(initial, Duration::from_secs(1), Duration::ZERO),
            Err(OutputError::StaleLease.into())
        );
        assert!(authority.snapshot.lease.is_none());
        assert_eq!(authority.snapshot.state, OutputState::SafePending);
    }
}
