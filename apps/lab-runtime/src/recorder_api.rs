//! Stable semantic Recorder operations and projections for the Application boundary.
//!
//! This module validates Recorder-facing DTOs and projects status/history pages.
//! Accepted-operation bookkeeping and connection-local delivery remain with
//! `Application`; the SQLite worker, journal, connection, and schema remain behind
//! the Recorder contract.

use crate::{
    application::{common::id_field, projections::nanos},
    recorder::{
        HistoryPage, RecordingPolicy, RecordingState, RecordingStatus, RunsPage,
        bounded_annotation_data,
    },
    sessions::Mutation,
};
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

/// Decode one Recorder/history mutation after registry shape validation.
pub(crate) fn decode_mutation(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match operation {
        "recording_start" => {
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if label.trim().is_empty() || label.len() > 128 {
                return Err("invalid_args");
            }
            Mutation::RecordingStart {
                label: label.to_owned(),
            }
        }
        "recording_stop" => {
            let run = args.get("run_id").ok_or("invalid_args")?;
            let boot_id = run
                .get("boot_id")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if !lower_hex_id(boot_id) {
                return Err("invalid_args");
            }
            Mutation::RecordingStop {
                boot_id: boot_id.to_owned(),
                run_no: id_field(run, "run_no")?,
            }
        }
        "experiment_annotate" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            let data = args.get("data").ok_or("invalid_args")?;
            if name.trim().is_empty() || name.len() > 64 || !bounded_annotation_data(data) {
                return Err("invalid_args");
            }
            Mutation::ExperimentAnnotate {
                name: name.to_owned(),
                data_json: data.to_string(),
            }
        }
        "history_read" => decode_history(args)?,
        _ => return Err("unsupported_operation"),
    })
}

fn decode_history(args: &Value) -> Result<Mutation, &'static str> {
    if args.get("mode").and_then(Value::as_str) == Some("runs") {
        let database_id = args
            .get("database_id")
            .and_then(Value::as_str)
            .ok_or("invalid_args")?;
        if !lower_hex_id(database_id) {
            return Err("invalid_args");
        }
        let max = args
            .get("max_records")
            .and_then(Value::as_u64)
            .ok_or("invalid_args")?;
        if !(1..=32).contains(&max) {
            return Err("invalid_args");
        }
        let cursor = decode_cursor(args)?;
        Ok(Mutation::HistoryReadRuns {
            database_id: database_id.to_owned(),
            max_records: max as u8,
            cursor,
        })
    } else if args.get("mode").and_then(Value::as_str) == Some("measurements") {
        let database_id = args
            .get("database_id")
            .and_then(Value::as_str)
            .ok_or("invalid_args")?;
        let boot_id = args
            .get("boot_id")
            .and_then(Value::as_str)
            .ok_or("invalid_args")?;
        if !lower_hex_id(database_id) || !lower_hex_id(boot_id) {
            return Err("invalid_args");
        }
        let run = args.get("run_id").ok_or("invalid_args")?;
        if run.get("boot_id").and_then(Value::as_str) != Some(boot_id) {
            return Err("invalid_args");
        }
        let signal = args.get("signal").ok_or("invalid_args")?;
        let from_ns = id_field(args, "from_ns")?;
        let to_ns = id_field(args, "to_ns")?;
        if from_ns >= to_ns {
            return Err("invalid_args");
        }
        let max = args
            .get("max_records")
            .and_then(Value::as_u64)
            .ok_or("invalid_args")?;
        if !(1..=128).contains(&max) {
            return Err("invalid_args");
        }
        Ok(Mutation::HistoryReadMeasurements {
            database_id: database_id.to_owned(),
            boot_id: boot_id.to_owned(),
            run_no: id_field(run, "run_no")?,
            instrument: id_field(signal, "instrument")?,
            parameter: id_field(signal, "parameter")?,
            from_ns,
            to_ns,
            max_records: max as u16,
            cursor: decode_cursor(args)?,
        })
    } else {
        Err("unsupported_history_mode")
    }
}

fn decode_cursor(args: &Value) -> Result<Option<String>, &'static str> {
    match args.get("cursor") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(token)) if !token.is_empty() && token.len() <= 128 => {
            Ok(Some(token.clone()))
        }
        _ => Err("invalid_args"),
    }
}

fn lower_hex_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Project one immutable durable measurement-history page.
pub(crate) fn history_page_json(page: &HistoryPage, cursor_token: Option<&str>) -> Value {
    let rows: Vec<Value> = page
        .rows
        .iter()
        .map(|row| {
            let value = match &row.value {
                Some(lab_core::Value::Float(value)) => json!({"kind":"float","value":value}),
                Some(lab_core::Value::Integer(value)) => {
                    json!({"kind":"integer","value":value.to_string()})
                }
                Some(lab_core::Value::Boolean(value)) => json!({"kind":"boolean","value":value}),
                Some(lab_core::Value::Text(value)) => json!({"kind":"text","value":value}),
                Some(lab_core::Value::Enum(value)) => json!({"kind":"enum","value":value}),
                None => Value::Null,
            };
            let lineage = row.lineage.as_ref().map(|input| json!({
                "signal":{"instrument":input.signal.instrument().get().to_string(),
                          "parameter":input.signal.parameter().get().to_string()},
                "value":input.value,"unit":input.unit,
                "published_at_ns":nanos(input.published_at),
                "observed_at_ns":nanos(input.observed_at),
                "source_generation":input.source_generation.to_string(),
                "source_revision":input.source_revision.to_string(),
                "source_state_revision":input.source_state_revision.map(|revision|revision.to_string())}));
            json!({"record_seq":row.record_sequence.to_string(),
            "published_at_ns":nanos(row.published_at),
            "observed_at_ns":nanos(row.observed_at),"unit":row.unit,
            "quality":row.quality,"failure":row.failure,"value":value,
            "generation":row.generation.to_string(),"revision":row.revision.to_string(),
            "state_revision":row.state_revision.map(|revision|revision.to_string()),
            "lineage":lineage})
        })
        .collect();
    json!({"rows":rows,"watermark":page.watermark.to_string(),
        "coverage":page.coverage,"next_cursor":cursor_token,
        "loss":page.loss.as_ref().map(|loss| json!({
            "reason":loss.reason,
            "first_missing_fact_seq":loss.first_missing_fact.map(|id|id.to_string()),
            "known_missing_count":loss.known_missing_count.map(|id|id.to_string()),
            "last_accepted_fact_seq":loss.last_accepted_fact.map(|id|id.to_string()),
            "cutoff_monotonic_ns":nanos(loss.cutoff),
            "last_confirmed_record_seq":loss.last_confirmed_record.to_string()})),
        "has_more":cursor_token.is_some(),"raw":true})
}

/// Project one immutable durable run-discovery page.
pub(crate) fn runs_page_json(page: &RunsPage, cursor_token: Option<&str>) -> Value {
    let runs: Vec<Value> = page
        .runs
        .iter()
        .map(|run| {
            json!({
                "run_id":{"boot_id":run.boot_id,"run_no":run.run_no.to_string()},
                "label":run.label,"policy":run.policy,"state":run.state,
                "coverage":run.coverage,
            })
        })
        .collect();
    json!({"runs":runs,"next_cursor":cursor_token,
        "has_more":cursor_token.is_some(),"mode":"runs"})
}
