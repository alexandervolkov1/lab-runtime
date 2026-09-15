//! The serialized service owner maps fixed wire operations to pure Core queries
//! or bounded mutations. Accepted records precede dispatch; terminal records
//! precede any attempt to send a reply.

use crate::{
    host::Clock,
    service::ServiceHost,
    sessions::{Admission, Mutation, OperationState, SessionError, SessionStore},
    wire::{WireRequest, WireRequestId, decimal_u64},
};
use lab_core::control::{
    ControllerError, ControllerId, ControllerSnapshot, ControllerState, PidConfig,
};
use lab_core::reference::{ReferenceId, ReferenceSnapshot};
use lab_core::{
    Command, CommandResult, Error, InstrumentId, ParameterId, Query, QueryResult, SignalId,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Single-owner fixed API state; a TCP connection carries no domain authority.
pub struct Application {
    sessions: SessionStore,
    clients: BTreeMap<u64, String>,
}
impl Application {
    /// Create bounded process-local coordination state for one fresh boot.
    pub fn new(boot_id: &str) -> Result<Self, SessionError> {
        Ok(Self {
            sessions: SessionStore::new(boot_id)?,
            clients: BTreeMap::new(),
        })
    }

    /// Detach a connection while retaining already admitted operation outcomes.
    pub fn detach(&mut self, service: &ServiceHost, connection: u64) {
        self.clients.remove(&connection);
        self.sessions.detach(connection, service.clock().now());
    }

    /// Process one parsed request in owner order. Replies are owned and bounded by
    /// the network encoder; a disconnected client cannot roll back a mutation.
    pub fn handle(
        &mut self,
        service: &mut ServiceHost,
        connection: u64,
        request: WireRequest,
    ) -> Vec<Value> {
        let msg = request.msg_id.clone();
        let result = if request.op == "hello" {
            if self.clients.contains_key(&connection) {
                Err("already_hello")
            } else {
                let old = request.args.get("scope").and_then(Value::as_str);
                if request
                    .args
                    .get("scope")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
                {
                    Err("invalid_args")
                } else {
                    self.sessions
                        .open(old, connection, service.clock().now())
                        .map_err(session_code)
                        .map(|opened| {
                            self.clients.insert(connection, opened.scope.clone());
                            json!({"boot_id":service.boot_id(),"v":1,"scope":opened.scope,
                                "next_seq":opened.next_seq.to_string(),"state":"ready",
                                "capabilities":["virtual","native_controller","ramp_reference"],
                                "limits":{"clients":8,"scopes":16,"frame_bytes":16384},
                                "event_oldest":{"boot_id":service.boot_id(),"seq":"0"},
                                "event_latest":{"boot_id":service.boot_id(),"seq":"0"}})
                        })
                }
            }
        } else if !self.clients.contains_key(&connection) {
            Err("hello_required")
        } else if request.request_id.is_some() {
            return self.handle_mutation(service, connection, request);
        } else {
            self.handle_query(service, connection, &request)
        };
        vec![match result {
            Ok(value) => json!({"v":1,"msg_id":msg,"type":"result","result":value}),
            Err(code) => error_reply(&msg, code),
        }]
    }

    fn handle_query(
        &self,
        service: &ServiceHost,
        connection: u64,
        request: &WireRequest,
    ) -> Result<Value, &'static str> {
        let args = &request.args;
        let scope = self.clients.get(&connection).ok_or("hello_required")?;
        let owner = service.owner();
        let result = match request.op.as_str() {
            "reference" => {
                let id = id_field(args, "reference")?;
                match owner
                    .query(Query::Reference(ReferenceId::new(id)))
                    .map_err(domain_code)?
                {
                    QueryResult::Reference(snap) => reference_json(id, snap),
                    _ => return Err("internal_error"),
                }
            }
            "controller" => {
                let id = id_field(args, "controller")?;
                match owner
                    .query(Query::Controller(ControllerId::new(id)))
                    .map_err(domain_code)?
                {
                    QueryResult::Controller(snap) => controller_json(snap),
                    _ => return Err("internal_error"),
                }
            }
            "latest" => {
                let signal = args.get("signal").ok_or("invalid_args")?;
                let instrument = id_field(signal, "instrument")?;
                let parameter = id_field(signal, "parameter")?;
                match owner
                    .query(Query::GetLatestSignal(SignalId::new(
                        InstrumentId::new(instrument),
                        ParameterId::new(parameter),
                    )))
                    .map_err(domain_code)?
                {
                    QueryResult::Latest(sample) => match sample {
                        None => Value::Null,
                        Some(sample) => {
                            json!({"value":sample_value(&sample),"quality":quality_name(sample.quality()),
                            "observed_at":nanos(sample.at()),"unit":{"id":sample.unit().id().to_string(),"symbol":sample.unit().symbol()}})
                        }
                    },
                    _ => return Err("internal_error"),
                }
            }
            "operation_status" => {
                let rid = parse_rid(args.get("request_id").ok_or("invalid_args")?)?;
                if rid.scope != *scope {
                    return Err("scope_unknown");
                }
                operation_status(self.sessions.status(scope, rid.seq))
            }
            "discover" => {
                let QueryResult::Instruments(instruments) =
                    owner.query(Query::Discover).map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let list: Vec<_> = instruments
                    .iter()
                    .map(|d| json!({"id":d.id.get().to_string(),"name":d.name}))
                    .collect();
                json!({"instruments":list,"controllers":[{"id":"1","kind":"native_pid"}],
                    "references":[{"id":"1","kind":"ramp"}],"components":[],
                    "outputs":[{"instrument":"1","parameter":lab_core::HEATER_POWER.get().to_string()}]})
            }
            _ => return Err("unsupported_operation"),
        };
        Ok(result)
    }

    fn handle_mutation(
        &mut self,
        service: &mut ServiceHost,
        connection: u64,
        request: WireRequest,
    ) -> Vec<Value> {
        let msg = request.msg_id;
        let Some(scope) = self.clients.get(&connection).cloned() else {
            return vec![error_reply(&msg, "hello_required")];
        };
        let Some(rid) = request.request_id else {
            return vec![error_reply(&msg, "invalid_args")];
        };
        if rid.scope != scope {
            return vec![error_reply(&msg, "scope_unknown")];
        }
        let payload = match typed_mutation(&request.op, &request.args) {
            Ok(payload) => payload,
            Err(code) => return vec![error_reply(&msg, code)],
        };
        let now = service.clock().now();
        match self.sessions.admit(&scope, rid.seq, payload.clone(), now) {
            Admission::Known(state) => return vec![operation_reply(&msg, &rid, state)],
            Admission::Conflict => return vec![error_reply(&msg, "request_id_conflict")],
            Admission::Unknown => return vec![error_reply(&msg, "outcome_unknown")],
            Admission::Gap => return vec![error_reply(&msg, "sequence_gap")],
            Admission::Busy => return vec![error_reply(&msg, "busy")],
            Admission::ScopeUnknown => return vec![error_reply(&msg, "scope_unknown")],
            Admission::Accepted => {}
        }
        let accepted = operation_reply(&msg, &rid, OperationState::Accepted);
        let outcome = dispatch(service, payload).map_or_else(
            |error| OperationState::Failed(domain_code(error).into()),
            |value| OperationState::Completed(value.to_string()),
        );
        // The terminal record is committed even if the connection closes before delivery.
        self.sessions
            .complete(&scope, rid.seq, outcome.clone(), service.clock().now())
            .expect("bounded typed terminal record for admitted scope");
        vec![accepted, operation_reply(&msg, &rid, outcome)]
    }
}

fn typed_mutation(op: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match op {
        "reference_retune" => Mutation::RetuneRamp {
            reference: id_field(args, "reference")?,
            expected_revision: id_field(args, "expected_revision")?,
            target: float_field(args, "target")?,
            rate: float_field(args, "rate")?,
        },
        "controller_configure_pid" => {
            let pid = args.get("pid").ok_or("invalid_args")?;
            Mutation::ConfigurePid {
                controller: id_field(args, "controller")?,
                expected_revision: id_field(args, "expected_revision")?,
                kp: float_field(pid, "kp")?,
                ki: float_field(pid, "ki")?,
                kd: float_field(pid, "kd")?,
                output_min: float_field(pid, "output_min")?,
                output_max: float_field(pid, "output_max")?,
            }
        }
        "controller_start" => Mutation::Start {
            controller: id_field(args, "controller")?,
        },
        "controller_pause" => Mutation::Pause {
            controller: id_field(args, "controller")?,
        },
        "controller_resume" => Mutation::Resume {
            controller: id_field(args, "controller")?,
        },
        "runtime_shutdown" => Mutation::Shutdown,
        _ => return Err("unsupported_operation"),
    })
}
fn dispatch(service: &mut ServiceHost, mutation: Mutation) -> Result<Value, Error> {
    let at = service.clock().now();
    let command = match mutation {
        Mutation::RetuneRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => Command::RetuneRampReference {
            reference: ReferenceId::new(reference),
            expected_revision,
            target,
            rate,
            at,
        },
        Mutation::ConfigurePid {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
        } => Command::ConfigureControllerPid {
            controller: ControllerId::new(controller),
            expected_revision,
            pid: PidConfig {
                kp,
                ki,
                kd,
                output_min,
                output_max,
            },
        },
        Mutation::Start { controller } => Command::StartController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Pause { controller } => Command::PauseController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Resume { controller } => Command::ResumeController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Shutdown => {
            service.request_shutdown()?;
            return Ok(json!({"state":"stopping"}));
        }
    };
    match service.owner_mut().command(command)? {
        CommandResult::ReferenceRetuned(retuned) => Ok(
            json!({"reference":"1","revision":retuned.revision.to_string(),
            "value":retuned.state.current,"target":retuned.state.target,"rate":retuned.state.rate,
            "committed_at":nanos(retuned.state.last_at)}),
        ),
        CommandResult::ControllerUpdated(snapshot) => Ok(controller_json(snapshot)),
        _ => Err(Error::InvalidConfiguration("unexpected command result")),
    }
}
fn parse_rid(value: &Value) -> Result<WireRequestId, &'static str> {
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
        scope: scope.into(),
        seq,
    })
}
fn id_field(args: &Value, field: &str) -> Result<u64, &'static str> {
    args.get(field)
        .and_then(Value::as_str)
        .and_then(decimal_u64)
        .ok_or("invalid_args")
}
fn float_field(args: &Value, field: &str) -> Result<f64, &'static str> {
    args.get(field)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or("invalid_args")
}
fn controller_json(s: ControllerSnapshot) -> Value {
    json!({"controller":s.id.get().to_string(),"state":state_name(s.state),
        "revision":s.config_revision.to_string(),"last_tick":s.last_tick.map(nanos),
        "latest_output":s.latest_output.map(|v|v.output)})
}
fn reference_json(id: u64, s: ReferenceSnapshot) -> Value {
    match s {
        ReferenceSnapshot::Fixed {
            value,
            unit,
            revision,
            ..
        } => json!({"reference":id.to_string(),"kind":"fixed","value":value,
            "revision":revision.to_string(),"unit":{"id":unit.id().to_string(),"symbol":unit.symbol()}}),
        ReferenceSnapshot::Ramp {
            state, revision, ..
        } => json!({"reference":id.to_string(),"kind":"ramp","value":state.current,
            "target":state.target,"rate":state.rate,"revision":revision.to_string(),"last_at":nanos(state.last_at),
            "unit":{"id":state.unit.id().to_string(),"symbol":state.unit.symbol()}}),
    }
}
fn state_name(s: ControllerState) -> &'static str {
    match s {
        ControllerState::Created => "created",
        ControllerState::Ready => "ready",
        ControllerState::Warming => "warming",
        ControllerState::Running => "running",
        ControllerState::Paused => "paused",
        ControllerState::Failed => "failed",
    }
}
fn quality_name(s: lab_core::SampleQuality) -> &'static str {
    match s {
        lab_core::SampleQuality::Good => "good",
        lab_core::SampleQuality::Unavailable => "unavailable",
    }
}
fn sample_value(sample: &lab_core::Sample) -> Value {
    match sample.value() {
        Some(lab_core::Value::Float(v)) => json!(v),
        _ => Value::Null,
    }
}
fn nanos(at: std::time::Duration) -> String {
    at.as_nanos().to_string()
}
fn operation_status(a: Admission) -> Value {
    match a {
        Admission::Known(s) => operation_state(s),
        _ => json!({"state":"outcome_unknown"}),
    }
}
fn operation_state(s: OperationState) -> Value {
    match s {
        OperationState::Accepted => json!({"state":"accepted"}),
        OperationState::Completed(text) => {
            json!({"state":"completed","result":serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)})
        }
        OperationState::Failed(code) => json!({"state":"failed","code":code}),
    }
}
fn operation_reply(msg: &str, rid: &WireRequestId, s: OperationState) -> Value {
    let mut value = json!({"v":1,"msg_id":msg,"type":"operation","request_id":{"scope":rid.scope,"seq":rid.seq.to_string()}});
    let state = operation_state(s);
    for (key, value_part) in state.as_object().expect("fixed state object") {
        value[key] = value_part.clone();
    }
    value
}
fn error_reply(msg: &str, code: &str) -> Value {
    json!({"v":1,"msg_id":msg,"type":"error","code":code,"message":code,"accepted":false})
}
fn session_code(e: SessionError) -> &'static str {
    match e {
        SessionError::InstanceChanged => "instance_changed",
        SessionError::ScopeInUse => "scope_in_use",
        SessionError::ScopeCapacity => "busy",
        _ => "scope_unknown",
    }
}
fn domain_code(e: Error) -> &'static str {
    match e {
        Error::Controller(ControllerError::RevisionConflict) => "revision_conflict",
        Error::Controller(ControllerError::UnknownController) => "unknown_controller",
        Error::Controller(ControllerError::InvalidState) => "invalid_state",
        Error::Controller(ControllerError::InputUnavailable) => "input_unavailable",
        Error::Controller(ControllerError::StaleInput) => "stale_input",
        Error::Controller(ControllerError::Output) => "output_rejected",
        Error::UnknownInstrument(_) => "unknown_instrument",
        Error::UnknownParameter { .. } => "unknown_parameter",
        Error::UnknownSignal(_) => "unknown_signal",
        Error::InvalidConfiguration(_) => "invalid_configuration",
        _ => "domain_rejected",
    }
}
