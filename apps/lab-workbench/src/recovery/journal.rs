//! Bounded durable recovery journal for exact mutation reconciliation.

use crate::{
    client::types::{HelloState, KnownAdmission, MAX_IN_FLIGHT, MutationIdentity, RecoveryRecord},
    storage::{read_bounded, write_replace},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    env, io,
    path::{Path, PathBuf},
};

/// Only supported journal persistence version.
pub(crate) const JOURNAL_FORMAT_VERSION: u32 = 1;
/// Maximum journal bytes on disk.
pub(crate) const JOURNAL_FILE_BYTES: usize = 64 * 1024;
const MAX_JOURNAL_STRING_BYTES: usize = 512;

/// Validated durable session/recovery state, separate from presentation state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryJournal {
    /// Persistence schema version; currently exactly one.
    pub(crate) format_version: u32,
    /// Runtime boot identity observed for these records.
    pub(crate) boot_id: String,
    /// Retained Application scope for these records.
    pub(crate) scope: String,
    /// Last authoritative hello sequence observation, encoded as decimal text.
    pub(crate) next_seq: String,
    /// At most eight exact mutation payload records.
    pub(crate) records: Vec<JournalRecord>,
}

/// Exact payload and last known admission state for one mutation identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalRecord {
    /// Request sequence encoded as accepted decimal text.
    pub(crate) seq: String,
    /// Exact Application operation name.
    pub(crate) op: String,
    /// Exact normalized args retained for worker-owned retry.
    pub(crate) args: Value,
    /// Last durable admission observation.
    pub(crate) admission: JournalAdmission,
}

/// Durable form of M14.2 admission knowledge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalAdmission {
    /// Recorded before wire emission; outcome may still require reconciliation.
    Pending,
    /// Runtime authoritatively reported admission.
    Accepted,
    /// Transport loss made the outcome ambiguous.
    Ambiguous,
    /// Runtime authoritatively reported completion.
    Completed,
    /// Runtime authoritatively reported terminal failure.
    Failed,
}

impl RecoveryJournal {
    /// Builds and validates the durable form of worker-owned records.
    pub(crate) fn from_records(
        boot_id: String,
        scope: String,
        next_seq: u64,
        records: &[RecoveryRecord],
    ) -> Result<Self, JournalError> {
        let candidate = Self {
            format_version: JOURNAL_FORMAT_VERSION,
            boot_id,
            scope,
            next_seq: next_seq.to_string(),
            records: records.iter().map(JournalRecord::from).collect(),
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Validates all bounds and identities without executing any mutation.
    pub(crate) fn validate(&self) -> Result<(), JournalError> {
        if self.format_version != JOURNAL_FORMAT_VERSION {
            return Err(JournalError::UnsupportedVersion(self.format_version));
        }
        validate_string(&self.boot_id)?;
        validate_string(&self.scope)?;
        parse_decimal(&self.next_seq)?;
        if self.records.len() > MAX_IN_FLIGHT {
            return Err(JournalError::Limit("records"));
        }
        let mut sequences = BTreeSet::new();
        for record in &self.records {
            validate_string(&record.op)?;
            let seq = parse_decimal(&record.seq)?;
            if seq == 0 {
                return Err(JournalError::InvalidSequence);
            }
            if !sequences.insert(seq) {
                return Err(JournalError::DuplicateSequence(seq));
            }
            let args_bytes = serde_json::to_vec(&record.args)
                .map_err(|error| JournalError::Json(error.to_string()))?;
            if args_bytes.len() > crate::client::types::APPLICATION_JSON_LIMIT {
                return Err(JournalError::Limit("record_args"));
            }
        }
        Ok(())
    }

    /// Classifies a loaded journal after authoritative hello; never auto-sends it.
    pub(crate) fn reconcile(&self, hello: &HelloState) -> JournalDisposition {
        if self.boot_id != hello.boot_id {
            JournalDisposition::DifferentBoot
        } else if self.scope != hello.scope {
            JournalDisposition::DifferentScope
        } else {
            JournalDisposition::NeedsAuthoritativeStatus(self.to_recovery_records())
        }
    }

    /// Converts validated records to the worker-owned in-memory shape.
    pub(crate) fn to_recovery_records(&self) -> Vec<RecoveryRecord> {
        self.records
            .iter()
            .filter_map(|record| {
                let seq = parse_decimal(&record.seq).ok()?;
                Some(RecoveryRecord {
                    boot_id: self.boot_id.clone(),
                    identity: MutationIdentity {
                        scope: self.scope.clone(),
                        seq,
                    },
                    op: record.op.clone(),
                    args: record.args.clone(),
                    admission: record.admission.into(),
                })
            })
            .collect()
    }
}

impl From<&RecoveryRecord> for JournalRecord {
    fn from(record: &RecoveryRecord) -> Self {
        Self {
            seq: record.identity.seq.to_string(),
            op: record.op.clone(),
            args: record.args.clone(),
            admission: record.admission.into(),
        }
    }
}

impl From<KnownAdmission> for JournalAdmission {
    fn from(admission: KnownAdmission) -> Self {
        match admission {
            KnownAdmission::Pending => Self::Pending,
            KnownAdmission::Accepted => Self::Accepted,
            KnownAdmission::Ambiguous => Self::Ambiguous,
            KnownAdmission::Completed => Self::Completed,
            KnownAdmission::Failed => Self::Failed,
        }
    }
}

impl From<JournalAdmission> for KnownAdmission {
    fn from(admission: JournalAdmission) -> Self {
        match admission {
            JournalAdmission::Pending => Self::Pending,
            JournalAdmission::Accepted => Self::Accepted,
            JournalAdmission::Ambiguous => Self::Ambiguous,
            JournalAdmission::Completed => Self::Completed,
            JournalAdmission::Failed => Self::Failed,
        }
    }
}

/// Loaded journal disposition after authoritative hello.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JournalDisposition {
    /// Boot changed; records are uncertainty evidence, not retry authority.
    DifferentBoot,
    /// Scope could not be retained/reattached; records are not retry authority.
    DifferentScope,
    /// Exact records must be reconciled through `operation_status` before retry.
    NeedsAuthoritativeStatus(Vec<RecoveryRecord>),
}

/// Returns the Workbench-owned per-user recovery journal path.
pub(crate) fn default_journal_path() -> io::Result<PathBuf> {
    let root = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Windows user-data directory unavailable",
            )
        })?;
    Ok(PathBuf::from(root)
        .join("lab-runtime")
        .join("workbench")
        .join("recovery-v1.json"))
}

/// Loads and validates a journal candidate without executing any mutation.
pub(crate) fn load_journal(path: &Path) -> Result<RecoveryJournal, JournalError> {
    let bytes = read_bounded(path, JOURNAL_FILE_BYTES).map_err(JournalError::Io)?;
    if bytes.is_empty() {
        return Err(JournalError::Json("empty recovery journal".into()));
    }
    let journal: RecoveryJournal =
        serde_json::from_slice(&bytes).map_err(|error| JournalError::Json(error.to_string()))?;
    journal.validate()?;
    Ok(journal)
}

/// Validates and synchronously replaces the bounded durable journal.
pub(crate) fn save_journal(path: &Path, journal: &RecoveryJournal) -> Result<(), JournalError> {
    journal.validate()?;
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| JournalError::Json(error.to_string()))?;
    if bytes.len() > JOURNAL_FILE_BYTES {
        return Err(JournalError::Limit("file_bytes"));
    }
    write_replace(path, &bytes).map_err(JournalError::Io)
}

/// Removes a journal only after no exact records remain.
pub(crate) fn retire_journal(path: &Path) -> Result<(), JournalError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(JournalError::Io(error)),
    }
}

/// Journal validation or I/O failure. Callers must surface uncertainty and not send.
#[derive(Debug)]
pub(crate) enum JournalError {
    /// Filesystem or bounded-read/write failure.
    Io(io::Error),
    /// JSON/UTF-8/root/schema decoding failure.
    Json(String),
    /// Unsupported or missing format version.
    UnsupportedVersion(u32),
    /// Frozen bound exceeded.
    Limit(&'static str),
    /// Empty, oversized, or otherwise invalid identity/operation string.
    InvalidString,
    /// Sequence text was invalid or zero where prohibited.
    InvalidSequence,
    /// Two records claimed one request sequence.
    DuplicateSequence(u64),
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "recovery journal I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "recovery journal JSON failed: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported recovery journal version {version}")
            }
            Self::Limit(name) => write!(formatter, "recovery journal limit exceeded: {name}"),
            Self::InvalidString => formatter.write_str("invalid recovery journal string"),
            Self::InvalidSequence => formatter.write_str("invalid recovery journal sequence"),
            Self::DuplicateSequence(seq) => {
                write!(formatter, "duplicate recovery journal sequence {seq}")
            }
        }
    }
}

impl std::error::Error for JournalError {}

fn validate_string(value: &str) -> Result<(), JournalError> {
    if value.is_empty() || value.len() > MAX_JOURNAL_STRING_BYTES {
        return Err(JournalError::InvalidString);
    }
    Ok(())
}

fn parse_decimal(value: &str) -> Result<u64, JournalError> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(JournalError::InvalidSequence);
    }
    value.parse().map_err(|_| JournalError::InvalidSequence)
}
