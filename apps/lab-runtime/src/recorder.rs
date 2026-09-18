//! Durable Recorder contract and SQLite version-one storage outside `lab-core`.
//!
//! # Recorder index
//!
//! - Semantic experiment facts originate in [`lab_core::recording`] at the
//!   authoritative Runtime transition.
//! - [`crate::recorder::RecorderWorker`] owns bounded nonblocking ingress, lifecycle receipts, history
//!   jobs, gap accounting and finite finish coordination.
//! - [`crate::recorder::SqliteStore`] is the synchronous worker-owned implementation for schema,
//!   writes, history, provenance, sealing and completeness.
//! - [`crate::recorder::TimeAnchor`] projects monotonic Runtime time to wall-clock recording fields;
//!   wall-clock changes never drive control.
//!
//! Only the storage worker or an offline test may own `SqliteStore`. The Runtime
//! owner hands facts to bounded ingress and never calls SQLite on its safety lane.
//! Transient Application events and durable Recorder history are separate.
//!
//! `Application API != Recorder contract != SQLite schema`.

use std::time::Duration;

mod history;
mod provenance;
mod sqlite;
mod time;
mod worker;
pub use history::{
    HistoryBudget, HistoryCursor, HistoryFilter, HistoryLoss, HistoryPage, MeasurementLineage,
    MeasurementRow, RunSummary, RunsCursor, RunsPage,
};
pub use provenance::{ProvenanceEntry, ProvenanceObject};
pub use sqlite::{SqliteSettings, SqliteStore, StorageError, StorageHealth};
pub use time::TimeAnchor;
pub use worker::{RecorderLimits, RecorderWorker, RecordingState, RecordingStatus, WriterBarrier};

/// Recording availability policy selected at trusted host startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingPolicy {
    /// Preserve experiment progress while retaining a visible coverage failure.
    BestEffort,
    /// Recorder failure closes ordinary control and invokes Rust safe/fault work.
    Required,
}

/// Owner-frozen loss boundary; an absent count is an honest unknown count.
#[derive(Clone, Debug)]
pub struct RecorderGap {
    /// First stable failure code/context, capped before entering the seal slot.
    pub reason: String,
    /// Runtime monotonic cutoff of this coverage interval.
    pub at: Duration,
    /// First source fact identity not transferred to durable ingress, if known.
    pub first_missing_fact: Option<u64>,
    /// Known count of rejected source facts at this cutoff, if known.
    pub known_missing_count: Option<u64>,
    /// Last source fact identity accepted into ingress before the gap.
    pub last_accepted_fact: Option<u64>,
}

/// Immutable owner-captured run boundary. Existing samples inside it are
/// snapshots, never newly observed measurement rows or physical evidence.
#[derive(Clone, Debug)]
pub struct BoundarySnapshot {
    /// Actual owner monotonic time of capture.
    pub at: Duration,
    /// Canonical bounded JSON of config revisions, latest attempts and pending IDs.
    pub data: String,
}
impl BoundarySnapshot {
    /// Empty trusted boundary for offline adapter tests with no Runtime owner.
    pub fn empty(at: Duration) -> Self {
        Self {
            at,
            data: serde_json::json!({"latest_samples":[],
            "controller_revisions":[],"reference_revisions":[],
            "managed_revisions":[],"pending_operations":[]})
            .to_string(),
        }
    }
    fn valid(&self) -> bool {
        !self.data.is_empty()
            && self.data.len() <= 64 * 1024
            && serde_json::from_str::<serde_json::Value>(&self.data).is_ok()
    }
}
impl RecorderGap {
    fn valid(&self) -> bool {
        !self.reason.is_empty()
            && self.reason.len() <= 512
            && self.known_missing_count.is_none_or(|count| count > 0)
    }
}

/// Immutable application-boundary fact; it is a Recorder domain payload, not a
/// wire frame or an M6 event-ring entry. Its original request identity survives
/// client disconnect and its phase never predicts a future physical outcome.
#[derive(Clone, Debug)]
pub struct OperationRecord {
    /// Original logical client scope, independent of the connection lifetime.
    pub scope: String,
    /// Original checked request sequence.
    pub request_seq: u64,
    /// Stable application command name.
    pub command: &'static str,
    /// `accepted`, `completed`, or `failed` at the true application boundary.
    pub phase: &'static str,
    /// Canonical bounded JSON of typed intent or typed terminal result.
    pub data: String,
    /// Truth source of this fact, never caller-supplied evidence.
    pub outcome_basis: &'static str,
    /// Owner monotonic publication time within this boot.
    pub at: Duration,
}

/// One bounded configuration-activation fact committed with its immutable baseline.
#[derive(Clone, Debug)]
pub struct ConfigurationLifecycleRecord {
    /// Process-local staged candidate or explicit lifecycle operation identity.
    pub operation_id: u64,
    /// Stable Runtime operation name, never a caller-defined free-form tag.
    pub operation_kind: &'static str,
    /// Active deployment revision against which the candidate was staged.
    pub base_revision: u64,
    /// Deployment revision that became authoritative at the owner commit.
    pub committed_revision: u64,
    /// Hash of the exact loaded `runtime.toml` bytes retained in provenance.
    pub toml_hash: [u8; 32],
    /// Bounded logical identities with their committed generation/revision facts.
    pub affected: Vec<String>,
    /// Optional bounded reason for a non-routine activation classification.
    pub reason: Option<String>,
    /// Runtime monotonic owner commit time.
    pub at: Duration,
}
impl ConfigurationLifecycleRecord {
    fn charge(&self) -> Option<usize> {
        self.affected
            .iter()
            .try_fold(1024usize, |total, item| {
                total.checked_add(item.capacity().checked_add(64)?)
            })?
            .checked_add(self.reason.as_ref().map_or(0, String::capacity))
    }

    fn valid(&self) -> bool {
        // The first two names in this compatibility arm are no longer emitted by
        // the active API. They remain accepted so pre-M9C lifecycle facts can be
        // validated when an existing historical archive is inspected or reopened.
        let revision_valid = match self.operation_kind {
            "reload_configuration" | "apply_configuration" => {
                self.committed_revision == self.base_revision.checked_add(1).unwrap_or(0)
            }
            "reload_managed_sources"
            | "restart_models"
            | "virtual_models_restart"
            | "emulator_publish"
            | "reconnect_resource" => self.committed_revision == self.base_revision,
            _ => false,
        };
        self.operation_id > 0
            && self.base_revision > 0
            && revision_valid
            && self.affected.len() <= 256
            && self
                .affected
                .iter()
                .all(|item| !item.is_empty() && item.len() <= 128)
            && self
                .reason
                .as_ref()
                .is_none_or(|reason| reason.len() <= 512)
            && self.charge().is_some_and(|charge| charge <= 64 * 1024)
    }
}
impl OperationRecord {
    pub(crate) fn charge(&self) -> Option<usize> {
        self.scope
            .capacity()
            .checked_add(self.data.capacity())?
            .checked_add(256)
    }
    fn valid(&self) -> bool {
        self.scope.len() <= 64
            && self.request_seq > 0
            && self.data.len() <= 16 * 1024
            && matches!(self.phase, "accepted" | "completed" | "failed")
            && matches!(
                self.outcome_basis,
                "application_admission" | "domain_result"
            )
    }
}

/// Validated informational client record; Runtime assigns origin and identity.
#[derive(Clone, Debug)]
pub struct AnnotationRecord {
    /// Original logical actor scope, independent of connection lifetime.
    pub scope: String,
    /// Original checked logical request sequence.
    pub request_seq: u64,
    /// Validated annotation name, at most 64 UTF-8 bytes.
    pub name: String,
    /// Validated canonical JSON data; it cannot supply physical evidence.
    pub data_json: String,
    /// Runtime-owner monotonic admission time in this boot.
    pub at: Duration,
}

impl AnnotationRecord {
    fn valid(&self) -> bool {
        self.scope.len() <= 64
            && self.request_seq > 0
            && !self.name.trim().is_empty()
            && self.name.len() <= 64
            && self.data_json.len() <= 2 * 1024
            && serde_json::from_str(&self.data_json)
                .ok()
                .is_some_and(|data| bounded_annotation_data(&data))
    }

    fn charge(&self) -> Option<usize> {
        self.scope
            .capacity()
            .checked_add(self.name.capacity())?
            .checked_add(self.data_json.capacity())?
            .checked_add(self.data_json.len().checked_mul(3)?)?
            .checked_add(512)
    }
}

/// Check all nested annotation bounds before a logical operation is admitted.
/// Sixfold string charges cover JSON control-character escaping.
pub fn bounded_annotation_data(data: &serde_json::Value) -> bool {
    fn visit(
        value: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> Option<()> {
        if depth > 4 {
            return None;
        }
        *nodes = nodes.checked_add(1)?;
        if *nodes > 64 {
            return None;
        }
        match value {
            serde_json::Value::Null => *bytes = bytes.checked_add(4)?,
            serde_json::Value::Bool(_) => *bytes = bytes.checked_add(5)?,
            serde_json::Value::Number(number) => {
                *bytes = bytes.checked_add(number.to_string().len())?;
            }
            serde_json::Value::String(string) => {
                if string.len() > 512 {
                    return None;
                }
                *bytes = bytes.checked_add(string.len().checked_mul(6)?.checked_add(2)?)?;
            }
            serde_json::Value::Array(values) => {
                *bytes = bytes.checked_add(values.len().checked_add(2)?)?;
                for child in values {
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            serde_json::Value::Object(members) => {
                *bytes = bytes.checked_add(members.len().checked_add(2)?)?;
                for (key, child) in members {
                    if key.len() > 512 {
                        return None;
                    }
                    *nodes = nodes.checked_add(1)?;
                    if *nodes > 64 {
                        return None;
                    }
                    *bytes = bytes.checked_add(key.len().checked_mul(6)?.checked_add(3)?)?;
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
        }
        (*bytes <= 2 * 1024).then_some(())
    }
    let mut nodes = 0;
    let mut bytes = 0;
    visit(data, 1, &mut nodes, &mut bytes).is_some()
}
