//! Manual output admission and bounded delivery correlation, never output ownership.

use super::{
    Application,
    common::{domain_code, float_field, id_field, operation_reply, operation_state},
    operations::recorded_terminal,
    projections::{manual_attempt_json, output_json},
};
use crate::{
    host::Clock,
    recorder::OperationRecord,
    service::ServiceHost,
    sessions::{Mutation, OperationState},
    wire::WireRequestId,
};
use lab_core::{
    Command, CommandResult, InstrumentId, ParameterId, Query, QueryResult,
    output::{ActuatorId, DispatchOutcome, ManualOutputAttempt, ManualOutputCommand},
};
use serde_json::{Value, json};
use std::time::Duration;

// This bounds Application correlation even if a transport never settles. Expiry
// means outcome unknown, not cancellation, absence of effect, or permission to retry.
const COMPLETION_WAIT: Duration = Duration::from_secs(30);

pub(super) struct PendingOutput {
    connection: u64,
    request_id: WireRequestId,
    message: String,
    instance: u64,
    attempt: u64,
    admitted: Duration,
}

pub(super) fn decode(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    let actuator = args.get("actuator").ok_or("invalid_args")?;
    let action = match operation {
        "output_acquire" => ManualOutputCommand::Acquire {
            lifetime: Duration::from_nanos(id_field(args, "lease_lifetime_ns")?),
        },
        "output_renew" => ManualOutputCommand::Renew {
            lifetime: Duration::from_nanos(id_field(args, "lease_lifetime_ns")?),
        },
        "output_write" => ManualOutputCommand::Write {
            value: lab_core::Value::Float(float_field(args, "value")?),
            ttl: Duration::from_nanos(id_field(args, "proposal_ttl_ns")?),
        },
        "output_safe" => ManualOutputCommand::Safe,
        _ => return Err("unsupported_operation"),
    };
    Ok(Mutation::ManualOutput {
        instrument: id_field(actuator, "instrument")?,
        parameter: id_field(actuator, "parameter")?,
        expected_instance: id_field(args, "expected_instance")?,
        expected_epoch: id_field(args, "expected_epoch")?,
        action,
    })
}

pub(super) fn recorded_intent(mutation: &Mutation) -> (&'static str, String) {
    let Mutation::ManualOutput {
        instrument,
        parameter,
        expected_instance,
        expected_epoch,
        action,
    } = mutation
    else {
        unreachable!("manual mutation")
    };
    let mut fields = json!({"actuator":{"instrument":instrument.to_string(),"parameter":parameter.to_string()},
        "expected_instance":expected_instance.to_string(),"expected_epoch":expected_epoch.to_string()});
    let name = match action {
        ManualOutputCommand::Acquire { lifetime } | ManualOutputCommand::Renew { lifetime } => {
            fields["lease_lifetime_ns"] = json!(lifetime.as_nanos().to_string());
            if matches!(action, ManualOutputCommand::Acquire { .. }) {
                "output_acquire"
            } else {
                "output_renew"
            }
        }
        ManualOutputCommand::Write { value, ttl } => {
            fields["value"] = match value {
                lab_core::Value::Float(value) => json!(value),
                _ => Value::Null,
            };
            fields["proposal_ttl_ns"] = json!(ttl.as_nanos().to_string());
            "output_write"
        }
        ManualOutputCommand::Safe => "output_safe",
    };
    (name, fields.to_string())
}

fn delivery_outcome(attempt: ManualOutputAttempt) -> OperationState {
    let successful = matches!(
        attempt.outcome,
        Some(DispatchOutcome::Acknowledged | DispatchOutcome::ReadbackVerified)
    );
    let result = manual_attempt_json(attempt).to_string();
    if successful {
        OperationState::Completed(result)
    } else {
        OperationState::FailedWithResult {
            code: "output_rejected".into(),
            detail: result,
        }
    }
}

impl Application {
    pub(super) fn handle_output_mutation(
        &mut self,
        service: &mut ServiceHost,
        connection: u64,
        payload: Mutation,
        request_id: WireRequestId,
        message: String,
    ) -> Vec<Value> {
        let accepted = operation_reply(&message, &request_id, OperationState::Accepted);
        let (operation, _) = recorded_intent(&payload);
        let Mutation::ManualOutput {
            instrument,
            parameter,
            expected_instance,
            expected_epoch,
            action,
        } = payload
        else {
            unreachable!("manual mutation")
        };
        let actuator = ActuatorId::new(InstrumentId::new(instrument), ParameterId::new(parameter));
        let at = service.clock().now();
        // The attached, server-issued scope was validated before dedup/admission.
        // Its boot-local nonwrapping counter is the existing Manual owner ID.
        let owner = request_id
            .scope
            .rsplit_once(':')
            .and_then(|(_, counter)| counter.parse::<u64>().ok())
            .expect("server-issued attached scope");
        let safe = matches!(action, ManualOutputCommand::Safe);
        let write = matches!(action, ManualOutputCommand::Write { .. });
        let prior_attempt = match service.owner().query(Query::Output(actuator)) {
            Ok(QueryResult::Output(snapshot)) => snapshot.manual_attempt.map(|attempt| attempt.id),
            _ => None,
        };
        let result = if self.pending_outputs.contains_key(&actuator) && !safe {
            Err(lab_core::output::OutputError::Busy.into())
        } else {
            service.owner_mut().command_with_cause(
                Command::ManualOutput {
                    actuator,
                    owner,
                    expected_instance,
                    expected_epoch,
                    command: action,
                    at,
                },
                Some((request_id.scope.clone(), request_id.seq)),
            )
        };
        // Host publication can fail after Core admitted or completed a write.
        // Retain its actual receipt instead of reporting a pre-effect failure or
        // abandoning an accepted physical operation that still owns its queue slot.
        let result = if write && result.is_err() {
            match service.owner().query(Query::Output(actuator)) {
                Ok(QueryResult::Output(snapshot))
                    if snapshot.instance == expected_instance
                        && snapshot
                            .manual_attempt
                            .as_ref()
                            .is_some_and(|attempt| Some(attempt.id) != prior_attempt) =>
                {
                    Ok(CommandResult::ManualOutput(snapshot))
                }
                _ => result,
            }
        } else {
            result
        };
        let outcome = match result {
            Ok(CommandResult::ManualOutput(snapshot)) if write => {
                let attempt = snapshot.manual_attempt.expect("successful manual write has receipt");
                if !attempt.terminal {
                    self.pending_outputs.insert(actuator, PendingOutput {connection, request_id, message,
                        instance: snapshot.instance, attempt: attempt.id, admitted: at});
                    return vec![accepted];
                }
                delivery_outcome(attempt)
            }
            Ok(CommandResult::ManualOutput(snapshot)) => OperationState::Completed(json!({
                "output":output_json(snapshot),"safe_requested":safe,
                "completion_basis":if safe {"authority_revoked_safe_requested"} else {"authority_transition"}
            }).to_string()),
            Ok(_) => unreachable!("manual command result"),
            Err(error) => OperationState::Failed(domain_code(error).into()),
        };
        self.finish_output(service, &request_id, operation, outcome.clone());
        vec![accepted, operation_reply(&message, &request_id, outcome)]
    }

    /// Poll retained manual delivery receipts without sending, renewing or retrying output.
    pub fn poll_outputs(&mut self, service: &mut ServiceHost) -> Vec<(u64, Value)> {
        let at = service.clock().now();
        let mut completed = Vec::new();
        for (actuator, pending) in &self.pending_outputs {
            let current = service.owner().query(Query::Output(*actuator));
            let attempt = match current {
                Ok(QueryResult::Output(snapshot)) if snapshot.instance == pending.instance => {
                    snapshot
                        .manual_attempt
                        .filter(|attempt| attempt.id == pending.attempt)
                }
                _ => None,
            };
            let outcome = match attempt {
                Some(attempt) if attempt.terminal => Some(delivery_outcome(attempt)),
                Some(_) if at.saturating_sub(pending.admitted) < COMPLETION_WAIT => None,
                attempt => Some(OperationState::FailedWithResult { code: if attempt.is_some() {"timeout"} else {"instance_changed"}.into(),
                    detail: json!({"attempt_id":pending.attempt.to_string(),"delivery":attempt.map(manual_attempt_json),
                        "outcome":"unknown",
                        "reason":"delivery_not_settled_or_authority_replaced"}).to_string() }),
            };
            if let Some(outcome) = outcome {
                completed.push((*actuator, outcome));
            }
        }
        let mut replies = Vec::new();
        for (actuator, outcome) in completed {
            let pending = self
                .pending_outputs
                .remove(&actuator)
                .expect("retained output correlation");
            self.finish_output(
                service,
                &pending.request_id,
                "output_write",
                outcome.clone(),
            );
            replies.push((
                pending.connection,
                operation_reply(&pending.message, &pending.request_id, outcome),
            ));
        }
        replies
    }

    fn finish_output(
        &mut self,
        service: &mut ServiceHost,
        request_id: &WireRequestId,
        command: &'static str,
        outcome: OperationState,
    ) {
        let at = service.clock().now();
        self.sessions
            .complete(&request_id.scope, request_id.seq, outcome.clone(), at)
            .expect("bounded terminal for admitted output operation");
        let (phase, data) = recorded_terminal(&outcome);
        service.owner_mut().record_operation(OperationRecord {
            scope: request_id.scope.clone(),
            request_seq: request_id.seq,
            command,
            phase,
            data,
            outcome_basis: "domain_result",
            at,
        });
        if service
            .owner_mut()
            .event_log_mut()
            .operation_terminal(
                at,
                &request_id.scope,
                request_id.seq,
                operation_state(outcome),
            )
            .is_err()
        {
            let _ = service.request_shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        service::ServiceOptions,
        wire::{decode_frame, encode_frame},
    };

    fn ask(app: &mut Application, service: &mut ServiceHost, request: Value) -> Vec<Value> {
        app.handle(
            service,
            1,
            decode_frame(&encode_frame(&request).unwrap()).unwrap(),
        )
    }

    #[test]
    fn publication_failure_after_delivery_preserves_the_actual_operation_receipt() {
        let mut service = ServiceHost::startup(
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap(),
        )
        .unwrap();
        let mut app = Application::new(service.boot_id()).unwrap();
        let hello = ask(
            &mut app,
            &mut service,
            json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
        );
        let scope = hello[0]["result"]["scope"].clone();
        let actuator = json!({"instrument":"1","parameter":"2"});
        let before = ask(
            &mut app,
            &mut service,
            json!({"v":1,"msg_id":"q","op":"output","args":{"actuator":actuator}}),
        )[0]["result"]
            .clone();
        let acquired = ask(
            &mut app,
            &mut service,
            json!({"v":1,"msg_id":"a","op":"output_acquire","request_id":{"scope":scope,"seq":"1"},"args":{"actuator":actuator,"expected_instance":before["authority_instance"],"expected_epoch":before["epoch"],"lease_lifetime_ns":"2000000000"}}),
        );
        assert_eq!(acquired[1]["state"], "completed");
        let state = acquired[1]["result"]["output"].clone();
        service
            .owner_mut()
            .event_log_mut()
            .force_sequence_for_test(u64::MAX);
        let delivered = ask(
            &mut app,
            &mut service,
            json!({"v":1,"msg_id":"w","op":"output_write","request_id":{"scope":scope,"seq":"2"},"args":{"actuator":actuator,"expected_instance":state["authority_instance"],"expected_epoch":state["epoch"],"value":20.0,"proposal_ttl_ns":"100000000"}}),
        );
        assert_eq!(delivered[1]["state"], "completed", "{delivered:?}");
        assert_eq!(delivered[1]["result"]["readback"]["value"], 20.0);
        assert_eq!(
            delivered[1]["result"]["evidence_source"],
            "virtual_simulation"
        );
    }
}
