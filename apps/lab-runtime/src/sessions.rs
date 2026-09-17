//! Session scopes and request IDs are process-local, bounded coordination state.
//!
//! An evicted accepted ID remains below its scope high-water mark and never
//! executes again. Unknown means unknowable after retention, not unexecuted.

use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

/// Hard maximum of retained logical client scopes in one Runtime process.
pub const MAX_SCOPES: usize = 16;
/// Hard maximum accepted nonterminal operations across scopes.
pub const MAX_PENDING: usize = 64;
/// Hard maximum retained terminal outcomes across scopes.
pub const MAX_TERMINAL: usize = 256;
/// Maximum accepted nonterminal operations retained by one scope.
pub(crate) const MAX_PENDING_SCOPE: usize = 8;
/// Maximum terminal outcomes retained by one scope.
pub(crate) const MAX_TERMINAL_SCOPE: usize = 32;
/// Finite terminal outcome retention after completion.
pub(crate) const TERMINAL_TTL: Duration = Duration::from_secs(600);
/// Finite detached-scope retention.
pub(crate) const IDLE_SCOPE_TTL: Duration = Duration::from_secs(1800);
/// Maximum serialized result/code bytes retained for one operation.
pub(crate) const OUTCOME_SIZE_LIMIT: usize = 4_096;

/// Complete typed mutation identity; connection/msg_id do not affect equality.
#[derive(Clone, Debug, PartialEq)]
pub enum Mutation {
    /// Replace a complete Reference policy while retaining its stable identity and unit.
    ConfigureReferenceFixed {
        /// Existing Reference identity.
        reference: u64,
        /// Configuration revision compared in Core.
        expected_revision: u64,
        /// New finite constant value in the existing engineering unit.
        value: f64,
    },
    /// Replace a complete Ramp policy from its current committed value.
    ConfigureReferenceRamp {
        /// Existing Reference identity.
        reference: u64,
        /// Configuration revision compared in Core.
        expected_revision: u64,
        /// New finite target in the existing engineering unit.
        target: f64,
        /// Positive engineering units per second.
        rate: f64,
    },
    /// Retune existing Ramp without changing its stable identity.
    RetuneRamp {
        /// Existing Reference identity.
        reference: u64,
        /// Configuration revision compared in Core.
        expected_revision: u64,
        /// Finite target in the Reference's existing unit.
        target: f64,
        /// Positive engineering units per second.
        rate: f64,
    },
    /// Replace a complete native PID configuration in a safe lifecycle state.
    ConfigurePid {
        /// Existing native controller identity.
        controller: u64,
        /// Configuration revision compared in Core.
        expected_revision: u64,
        /// Proportional gain.
        kp: f64,
        /// Integral gain.
        ki: f64,
        /// Derivative-on-measurement gain.
        kd: f64,
        /// Inclusive output lower limit.
        output_min: f64,
        /// Inclusive output upper limit.
        output_max: f64,
    },
    /// Replace all mutable native-controller policy while preserving read-only bindings.
    ConfigureController {
        /// Existing native controller identity.
        controller: u64,
        /// Configuration revision compared in Core.
        expected_revision: u64,
        /// Proportional gain.
        kp: f64,
        /// Integral gain.
        ki: f64,
        /// Derivative-on-measurement gain.
        kd: f64,
        /// Inclusive output lower limit.
        output_min: f64,
        /// Inclusive output upper limit.
        output_max: f64,
        /// Native EMA time constant in nanoseconds.
        ema_time_constant_ns: u64,
        /// Distinct good observations required before readiness.
        ema_warmup_samples: u64,
        /// Exclusive input freshness threshold in nanoseconds.
        max_input_age_ns: u64,
        /// Maximum controller service gap in nanoseconds.
        max_tick_gap_ns: u64,
        /// Finite automatic authority lifetime in nanoseconds.
        lease_lifetime_ns: u64,
        /// Finite proposal lifetime in nanoseconds.
        proposal_ttl_ns: u64,
    },
    /// Start an existing Ready native controller.
    Start {
        /// Existing native controller identity.
        controller: u64,
    },
    /// Pause an active native controller and establish safe evidence.
    Pause {
        /// Existing native controller identity.
        controller: u64,
    },
    /// Resume a safely Paused native controller through Warming.
    Resume {
        /// Existing native controller identity.
        controller: u64,
    },
    /// Deliberately acknowledge a safely failed controller into Paused.
    ResetFailed {
        /// Existing native controller identity.
        controller: u64,
    },
    /// Commit a bounded new durable run before admitting its facts.
    RecordingStart {
        /// Human-readable label, never an identity.
        label: String,
    },
    /// Seal exactly the requested active run after Required control has paused.
    RecordingStop {
        /// Boot identity of the current recording run.
        boot_id: String,
        /// Checked run counter within that boot.
        run_no: u64,
    },
    /// Bounded informational record accepted by Recorder ingress, not evidence.
    ExperimentAnnotate {
        /// Validated human-readable annotation name.
        name: String,
        /// Validated canonical JSON data, with no caller-supplied authority.
        data_json: String,
    },
    /// Explicit indexed historical selection, with no Runtime mutation.
    HistoryReadMeasurements {
        /// Stable archive database identity.
        database_id: String,
        /// Archive boot identity, possibly older than the serving process.
        boot_id: String,
        /// Checked run counter within the archived boot.
        run_no: u64,
        /// Stable signal instrument ID.
        instrument: u64,
        /// Stable signal parameter ID.
        parameter: u64,
        /// Inclusive publication time in archive-boot nanoseconds.
        from_ns: u64,
        /// Exclusive publication time in archive-boot nanoseconds.
        to_ns: u64,
        /// Whole raw rows requested, 1..=128.
        max_records: u16,
        /// One connection-retained continuation token, if any.
        cursor: Option<String>,
    },
    /// Explicit bounded archive-run discovery with a frozen upper key.
    HistoryReadRuns {
        /// Stable archive database identity.
        database_id: String,
        /// Whole run summaries requested, 1..=32.
        max_records: u8,
        /// One connection-retained continuation token, if any.
        cursor: Option<String>,
    },
    /// Reload and apply the configured deployment through its classified lifecycle.
    ReloadConfiguration,
    /// Load and retain one bounded configuration candidate without active mutation.
    StageConfiguration,
    /// Apply one retained candidate under its candidate/revision fence.
    ApplyConfiguration {
        /// Process-local staged candidate identity.
        candidate_id: u64,
        /// Active deployment revision observed at stage time.
        expected_revision: u64,
    },
    /// Validate and apply one supported scalar property through deployment lifecycle.
    ConfigureProperty {
        /// Semantic owning object kind.
        target_kind: String,
        /// Stable owning object identity.
        target_id: u64,
        /// Stable property identity.
        property: String,
        /// Bounded typed scalar value.
        value: PropertyMutationValue,
        /// Active deployment revision observed by the caller.
        expected_revision: u64,
    },
    /// Publish one Runtime-timed observation to an explicitly writable virtual signal.
    PublishEmulatorMeasurement {
        /// Stable instrument identity shared with discovery/history/events.
        instrument: u64,
        /// Stable parameter identity shared with discovery/history/events.
        parameter: u64,
        /// Generation observed from discovery/current state.
        expected_generation: u64,
        /// Good scalar or explicit virtual Unavailable.
        publication: EmulatorPublication,
    },
    /// Reload only managed implementation sources; TOML and built-ins are unchanged.
    /// Reinitialize configured managed components and native virtual models.
    RestartVirtualModels,
    /// Explicitly replace one configured read-only resource session.
    ReconnectResource {
        /// Stable logical resource identity.
        resource: u64,
        /// Binding generation observed before the reconnect.
        expected_binding_generation: u64,
    },
    /// Initiate host process shutdown; safe evidence is the later terminal result.
    Shutdown,
}

/// Typed scalar values admitted by generic configuration mutation.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyMutationValue {
    /// Signed integer.
    Integer(i64),
    /// Bounded UTF-8 text.
    Text(String),
}

/// Bounded states accepted by external virtual measurement publication.
#[derive(Clone, Debug, PartialEq)]
pub enum EmulatorPublication {
    /// One finite floating-point value in the signal's configured unit and range.
    Good(f64),
    /// Explicit virtual disabled/unavailable observation.
    Unavailable,
}
impl Mutation {
    fn normalized(mut self) -> Self {
        fn zero(value: &mut f64) {
            if *value == 0.0 {
                *value = 0.0
            }
        }
        match &mut self {
            Self::RetuneRamp { target, rate, .. } => {
                zero(target);
                zero(rate);
            }
            Self::ConfigurePid {
                kp,
                ki,
                kd,
                output_min,
                output_max,
                ..
            } => {
                for value in [kp, ki, kd, output_min, output_max] {
                    zero(value);
                }
            }
            _ => {}
        }
        self
    }
}

/// Known in-process operation progress or terminal result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationState {
    /// Reserved before dispatch; absence of a reply is not cancellation.
    Accepted,
    /// Domain action committed, with a bounded summary/result value.
    Completed(String),
    /// Domain action rejected or failed after admission, with a bounded code.
    Failed(String),
    /// Failed domain/host outcome with a retained bounded evidence summary.
    FailedWithResult {
        /// Stable machine-readable failure code.
        code: String,
        /// Bounded JSON summary available to reconnecting clients.
        detail: String,
    },
}
impl OperationState {
    fn terminal(&self) -> bool {
        !matches!(self, Self::Accepted)
    }
}

/// Dedup/admission decision; only Accepted permits first execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// New consecutive ID reserved before owner dispatch.
    Accepted,
    /// Retained equal payload; return this outcome without re-execution.
    Known(OperationState),
    /// Retained ID with a different normalized typed payload.
    Conflict,
    /// Evicted/unrecorded old result; never execute the ID again.
    Unknown,
    /// New number is not the scope's next consecutive number.
    Gap,
    /// Bounded pending admission is full; number did not advance.
    Busy,
    /// No such live process-local scope; caller must reconcile and obtain a new one.
    ScopeUnknown,
}

/// Scope creation/resumption failure with no domain action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// Invalid boot ID supplied to the process-local store.
    InvalidBoot,
    /// All sixteen live/nonexpired scopes are occupied.
    ScopeCapacity,
    /// The supplied scope was evicted or never server-issued here.
    ScopeUnknown,
    /// A different Runtime process boot identity issued this scope.
    InstanceChanged,
    /// The scope already belongs to another live connection.
    ScopeInUse,
    /// Checked scope/request counter cannot advance.
    CounterExhausted,
    /// Terminal result exceeds the bounded record shape or record is absent.
    InvalidOutcome,
}

/// Server-issued reconnect scope and exact next command sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenScope {
    /// `<boot-id>:<checked-decimal-counter>` identity.
    pub scope: String,
    /// High-water + 1; retained result eviction never lowers it.
    pub next_seq: u64,
}

#[derive(Clone)]
struct Record {
    seq: u64,
    payload: Mutation,
    state: OperationState,
    terminal_at: Option<Duration>,
}
struct Scope {
    attached: Option<u64>,
    high_water: u64,
    last_active: Duration,
    records: VecDeque<Record>,
}

/// Single-owner finite session/operation store, independent of TCP connection life.
pub struct SessionStore {
    boot_id: String,
    next_scope: u64,
    scopes: BTreeMap<String, Scope>,
}
impl SessionStore {
    /// Create a fresh process-local store from the OS-issued 128-bit boot ID.
    pub fn new(boot_id: &str) -> Result<Self, SessionError> {
        if boot_id.len() != 32
            || !boot_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(SessionError::InvalidBoot);
        }
        Ok(Self {
            boot_id: boot_id.into(),
            next_scope: 1,
            scopes: BTreeMap::new(),
        })
    }

    /// Create or resume one scope after a successful connection hello.
    pub fn open(
        &mut self,
        old: Option<&str>,
        connection: u64,
        now: Duration,
    ) -> Result<OpenScope, SessionError> {
        self.expire(now);
        if let Some(old) = old {
            let Some((boot, _)) = old.split_once(':') else {
                return Err(SessionError::ScopeUnknown);
            };
            if boot != self.boot_id {
                return Err(SessionError::InstanceChanged);
            }
            let scope = self.scopes.get_mut(old).ok_or(SessionError::ScopeUnknown)?;
            if scope
                .attached
                .is_some_and(|attached| attached != connection)
            {
                return Err(SessionError::ScopeInUse);
            }
            scope.attached = Some(connection);
            scope.last_active = now;
            let next_seq = scope
                .high_water
                .checked_add(1)
                .ok_or(SessionError::CounterExhausted)?;
            return Ok(OpenScope {
                scope: old.into(),
                next_seq,
            });
        }
        if self.scopes.len() >= MAX_SCOPES {
            return Err(SessionError::ScopeCapacity);
        }
        let counter = self.next_scope;
        self.next_scope = counter
            .checked_add(1)
            .ok_or(SessionError::CounterExhausted)?;
        let scope_id = format!("{}:{}", self.boot_id, counter);
        self.scopes.insert(
            scope_id.clone(),
            Scope {
                attached: Some(connection),
                high_water: 0,
                last_active: now,
                records: VecDeque::new(),
            },
        );
        Ok(OpenScope {
            scope: scope_id,
            next_seq: 1,
        })
    }

    /// Detach only the exact old connection; accepted work/outcomes remain.
    pub fn detach(&mut self, connection: u64, now: Duration) {
        for scope in self.scopes.values_mut() {
            if scope.attached == Some(connection) {
                scope.attached = None;
                scope.last_active = now;
            }
        }
    }

    /// Reserve a new consecutive ID before dispatch or return a retained/unknown outcome.
    pub fn admit(
        &mut self,
        scope_id: &str,
        seq: u64,
        payload: Mutation,
        now: Duration,
    ) -> Admission {
        self.expire(now);
        let payload = payload.normalized();
        let Some(scope) = self.scopes.get(scope_id) else {
            return Admission::ScopeUnknown;
        };
        if let Some(old) = scope.records.iter().find(|record| record.seq == seq) {
            return if old.payload == payload {
                Admission::Known(old.state.clone())
            } else {
                Admission::Conflict
            };
        }
        if seq <= scope.high_water {
            return Admission::Unknown;
        }
        if scope.high_water.checked_add(1) != Some(seq) {
            return Admission::Gap;
        }
        if scope
            .records
            .iter()
            .filter(|record| !record.state.terminal())
            .count()
            >= MAX_PENDING_SCOPE
            || self.pending_count() >= MAX_PENDING
        {
            return Admission::Busy;
        }
        let scope = self.scopes.get_mut(scope_id).expect("looked up above");
        scope.high_water = seq;
        scope.last_active = now;
        scope.records.push_back(Record {
            seq,
            payload,
            state: OperationState::Accepted,
            terminal_at: None,
        });
        Admission::Accepted
    }

    /// Store terminal outcome before attempting any TCP delivery.
    pub fn complete(
        &mut self,
        scope_id: &str,
        seq: u64,
        state: OperationState,
        now: Duration,
    ) -> Result<(), SessionError> {
        let size = match &state {
            OperationState::Completed(text) | OperationState::Failed(text) => text.len(),
            OperationState::FailedWithResult { code, detail } => code.len() + detail.len(),
            _ => return Err(SessionError::InvalidOutcome),
        };
        if size > OUTCOME_SIZE_LIMIT {
            return Err(SessionError::InvalidOutcome);
        }
        let scope = self
            .scopes
            .get_mut(scope_id)
            .ok_or(SessionError::ScopeUnknown)?;
        let record = scope
            .records
            .iter_mut()
            .find(|record| record.seq == seq)
            .filter(|record| !record.state.terminal())
            .ok_or(SessionError::InvalidOutcome)?;
        record.state = state;
        record.terminal_at = Some(now);
        self.enforce_terminal_bounds();
        Ok(())
    }

    /// Return retained known state or honest unknown, never never-executed proof.
    pub fn status(&self, scope_id: &str, seq: u64) -> Admission {
        let Some(scope) = self.scopes.get(scope_id) else {
            return Admission::ScopeUnknown;
        };
        scope
            .records
            .iter()
            .find(|record| record.seq == seq)
            .map_or(Admission::Unknown, |record| {
                Admission::Known(record.state.clone())
            })
    }

    /// Return exact next scope sequence without resurrecting evicted IDs.
    pub fn next_seq(&self, scope_id: &str) -> Result<u64, SessionError> {
        self.scopes
            .get(scope_id)
            .ok_or(SessionError::ScopeUnknown)?
            .high_water
            .checked_add(1)
            .ok_or(SessionError::CounterExhausted)
    }

    /// Expire old terminal outcomes and detached idle scopes at their exact deadlines.
    pub fn expire(&mut self, now: Duration) {
        for scope in self.scopes.values_mut() {
            scope.records.retain(|record| {
                record
                    .terminal_at
                    .is_none_or(|at| now < at || now - at < TERMINAL_TTL)
            });
        }
        self.scopes.retain(|_, scope| {
            scope.attached.is_some()
                || scope.records.iter().any(|record| !record.state.terminal())
                || now < scope.last_active
                || now - scope.last_active < IDLE_SCOPE_TTL
        });
    }

    /// Bounded status useful for pressure tests and host diagnostics.
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.scopes.len(),
            self.pending_count(),
            self.terminal_count(),
        )
    }

    fn pending_count(&self) -> usize {
        self.scopes
            .values()
            .flat_map(|scope| &scope.records)
            .filter(|record| !record.state.terminal())
            .count()
    }
    fn terminal_count(&self) -> usize {
        self.scopes
            .values()
            .flat_map(|scope| &scope.records)
            .filter(|record| record.state.terminal())
            .count()
    }
    fn enforce_terminal_bounds(&mut self) {
        for scope in self.scopes.values_mut() {
            while scope
                .records
                .iter()
                .filter(|record| record.state.terminal())
                .count()
                > MAX_TERMINAL_SCOPE
            {
                let Some(index) = scope
                    .records
                    .iter()
                    .position(|record| record.state.terminal())
                else {
                    break;
                };
                scope.records.remove(index);
            }
        }
        while self.terminal_count() > MAX_TERMINAL {
            let oldest = self
                .scopes
                .iter()
                .flat_map(|(id, scope)| {
                    scope.records.iter().filter_map(move |record| {
                        record.terminal_at.map(|at| (at, id.clone(), record.seq))
                    })
                })
                .min_by_key(|(at, _, _)| *at);
            let Some((_, id, seq)) = oldest else { break };
            if let Some(scope) = self.scopes.get_mut(&id)
                && let Some(index) = scope.records.iter().position(|record| record.seq == seq)
            {
                scope.records.remove(index);
            }
        }
    }
}
