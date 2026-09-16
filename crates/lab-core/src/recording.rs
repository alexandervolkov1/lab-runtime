//! Storage-independent facts captured at the authoritative state transition.
//!
//! The Core owner emits facts only when recording is enabled. This bounded outbox
//! is a short handoff to the host; it is not durable history or a client event ring.

use crate::{
    Sample, Unit, Value,
    control::{ControllerId, ControllerState, PidConfig},
    output::{ActuatorId, DispatchId, OutputIntent},
    reference::ReferenceId,
    transport::ResourceId,
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

/// Authoritative output binding captured before a fact enters the outbox.
/// Runtime refreshes this cache; storage never reconstructs it from later state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutputContext {
    pub(crate) unit: Option<Unit>,
    pub(crate) authority_epoch: Option<u64>,
    pub(crate) resource: Option<ResourceId>,
    pub(crate) binding_generation: Option<u64>,
    pub(crate) mapping_revision: Option<u64>,
}

/// Maximum facts awaiting host admission after one or more owner units.
pub const MAX_RECORDING_FACTS: usize = 256;
/// Conservative Core memory credit for pending facts.
pub const MAX_RECORDING_FACT_BYTES: usize = 256 * 1024;
/// Maximum unconfirmed durable-progress age for the reviewed M7 virtual profile.
pub const REQUIRED_PROGRESS_AGE: Duration = Duration::from_secs(2);

/// Plain Core prerequisite; only trusted host lifecycle methods may update it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequiredGate {
    pub(crate) enabled: bool,
    pub(crate) open: bool,
    pub(crate) failed: bool,
    pub(crate) generation: u64,
    pub(crate) valid_until: Option<Duration>,
    pub(crate) confirmed_submission: Option<Duration>,
}

// Group a Reference revision with its evaluated value so capture cannot mix
// fields from two snapshots while the caller constructs one publication fact.
pub(crate) struct ReferenceDetails {
    pub(crate) reference: ReferenceId,
    pub(crate) revision: u64,
    pub(crate) value: f64,
    pub(crate) target: Option<f64>,
    pub(crate) rate: Option<f64>,
    pub(crate) unit: Unit,
    pub(crate) at: Duration,
}

/// Distinct stages of an output attempt. A requested value is never delivery evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStage {
    /// An earlier queued intent was displaced by a Rust-owned safe transition.
    SupersededBeforeSend,
    /// Admission rejected the candidate before a dispatch identity or send exists.
    RejectedBeforeSend,
    /// Accepted proposal reached its exclusive deadline before any send.
    ExpiredBeforeSend,
    /// A validated candidate was accepted into authority's pending slot.
    Requested,
    /// Authority approved the candidate at the final dispatch check.
    Authorized,
    /// Authority admitted the attempt immediately before its first possible send.
    SendStarted,
    /// A correlated protocol acknowledgement was received; no readback is implied.
    Acknowledged,
    /// A correlated readback was verified; no independent physical proof is implied.
    ReadbackVerified,
    /// A correlated attempt failed.
    Failed,
    /// The effect remains unknown, including after a partial send or timeout.
    Ambiguous,
    /// A partial send or framing failure made effect uncertain before settlement.
    TransportUncertain,
    /// Rust requested the reserved safe procedure; no evidence yet exists.
    SafeRequested,
    /// Trusted safe dispatch reached the first possible send boundary.
    SafeSendStarted,
    /// Trusted protocol ACK for a safe attempt, without readback.
    SafeAcknowledged,
    /// Trusted virtual readback for a safe attempt.
    SafeReadbackVerified,
    /// Ordinary authority was revoked or fault-latched.
    Revoked,
}

/// Origin of a correlated completion; clients cannot assign this value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEvidenceSource {
    /// Intent or authorization, without terminal delivery evidence.
    None,
    /// Deterministic Rust virtual actuator simulation.
    VirtualSimulation,
    /// Rust-owned bounded transport and protocol decoder.
    TransportProtocol,
}

/// One immutable source fact with a process-local, monotonically increasing order.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordingFact {
    /// Observation captured on commit, including an unavailable attempt.
    Measurement {
        /// Order in this Runtime's fact stream.
        sequence: u64,
        /// Exact committed reading, quality, unit, and time.
        sample: Sample,
        /// Instrument or managed-component generation at commit.
        generation: u64,
        /// Active definition or state revision at commit.
        revision: u64,
    },
    /// Output transition captured before later transitions can overwrite snapshots.
    Output {
        /// Order in this Runtime's fact stream.
        sequence: u64,
        /// Exact actuator binding.
        actuator: ActuatorId,
        /// Process-local attempt order; the Recorder scopes it with the boot ID.
        attempt_id: Option<u64>,
        /// Trusted dispatch correlation, assigned only after admission.
        dispatch_id: Option<DispatchId>,
        /// Engineering unit at this transition.
        unit: Option<Unit>,
        /// Rust authority epoch, not a client-provided permit.
        authority_epoch: Option<u64>,
        /// Physical resource when this actuator has a Metakon binding.
        resource: Option<ResourceId>,
        /// Instrument binding generation, independent of transport generation.
        binding_generation: Option<u64>,
        /// Instrument mapping revision.
        mapping_revision: Option<u64>,
        /// Delivery/evidence stage.
        stage: OutputStage,
        /// Authorized scalar for this attempt when known.
        value: Option<f64>,
        /// Trusted origin of terminal delivery evidence, if any.
        source: OutputEvidenceSource,
        /// Runtime monotonic time of this transition.
        at: Duration,
    },
    /// Native controller lifecycle/configuration committed by Rust.
    Controller {
        /// Order in this Runtime's fact stream.
        sequence: u64,
        /// Stable native controller identity.
        controller: ControllerId,
        /// Committed lifecycle state at capture.
        state: ControllerState,
        /// Current configuration revision, independent of tick count.
        config_revision: u64,
        /// Complete PID settings on a configuration revision/baseline.
        pid: Option<PidConfig>,
        /// Monotonic time of this transition when supplied.
        at: Duration,
    },
    /// Reference evaluation or revision committed by Rust.
    Reference {
        /// Order in this Runtime's fact stream.
        sequence: u64,
        /// Stable Reference identity.
        reference: ReferenceId,
        /// Current configuration revision, independent of evaluation count.
        revision: u64,
        /// Evaluated current value at this time.
        value: f64,
        /// Target for a Ramp, if applicable.
        target: Option<f64>,
        /// Rate for a Ramp, if applicable.
        rate: Option<f64>,
        /// Exact engineering unit.
        unit: Unit,
        /// Monotonic evaluation/configuration time.
        at: Duration,
    },
}

impl RecordingFact {
    /// Return the order assigned at the authoritative transition.
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Measurement { sequence, .. }
            | Self::Output { sequence, .. }
            | Self::Controller { sequence, .. }
            | Self::Reference { sequence, .. } => *sequence,
        }
    }
}

/// Bounded Core-to-host handoff. Overflow is sticky until a new run is explicitly armed.
#[derive(Default)]
pub(crate) struct FactOutbox {
    enabled: bool,
    next_sequence: u64,
    facts: VecDeque<RecordingFact>,
    output_contexts: BTreeMap<ActuatorId, OutputContext>,
    bytes: usize,
    overflowed: bool,
    first_lost_sequence: Option<u64>,
    suppressed_facts: u64,
    suppressed_count_unknown: bool,
}

impl FactOutbox {
    pub(crate) fn enable(&mut self) {
        self.enabled = true;
        self.overflowed = false;
        self.first_lost_sequence = None;
        self.suppressed_facts = 0;
        self.suppressed_count_unknown = false;
    }

    pub(crate) fn disable(&mut self) {
        self.enabled = false;
        self.bytes = 0;
        self.facts.clear();
        self.output_contexts.clear();
    }

    /// Replace only the current trusted capture context for an accepted actuator.
    pub(crate) fn output_context(&mut self, actuator: ActuatorId, context: OutputContext) {
        if self.enabled {
            self.output_contexts.insert(actuator, context);
        }
    }

    pub(crate) fn take(&mut self) -> Vec<RecordingFact> {
        self.bytes = 0;
        self.facts.drain(..).collect()
    }

    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub(crate) fn first_lost_sequence(&self) -> Option<u64> {
        self.first_lost_sequence
    }

    /// Mark a missing trusted correlation as one lost fact without blocking
    /// the reserved safe transition that exposed it.
    pub(crate) fn lose_correlation(&mut self) {
        if !self.enabled {
            return;
        }
        let Some(sequence) = self.next_sequence.checked_add(1) else {
            self.overflowed = true;
            self.suppressed_count_unknown = true;
            return;
        };
        self.next_sequence = sequence;
        self.overflowed = true;
        self.suppress(sequence);
    }

    pub(crate) fn suppressed_facts(&self) -> Option<u64> {
        (!self.suppressed_count_unknown).then_some(self.suppressed_facts)
    }

    pub(crate) fn measurement(&mut self, sample: Sample, generation: u64, revision: u64) {
        let bytes = 256
            + match sample.value() {
                Some(Value::Text(value) | Value::Enum(value)) => value.capacity(),
                _ => 0,
            };
        self.push(bytes, |sequence| RecordingFact::Measurement {
            sequence,
            sample,
            generation,
            revision,
        });
    }

    pub(crate) fn output(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        at: Duration,
    ) {
        self.output_with_source(actuator, stage, value, at, OutputEvidenceSource::None);
    }

    pub(crate) fn output_with_source(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        at: Duration,
        source: OutputEvidenceSource,
    ) {
        self.output_correlated(actuator, stage, value, at, source, None, None);
    }

    // The explicit source, attempt and dispatch fields prevent a caller from
    // accidentally treating a requested value as trusted delivery evidence.
    #[expect(
        clippy::too_many_arguments,
        reason = "explicit evidence and correlation"
    )]
    pub(crate) fn output_correlated(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        at: Duration,
        source: OutputEvidenceSource,
        attempt_id: Option<u64>,
        dispatch_id: Option<DispatchId>,
    ) {
        self.output_with_unit(
            actuator,
            stage,
            value,
            at,
            source,
            attempt_id,
            dispatch_id,
            None,
            None,
        );
    }

    /// Freeze the queued M3 intent's original authority and binding identity.
    /// A late terminal/recovery event may arrive after today's epoch or mapping
    /// changes; it must still describe the bytes attempted under this intent.
    pub(crate) fn output_transport(
        &mut self,
        intent: OutputIntent,
        resource: ResourceId,
        stage: OutputStage,
        at: Duration,
        source: OutputEvidenceSource,
        dispatch_id: Option<DispatchId>,
    ) {
        self.push(128, |sequence| RecordingFact::Output {
            sequence,
            actuator: intent.actuator,
            attempt_id: intent.attempt_id,
            dispatch_id,
            unit: Some(intent.unit),
            authority_epoch: Some(intent.epoch),
            resource: Some(resource),
            binding_generation: Some(intent.binding_generation),
            mapping_revision: Some(intent.mapping_revision),
            stage,
            value: Some(intent.value),
            source,
            at,
        });
    }

    /// Preserve the exact proposed unit even when validation rejects a candidate.
    pub(crate) fn output_proposal(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        proposed_unit: Unit,
        at: Duration,
        attempt_id: Option<u64>,
    ) {
        self.output_with_unit(
            actuator,
            stage,
            value,
            at,
            OutputEvidenceSource::None,
            attempt_id,
            None,
            Some(proposed_unit),
            None,
        );
    }

    /// Record a displaced or expired proposal in its original authority epoch.
    pub(crate) fn output_displaced(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        at: Duration,
        attempt_id: Option<u64>,
        original_epoch: u64,
    ) {
        debug_assert!(matches!(
            stage,
            OutputStage::SupersededBeforeSend | OutputStage::ExpiredBeforeSend
        ));
        self.output_with_unit(
            actuator,
            stage,
            value,
            at,
            OutputEvidenceSource::None,
            attempt_id,
            None,
            None,
            Some(original_epoch),
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "explicit output proposal identity"
    )]
    fn output_with_unit(
        &mut self,
        actuator: ActuatorId,
        stage: OutputStage,
        value: Option<f64>,
        at: Duration,
        source: OutputEvidenceSource,
        attempt_id: Option<u64>,
        dispatch_id: Option<DispatchId>,
        proposed_unit: Option<Unit>,
        original_epoch: Option<u64>,
    ) {
        let context = self
            .output_contexts
            .get(&actuator)
            .copied()
            .unwrap_or_default();
        self.push(128, |sequence| RecordingFact::Output {
            sequence,
            actuator,
            attempt_id,
            dispatch_id,
            unit: proposed_unit.or(context.unit),
            authority_epoch: original_epoch.or(context.authority_epoch),
            resource: context.resource,
            binding_generation: context.binding_generation,
            mapping_revision: context.mapping_revision,
            stage,
            value,
            source,
            at,
        });
    }

    pub(crate) fn controller(
        &mut self,
        controller: ControllerId,
        state: ControllerState,
        config_revision: u64,
        pid: Option<PidConfig>,
        at: Duration,
    ) {
        self.push(256, |sequence| RecordingFact::Controller {
            sequence,
            controller,
            state,
            config_revision,
            pid,
            at,
        });
    }

    pub(crate) fn reference(&mut self, details: ReferenceDetails) {
        let ReferenceDetails {
            reference,
            revision,
            value,
            target,
            rate,
            unit,
            at,
        } = details;
        self.push(256, |sequence| RecordingFact::Reference {
            sequence,
            reference,
            revision,
            value,
            target,
            rate,
            unit,
            at,
        });
    }

    fn push(&mut self, bytes: usize, create: impl FnOnce(u64) -> RecordingFact) {
        if !self.enabled {
            return;
        }
        // Fact identity advances even when capacity denies the payload, so a
        // later run cannot reuse an identity whose absence was already reported.
        let Some(sequence) = self.next_sequence.checked_add(1) else {
            self.overflowed = true;
            self.suppressed_count_unknown = true;
            return;
        };
        self.next_sequence = sequence;
        if self.overflowed {
            self.suppress(sequence);
            return;
        }
        let Some(next_bytes) = self.bytes.checked_add(bytes) else {
            self.overflowed = true;
            self.suppress(sequence);
            return;
        };
        if self.facts.len() >= MAX_RECORDING_FACTS || next_bytes > MAX_RECORDING_FACT_BYTES {
            self.overflowed = true;
            self.suppress(sequence);
            return;
        }
        self.bytes = next_bytes;
        self.facts.push_back(create(sequence));
    }

    fn suppress(&mut self, sequence: u64) {
        self.first_lost_sequence.get_or_insert(sequence);
        if let Some(next) = self.suppressed_facts.checked_add(1) {
            self.suppressed_facts = next;
        } else {
            self.suppressed_count_unknown = true;
        }
    }
}
