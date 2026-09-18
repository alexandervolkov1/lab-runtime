//! Shared bounded parsing, operation projection, and error mapping.
//!
//! Domain handlers return typed Core/host errors or stable internal codes. This
//! module is the single Application layer that maps them into the accepted public
//! operation/error representation.

use crate::{
    protocol::{self, PublicError},
    service::LifecycleOperationError,
    sessions::{Admission, OperationState, SessionError},
    wire::{WireRequestId, decimal_u64},
};
use lab_core::{Error, control::ControllerError, output::OutputError, transport::TransportError};
use serde_json::{Value, json};

pub(crate) fn parse_request_id(value: &Value) -> Result<WireRequestId, &'static str> {
    let scope = value
        .get("scope")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    let seq = value
        .get("seq")
        .and_then(Value::as_str)
        .and_then(decimal_u64)
        .ok_or("invalid_args")?;
    Ok(WireRequestId {
        scope: scope.to_owned(),
        seq,
    })
}

pub(crate) fn id_field(args: &Value, field: &str) -> Result<u64, &'static str> {
    args.get(field)
        .and_then(Value::as_str)
        .and_then(decimal_u64)
        .ok_or("invalid_args")
}

pub(crate) fn float_field(args: &Value, field: &str) -> Result<f64, &'static str> {
    args.get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or("invalid_args")
}

pub(super) fn operation_status(admission: Admission) -> Value {
    match admission {
        Admission::Known(state) => operation_state(state),
        _ => json!({"state":"outcome_unknown"}),
    }
}

pub(super) fn operation_state(state: OperationState) -> Value {
    match state {
        OperationState::Accepted => json!({"state":"accepted"}),
        OperationState::Completed(text) => {
            json!({"state":"completed","result":serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)})
        }
        OperationState::Failed(code) => {
            let mut value = json!({"state":"failed"});
            PublicError::from_internal_code(&code).apply_to(&mut value);
            value
        }
        OperationState::FailedWithResult { code, detail } => {
            let mut value = json!({"state":"failed",
                "result":serde_json::from_str::<Value>(&detail).unwrap_or(Value::Null)});
            PublicError::from_internal_code(&code).apply_to(&mut value);
            value
        }
    }
}

pub(super) fn operation_reply(
    message: &str,
    request_id: &WireRequestId,
    state: OperationState,
) -> Value {
    let mut value = json!({"v":protocol::PROTOCOL_VERSION,"msg_id":message,
        "type":"operation","request_id":{"scope":request_id.scope,"seq":request_id.seq.to_string()}});
    let state = operation_state(state);
    for (key, value_part) in state.as_object().expect("fixed state object") {
        value[key] = value_part.clone();
    }
    value
}

pub(super) fn error_reply(message: &str, code: &str) -> Value {
    let mut value = json!({"v":protocol::PROTOCOL_VERSION,"msg_id":message,
        "type":"error","accepted":false});
    PublicError::from_internal_code(code).apply_to(&mut value);
    value
}

pub(super) const fn session_code(error: SessionError) -> &'static str {
    match error {
        SessionError::InstanceChanged => "instance_changed",
        SessionError::ScopeInUse => "scope_in_use",
        SessionError::ScopeCapacity => "busy",
        _ => "scope_unknown",
    }
}

pub(crate) const fn domain_code(error: Error) -> &'static str {
    match error {
        Error::Controller(ControllerError::RevisionConflict) => "revision_conflict",
        Error::Controller(ControllerError::UnknownController) => "unknown_controller",
        Error::Controller(ControllerError::UnknownReference) => "unknown_reference",
        Error::Controller(
            ControllerError::DuplicateController
            | ControllerError::DuplicateReference
            | ControllerError::InvalidConfiguration
            | ControllerError::InvalidTickTime
            | ControllerError::Algorithm
            | ControllerError::RevisionExhausted,
        ) => "invalid_configuration",
        Error::Controller(ControllerError::InvalidState) => "invalid_state",
        Error::Controller(ControllerError::InputUnavailable) => "input_unavailable",
        Error::Controller(ControllerError::StaleInput) => "stale_input",
        Error::Controller(ControllerError::Output) => "output_rejected",
        Error::Output(OutputError::UnknownActuator) => "unknown_actuator",
        Error::Output(OutputError::Busy) => "busy",
        Error::Output(OutputError::Expired) => "timeout",
        Error::Output(_) => "output_rejected",
        Error::UnknownInstrument(_) => "unknown_instrument",
        Error::UnknownParameter { .. } => "unknown_parameter",
        Error::UnknownSignal(_) => "unknown_signal",
        Error::Transport(TransportError::UnknownResource) => "unknown_resource",
        Error::Transport(
            TransportError::QueueFull
            | TransportError::ResourceLimit
            | TransportError::ResourceBusy,
        ) => "busy",
        Error::Transport(
            TransportError::InvalidTransaction
            | TransportError::InvalidTime
            | TransportError::CounterExhausted
            | TransportError::DuplicateResource,
        ) => "invalid_configuration",
        Error::Transport(TransportError::ResourceUnavailable) => "transport_unavailable",
        Error::InvalidConfiguration(_) => "invalid_configuration",
        Error::RecordingUnavailable => "recording_unavailable",
        _ => "domain_rejected",
    }
}

pub(crate) const fn lifecycle_domain_error(error: LifecycleOperationError) -> Error {
    match error {
        LifecycleOperationError::RecordingUnavailable => Error::RecordingUnavailable,
        LifecycleOperationError::TransportUnavailable => {
            Error::Transport(TransportError::ResourceUnavailable)
        }
        LifecycleOperationError::Conflict => Error::Controller(ControllerError::RevisionConflict),
        _ => Error::InvalidConfiguration("lifecycle operation failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_lifecycle_failure_uses_existing_recording_unavailable_wire_code() {
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::RecordingUnavailable
            )),
            "recording_unavailable"
        );
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::InvalidCandidate
            )),
            "invalid_configuration"
        );
    }

    #[test]
    fn reconnect_transport_failure_and_stale_generation_keep_distinct_public_codes() {
        assert_eq!(
            domain_code(lifecycle_domain_error(
                LifecycleOperationError::TransportUnavailable
            )),
            "transport_unavailable"
        );
        assert_eq!(
            domain_code(lifecycle_domain_error(LifecycleOperationError::Conflict)),
            "revision_conflict"
        );
    }
}
