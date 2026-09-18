//! Measurement read-side operations and their stable public projections.
//!
//! `latest`, `measurements_current`, and `measurement_window` query committed
//! Runtime signal buffers. They do not poll instruments and do not own history;
//! durable Recorder history remains a separate API path.

use crate::{
    application::common::{domain_code, id_field},
    service::ServiceHost,
};

use lab_core::{
    InstrumentId, MeasurementFailure, ParameterId, Query, QueryResult, Sample, SignalId, Value,
};
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

/// Project one authoritative current signal selected by stable identity.
pub(crate) fn latest(service: &ServiceHost, args: &JsonValue) -> Result<JsonValue, &'static str> {
    let signal = args.get("signal").ok_or("invalid_args")?;
    let instrument = id_field(signal, "instrument")?;
    let parameter = id_field(signal, "parameter")?;
    let owner = service.owner();
    match owner
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(instrument),
            ParameterId::new(parameter),
        )))
        .map_err(domain_code)?
    {
        QueryResult::Latest(sample) => {
            let QueryResult::Descriptor(descriptor) = owner
                .query(Query::DescribeInstrument(InstrumentId::new(instrument)))
                .map_err(domain_code)?
            else {
                return Err("internal_error");
            };
            let descriptor = descriptor
                .parameter(ParameterId::new(parameter))
                .ok_or("unknown_parameter")?;
            Ok(current_json(
                descriptor.signal.ok_or("unknown_signal")?,
                descriptor.unit,
                sample.as_ref(),
                owner.signal_generation(SignalId::new(
                    InstrumentId::new(instrument),
                    ParameterId::new(parameter),
                )),
            ))
        }
        _ => Err("internal_error"),
    }
}

/// Build deterministic current-measurement records for one frozen projection.
pub(crate) fn current_records(service: &ServiceHost) -> Result<Vec<JsonValue>, &'static str> {
    let owner = service.owner();
    let QueryResult::Instruments(instruments) =
        owner.query(Query::Discover).map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let mut records = Vec::new();
    for descriptor in instruments {
        for parameter in descriptor
            .parameters
            .iter()
            .filter(|parameter| parameter.signal.is_some())
        {
            let signal = parameter.signal.expect("filtered above");
            let QueryResult::Latest(latest) = owner
                .query(Query::GetLatestSignal(signal))
                .map_err(domain_code)?
            else {
                return Err("internal_error");
            };
            records.push(current_json(
                signal,
                parameter.unit,
                latest.as_ref(),
                owner.signal_generation(signal),
            ));
        }
    }
    Ok(records)
}

/// Project bounded recent in-memory signal history, never durable Recorder history.
pub(crate) fn window(service: &ServiceHost, args: &JsonValue) -> Result<JsonValue, &'static str> {
    let signal = args.get("signal").ok_or("invalid_args")?;
    let signal = SignalId::new(
        InstrumentId::new(id_field(signal, "instrument")?),
        ParameterId::new(id_field(signal, "parameter")?),
    );
    let limit = args
        .get("max_records")
        .and_then(JsonValue::as_u64)
        .ok_or("invalid_args")? as usize;
    if !(1..=128).contains(&limit) {
        return Err("invalid_args");
    }
    let owner = service.owner();
    let QueryResult::Window(window) = owner
        .query(Query::GetSignalWindow(signal))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let total = window.len();
    let start = total.saturating_sub(limit);
    let rows = window[start..]
        .iter()
        .map(|sample| sample_json(sample, owner.signal_generation(signal)))
        .collect::<Vec<_>>();
    let result = json!({"signal":signal_id_json(signal),"ordering":"oldest_first",
        "source":"runtime_recent","capacity":total.to_string(),"truncated":start>0,"rows":rows});
    if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > 8 * 1024) {
        return Err("response_too_large");
    }
    Ok(result)
}
