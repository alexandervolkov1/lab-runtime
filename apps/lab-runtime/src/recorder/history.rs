//! Semantic durable-history request and result values.
//!
//! These types describe bounded archive queries and pages. They contain no SQL,
//! connection, worker, or Application wire behavior.

use lab_core::{InstrumentId, ParameterId, SignalId, Value};
use std::time::Duration;

/// Cooperative VM/time limit for one indexed history selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryBudget {
    pub(super) vm_ops: u32,
    pub(super) elapsed: Duration,
    pub(super) stride: i32,
}
impl Default for HistoryBudget {
    fn default() -> Self {
        Self {
            vm_ops: 100_000,
            elapsed: Duration::from_millis(50),
            stride: 1_000,
        }
    }
}
impl HistoryBudget {
    /// Smaller trusted fault-profile limit; no public client can choose budgets.
    pub fn for_testing(vm_ops: u32, elapsed: Duration) -> Self {
        Self {
            vm_ops: vm_ops.clamp(1, 100_000),
            elapsed: elapsed
                .min(Duration::from_millis(50))
                .max(Duration::from_micros(1)),
            stride: vm_ops.clamp(1, 1_000) as i32,
        }
    }
}

/// One raw committed measurement; publication and source observation times differ.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementRow {
    /// Immutable canonical record identity within its boot.
    pub record_sequence: u64,
    /// Monotonic publication time scoped to the archived boot.
    pub published_at: Duration,
    /// Monotonic source/model observation time.
    pub observed_at: Duration,
    /// Exact engineering unit symbol captured at commit.
    pub unit: String,
    /// `good` or `unavailable`; no interpolation is performed.
    pub quality: String,
    /// Exact typed value, absent for a failed attempt.
    pub value: Option<Value>,
    /// Failure reason when unavailable.
    pub failure: Option<String>,
    /// Source generation at commit.
    pub generation: u64,
    /// Source revision at commit.
    pub revision: u64,
    /// Managed state revision, separate from the immutable definition revision.
    pub state_revision: Option<u64>,
    /// Captured transform input; absent for a native or managed source reading.
    pub lineage: Option<MeasurementLineage>,
}

/// Immutable upstream identity/time frozen before an asynchronous transform ran.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementLineage {
    /// Input signal selected at invocation admission.
    pub signal: SignalId,
    /// Exact finite input scalar passed to the worker.
    pub value: f64,
    /// Stable engineering-unit identity.
    pub unit: String,
    /// Upstream publication time in the same archived boot.
    pub published_at: Duration,
    /// Original upstream observation time.
    pub observed_at: Duration,
    /// Replacement generation at input capture.
    pub source_generation: u64,
    /// Upstream configuration/definition or mapping revision.
    pub source_revision: u64,
    /// Upstream managed state revision when applicable.
    pub source_state_revision: Option<u64>,
}

/// Exact indexed archive filter; elapsed times are scoped to the selected boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryFilter {
    /// Archived process boot, which may differ from the serving boot.
    pub boot_id: String,
    /// Checked run counter within that archived boot.
    pub run_no: u64,
    /// Stable instrument identity.
    pub instrument: InstrumentId,
    /// Stable parameter identity within the instrument.
    pub parameter: ParameterId,
    /// Inclusive monotonic publication bound.
    pub from: Duration,
    /// Exclusive monotonic publication bound.
    pub to: Duration,
}

/// One server-retained keyset continuation at a frozen durable checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    pub(super) filter: HistoryFilter,
    pub(super) database_id: String,
    pub(super) watermark: u64,
    pub(super) last: (u64, u64),
    pub(super) upper: (u64, u64),
}
impl HistoryCursor {
    /// Original archived signal/range bound to this process-local continuation.
    pub fn filter(&self) -> &HistoryFilter {
        &self.filter
    }
}

/// At most 128 complete raw rows and one continuation token.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryPage {
    /// Matching committed rows, with Unavailable preserved as a raw row.
    pub rows: Vec<MeasurementRow>,
    /// Rows decoded by this indexed selection, including at most one lookahead.
    /// This worker-side diagnostic is bounded by `rows.len() + 1`.
    pub selected_rows: usize,
    /// Durable record checkpoint frozen on the first page.
    pub watermark: u64,
    /// Continuation only if more matching rows existed at that checkpoint.
    pub next_cursor: Option<HistoryCursor>,
    /// `complete`, `gap`, or `unknown_tail` interval/run coverage.
    pub coverage: String,
    /// Known loss boundary, if this run committed a failure seal.
    pub loss: Option<HistoryLoss>,
}

/// Bounded, committed gap metadata; absent fields mean genuinely unknown loss.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryLoss {
    /// Stable bounded reason captured at the first coverage failure.
    pub reason: String,
    /// First missing source-fact identity if transfer rejection was observed.
    pub first_missing_fact: Option<u64>,
    /// Known number of missing source facts, or None for an unknown tail.
    pub known_missing_count: Option<u64>,
    /// Last source fact accepted before this failure, if known.
    pub last_accepted_fact: Option<u64>,
    /// Trusted monotonic failure cutoff in the archived boot.
    pub cutoff: Duration,
    /// Last record confirmed committed before the gap seal.
    pub last_confirmed_record: u64,
}

/// One archived run discoverable after a fresh process boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// Archive boot containing this run.
    pub boot_id: String,
    /// Checked run counter scoped to that boot.
    pub run_no: u64,
    /// Human-readable label, never an identity.
    pub label: String,
    /// Selected trusted recording policy.
    pub policy: String,
    /// `recording`, `sealed`, `interrupted`, or failed lifecycle state.
    pub state: String,
    /// `complete`, `gap`, or `unknown_tail` coverage.
    pub coverage: String,
}

/// One retained run-list keyset continuation, scoped to this database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunsCursor {
    pub(super) database_id: String,
    pub(super) last: ([u8; 16], u64),
    pub(super) upper: ([u8; 16], u64),
}

/// At most 32 indexed run summaries and one continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunsPage {
    /// Archived runs in immutable boot/run key order.
    pub runs: Vec<RunSummary>,
    /// Continuation only if another run existed below the frozen upper key.
    pub next_cursor: Option<RunsCursor>,
}
