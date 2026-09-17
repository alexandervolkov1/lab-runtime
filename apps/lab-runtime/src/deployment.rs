//! One bounded staged deployment and its explicit safe apply protocol.
//!
//! The lifecycle owns immutable candidates but delegates authoritative domain
//! mutation to one serialized owner through [`crate::deployment::ApplyPort`]. The port intentionally
//! exposes no rearm operation: a failed safe barrier can retain the old committed
//! configuration, but cannot recreate an old lease or armed epoch.

use crate::configuration::{DeploymentChanges, FrozenDeployment};
use std::{collections::BTreeSet, time::Duration};

const CANDIDATE_LIFETIME: Duration = Duration::from_secs(30);

/// One required effect in a bounded explicit configuration diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiffEffect {
    /// Display/provenance-only state can change without resetting execution.
    LiveSafe,
    /// A schedule cadence changes from commit time without catch-up work.
    OrdinaryLive,
    /// A virtual or managed component must reset and warm again.
    Reinitialize,
    /// A physical resource binding must be replaced and fenced.
    TransportRebind,
    /// Dependent native control must stop and warm again.
    ControllerRewarm,
    /// Current authority must be revoked and safe evidence obtained first.
    OutputSafeBarrier,
    /// M8 live apply is unsupported; process restart is required.
    RestartRequired,
}

/// Bounded aggregate effects computed before any Runtime mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationDiff {
    effects: BTreeSet<DiffEffect>,
}

impl ConfigurationDiff {
    /// Ordered unique effect set suitable for a bounded snapshot.
    pub const fn effects(&self) -> &BTreeSet<DiffEffect> {
        &self.effects
    }

    /// Whether apply must enter the existing Rust-owned safe path.
    pub fn requires_safe_barrier(&self) -> bool {
        self.effects.contains(&DiffEffect::OutputSafeBarrier)
    }

    fn from_changes(changes: DeploymentChanges) -> Self {
        let mut effects = BTreeSet::new();
        for (enabled, effect) in [
            (changes.live_safe, DiffEffect::LiveSafe),
            (changes.ordinary_live, DiffEffect::OrdinaryLive),
            (changes.reinitialize, DiffEffect::Reinitialize),
            (changes.rebind, DiffEffect::TransportRebind),
            (changes.controller_rewarm, DiffEffect::ControllerRewarm),
            (changes.safe_barrier, DiffEffect::OutputSafeBarrier),
            (changes.restart_required, DiffEffect::RestartRequired),
        ] {
            if enabled {
                effects.insert(effect);
            }
        }
        Self { effects }
    }
}

/// Read-only identity and diff returned by staging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedConfiguration {
    id: u64,
    base_revision: u64,
    diff: ConfigurationDiff,
}

impl StagedConfiguration {
    /// Process-local bounded candidate identity.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Active revision against which apply performs optimistic validation.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// Effects that will occur if the candidate is applied.
    pub const fn diff(&self) -> &ConfigurationDiff {
        &self.diff
    }
}

#[derive(Clone)]
struct Candidate {
    snapshot: StagedConfiguration,
    loaded: FrozenDeployment,
    expires_at: Duration,
}

/// Failure to retain a new staged candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageError {
    /// One candidate already occupies the fixed slot.
    Busy,
    /// The process-local candidate counter cannot advance safely.
    CounterExhausted,
    /// Monotonic candidate expiry could not be represented.
    DeadlineOverflow,
}

/// Failure before or during explicit apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// Candidate identity is not the retained staged candidate.
    UnknownCandidate,
    /// Caller or candidate was based on a different committed revision.
    Conflict,
    /// The 30-second staged-candidate residence elapsed.
    Expired,
    /// Diff requires process restart and caused no barrier side effect.
    RestartRequired,
    /// The trusted owner could not perform a bounded preparation/commit step.
    OwnerFailure,
}

/// Honest terminal result after apply entered the owner lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyResult {
    /// Candidate became the coherent active deployment revision.
    Applied {
        /// New nonzero checked active deployment revision.
        revision: u64,
    },
    /// Old configuration remains selected; completed safety side effects remain.
    FailedBeforeCommit,
}

/// Serialized owner seam used by the deployment lifecycle.
pub trait ApplyPort {
    /// Stop affected production, revoke authority and prove current-profile safe evidence.
    fn enter_safe_barrier(&mut self, at: Duration) -> Result<bool, ApplyError>;
    /// Prepare a replacement binding after the safe fence, without committing config.
    fn prepare_bindings(&mut self, candidate: &FrozenDeployment) -> Result<(), ApplyError>;
    /// Atomically install one already validated immutable candidate.
    fn commit_configuration(&mut self, candidate: &FrozenDeployment) -> Result<(), ApplyError>;
}

/// Active immutable deployment plus exactly one optional staged candidate.
pub struct DeploymentLifecycle {
    active: FrozenDeployment,
    revision: u64,
    next_candidate: u64,
    staged: Option<Candidate>,
}

impl DeploymentLifecycle {
    /// Start at deployment revision one with no staged candidate.
    pub const fn new(active: FrozenDeployment) -> Self {
        Self {
            active,
            revision: 1,
            next_candidate: 1,
            staged: None,
        }
    }

    /// Current committed deployment revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Current exact immutable deployment bundle.
    pub const fn active(&self) -> &FrozenDeployment {
        &self.active
    }

    /// Replace only the frozen managed-source bundle without changing TOML
    /// identity or configuration revision.
    pub(crate) fn validate_managed_sources_candidate(
        &self,
        candidate: &FrozenDeployment,
    ) -> Result<(), ApplyError> {
        if candidate.toml_hash() != self.active.toml_hash()
            || candidate.effective() != self.active.effective()
        {
            return Err(ApplyError::Conflict);
        }
        Ok(())
    }

    /// Commit a source bundle whose identity was checked before owner mutation.
    pub(crate) fn commit_validated_managed_sources(&mut self, candidate: FrozenDeployment) {
        debug_assert!(self.validate_managed_sources_candidate(&candidate).is_ok());
        self.active = candidate;
    }

    /// Validate and retain one candidate without touching the Runtime owner.
    pub fn stage(
        &mut self,
        candidate: FrozenDeployment,
        at: Duration,
    ) -> Result<StagedConfiguration, StageError> {
        if self.staged.is_some() {
            return Err(StageError::Busy);
        }
        let id = self.next_candidate;
        self.next_candidate = id.checked_add(1).ok_or(StageError::CounterExhausted)?;
        let expires_at = at
            .checked_add(CANDIDATE_LIFETIME)
            .ok_or(StageError::DeadlineOverflow)?;
        let snapshot = StagedConfiguration {
            id,
            base_revision: self.revision,
            diff: ConfigurationDiff::from_changes(candidate.changes_from(&self.active)),
        };
        self.staged = Some(Candidate {
            snapshot: snapshot.clone(),
            loaded: candidate,
            expires_at,
        });
        Ok(snapshot)
    }

    /// Apply through an explicit barrier/prepare/atomic-commit sequence.
    pub fn apply(
        &mut self,
        candidate_id: u64,
        expected_revision: u64,
        at: Duration,
        owner: &mut impl ApplyPort,
    ) -> Result<ApplyResult, ApplyError> {
        let candidate = self.staged.as_ref().ok_or(ApplyError::UnknownCandidate)?;
        if candidate.snapshot.id != candidate_id {
            return Err(ApplyError::UnknownCandidate);
        }
        if expected_revision != self.revision || candidate.snapshot.base_revision != self.revision {
            return Err(ApplyError::Conflict);
        }
        if at >= candidate.expires_at {
            self.staged = None;
            return Err(ApplyError::Expired);
        }
        if candidate
            .snapshot
            .diff
            .effects
            .contains(&DiffEffect::RestartRequired)
        {
            return Err(ApplyError::RestartRequired);
        }
        let candidate = self.staged.take().expect("candidate checked above");
        if candidate.snapshot.diff.requires_safe_barrier() && !owner.enter_safe_barrier(at)? {
            return Ok(ApplyResult::FailedBeforeCommit);
        }
        if candidate
            .snapshot
            .diff
            .effects
            .contains(&DiffEffect::TransportRebind)
            && owner.prepare_bindings(&candidate.loaded).is_err()
        {
            return Ok(ApplyResult::FailedBeforeCommit);
        }
        owner.commit_configuration(&candidate.loaded)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(ApplyError::OwnerFailure)?;
        self.active = candidate.loaded;
        self.revision = revision;
        Ok(ApplyResult::Applied { revision })
    }
}
