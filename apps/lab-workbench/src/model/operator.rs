//! Typed operator intent, validation, confirmation, and mutation lifecycle.
//!
//! This module is the only Workbench layer allowed to translate operator intent
//! into Application mutation names and arguments. It owns no experiment state:
//! every optimistic-concurrency identity is captured from one fresh Runtime
//! observation and Runtime remains the final validator.

use super::{Freshness, WorkbenchModel};
use crate::{
    client::types::{CommandSendError, ConnectionState, MutationIdentity, ReplyKind},
    client::{ClientHandle, ClientUpdate},
    presentation::RuntimeRef,
};
use serde_json::{Value, json};

pub(crate) const RECORDING_LABEL_BYTES: usize = 128;
pub(crate) const PROPERTY_TEXT_BYTES: usize = 512;
const OPERATOR_WARNING_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControllerLifecycleIntent {
    Start,
    Pause,
    Resume,
    ResetFailed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PidCandidate {
    pub(crate) kp: f64,
    pub(crate) ki: f64,
    pub(crate) kd: f64,
    pub(crate) output_min: f64,
    pub(crate) output_max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PropertyMutationCandidate {
    Integer(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OperatorWarning {
    ControllerPolicyChange,
    ResourceReconnect,
    RecorderStop,
    PropertyMutationClass(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OperatorIntent {
    ConfigureReferenceFixed {
        reference: String,
        value: f64,
    },
    ConfigureReferenceRamp {
        reference: String,
        target: f64,
        rate: f64,
    },
    RetuneReference {
        reference: String,
        target: f64,
        rate: f64,
    },
    ControllerLifecycle {
        controller: String,
        action: ControllerLifecycleIntent,
    },
    ConfigureControllerPid {
        controller: String,
        pid: PidCandidate,
    },
    ConfigureProperty {
        target: RuntimeRef,
        value: PropertyMutationCandidate,
    },
    ReconnectResource {
        resource: String,
    },
    StartRecording {
        label: String,
    },
    StopRecording,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OperatorIntentError {
    ClientNotReady,
    RebuildNotFresh,
    TargetNotFresh,
    OperationUnavailable,
    RecoveryBlocked,
    ActionCapacityFull,
    WorkflowBusy,
    InvalidCandidate(&'static str),
    DraftStale,
    LocalSubmission(CommandSendError),
}

impl std::fmt::Display for OperatorIntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientNotReady => f.write_str("Application client is not Ready"),
            Self::RebuildNotFresh => f.write_str("Workbench projections are not Fresh"),
            Self::TargetNotFresh => f.write_str("target projection is not Fresh"),
            Self::OperationUnavailable => f.write_str("operation is not advertised by hello"),
            Self::RecoveryBlocked => {
                f.write_str("recovery or reconciliation blocks new mutation sequencing")
            }
            Self::ActionCapacityFull => {
                f.write_str("operator action history cannot retain another active command")
            }
            Self::WorkflowBusy => f.write_str("another operator workflow is active"),
            Self::InvalidCandidate(reason) => f.write_str(reason),
            Self::DraftStale => f.write_str("draft identity is stale; reload the projection"),
            Self::LocalSubmission(error) => write!(f, "local submission failed: {error}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ObservationGuard {
    boot_id: String,
    target: RuntimeRef,
    pointer: &'static str,
    value: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthorityGuard {
    ObservationOnly,
    ControllerDetail {
        controller: String,
        revision: Value,
    },
    ResourceDetail {
        resource: String,
        transport_generation: Value,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreparedOperatorIntent {
    pub(crate) operation: &'static str,
    pub(crate) args: Value,
    pub(crate) target: RuntimeRef,
    pub(crate) confirmation: String,
    pub(crate) warning: Option<OperatorWarning>,
    guard: ObservationGuard,
    authority: AuthorityGuard,
}

impl PreparedOperatorIntent {
    pub(crate) fn is_current(&self, model: &WorkbenchModel) -> bool {
        let observation_current = model
            .hello
            .as_ref()
            .is_some_and(|hello| hello.boot_id == self.guard.boot_id)
            && model
                .observations
                .entities
                .get(&self.guard.target)
                .is_some_and(|observation| {
                    observation.freshness == Freshness::Fresh
                        && observation.value.pointer(self.guard.pointer) == Some(&self.guard.value)
                });
        observation_current
            && match &self.authority {
                AuthorityGuard::ObservationOnly => true,
                AuthorityGuard::ControllerDetail {
                    controller,
                    revision,
                } => {
                    model.controller_detail_is_fresh(controller)
                        && model.controller_detail_revision(controller) == Some(revision)
                }
                AuthorityGuard::ResourceDetail {
                    resource,
                    transport_generation,
                } => {
                    model.resource_detail_is_fresh(resource)
                        && model.resource_detail_generation(resource) == Some(transport_generation)
                }
            }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OperatorWorkflowState {
    Idle,
    AwaitingConfirmation(PreparedOperatorIntent),
    Submitted {
        command_id: u64,
        prepared: PreparedOperatorIntent,
    },
    Accepted {
        command_id: u64,
        prepared: PreparedOperatorIntent,
        identity: Option<MutationIdentity>,
    },
    Completed {
        prepared: PreparedOperatorIntent,
        envelope: Value,
    },
    Failed {
        prepared: PreparedOperatorIntent,
        envelope: Value,
        conflict: bool,
    },
    Ambiguous {
        prepared: PreparedOperatorIntent,
        identity: Option<MutationIdentity>,
    },
}

impl OperatorWorkflowState {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

pub(crate) trait MutationSubmitter {
    fn submit_mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError>;
}

impl MutationSubmitter for ClientHandle {
    fn submit_mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        self.mutation(op, args)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OperatorWorkflow {
    pub(crate) state: OperatorWorkflowState,
    /// True once an explicit Runtime accepted reply was observed for this workflow.
    pub(crate) accepted_observed: bool,
}

impl Default for OperatorWorkflow {
    fn default() -> Self {
        Self {
            state: OperatorWorkflowState::Idle,
            accepted_observed: false,
        }
    }
}

impl OperatorWorkflow {
    pub(crate) fn begin(
        &mut self,
        model: &WorkbenchModel,
        intent: OperatorIntent,
    ) -> Result<(), OperatorIntentError> {
        if self.state.is_active() {
            return Err(OperatorIntentError::WorkflowBusy);
        }
        self.accepted_observed = false;
        self.state = OperatorWorkflowState::AwaitingConfirmation(prepare(model, intent)?);
        Ok(())
    }

    pub(crate) fn confirm(
        &mut self,
        model: &WorkbenchModel,
        client: &impl MutationSubmitter,
    ) -> Result<u64, OperatorIntentError> {
        let OperatorWorkflowState::AwaitingConfirmation(prepared) = &self.state else {
            return Err(OperatorIntentError::InvalidCandidate(
                "no operator intent is awaiting confirmation",
            ));
        };
        readiness(model, &prepared.target, prepared.operation)?;
        if !prepared.is_current(model) {
            return Err(OperatorIntentError::DraftStale);
        }
        if !model.operator_action_capacity_available() {
            return Err(OperatorIntentError::ActionCapacityFull);
        }
        let command_id = client
            .submit_mutation(prepared.operation, prepared.args.clone())
            .map_err(OperatorIntentError::LocalSubmission)?;
        self.state = OperatorWorkflowState::Submitted {
            command_id,
            prepared: prepared.clone(),
        };
        Ok(command_id)
    }

    pub(crate) fn cancel_or_acknowledge(&mut self) {
        self.state = OperatorWorkflowState::Idle;
        self.accepted_observed = false;
    }

    pub(crate) fn after_update(&mut self, update: &ClientUpdate) {
        let (command_id, prepared, known_identity) = match &self.state {
            OperatorWorkflowState::Submitted {
                command_id,
                prepared,
            } => (*command_id, prepared.clone(), None),
            OperatorWorkflowState::Accepted {
                command_id,
                prepared,
                identity,
            } => (*command_id, prepared.clone(), identity.clone()),
            _ => return,
        };
        match update {
            ClientUpdate::Reply {
                command_id: received,
                kind,
                envelope,
                recovery,
                ..
            } if *received == command_id => match kind {
                ReplyKind::MutationAccepted => {
                    self.accepted_observed = true;
                    self.state = OperatorWorkflowState::Accepted {
                        command_id,
                        prepared,
                        identity: recovery
                            .as_ref()
                            .map(|record| record.identity.clone())
                            .or(known_identity),
                    };
                }
                ReplyKind::MutationCompleted => {
                    self.state = OperatorWorkflowState::Completed {
                        prepared,
                        envelope: envelope.clone(),
                    };
                }
                ReplyKind::MutationFailed | ReplyKind::PublicError => {
                    self.state = OperatorWorkflowState::Failed {
                        prepared,
                        conflict: envelope.get("code").and_then(Value::as_str)
                            == Some("revision_conflict"),
                        envelope: envelope.clone(),
                    };
                }
                ReplyKind::Result => {}
            },
            ClientUpdate::LocalRejected {
                command_id: received,
                reason,
            } if *received == command_id => {
                self.state = OperatorWorkflowState::Failed {
                    prepared,
                    envelope: json!({"type":"local_error","code":"client_busy","message":reason}),
                    conflict: false,
                };
            }
            ClientUpdate::ReconciliationRequired { records }
                if records.iter().any(|record| {
                    record.op == prepared.operation
                        && known_identity
                            .as_ref()
                            .is_none_or(|identity| identity == &record.identity)
                }) =>
            {
                self.state = OperatorWorkflowState::Ambiguous {
                    prepared,
                    identity: known_identity,
                };
            }
            ClientUpdate::TransportFailure { .. }
            | ClientUpdate::ResnapshotRequired {
                connection_lost: true,
                ..
            } => {
                self.state = OperatorWorkflowState::Ambiguous {
                    prepared,
                    identity: known_identity,
                };
            }
            _ => {}
        }
    }
}

pub(crate) fn prepare(
    model: &WorkbenchModel,
    intent: OperatorIntent,
) -> Result<PreparedOperatorIntent, OperatorIntentError> {
    match intent {
        OperatorIntent::ConfigureReferenceFixed { reference, value } => {
            finite(value)?;
            reference_intent(
                model,
                reference,
                "reference_configure",
                |revision, reference| {
                    json!({"reference":reference,"expected_revision":revision,
                        "kind":"fixed","value":value})
                },
                format!("Configure Reference as fixed value {value}"),
            )
        }
        OperatorIntent::ConfigureReferenceRamp {
            reference,
            target,
            rate,
        } => {
            finite(target)?;
            positive_finite(rate, "Reference rate must be finite and positive")?;
            reference_intent(
                model,
                reference,
                "reference_configure",
                |revision, reference| {
                    json!({"reference":reference,"expected_revision":revision,
                        "kind":"ramp","target":target,"rate":rate})
                },
                format!("Configure Reference ramp target {target}, rate {rate}"),
            )
        }
        OperatorIntent::RetuneReference {
            reference,
            target,
            rate,
        } => {
            finite(target)?;
            positive_finite(rate, "Reference rate must be finite and positive")?;
            let identity = RuntimeRef::Reference {
                reference: reference.clone(),
            };
            if fresh_observation(model, &identity)?
                .get("kind")
                .and_then(Value::as_str)
                != Some("ramp")
            {
                return Err(OperatorIntentError::InvalidCandidate(
                    "reference_retune requires a fresh ramp Reference",
                ));
            }
            reference_intent(
                model,
                reference,
                "reference_retune",
                |revision, reference| {
                    json!({"reference":reference,"expected_revision":revision,
                        "target":target,"rate":rate})
                },
                format!("Retune Reference target {target}, rate {rate}"),
            )
        }
        OperatorIntent::ControllerLifecycle { controller, action } => {
            let target = RuntimeRef::Controller {
                controller: controller.clone(),
            };
            let observation = fresh_observation(model, &target)?;
            let state = observation.get("state").and_then(Value::as_str).ok_or(
                OperatorIntentError::InvalidCandidate("controller projection omitted state"),
            )?;
            let (operation, applicable) = match action {
                ControllerLifecycleIntent::Start => ("controller_start", state == "ready"),
                ControllerLifecycleIntent::Pause => {
                    ("controller_pause", matches!(state, "warming" | "running"))
                }
                ControllerLifecycleIntent::Resume => ("controller_resume", state == "paused"),
                ControllerLifecycleIntent::ResetFailed => {
                    ("controller_reset_failed", state == "failed")
                }
            };
            if !applicable {
                return Err(OperatorIntentError::InvalidCandidate(
                    "controller action is not applicable to the observed state",
                ));
            }
            readiness(model, &target, operation)?;
            Ok(prepared(
                model,
                operation,
                json!({"controller":controller}),
                target,
                "/state",
                json!(state),
                format!("Controller {controller}: {operation} from {state}"),
                None,
            ))
        }
        OperatorIntent::ConfigureControllerPid { controller, pid } => {
            for value in [pid.kp, pid.ki, pid.kd, pid.output_min, pid.output_max] {
                finite(value)?;
            }
            if pid.output_min > pid.output_max {
                return Err(OperatorIntentError::InvalidCandidate(
                    "PID output_min must not exceed output_max",
                ));
            }
            let target = RuntimeRef::Controller {
                controller: controller.clone(),
            };
            let observation = fresh_observation(model, &target)?;
            let revision = id_at(observation, "/revision", "controller revision")?;
            if !model.controller_detail_is_fresh(&controller) {
                return Err(OperatorIntentError::TargetNotFresh);
            }
            let detail_revision = model
                .controller_detail_revision(&controller)
                .cloned()
                .ok_or(OperatorIntentError::TargetNotFresh)?;
            readiness(model, &target, "controller_configure_pid")?;
            Ok(prepared_with_authority(
                model,
                "controller_configure_pid",
                json!({"controller":controller,"expected_revision":revision,
                    "pid":{"kp":pid.kp,"ki":pid.ki,"kd":pid.kd,
                        "output_min":pid.output_min,"output_max":pid.output_max}}),
                target,
                "/revision",
                json!(revision),
                format!("Configure PID for Controller {controller}"),
                Some(OperatorWarning::ControllerPolicyChange),
                AuthorityGuard::ControllerDetail {
                    controller: controller.clone(),
                    revision: detail_revision,
                },
            ))
        }
        OperatorIntent::ConfigureProperty { target, value } => {
            let observation = fresh_observation(model, &target)?;
            let RuntimeRef::ConfigurationProperty { owner, property } = &target else {
                return Err(OperatorIntentError::InvalidCandidate(
                    "property intent requires a ConfigurationProperty target",
                ));
            };
            if observation.get("access").and_then(Value::as_str) != Some("read_write") {
                return Err(OperatorIntentError::InvalidCandidate(
                    "property is not Runtime-writable",
                ));
            }
            let encoded = encode_property_value(observation, value)?;
            let revision = id_at(observation, "/revision", "property revision")?;
            readiness(model, &target, "property_configure")?;
            let warning = observation
                .get("mutation_class")
                .and_then(Value::as_str)
                .filter(|class| class.len() <= OPERATOR_WARNING_BYTES)
                .map(|class| OperatorWarning::PropertyMutationClass(class.to_owned()));
            let confirmation = format!("Configure property {property} to {encoded}");
            Ok(prepared(
                model,
                "property_configure",
                json!({"target":{"kind":owner_kind(owner),"id":owner_id(owner)},
                    "property":property,"value":encoded,"expected_revision":revision}),
                target,
                "/revision",
                json!(revision),
                confirmation,
                warning,
            ))
        }
        OperatorIntent::ReconnectResource { resource } => {
            let target = RuntimeRef::Resource {
                resource: resource.clone(),
            };
            let observation = fresh_observation(model, &target)?;
            if !model.resource_detail_is_fresh(&resource) {
                return Err(OperatorIntentError::TargetNotFresh);
            }
            let transport_generation = model
                .resource_detail_generation(&resource)
                .cloned()
                .ok_or(OperatorIntentError::TargetNotFresh)?;
            if observation.pointer("/capabilities/reconnect") != Some(&Value::Bool(true)) {
                return Err(OperatorIntentError::InvalidCandidate(
                    "resource is not reconnect-capable",
                ));
            }
            let generation = id_at(
                observation,
                "/binding_generation",
                "resource binding generation",
            )?;
            readiness(model, &target, "reconnect_resource")?;
            Ok(prepared_with_authority(
                model,
                "reconnect_resource",
                json!({"resource":resource,"expected_binding_generation":generation}),
                target,
                "/binding_generation",
                json!(generation),
                format!("Reconnect Resource {resource} at binding generation {generation}"),
                Some(OperatorWarning::ResourceReconnect),
                AuthorityGuard::ResourceDetail {
                    resource: resource.clone(),
                    transport_generation,
                },
            ))
        }
        OperatorIntent::StartRecording { label } => {
            if label.trim().is_empty() || label.len() > RECORDING_LABEL_BYTES {
                return Err(OperatorIntentError::InvalidCandidate(
                    "recording label must contain 1..=128 bytes",
                ));
            }
            let target = RuntimeRef::Recorder;
            let observation = fresh_observation(model, &target)?;
            if observation.get("state").and_then(Value::as_str) != Some("idle") {
                return Err(OperatorIntentError::InvalidCandidate(
                    "recording_start requires authoritative idle Recorder state",
                ));
            }
            readiness(model, &target, "recording_start")?;
            Ok(prepared(
                model,
                "recording_start",
                json!({"label":label}),
                target,
                "/state",
                observation.get("state").cloned().unwrap_or(Value::Null),
                "Start recording".into(),
                None,
            ))
        }
        OperatorIntent::StopRecording => {
            let target = RuntimeRef::Recorder;
            let observation = fresh_observation(model, &target)?;
            let run_id = observation
                .get("active_run")
                .filter(|run| !run.is_null())
                .cloned()
                .ok_or(OperatorIntentError::InvalidCandidate(
                    "Recorder projection has no active run",
                ))?;
            readiness(model, &target, "recording_stop")?;
            Ok(prepared(
                model,
                "recording_stop",
                json!({"run_id":run_id}),
                target,
                "/active_run",
                run_id.clone(),
                format!("Stop recording run {run_id}"),
                Some(OperatorWarning::RecorderStop),
            ))
        }
    }
}

fn reference_intent(
    model: &WorkbenchModel,
    reference: String,
    operation: &'static str,
    encode: impl FnOnce(&str, &str) -> Value,
    confirmation: String,
) -> Result<PreparedOperatorIntent, OperatorIntentError> {
    let target = RuntimeRef::Reference {
        reference: reference.clone(),
    };
    let observation = fresh_observation(model, &target)?;
    let revision = id_at(observation, "/revision", "Reference revision")?;
    readiness(model, &target, operation)?;
    Ok(prepared(
        model,
        operation,
        encode(revision, &reference),
        target,
        "/revision",
        json!(revision),
        format!("Reference {reference}; revision {revision}; {confirmation}"),
        None,
    ))
}

#[allow(clippy::too_many_arguments)]
fn prepared(
    model: &WorkbenchModel,
    operation: &'static str,
    args: Value,
    target: RuntimeRef,
    pointer: &'static str,
    value: Value,
    confirmation: String,
    warning: Option<OperatorWarning>,
) -> PreparedOperatorIntent {
    prepared_with_authority(
        model,
        operation,
        args,
        target,
        pointer,
        value,
        confirmation,
        warning,
        AuthorityGuard::ObservationOnly,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepared_with_authority(
    model: &WorkbenchModel,
    operation: &'static str,
    args: Value,
    target: RuntimeRef,
    pointer: &'static str,
    value: Value,
    confirmation: String,
    warning: Option<OperatorWarning>,
    authority: AuthorityGuard,
) -> PreparedOperatorIntent {
    let boot_id = model
        .hello
        .as_ref()
        .expect("readiness established hello before preparing intent")
        .boot_id
        .clone();
    PreparedOperatorIntent {
        operation,
        args,
        target: target.clone(),
        confirmation,
        warning,
        authority,
        guard: ObservationGuard {
            boot_id,
            target,
            pointer,
            value,
        },
    }
}

fn readiness(
    model: &WorkbenchModel,
    target: &RuntimeRef,
    operation: &str,
) -> Result<(), OperatorIntentError> {
    if model.connection != ConnectionState::Ready {
        return Err(OperatorIntentError::ClientNotReady);
    }
    if model.observations.freshness != Freshness::Fresh {
        return Err(OperatorIntentError::RebuildNotFresh);
    }
    let Some(observation) = model.observations.entities.get(target) else {
        return Err(OperatorIntentError::TargetNotFresh);
    };
    if observation.freshness != Freshness::Fresh {
        return Err(OperatorIntentError::TargetNotFresh);
    }
    if !model.hello.as_ref().is_some_and(|hello| {
        hello
            .operations
            .iter()
            .any(|candidate| candidate == operation)
    }) {
        return Err(OperatorIntentError::OperationUnavailable);
    }
    if model.recovery_problem.is_some()
        || !model.recovery.reconciliation_required.is_empty()
        || model.recovery.mutations.iter().any(|record| {
            matches!(
                record.admission,
                crate::client::types::KnownAdmission::Pending
                    | crate::client::types::KnownAdmission::Accepted
                    | crate::client::types::KnownAdmission::Ambiguous
            )
        })
    {
        return Err(OperatorIntentError::RecoveryBlocked);
    }
    Ok(())
}

fn fresh_observation<'a>(
    model: &'a WorkbenchModel,
    target: &RuntimeRef,
) -> Result<&'a Value, OperatorIntentError> {
    model
        .observations
        .entities
        .get(target)
        .filter(|observation| observation.freshness == Freshness::Fresh)
        .map(|observation| &observation.value)
        .ok_or(OperatorIntentError::TargetNotFresh)
}

fn id_at<'a>(
    observation: &'a Value,
    pointer: &str,
    name: &'static str,
) -> Result<&'a str, OperatorIntentError> {
    observation
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or(OperatorIntentError::InvalidCandidate(name))
}

fn finite(value: f64) -> Result<(), OperatorIntentError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(OperatorIntentError::InvalidCandidate(
            "numeric candidate must be finite",
        ))
    }
}

fn positive_finite(value: f64, message: &'static str) -> Result<(), OperatorIntentError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(OperatorIntentError::InvalidCandidate(message))
    }
}

fn encode_property_value(
    observation: &Value,
    candidate: PropertyMutationCandidate,
) -> Result<Value, OperatorIntentError> {
    let value_type = observation
        .get("value_type")
        .and_then(Value::as_str)
        .ok_or(OperatorIntentError::InvalidCandidate(
            "property projection omitted value_type",
        ))?;
    let value = match (value_type, candidate) {
        ("integer", PropertyMutationCandidate::Integer(value)) => json!(value),
        ("text", PropertyMutationCandidate::Text(value)) => {
            if value.len() > PROPERTY_TEXT_BYTES {
                return Err(OperatorIntentError::InvalidCandidate(
                    "text property exceeds the Workbench editor bound",
                ));
            }
            json!(value)
        }
        _ => {
            return Err(OperatorIntentError::InvalidCandidate(
                "v1 property mutation supports only writable integer or text values",
            ));
        }
    };
    if let Some(minimum) = observation
        .pointer("/constraints/minimum")
        .and_then(Value::as_f64)
        && value.as_f64().is_some_and(|candidate| candidate < minimum)
    {
        return Err(OperatorIntentError::InvalidCandidate(
            "property candidate is below the advertised minimum",
        ));
    }
    if let Some(maximum) = observation
        .pointer("/constraints/maximum")
        .and_then(Value::as_f64)
        && value.as_f64().is_some_and(|candidate| candidate > maximum)
    {
        return Err(OperatorIntentError::InvalidCandidate(
            "property candidate is above the advertised maximum",
        ));
    }
    Ok(value)
}

fn owner_kind(owner: &crate::presentation::ConfigurationOwner) -> &'static str {
    match owner {
        crate::presentation::ConfigurationOwner::Instrument { .. } => "instrument",
        crate::presentation::ConfigurationOwner::Component { .. } => "component",
        crate::presentation::ConfigurationOwner::Resource { .. } => "resource",
    }
}

fn owner_id(owner: &crate::presentation::ConfigurationOwner) -> &str {
    match owner {
        crate::presentation::ConfigurationOwner::Instrument { instrument } => instrument,
        crate::presentation::ConfigurationOwner::Component { component } => component,
        crate::presentation::ConfigurationOwner::Resource { resource } => resource,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::types::{EventCursor, HelloState},
        model::RuntimeObservations,
        presentation::{ConfigurationOwner, PresentationDocument},
    };
    use std::{cell::RefCell, collections::BTreeMap};

    fn model(operations: &[&str]) -> WorkbenchModel {
        let mut model = WorkbenchModel::new(PresentationDocument::empty("operator-test"));
        model.connection = ConnectionState::Ready;
        model.hello = Some(HelloState {
            boot_id: "boot".into(),
            scope: "scope".into(),
            next_seq: 1,
            operations: operations.iter().map(|value| (*value).to_owned()).collect(),
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
        });
        model.observations = RuntimeObservations::default();
        model.observations.complete_rebuild();
        model
    }

    fn observe(model: &mut WorkbenchModel, target: RuntimeRef, value: Value) {
        model.observations.observe(target, value, None);
        model.observations.complete_rebuild();
    }

    #[test]
    fn readiness_requires_ready_fresh_target_and_advertised_operation() {
        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let intent = || OperatorIntent::RetuneReference {
            reference: "1".into(),
            target: 10.0,
            rate: 1.0,
        };
        let mut value = model(&["reference_retune"]);
        assert_eq!(
            prepare(&value, intent()),
            Err(OperatorIntentError::TargetNotFresh)
        );
        observe(
            &mut value,
            target,
            json!({"reference":"1","revision":"7","kind":"ramp"}),
        );
        value.connection = ConnectionState::Stale;
        assert_eq!(
            prepare(&value, intent()),
            Err(OperatorIntentError::ClientNotReady)
        );
        value.connection = ConnectionState::Ready;
        value.observations.freshness = Freshness::Rebuilding;
        assert_eq!(
            prepare(&value, intent()),
            Err(OperatorIntentError::RebuildNotFresh)
        );
        value.observations.complete_rebuild();
        value.hello.as_mut().unwrap().operations.clear();
        assert_eq!(
            prepare(&value, intent()),
            Err(OperatorIntentError::OperationUnavailable)
        );
    }

    #[test]
    fn recovery_problem_and_unresolved_mutation_disable_new_intent() {
        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let mut model = model(&["reference_retune"]);
        observe(
            &mut model,
            target,
            json!({"reference":"1","revision":"7","kind":"ramp"}),
        );
        let intent = || OperatorIntent::RetuneReference {
            reference: "1".into(),
            target: 2.0,
            rate: 1.0,
        };
        model.recovery_problem = Some("journal".into());
        assert_eq!(
            prepare(&model, intent()),
            Err(OperatorIntentError::RecoveryBlocked)
        );
        model.recovery_problem = None;
        model
            .recovery
            .reconciliation_required
            .push(MutationIdentity {
                scope: "scope".into(),
                seq: 1,
            });
        assert_eq!(
            prepare(&model, intent()),
            Err(OperatorIntentError::RecoveryBlocked)
        );
    }

    #[test]
    fn reference_draft_freezes_revision_and_never_mutates_observation() {
        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let mut model = model(&["reference_retune"]);
        observe(
            &mut model,
            target.clone(),
            json!({"reference":"1","revision":"7","value":3.0,"kind":"ramp"}),
        );
        let before = model.observations.entities[&target].value.clone();
        let prepared = prepare(
            &model,
            OperatorIntent::RetuneReference {
                reference: "1".into(),
                target: 9.0,
                rate: 2.0,
            },
        )
        .unwrap();
        assert_eq!(prepared.args["expected_revision"], "7");
        assert_eq!(model.observations.entities[&target].value, before);
        model.observations.entities.get_mut(&target).unwrap().value["revision"] = json!("8");
        assert!(!prepared.is_current(&model));
    }

    #[test]
    fn controller_lifecycle_uses_stable_id_and_observed_state() {
        let target = RuntimeRef::Controller {
            controller: "4".into(),
        };
        let mut model = model(&["controller_pause"]);
        observe(
            &mut model,
            target,
            json!({"controller":"4","state":"running","revision":"2"}),
        );
        let prepared = prepare(
            &model,
            OperatorIntent::ControllerLifecycle {
                controller: "4".into(),
                action: ControllerLifecycleIntent::Pause,
            },
        )
        .unwrap();
        assert_eq!(prepared.operation, "controller_pause");
        assert_eq!(prepared.args, json!({"controller":"4"}));
    }

    #[test]
    fn pid_requires_full_detail_for_the_current_controller_revision() {
        let mut model = model(&["controller_configure_pid"]);
        model.apply_client_update(ClientUpdate::Reply {
            command_id: 1,
            msg_id: "1".into(),
            op: "controller".into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":{"controller":"4","state":"paused",
                "revision":"7","config":{"pid":{"kp":1.0,"ki":0.0,"kd":0.0,
                "output_min":0.0,"output_max":1.0}}}}),
            recovery: None,
        });
        model.observations.complete_rebuild();
        let old_prepared = prepare(
            &model,
            OperatorIntent::ConfigureControllerPid {
                controller: "4".into(),
                pid: PidCandidate {
                    kp: 1.0,
                    ki: 0.0,
                    kd: 0.0,
                    output_min: 0.0,
                    output_max: 1.0,
                },
            },
        )
        .unwrap();
        assert_eq!(
            old_prepared.authority,
            AuthorityGuard::ControllerDetail {
                controller: "4".into(),
                revision: json!("7"),
            }
        );
        assert!(old_prepared.is_current(&model));
        model.apply_client_update(ClientUpdate::Event {
            cursor: EventCursor {
                boot_id: "boot".into(),
                seq: 2,
            },
            envelope: json!({"kind":"controller","target":{"id":"4"},
                "data":{"state":"paused","status":"valid","failure":null,"active":false,
                    "paused":true,"revision":"8","last_tick":"1","latest_output":0.0}}),
        });
        assert!(!model.controller_detail_is_fresh("4"));
        assert!(!old_prepared.is_current(&model));
        assert_eq!(
            prepare(
                &model,
                OperatorIntent::ConfigureControllerPid {
                    controller: "4".into(),
                    pid: PidCandidate {
                        kp: 1.0,
                        ki: 0.0,
                        kd: 0.0,
                        output_min: 0.0,
                        output_max: 1.0,
                    },
                }
            ),
            Err(OperatorIntentError::TargetNotFresh)
        );
        model.apply_client_update(ClientUpdate::Reply {
            command_id: 2,
            msg_id: "2".into(),
            op: "controller".into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":{"controller":"4","state":"paused",
                "revision":"8","config":{"pid":{"kp":2.0,"ki":0.0,"kd":0.0,
                "output_min":0.0,"output_max":1.0}}}}),
            recovery: None,
        });
        assert!(model.controller_detail_is_fresh("4"));
        assert!(!old_prepared.is_current(&model));
        let prepared = prepare(
            &model,
            OperatorIntent::ConfigureControllerPid {
                controller: "4".into(),
                pid: PidCandidate {
                    kp: 2.0,
                    ki: 0.0,
                    kd: 0.0,
                    output_min: 0.0,
                    output_max: 1.0,
                },
            },
        )
        .unwrap();
        assert_eq!(
            prepared.authority,
            AuthorityGuard::ControllerDetail {
                controller: "4".into(),
                revision: json!("8"),
            }
        );
        assert!(prepared.is_current(&model));
        assert_eq!(prepared.args["expected_revision"], "8");
        assert_eq!(
            prepared.warning,
            Some(OperatorWarning::ControllerPolicyChange)
        );
    }

    #[test]
    fn pid_rejects_nonfinite_and_inverted_range() {
        let target = RuntimeRef::Controller {
            controller: "4".into(),
        };
        let mut model = model(&["controller_configure_pid"]);
        observe(
            &mut model,
            target,
            json!({"controller":"4","state":"paused","revision":"2"}),
        );
        let invalid = PidCandidate {
            kp: f64::NAN,
            ki: 0.0,
            kd: 0.0,
            output_min: 0.0,
            output_max: 1.0,
        };
        assert!(matches!(
            prepare(
                &model,
                OperatorIntent::ConfigureControllerPid {
                    controller: "4".into(),
                    pid: invalid
                }
            ),
            Err(OperatorIntentError::InvalidCandidate(_))
        ));
        let inverted = PidCandidate {
            kp: 1.0,
            ki: 0.0,
            kd: 0.0,
            output_min: 2.0,
            output_max: 1.0,
        };
        assert!(matches!(
            prepare(
                &model,
                OperatorIntent::ConfigureControllerPid {
                    controller: "4".into(),
                    pid: inverted
                }
            ),
            Err(OperatorIntentError::InvalidCandidate(_))
        ));
    }

    #[test]
    fn property_access_and_mutation_class_are_authoritative() {
        let target = RuntimeRef::ConfigurationProperty {
            owner: ConfigurationOwner::Instrument {
                instrument: "1".into(),
            },
            property: "limit".into(),
        };
        let mut model = model(&["property_configure"]);
        observe(
            &mut model,
            target.clone(),
            json!({"owner":{"kind":"instrument","id":"1"},
            "property":"limit","value_type":"integer","current":1,"access":"read_only",
            "revision":"3","mutation_class":"ordinary_live"}),
        );
        assert!(matches!(
            prepare(
                &model,
                OperatorIntent::ConfigureProperty {
                    target: target.clone(),
                    value: PropertyMutationCandidate::Integer(2)
                }
            ),
            Err(OperatorIntentError::InvalidCandidate(_))
        ));
        model.observations.entities.get_mut(&target).unwrap().value["access"] = json!("read_write");
        let prepared = prepare(
            &model,
            OperatorIntent::ConfigureProperty {
                target,
                value: PropertyMutationCandidate::Integer(2),
            },
        )
        .unwrap();
        assert_eq!(prepared.args["value"], 2);
        assert_eq!(
            prepared.warning,
            Some(OperatorWarning::PropertyMutationClass(
                "ordinary_live".into()
            ))
        );
    }

    #[test]
    fn v1_property_mutation_supports_only_integer_and_text() {
        let cases = [
            (
                "integer",
                PropertyMutationCandidate::Integer(7),
                json!(7),
                "ordinary_live",
            ),
            (
                "text",
                PropertyMutationCandidate::Text("x".into()),
                json!("x"),
                "live_safe",
            ),
        ];
        for (kind, candidate, expected, mutation_class) in cases {
            let target = RuntimeRef::ConfigurationProperty {
                owner: ConfigurationOwner::Instrument {
                    instrument: "1".into(),
                },
                property: kind.into(),
            };
            let mut model = model(&["property_configure"]);
            observe(
                &mut model,
                target.clone(),
                json!({"owner":{"kind":"instrument","id":"1"},
                "property":kind,"value_type":kind,"current":expected,"access":"read_write",
                "revision":"3","mutation_class":mutation_class}),
            );
            assert_eq!(
                prepare(
                    &model,
                    OperatorIntent::ConfigureProperty {
                        target,
                        value: candidate
                    }
                )
                .unwrap()
                .args["value"],
                expected
            );
        }

        for (kind, access, candidate) in [
            (
                "number",
                "deployment_only",
                PropertyMutationCandidate::Integer(1),
            ),
            (
                "boolean",
                "read_only",
                PropertyMutationCandidate::Integer(1),
            ),
        ] {
            let target = RuntimeRef::ConfigurationProperty {
                owner: ConfigurationOwner::Instrument {
                    instrument: "1".into(),
                },
                property: kind.into(),
            };
            let mut model = model(&["property_configure"]);
            observe(
                &mut model,
                target.clone(),
                json!({"owner":{"kind":"instrument","id":"1"},"property":kind,
                    "value_type":kind,"current":null,"access":access,"revision":"3"}),
            );
            assert!(matches!(
                prepare(
                    &model,
                    OperatorIntent::ConfigureProperty {
                        target,
                        value: candidate
                    }
                ),
                Err(OperatorIntentError::InvalidCandidate(_))
            ));
        }
    }

    #[test]
    fn resource_and_recorder_capture_authoritative_generation_and_run() {
        let mut model = model(&["reconnect_resource", "recording_stop"]);
        model.observations.observe_resource_full(
            "5".into(),
            json!({"resource":"5","binding_generation":"4","transport_generation":"9",
                "capabilities":{"reconnect":true}}),
        );
        model.observations.complete_rebuild();
        observe(
            &mut model,
            RuntimeRef::Recorder,
            json!({"state":"recording","active_run":{"boot_id":"0123456789abcdef0123456789abcdef","run_no":"4"}}),
        );
        let submitter = FakeSubmitter::default();
        let mut workflow = OperatorWorkflow::default();
        workflow
            .begin(
                &model,
                OperatorIntent::ReconnectResource {
                    resource: "5".into(),
                },
            )
            .unwrap();
        let OperatorWorkflowState::AwaitingConfirmation(prepared) = &workflow.state else {
            panic!()
        };
        assert_eq!(prepared.args["expected_binding_generation"], "4");
        assert_eq!(
            prepared.authority,
            AuthorityGuard::ResourceDetail {
                resource: "5".into(),
                transport_generation: json!("9"),
            }
        );
        assert_eq!(prepared.warning, Some(OperatorWarning::ResourceReconnect));
        assert!(prepared.is_current(&model));

        model.observations.observe_resource_event(
            "5".into(),
            json!({"state":"recovering","queue_len":0,"generation":"10",
                "active":null,"latest":null}),
            1,
        );
        let resource = RuntimeRef::Resource {
            resource: "5".into(),
        };
        assert_eq!(
            model.observations.entities[&resource].freshness,
            Freshness::Fresh
        );
        assert_eq!(
            model.observations.entities[&resource].value["binding_generation"],
            "4"
        );
        assert!(!model.resource_detail_is_fresh("5"));
        let OperatorWorkflowState::AwaitingConfirmation(prepared) = &workflow.state else {
            panic!()
        };
        assert!(!prepared.is_current(&model));
        assert_eq!(
            workflow.confirm(&model, &submitter),
            Err(OperatorIntentError::DraftStale)
        );
        assert!(submitter.sent.borrow().is_empty());

        let mut blocked = OperatorWorkflow::default();
        assert_eq!(
            blocked.begin(
                &model,
                OperatorIntent::ReconnectResource {
                    resource: "5".into()
                }
            ),
            Err(OperatorIntentError::TargetNotFresh)
        );
        assert!(
            submitter.sent.borrow().is_empty(),
            "stale full detail must reject before mutation submission"
        );
        model.observations.observe_resource_full(
            "5".into(),
            json!({"resource":"5","binding_generation":"4","transport_generation":"10",
                "capabilities":{"reconnect":true}}),
        );
        assert!(model.resource_detail_is_fresh("5"));
        let OperatorWorkflowState::AwaitingConfirmation(prepared) = &workflow.state else {
            panic!()
        };
        assert!(
            !prepared.is_current(&model),
            "old authority version must remain stale"
        );
        assert_eq!(
            workflow.confirm(&model, &submitter),
            Err(OperatorIntentError::DraftStale)
        );
        assert!(submitter.sent.borrow().is_empty());
        workflow.cancel_or_acknowledge();
        workflow
            .begin(
                &model,
                OperatorIntent::ReconnectResource {
                    resource: "5".into(),
                },
            )
            .unwrap();
        let OperatorWorkflowState::AwaitingConfirmation(prepared) = &workflow.state else {
            panic!()
        };
        assert_eq!(prepared.args["expected_binding_generation"], "4");
        assert_eq!(
            prepared.authority,
            AuthorityGuard::ResourceDetail {
                resource: "5".into(),
                transport_generation: json!("10"),
            }
        );
        workflow.confirm(&model, &submitter).unwrap();
        assert_eq!(
            submitter.sent.borrow().as_slice(),
            &[(
                "reconnect_resource".into(),
                json!({"resource":"5","expected_binding_generation":"4"})
            )]
        );

        let stop = prepare(&model, OperatorIntent::StopRecording).unwrap();
        assert_eq!(stop.args["run_id"]["run_no"], "4");
        assert_eq!(stop.warning, Some(OperatorWarning::RecorderStop));
    }

    #[derive(Default)]
    struct FakeSubmitter {
        result: RefCell<Option<Result<u64, CommandSendError>>>,
        sent: RefCell<Vec<(String, Value)>>,
    }

    impl MutationSubmitter for FakeSubmitter {
        fn submit_mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
            self.sent.borrow_mut().push((op.into(), args));
            self.result.borrow_mut().take().unwrap_or(Ok(7))
        }
    }

    fn reference_workflow() -> (WorkbenchModel, OperatorWorkflow) {
        let mut model = model(&["reference_retune"]);
        observe(
            &mut model,
            RuntimeRef::Reference {
                reference: "1".into(),
            },
            json!({"reference":"1","revision":"7","value":3.0,"kind":"ramp"}),
        );
        let mut workflow = OperatorWorkflow::default();
        workflow
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: 9.0,
                    rate: 2.0,
                },
            )
            .unwrap();
        (model, workflow)
    }

    #[test]
    fn submission_accepted_and_completed_are_distinct() {
        let (model, mut workflow) = reference_workflow();
        let sender = FakeSubmitter::default();
        assert_eq!(workflow.confirm(&model, &sender).unwrap(), 7);
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::Submitted { .. }
        ));
        workflow.after_update(&ClientUpdate::Reply {
            command_id: 7,
            msg_id: "1".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationAccepted,
            envelope: json!({"state":"accepted"}),
            recovery: None,
        });
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::Accepted { .. }
        ));
        workflow.after_update(&ClientUpdate::Reply {
            command_id: 7,
            msg_id: "1".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationCompleted,
            envelope: json!({"state":"completed"}),
            recovery: None,
        });
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::Completed { .. }
        ));
    }

    #[test]
    fn busy_submission_never_claims_submitted() {
        let (model, mut workflow) = reference_workflow();
        let sender = FakeSubmitter {
            result: RefCell::new(Some(Err(CommandSendError::Busy))),
            ..Default::default()
        };
        assert_eq!(
            workflow.confirm(&model, &sender),
            Err(OperatorIntentError::LocalSubmission(CommandSendError::Busy))
        );
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::AwaitingConfirmation(_)
        ));
    }

    #[test]
    fn full_nonterminal_action_bookkeeping_rejects_before_wire_submission() {
        let (mut model, mut workflow) = reference_workflow();
        for command_id in 0..crate::model::MAX_OPERATOR_ACTIONS as u64 {
            model.track_operator_intent(command_id).unwrap();
        }
        let sender = FakeSubmitter::default();
        assert_eq!(
            workflow.confirm(&model, &sender),
            Err(OperatorIntentError::ActionCapacityFull)
        );
        assert!(sender.sent.borrow().is_empty());
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::AwaitingConfirmation(_)
        ));
    }

    #[test]
    fn revision_conflict_fails_without_retry_or_observation_fabrication() {
        let (model, mut workflow) = reference_workflow();
        let before = model.observations.entities.clone();
        let sender = FakeSubmitter::default();
        workflow.confirm(&model, &sender).unwrap();
        workflow.after_update(&ClientUpdate::Reply {
            command_id: 7,
            msg_id: "1".into(),
            op: "reference_retune".into(),
            kind: ReplyKind::PublicError,
            envelope: json!({"code":"revision_conflict"}),
            recovery: None,
        });
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::Failed { conflict: true, .. }
        ));
        assert_eq!(sender.sent.borrow().len(), 1);
        assert_eq!(model.observations.entities, before);
    }

    #[test]
    fn ambiguous_update_preserves_exact_prepared_payload() {
        let (model, mut workflow) = reference_workflow();
        let sender = FakeSubmitter::default();
        workflow.confirm(&model, &sender).unwrap();
        workflow.after_update(&ClientUpdate::TransportFailure {
            reason: "lost".into(),
        });
        let OperatorWorkflowState::Ambiguous { prepared, .. } = &workflow.state else {
            panic!()
        };
        assert_eq!(prepared.args["expected_revision"], "7");
        assert_eq!(sender.sent.borrow().len(), 1);
    }

    #[test]
    fn boot_change_invalidates_reference_and_state_guarded_confirmations() {
        let (mut reference_model, mut reference_workflow) = reference_workflow();
        let mut next_hello = reference_model.hello.clone().unwrap();
        next_hello.boot_id = "boot-b".into();
        next_hello.event_oldest.boot_id = "boot-b".into();
        next_hello.event_latest.boot_id = "boot-b".into();
        reference_model.apply_client_update(ClientUpdate::Hello(next_hello));
        observe(
            &mut reference_model,
            RuntimeRef::Reference {
                reference: "1".into(),
            },
            json!({"reference":"1","revision":"7","value":3.0,"kind":"ramp"}),
        );
        let sender = FakeSubmitter::default();
        assert_eq!(
            reference_workflow.confirm(&reference_model, &sender),
            Err(OperatorIntentError::DraftStale)
        );
        assert!(sender.sent.borrow().is_empty());

        let mut controller_model = model(&["controller_start"]);
        observe(
            &mut controller_model,
            RuntimeRef::Controller {
                controller: "4".into(),
            },
            json!({"controller":"4","state":"ready","revision":"3"}),
        );
        let mut controller_workflow = OperatorWorkflow::default();
        controller_workflow
            .begin(
                &controller_model,
                OperatorIntent::ControllerLifecycle {
                    controller: "4".into(),
                    action: ControllerLifecycleIntent::Start,
                },
            )
            .unwrap();
        let mut next_hello = controller_model.hello.clone().unwrap();
        next_hello.boot_id = "boot-b".into();
        next_hello.event_oldest.boot_id = "boot-b".into();
        next_hello.event_latest.boot_id = "boot-b".into();
        controller_model.apply_client_update(ClientUpdate::Hello(next_hello));
        observe(
            &mut controller_model,
            RuntimeRef::Controller {
                controller: "4".into(),
            },
            json!({"controller":"4","state":"ready","revision":"3"}),
        );
        let sender = FakeSubmitter::default();
        assert_eq!(
            controller_workflow.confirm(&controller_model, &sender),
            Err(OperatorIntentError::DraftStale)
        );
        assert!(sender.sent.borrow().is_empty());
    }
}
