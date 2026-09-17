//! Stable semantic Recorder projections for the Application boundary.
//!
//! These DTOs deliberately describe recording and durable coverage, not the
//! SQLite worker, journal, connection, or schema used to implement the archive.

use crate::recorder::{RecordingPolicy, RecordingState, RecordingStatus};
use serde_json::{Value, json};

/// Build the complete bounded current Recorder projection.
pub(crate) fn status_json(
    status: Option<&RecordingStatus>,
    archive_id: Option<&str>,
    policy: Option<RecordingPolicy>,
    boot_id: &str,
) -> Value {
    let Some(status) = status else {
        return json!({
            "configured":false,"available":false,"state":"unconfigured",
            "status":"unavailable","accepting_facts":false,"policy":Value::Null,
            "archive":Value::Null,"active_run":Value::Null,"active_interval":Value::Null,
            "durability":Value::Null,"provenance":Value::Null,
            "failure":{"code":"recorder_not_configured","persisted":false}
        });
    };
    let failed = status.state == RecordingState::Failed;
    let closed = status.state == RecordingState::Closed;
    let policy = match policy {
        Some(RecordingPolicy::Required) => Some("required"),
        Some(RecordingPolicy::BestEffort) => Some("best_effort"),
        None => None,
    };
    json!({
        "configured":true,"available":!failed && !closed,"state":state_name(status.state),
        "status":if failed {"failed"} else if closed {"closed"} else {"available"},
        "accepting_facts":status.state==RecordingState::Recording,"policy":policy,
        "archive":{"id":archive_id,"boot_id":boot_id,"closed":status.worker_closed,
            "terminal_boot_seal_committed":status.terminal_seal_committed},
        "database_id":archive_id,"boot_id":boot_id,
        "run_id":status.run_no.map(|run_no|json!({
            "boot_id":boot_id,"run_no":run_no.to_string()})),
        "interval_id":status.interval_no.map(|interval_no|json!({
            "boot_id":boot_id,"interval_no":interval_no.to_string()})),
        "active_run":status.run_no.map(|run_no|json!({
            "boot_id":boot_id,"run_no":run_no.to_string()})),
        "active_interval":status.interval_no.map(|interval_no|json!({
            "boot_id":boot_id,"interval_no":interval_no.to_string(),
            "run_no":status.run_no.map(|run_no|run_no.to_string())})),
        "durability":{"persisted_through":status.persisted_through_sequence.to_string(),
            "coverage":status.coverage,"failure_sealed":status.failure_persisted,
            "pending":{"records":status.outstanding_records,"bytes":status.outstanding_bytes,
                "groups":status.outstanding_groups}},
        "outstanding_records":status.outstanding_records,
        "outstanding_bytes":status.outstanding_bytes,
        "outstanding_groups":status.outstanding_groups,
        "persisted_through_seq":status.persisted_through_sequence.to_string(),
        "provenance":{"available":status.activation_root.is_some(),
            "generation":status.activation_generation.to_string(),
            "identity":status.activation_root.map(|root|hex(&root))},
        "failure":failed.then(||json!({"code":"recording_failed",
            "persisted":status.failure_persisted,"coverage":status.coverage}))
    })
}

/// Whether two owner snapshots differ in public lifecycle semantics.
pub(crate) fn lifecycle_changed(before: Option<&RecordingStatus>, after: &RecordingStatus) -> bool {
    before.is_none_or(|before| {
        before.state != after.state
            || before.run_no != after.run_no
            || before.interval_no != after.interval_no
            || before.coverage != after.coverage
            || before.failure_persisted != after.failure_persisted
            || before.terminal_seal_committed != after.terminal_seal_committed
            || before.worker_closed != after.worker_closed
            || before.activation_generation != after.activation_generation
    })
}

fn state_name(state: RecordingState) -> &'static str {
    match state {
        RecordingState::Idle => "idle",
        RecordingState::Starting => "starting",
        RecordingState::Recording => "recording",
        RecordingState::Stopping => "stopping",
        RecordingState::Failed => "failed",
        RecordingState::Closed => "closed",
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
