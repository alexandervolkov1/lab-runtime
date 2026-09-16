//! Central output vocabulary and deterministic virtual dispatch.
//!
//! Runtime owns each authority. A proposal is only queued intent: the authority
//! rechecks its opaque lease at BeginDispatch, immediately before the simulated
//! send. Completing an old operation cannot restore ownership. ACK, readback and
//! safe evidence are deliberately distinct; software cancellation cannot undo a send.

use crate::{Error, InstrumentId, ParameterId, Unit, Value};
use std::time::Duration;

mod authority;
pub(crate) use authority::OutputAuthority;

/// Immutable trusted intent held by the transport queue. This is not public
/// because client code must never obtain or replay output authorization material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OutputIntent {
    pub(crate) actuator: ActuatorId,
    pub(crate) attempt_id: Option<u64>,
    pub(crate) instance: u64,
    pub(crate) lease: Option<OutputLease>,
    pub(crate) epoch: u64,
    pub(crate) value: f64,
    pub(crate) unit: Unit,
    pub(crate) expires: Duration,
    pub(crate) safe: bool,
    pub(crate) binding_generation: u64,
    pub(crate) mapping_revision: u64,
}

/// Canonical actuator binding, independent of its display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActuatorId {
    instrument: InstrumentId,
    parameter: ParameterId,
}
impl ActuatorId {
    /// Construct a binding; Runtime validates its descriptor before allowing output operations.
    pub const fn new(instrument: InstrumentId, parameter: ParameterId) -> Self {
        Self {
            instrument,
            parameter,
        }
    }
    /// Instrument whose trusted definition owns the operation.
    pub const fn instrument(self) -> InstrumentId {
        self.instrument
    }
    /// Parameter whose semantic role must explicitly be Actuator.
    pub const fn parameter(self) -> ParameterId {
        self.parameter
    }
}

/// Authority state, not an assertion that hardware has reached the requested value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputState {
    /// No safe procedure has established the configured evidence.
    Unverified,
    /// Ordinary authority is revoked; prior work must settle before safe completion.
    SafePending,
    /// No owner; the required safe evidence has been obtained.
    Disarmed,
    /// One manual producer may submit bounded proposals.
    ArmedManual,
    /// One automatic producer may submit bounded proposals, under identical checks.
    ArmedAuto,
    /// A failure is retained; recovery never automatically rearms.
    FaultLatched,
}

/// Producer identity and mode. Automatic is only an identity in M2, not a controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputOwner {
    /// A manually driven producer with a local ID.
    Manual(u64),
    /// A Runtime-managed algorithm producer with a local ID.
    Automatic(u64),
}

/// Minimum delivery evidence accepted by this particular safe profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceLevel {
    /// Protocol acknowledgement is sufficient only if explicitly selected by the profile.
    Acknowledgement,
    /// A matching readback is required; it is still not independent physical safety proof.
    Readback,
}

/// Explicit simulated safe policy. Zero has no privileged meaning in this type.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeProfile {
    /// Inclusive operating lower bound in the trusted actuator descriptor's unit.
    pub min: f64,
    /// Inclusive operating upper bound in the trusted actuator descriptor's unit.
    pub max: f64,
    /// Selected safe target inside the operating range.
    pub safe_value: f64,
    /// Largest duration allowed when issuing one owner lease.
    pub max_lease: Duration,
    /// Largest lifetime of a queued normal proposal.
    pub max_proposal_ttl: Duration,
    /// Evidence required before declaring the safe procedure complete.
    pub required_evidence: EvidenceLevel,
}
impl SafeProfile {
    /// Validate finite ordered limits, safe target and nonzero bounded-duration policy.
    ///
    /// This does not establish physical safety or activate an actuator.
    pub fn validate(&self) -> Result<(), Error> {
        if !self.min.is_finite()
            || !self.max.is_finite()
            || !self.safe_value.is_finite()
            || self.min >= self.max
            || !(self.min..=self.max).contains(&self.safe_value)
            || self.max_lease.is_zero()
            || self.max_proposal_ttl.is_zero()
        {
            return Err(OutputError::InvalidProfile.into());
        }
        Ok(())
    }
}

/// Opaque authorization for one owner, binding and authority epoch.
/// Copying a token does not extend its deadline or make a revoked token current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLease {
    actuator: ActuatorId,
    instance: u64,
    owner: OutputOwner,
    epoch: u64,
    expires: Duration,
}
impl OutputLease {
    /// Immutable authority instance identity; renewal never replaces this value.
    pub fn instance(self) -> u64 {
        self.instance
    }
    /// Revocation counter captured when the lease was issued.
    pub fn epoch(self) -> u64 {
        self.epoch
    }
    /// Producer for whom this token was issued.
    pub fn owner(self) -> OutputOwner {
        self.owner
    }
    /// Exclusive monotonic expiry; a proposal is invalid at this time or later.
    pub fn expires(self) -> Duration {
        self.expires
    }
}

/// A candidate value, never a direct driver command or a mutable permit.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputProposal {
    /// Opaque current owner token.
    pub lease: OutputLease,
    /// Candidate scalar, checked for exact type, finiteness and range.
    pub value: Value,
    /// Explicit engineering unit; implicit conversion is forbidden.
    pub unit: Unit,
    /// Requested lifetime, no larger than the profile maximum; the effective
    /// deadline is additionally clipped to the lease expiry.
    pub ttl: Duration,
}

/// A send already admitted at the final authority check; fields cannot be substituted.
///
/// This is a record of a send, not a transferable permission to send again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dispatch {
    id: DispatchId,
    value: f64,
    safe: bool,
    epoch: u64,
}
impl Dispatch {
    /// Local completion correlation; not an external request or deduplication ID.
    pub fn id(self) -> DispatchId {
        self.id
    }
    /// Exact authorized scalar value sent at this dispatch step.
    pub fn value(self) -> f64 {
        self.value
    }
    /// Whether this is the reserved Rust-owned safe procedure, not ordinary control.
    pub fn is_safe(self) -> bool {
        self.safe
    }
    /// Authority epoch frozen when this dispatch was authorized.
    pub fn epoch(self) -> u64 {
        self.epoch
    }
}

/// Completion correlation scoped to a unique process-local authority instance.
///
/// Private fields prevent accidental reconstruction from a reused numeric sequence.
/// This is not a persistent request ID or a claim of uniqueness across process restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchId {
    instance: u64,
    sequence: u64,
}
impl DispatchId {
    /// Read-only diagnostic correlation; these parts cannot reconstruct a permit.
    /// Recorder must pair them with its boot ID before persisting a dispatch ID.
    pub const fn diagnostic_parts(self) -> (u64, u64) {
        (self.instance, self.sequence)
    }
}

/// Terminal simulated outcomes. Ambiguous never means success or absence of effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The operation was acknowledged, without a readback claim.
    Acknowledged,
    /// A matching value was read back after the operation settled.
    ReadbackVerified,
    /// The attempt failed; ordinary authority must be revoked.
    Failed,
    /// The attempt has settled, but its effect cannot be established.
    Ambiguous,
}

/// Latest delivery observation at a specific monotonic time, not durable history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputObservation {
    /// Authorized value associated with this delivery stage.
    pub value: f64,
    /// Time when Runtime recorded this stage.
    pub at: Duration,
}

/// Bounded owned inspection view. Querying it does not expire or renew a lease.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputSnapshot {
    /// Current permission/lifecycle state.
    pub state: OutputState,
    /// Current owner, if any; old tokens do not imply current ownership.
    pub lease: Option<OutputLease>,
    /// Current revocation counter.
    pub epoch: u64,
    /// Whether a failure still requires deliberate recovery acknowledgement.
    pub fault_latched: bool,
    /// Whether the latest correlated safe procedure met its evidence requirement.
    pub safe_confirmed: bool,
    /// At most one ordinary proposal can await dispatch.
    pub pending: bool,
    /// At most one sent operation can await terminal completion.
    pub in_flight: Option<Dispatch>,
    /// Latest requested normal value, not proof it was sent.
    pub requested: Option<f64>,
    /// Latest value admitted at the send boundary.
    pub sent: Option<OutputObservation>,
    /// Acknowledged value for the latest send, when known.
    pub acknowledged: Option<OutputObservation>,
    /// Readback for the latest send; ACK alone leaves this absent.
    pub readback: Option<OutputObservation>,
    /// Latest terminal result; cleared when another send starts.
    pub outcome: Option<DispatchOutcome>,
}

/// Local output commands; time is supplied by the enclosing Runtime command.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputCommand {
    /// Install a validated profile while unverified/disarmed, invalidating prior safe evidence.
    BindProfile(SafeProfile),
    /// Revoke ordinary authority and request the reserved safe procedure.
    RequestSafe,
    /// Acquire exclusive authority only after safe verification.
    Acquire {
        /// Producer identity/mode; neither mode bypasses validation.
        owner: OutputOwner,
        /// Requested finite lease duration within the profile maximum.
        lifetime: Duration,
    },
    /// Release only this current owner; physical safety still requires a safe completion.
    Release(OutputLease),
    /// Latch a fault notification, revoke authority and request safe output.
    /// M2 has no persistent interlock input; a successful explicit safe recovery
    /// resolves the simulated cause, and acknowledgement still cannot rearm.
    Trip,
    /// Clear a latch only after explicit recovery has produced valid safe evidence.
    AcknowledgeFault,
    /// Queue validated intent; this does not send.
    Propose(OutputProposal),
    /// Perform the final check and begin one simulated send.
    BeginDispatch,
    /// Supply one terminal simulated result, correlated to the currently in-flight dispatch.
    Complete {
        /// Exact ID returned by BeginDispatch.
        dispatch_id: DispatchId,
        /// Evidence/failure observed by the deterministic simulated executor.
        outcome: DispatchOutcome,
    },
    /// Explicit watchdog step; queries never perform this mutation.
    Tick,
}

/// Completed local authority operation; Queued is deliberately distinct from Dispatched.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputResult {
    /// A lifecycle/profile/watchdog transition completed.
    Updated,
    /// An exclusive bounded owner token was issued.
    Lease(OutputLease),
    /// A normal candidate was queued, not sent.
    Queued,
    /// One value passed the final check and entered the simulated in-flight state.
    Dispatched(Dispatch),
    /// One correlated terminal result was recorded; inspect state/evidence for success.
    Completed,
}

/// Authority-specific failures, separate from scalar schema validation errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputError {
    /// The binding is not a registered actuator.
    UnknownActuator,
    /// No profile is installed or its constraints are inconsistent.
    InvalidProfile,
    /// The current state forbids this lifecycle operation.
    InvalidState,
    /// This lease belongs to a different owner, instance, binding or revoked epoch.
    StaleLease,
    /// A lease/proposal deadline has been reached.
    Expired,
    /// The candidate unit is incompatible with the actuator.
    WrongUnit,
    /// The single pending/in-flight slot is occupied.
    Busy,
    /// No normal or reserved safe operation is ready.
    NothingPending,
    /// A completion is not for the currently in-flight operation.
    UnknownDispatch,
    /// A supplied time moved backwards or a duration addition overflowed.
    InvalidTime,
    /// A nonwrapping local identity/epoch counter is exhausted.
    CounterExhausted,
}
impl From<OutputError> for Error {
    fn from(error: OutputError) -> Self {
        Error::Output(error)
    }
}
