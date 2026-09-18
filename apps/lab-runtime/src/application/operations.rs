//! Typed mutation routing and Recorder operation-fact projection.
//!
//! Session admission, request IDs, accepted/terminal lifecycle, and deduplication
//! stay centralized in `Application`. This module selects one semantic domain after
//! the authoritative protocol registry and wire layer have validated request shape.

use super::{controllers, references, virtuals};
use crate::{
    configuration_api, recorder_api,
    service::ServiceHost,
    sessions::{EmulatorPublication, Mutation, OperationState, PropertyMutationValue},
    wire::WireRequestId,
};
use lab_core::Error;
use serde_json::{Value, json};

pub(super) fn decode(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    match operation {
        "reference_configure" | "reference_retune" => references::decode(operation, args),
        "controller_configure_pid"
        | "controller_configure"
        | "controller_start"
        | "controller_pause"
        | "controller_resume"
        | "controller_reset_failed" => controllers::decode(operation, args),
        "recording_start" | "recording_stop" | "experiment_annotate" | "history_read" => {
            recorder_api::decode_mutation(operation, args)
        }
        "reload_configuration"
        | "stage_configuration"
        | "apply_configuration"
        | "property_configure"
        | "reconnect_resource" => configuration_api::decode_mutation(operation, args),
        "emulator_publish" | "virtual_models_restart" => virtuals::decode(operation, args),
        "runtime_shutdown" => Ok(Mutation::Shutdown),
        _ => Err("unsupported_operation"),
    }
}

pub(super) fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    request_id: &WireRequestId,
) -> Result<Value, Error> {
    match &mutation {
        Mutation::ConfigureReferenceFixed { .. }
        | Mutation::ConfigureReferenceRamp { .. }
        | Mutation::RetuneRamp { .. } => references::dispatch(service, mutation, request_id),
        Mutation::ConfigurePid { .. }
        | Mutation::ConfigureController { .. }
        | Mutation::Start { .. }
        | Mutation::Pause { .. }
        | Mutation::Resume { .. }
        | Mutation::ResetFailed { .. } => controllers::dispatch(service, mutation, request_id),
        Mutation::ReloadConfiguration
        | Mutation::StageConfiguration
        | Mutation::ApplyConfiguration { .. }
        | Mutation::ConfigureProperty { .. }
        | Mutation::ReconnectResource { .. } => {
            configuration_api::dispatch_mutation(service, mutation)
        }
        Mutation::PublishEmulatorMeasurement { .. } | Mutation::RestartVirtualModels => {
            virtuals::dispatch(service, mutation, request_id)
        }
        Mutation::Shutdown => Err(Error::InvalidConfiguration(
            "shutdown dispatch is host-only",
        )),
        Mutation::RecordingStart { .. }
        | Mutation::RecordingStop { .. }
        | Mutation::ExperimentAnnotate { .. }
        | Mutation::HistoryReadMeasurements { .. }
        | Mutation::HistoryReadRuns { .. } => Err(Error::InvalidConfiguration(
            "recording dispatch is host-only",
        )),
    }
}

// Stable, language-neutral command fields are selected from the admitted typed
// mutation. This storage encoding is not the wire request and cannot supply ACK or
// safe evidence. History selections remain process-local to avoid recursion.
pub(super) fn recorded_intent(mutation: &Mutation) -> Option<(&'static str, String)> {
    let (command, fields) = match mutation {
        Mutation::ConfigureReferenceFixed {
            reference,
            expected_revision,
            value,
        } => (
            "reference_configure",
            json!({"reference":reference.to_string(),"expected_revision":expected_revision.to_string(),
                "kind":"fixed","value":value}),
        ),
        Mutation::ConfigureReferenceRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => (
            "reference_configure",
            json!({"reference":reference.to_string(),"expected_revision":expected_revision.to_string(),
                "kind":"ramp","target":target,"rate":rate}),
        ),
        Mutation::RetuneRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => (
            "reference_retune",
            json!({"reference":reference.to_string(),
                "expected_revision":expected_revision.to_string(),"target":target,"rate":rate}),
        ),
        Mutation::ConfigurePid {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
        } => (
            "controller_configure",
            json!({"controller":controller.to_string(),
                "expected_revision":expected_revision.to_string(),"kp":kp,"ki":ki,"kd":kd,
                "output_min":output_min,"output_max":output_max}),
        ),
        Mutation::ConfigureController {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
            ema_time_constant_ns,
            ema_warmup_samples,
            max_input_age_ns,
            max_tick_gap_ns,
            lease_lifetime_ns,
            proposal_ttl_ns,
        } => (
            "controller_configure",
            json!({"controller":controller.to_string(),"expected_revision":expected_revision.to_string(),
                "pid":{"kp":kp,"ki":ki,"kd":kd,"output_min":output_min,"output_max":output_max},
                "ema":{"time_constant_ns":ema_time_constant_ns.to_string(),"warmup_samples":ema_warmup_samples.to_string()},
                "max_input_age_ns":max_input_age_ns.to_string(),"max_tick_gap_ns":max_tick_gap_ns.to_string(),
                "lease_lifetime_ns":lease_lifetime_ns.to_string(),"proposal_ttl_ns":proposal_ttl_ns.to_string()}),
        ),
        Mutation::Start { controller } => (
            "controller_start",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::Pause { controller } => (
            "controller_pause",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::Resume { controller } => (
            "controller_resume",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::ResetFailed { controller } => (
            "controller_reset_failed",
            json!({"controller":controller.to_string()}),
        ),
        Mutation::ReloadConfiguration => ("reload_configuration", json!({})),
        Mutation::StageConfiguration => ("stage_configuration", json!({})),
        Mutation::ApplyConfiguration {
            candidate_id,
            expected_revision,
        } => (
            "apply_configuration",
            json!({"candidate_id":candidate_id.to_string(),
                "expected_revision":expected_revision.to_string()}),
        ),
        Mutation::ConfigureProperty {
            target_kind,
            target_id,
            property,
            value,
            expected_revision,
        } => (
            "property_configure",
            json!({"target":{"kind":target_kind,"id":target_id.to_string()},
                "property":property,"value":match value {
                    PropertyMutationValue::Integer(value)=>json!(value),
                    PropertyMutationValue::Text(value)=>json!(value)},
                "expected_revision":expected_revision.to_string()}),
        ),
        Mutation::PublishEmulatorMeasurement {
            instrument,
            parameter,
            expected_generation,
            publication,
        } => (
            "emulator_publish",
            json!({"signal":{"instrument":instrument.to_string(),"parameter":parameter.to_string()},
                "expected_generation":expected_generation.to_string(),
                "state":match publication {EmulatorPublication::Good(_)=>"good",EmulatorPublication::Unavailable=>"unavailable"},
                "value":match publication {EmulatorPublication::Good(value)=>json!(value),EmulatorPublication::Unavailable=>Value::Null}}),
        ),
        Mutation::RestartVirtualModels => ("virtual_models_restart", json!({})),
        Mutation::ReconnectResource {
            resource,
            expected_binding_generation,
        } => (
            "reconnect_resource",
            json!({"resource":resource.to_string(),
                "expected_binding_generation":expected_binding_generation.to_string()}),
        ),
        Mutation::Shutdown => ("shutdown", json!({})),
        Mutation::RecordingStart { .. }
        | Mutation::RecordingStop { .. }
        | Mutation::ExperimentAnnotate { .. }
        | Mutation::HistoryReadMeasurements { .. }
        | Mutation::HistoryReadRuns { .. } => return None,
    };
    Some((command, fields.to_string()))
}

pub(super) fn recorded_terminal(outcome: &OperationState) -> (&'static str, String) {
    match outcome {
        OperationState::Completed(result) => (
            "completed",
            json!({"result":serde_json::from_str::<Value>(result).unwrap_or(Value::Null)})
                .to_string(),
        ),
        OperationState::Failed(code) => ("failed", json!({"code":code}).to_string()),
        OperationState::FailedWithResult { code, detail } => (
            "failed",
            json!({"code":code,"detail":serde_json::from_str::<Value>(detail)
                .unwrap_or(Value::Null)})
            .to_string(),
        ),
        OperationState::Accepted => unreachable!("terminal fact requires domain outcome"),
    }
}
