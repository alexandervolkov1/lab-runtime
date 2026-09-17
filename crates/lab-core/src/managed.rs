//! Language-neutral, bounded observation components owned by Runtime.
//!
//! The executor receives owned plain data. Its callbacks cannot borrow Runtime,
//! submit output proposals, claim physical evidence, or mutate committed state.

use crate::{InstrumentId, ParameterId, SignalId, Unit};
use std::{collections::BTreeMap, time::Duration};

/// At most eight managed observation components may be registered per Runtime.
pub const MAX_COMPONENTS: usize = 8;
/// Stable implementation identifiers are small semantic names, not Rust type names.
pub const MAX_IMPLEMENTATION_ID_BYTES: usize = 64;
/// Bound on each persistent plain-data map, including keys and typed payloads.
pub const MAX_PLAIN_BYTES: usize = 4096;

/// Stable identity distinct from the descriptor's instrument identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentId(u64);

impl ComponentId {
    /// Construct an opaque Runtime-local identity.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Return the numeric representation for bounded diagnostic correlation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Only a virtual source or a single-input observation transform is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentKind {
    /// A model publishing one read-only virtual measurement.
    Source,
    /// A transform consuming one typed native or managed Source observation.
    Transform {
        /// One bound Float input carrying its original observation lineage.
        input: SignalId,
    },
}

/// Trusted immutable descriptor and processing policy; implementations cannot revise it.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentManifest {
    /// The only supported component-data schema version.
    pub schema_version: u8,
    /// Runtime-local component identity.
    pub id: ComponentId,
    /// Generic instrument identity used by existing discovery and SignalId.
    pub instrument: InstrumentId,
    /// Bounded human-readable name, independent of identity.
    pub name: String,
    /// The one read-only Float measurement parameter.
    pub parameter: ParameterId,
    /// One virtual Source or one observation Transform input.
    pub kind: ComponentKind,
    /// Trusted output engineering-unit identity.
    pub unit: Unit,
    /// Inclusive minimum finite output value.
    pub min: f64,
    /// Inclusive maximum finite output value.
    pub max: f64,
    /// Number of distinct accepted steps before Ready is permitted.
    pub warmup_samples: usize,
    /// Exclusive freshness limit for an input and dependent controller.
    pub max_input_age: Duration,
    /// Bound on the normal recent-attempt signal window.
    pub history_capacity: usize,
}

/// Stable semantic implementation identity selected by trusted host composition.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentImplementationId(String);

impl ComponentImplementationId {
    /// Validate a bounded lowercase semantic identifier such as `native.moving_mean.v1`.
    pub fn new(value: impl Into<String>) -> Result<Self, ComponentError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_IMPLEMENTATION_ID_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(ComponentError::InvalidConfiguration);
        }
        Ok(Self(value))
    }

    /// Borrow the stable wire/provenance representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicit selection of one compile-time trusted implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentImplementation {
    id: ComponentImplementationId,
}

impl ComponentImplementation {
    /// Select one compile-time trusted implementation without executable source text.
    pub fn built_in(id: impl Into<String>) -> Result<Self, ComponentError> {
        Ok(Self {
            id: ComponentImplementationId::new(id)?,
        })
    }

    /// Stable semantic implementation identity.
    pub const fn id(&self) -> &ComponentImplementationId {
        &self.id
    }
}

/// Immutable implementation, manifest and configuration selected by trusted composition.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentDefinition {
    /// Validated shape and descriptor; no actuator role is expressible here.
    pub manifest: ComponentManifest,
    /// Explicit trusted implementation selection; Core never interprets its artifact.
    pub implementation: ComponentImplementation,
    /// Runtime-owned finite scalar configuration, copied per invocation.
    pub config: PlainData,
}

/// One leaf in a bounded map. Nested maps and object graphs cannot cross this port.
#[derive(Clone, Debug, PartialEq)]
pub enum PlainValue {
    /// Finite scalar number.
    Number(f64),
    /// Logical value without numeric coercion.
    Boolean(bool),
    /// UTF-8 text of at most 128 bytes.
    Text(String),
    /// Dense list of at most 64 finite numbers.
    Numbers(Vec<f64>),
}

/// Bounded immutable-at-invocation configuration or committed persistent state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlainData {
    /// At most sixteen unique ASCII field names, each 1..=32 bytes.
    pub fields: BTreeMap<String, PlainValue>,
}

impl PlainData {
    /// Check logical size and leaves before allocating any invocation copy.
    pub fn validate(&self) -> Result<(), ComponentError> {
        if self.fields.len() > 16 {
            return Err(ComponentError::DataLimit);
        }
        let mut bytes = 0usize;
        let mut leaves = 0usize;
        for (key, value) in &self.fields {
            if key.is_empty() || key.len() > 32 || !key.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(ComponentError::InvalidConfiguration);
            }
            // Checked logical size is key bytes, one tag and scalar/length payload.
            let size = match value {
                PlainValue::Number(number) if number.is_finite() => {
                    leaves += 1;
                    9
                }
                PlainValue::Boolean(_) => {
                    leaves += 1;
                    2
                }
                PlainValue::Text(text) if text.len() <= 128 => {
                    leaves += 1;
                    3 + text.len()
                }
                PlainValue::Numbers(numbers)
                    if numbers.len() <= 64 && numbers.iter().all(|n| n.is_finite()) =>
                {
                    leaves += numbers.len();
                    3 + numbers.len() * 8
                }
                _ => return Err(ComponentError::DataLimit),
            };
            bytes = bytes
                .checked_add(key.len() + 1 + size)
                .ok_or(ComponentError::DataLimit)?;
            if bytes > MAX_PLAIN_BYTES || leaves > 256 {
                return Err(ComponentError::DataLimit);
            }
        }
        Ok(())
    }
}

/// Typed bounded rejection; no unbounded implementation diagnostic crosses into Core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentError {
    /// A referenced component was not committed.
    Unknown,
    /// Manifest, binding or configured shape was inconsistent.
    InvalidConfiguration,
    /// The fixed stage/worker/pending capacity was already occupied.
    Busy,
    /// Plain data, result, source or diagnostics exceeded a fixed limit.
    DataLimit,
    /// Input was absent, invalid, or expired under the exclusive age rule.
    InputUnavailable,
    /// Result type, status, unit or finite/range value was invalid.
    InvalidResult,
    /// Host's independent 100-ms acceptance deadline expired.
    Deadline,
    /// An implementation reached its accounted memory ceiling.
    MemoryLimit,
    /// An implementation exceeded its bounded execution budget.
    ExecutionLimit,
    /// Trusted adapter calls exceeded their fixed invocation budget.
    AdapterCallLimit,
    /// The selected implementation artifact could not be admitted.
    InvalidImplementation,
    /// The supplied nonblocking executor rejected or failed the job.
    Executor,
    /// Generation, state revision or attempt counter would wrap/reuse an identity.
    IdentityExhausted,
}

impl From<ComponentError> for crate::Error {
    fn from(error: ComponentError) -> Self {
        Self::Component(error)
    }
}

/// Expected callback phase. Init cannot publish a measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationPhase {
    /// Compile/load and return bounded initial state, without Good output.
    Init,
    /// Evaluate one distinct model time or one captured input observation.
    Step,
}

/// Checked full-width correlation stays outside implementation code to avoid forgery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Correlation {
    /// Unique process-local Runtime instance.
    pub runtime: u64,
    /// Exact managed component identity.
    pub component: ComponentId,
    /// Checked replacement generation, including staged candidates.
    pub generation: u64,
    /// Monotonic step/initialization attempt within a generation.
    pub attempt: u64,
    /// Expected committed state revision at admission.
    pub revision: u64,
}

/// Immutable typed input and original physical/model freshness lineage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedInput {
    /// Signal identity of the original input branch.
    pub signal: SignalId,
    /// Finite typed Float observed from a Good input.
    pub value: f64,
    /// Declared engineering unit.
    pub unit: Unit,
    /// Publication time of the captured attempt.
    pub at: Duration,
    /// Original observation time used to bound freshness after worker latency.
    pub freshness_at: Duration,
    /// Upstream replacement generation captured before asynchronous execution.
    pub source_generation: u64,
    /// Upstream definition/configuration or binding mapping revision.
    pub source_revision: u64,
    /// Upstream managed state revision, when the input is managed.
    pub source_state_revision: Option<u64>,
}

/// Fully owned job, never a mutable Runtime reference or an output capability.
#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    /// Runtime-generated correlation, never supplied by implementation code.
    pub correlation: Correlation,
    /// Init or one Step.
    pub phase: InvocationPhase,
    /// Immutable bounded trusted definition.
    pub definition: ComponentDefinition,
    /// Last committed plain-data state, copied into this disposable job.
    pub state: PlainData,
    /// Core's authoritative explicit service/model time.
    pub at: Duration,
    /// Checked model/input elapsed interval, not implementation-provided arithmetic.
    pub dt: Duration,
    /// One captured Good input for a Transform, absent for a Source.
    pub input: Option<CapturedInput>,
}

/// One component callback's status. Ready alone cannot bypass Rust validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentStatus {
    /// Initial state, never a Good sample.
    Init,
    /// Incomplete preparation; no value is control-usable.
    Warming,
    /// A finite candidate may become Good after Core validates it.
    Ready,
    /// Failed attempt; candidate state/value are discarded.
    Unavailable,
}

/// Bounded data returned by the executor; it cannot represent physical evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentResult {
    /// Callback result status.
    pub status: ComponentStatus,
    /// Float candidate only for a valid Ready Step.
    pub value: Option<f64>,
    /// Exact trusted output unit required by the manifest.
    pub unit: Unit,
    /// Candidate replacement plain-data state, committed atomically with Good/warm-up.
    pub state: PlainData,
    /// Zero to four bounded UTF-8 diagnostic messages, not unbounded traceback.
    pub diagnostics: Vec<String>,
}

/// One owned, bounded terminal result or typed failure from an isolated worker.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentCompletion {
    /// Exact job identity echoed by the trusted executor, not component output.
    pub correlation: Correlation,
    /// Worker finished construction, conversion and cleanup before its deadline.
    pub timely: bool,
    /// No dispatch, ACK/readback or safe-completion type exists in this result.
    pub outcome: Result<ComponentResult, ComponentError>,
}

/// Nonblocking trusted adapter seam; two real slots and deterministic fake share it.
/// None of these methods is exposed as an implementation callback or public client command.
pub trait ComponentExecutor: Send {
    /// Admit only if one bounded worker mailbox is free; never wait for its VM.
    fn try_submit(&mut self, job: Invocation) -> Result<(), ComponentError>;
    /// Poll at most one bounded owned completion without waiting.
    fn try_poll(&mut self) -> Option<ComponentCompletion>;
    /// Fence an obsolete job without waiting for physical callback termination.
    fn try_cancel(&mut self, correlation: Correlation);
    /// Check independent adapter deadline while the worker may remain quarantined.
    fn try_expire(&mut self, _correlation: Correlation) -> bool {
        false
    }
    /// Fence new jobs and request cancellation without sleeping or joining.
    /// Existing jobs may remain physically blocked; this is an acceptance fence.
    fn begin_shutdown(&mut self) {}
    /// Count workers that have not finished, without waiting for them.
    fn unfinished_workers(&self) -> usize {
        0
    }
}

/// Domain-owned lifecycle of a committed, read-only managed observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentState {
    /// Initial callback committed state but has produced no Good reading.
    Warming,
    /// A validated Good reading is published.
    Ready,
    /// An invalid/unavailable/deadline result is latched until deliberate replacement.
    Failed,
}

/// Owned bounded status query; reading it never runs callbacks or safety service.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentSnapshot {
    /// Stable identity independent of generation or display name.
    pub id: ComponentId,
    /// Descriptor identity shared with generic measurement queries.
    pub instrument: InstrumentId,
    /// Stable semantic implementation identity, independent of Rust type names.
    pub implementation: ComponentImplementationId,
    /// Checked committed generation.
    pub generation: u64,
    /// Checked state revision.
    pub revision: u64,
    /// Warming, Ready or Failed.
    pub state: ComponentState,
    /// Bounded count of distinct accepted warm-up steps.
    pub good_steps: usize,
    /// Most recent committed plain-data state.
    pub committed_state: PlainData,
    /// At most one pending job correlation.
    pub pending: Option<Correlation>,
    /// Last bounded validated diagnostics, not arbitrary VM output.
    pub diagnostics: Vec<String>,
}
