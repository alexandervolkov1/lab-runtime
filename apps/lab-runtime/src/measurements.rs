//! Stable public measurement projections shared by current, window, and events.

use lab_core::{MeasurementFailure, Sample, Value};
use serde_json::{Value as JsonValue, json};

/// Encode one stable signal identity. Instrument and parameter positions are not identities.
pub fn signal_id_json(signal: lab_core::SignalId) -> JsonValue {
    json!({"instrument":signal.instrument().get().to_string(),
        "parameter":signal.parameter().get().to_string()})
}

/// Encode every scalar kind supported by the Core value contract.
pub fn value_json(value: &Value) -> JsonValue {
    match value {
        Value::Float(value) => json!(value),
        Value::Integer(value) => json!(value.to_string()),
        Value::Boolean(value) => json!(value),
        Value::Text(value) | Value::Enum(value) => json!(value),
    }
}

fn failure_name(failure: MeasurementFailure) -> &'static str {
    match failure {
        MeasurementFailure::Disabled => "disabled",
        MeasurementFailure::SensorFault => "sensor_fault",
        MeasurementFailure::Transport => "transport",
        MeasurementFailure::ProcessingWarmup => "processing_warmup",
        MeasurementFailure::ComponentFailure => "component_failure",
    }
}

/// Encode a complete attempt; absence is represented separately by [`current_json`].
pub fn sample_json(sample: &Sample, generation: u64) -> JsonValue {
    let quality = match sample.quality() {
        lab_core::SampleQuality::Good => "good",
        lab_core::SampleQuality::Unavailable => "unavailable",
    };
    json!({
        "signal":signal_id_json(sample.signal()),
        "value":sample.value().map(value_json),
        "unit":{"id":sample.unit().id(),"symbol":sample.unit().symbol()},
        "quality":quality,
        "status":if sample.value().is_some(){"available"}else{"unavailable"},
        "failure":sample.failure().map(failure_name),
        "observed_at_ns":sample.at().as_nanos().to_string(),
        "observed_at":sample.at().as_nanos().to_string(),
        "source_at_ns":sample.freshness_at().as_nanos().to_string(),
        "generation":generation.to_string()
    })
}

/// Encode authoritative current state without using missing fields as status.
pub fn current_json(
    signal: lab_core::SignalId,
    unit: lab_core::Unit,
    sample: Option<&Sample>,
    generation: u64,
) -> JsonValue {
    sample.map_or_else(
        || {
            json!({
                "signal":signal_id_json(signal),"value":JsonValue::Null,
                "unit":{"id":unit.id(),"symbol":unit.symbol()},
                "quality":"unavailable","status":"not_observed","failure":"not_observed",
            "observed_at_ns":JsonValue::Null,"source_at_ns":JsonValue::Null,
            "observed_at":JsonValue::Null,
                "generation":generation.to_string()
            })
        },
        |sample| sample_json(sample, generation),
    )
}
