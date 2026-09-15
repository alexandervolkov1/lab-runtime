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
use lab_core::managed::{ComponentId, ComponentState};
use lab_core::output::{ActuatorId, DispatchOutcome, OutputOwner, OutputSnapshot, OutputState};
use lab_core::reference::{ReferenceId, ReferenceSnapshot};
use lab_core::{
    AccessMode, Command, CommandResult, Error, InstrumentId, ParameterDescriptor, ParameterId,
    ParameterRole, Query, QueryResult, SignalId, ValueSpec, WriteEffect,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

struct FrozenSnapshot {
    token: String,
    cursor: u64,
    records: Vec<Value>,
    expires: Duration,
}
struct Subscription {
    token: String,
    scan: u64,
    kinds: Vec<String>,
    targets: Vec<FilterTarget>,
}
struct FilterTarget {
    kind: String,
    target: Value,
}
struct PendingShutdown {
    connection: u64,
    scope: String,
    seq: u64,
    msg: String,
}

/// Single-owner fixed API state; a TCP connection carries no domain authority.
pub struct Application {
    sessions: SessionStore,
    clients: BTreeMap<u64, String>,
    snapshots: BTreeMap<u64, FrozenSnapshot>,
    subscriptions: BTreeMap<u64, Subscription>,
    next_token: u64,
    pending_shutdown: Option<PendingShutdown>,
}
impl Application {
    /// Create bounded process-local coordination state for one fresh boot.
    pub fn new(boot_id: &str) -> Result<Self, SessionError> {
        Ok(Self {
            sessions: SessionStore::new(boot_id)?,
            clients: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            next_token: 1,
            pending_shutdown: None,
        })
    }

    /// Detach a connection while retaining already admitted operation outcomes.
    pub fn detach(&mut self, service: &ServiceHost, connection: u64) {
        self.clients.remove(&connection);
        self.snapshots.remove(&connection);
        self.subscriptions.remove(&connection);
        self.sessions.detach(connection, service.clock().now());
    }
    /// Expire frozen connection snapshots at a trusted monotonic owner instant.
    pub fn expire_snapshots_at(&mut self, now: Duration) {
        self.snapshots.retain(|_, s| now < s.expires);
    }
    /// Store the safe/cleanup terminal outcome before any network offer.
    pub fn finish_shutdown(
        &mut self,
        service: &mut ServiceHost,
        status: crate::host::ShutdownStatus,
    ) -> Vec<(u64, Value)> {
        let Some(pending) = self.pending_shutdown.take() else {
            return Vec::new();
        };
        let result = json!({"safe_confirmed":status.safe_confirmed,"unfinished_workers":status.unfinished_workers,
            "cleanup_complete":status.unfinished_workers==0,"exit_success":status.exit_success,
            "outputs":service.owner().output_safe_records()});
        let state = if status.exit_success {
            OperationState::Completed(result.to_string())
        } else {
            OperationState::FailedWithResult {
                code: if status.safe_confirmed {
                    "cleanup_incomplete"
                } else {
                    "safe_unconfirmed"
                }
                .into(),
                detail: result.to_string(),
            }
        };
        self.sessions
            .complete(
                &pending.scope,
                pending.seq,
                state.clone(),
                service.clock().now(),
            )
            .expect("retained admitted shutdown record");
        let event_state = operation_state(state.clone());
        let published = service.clock().now();
        let owner = service.owner_mut();
        if owner
            .event_log_mut()
            .host_state(
                published,
                if status.exit_success {
                    "stopped"
                } else {
                    "failed"
                },
                result.clone(),
            )
            .is_ok()
        {
            let _ = owner.event_log_mut().operation_terminal(
                published,
                &pending.scope,
                pending.seq,
                event_state,
            );
        }
        if self.clients.get(&pending.connection) != Some(&pending.scope) {
            return Vec::new();
        }
        let rid = WireRequestId {
            scope: pending.scope,
            seq: pending.seq,
        };
        vec![(
            pending.connection,
            operation_reply(&pending.msg, &rid, state),
        )]
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
                                "capabilities":["virtual","native_controller","ramp_reference","lua_source","lua_transform","safe_readback"],
                                "operations":["hello","discover","describe","latest","controller","reference","component","output","runtime_snapshot",
                                    "operation_status","snapshot_page","snapshot_release","subscribe","unsubscribe",
                                    "reference_retune","controller_configure_pid","controller_start","controller_pause","controller_resume","runtime_shutdown"],
                                "limits":{"clients":8,"scopes":16,"frame_bytes":16384},
                                "event_oldest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().oldest_cursor().to_string()},
                                "event_latest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().latest_cursor().to_string()}})
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
            Err("event_gap") => {
                json!({"v":1,"msg_id":msg,"type":"error","accepted":false,"code":"event_gap",
                "message":"event_gap","resync_required":true,
                "oldest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().oldest_cursor().to_string()},
                "latest":{"boot_id":service.boot_id(),"seq":service.owner().event_log().latest_cursor().to_string()}})
            }
            Err(code) => error_reply(&msg, code),
        }]
    }

    fn handle_query(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        request: &WireRequest,
    ) -> Result<Value, &'static str> {
        let args = &request.args;
        let scope = self.clients.get(&connection).ok_or("hello_required")?;
        let owner = service.owner();
        let result = match request.op.as_str() {
            "runtime_snapshot" => {
                let cursor = owner.event_log().latest_cursor();
                let mut records = owner.event_log().snapshot_records();
                records.extend(owner.resource_records());
                let QueryResult::Instruments(instruments) =
                    owner.query(Query::Discover).map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                records.insert(0,json!({"kind":"catalog","target":{"id":"runtime"},"data":{
                    "instruments":instruments.iter().map(|d|json!({"id":d.id.get().to_string(),"name":d.name})).collect::<Vec<_>>()}}));
                let footprint: usize = records
                    .iter()
                    .map(|r| {
                        serde_json::to_vec(r)
                            .map(|v| v.len() * 2 + 128)
                            .unwrap_or(usize::MAX)
                    })
                    .sum();
                if footprint > 256 * 1024
                    || records.iter().any(|r| {
                        serde_json::to_vec(r)
                            .map(|v| v.len() > 4096)
                            .unwrap_or(true)
                    })
                {
                    return Err("snapshot_capacity");
                }
                let token = self.issue_token(service.boot_id())?;
                let expires = service.clock().now() + Duration::from_secs(5);
                self.snapshots.insert(
                    connection,
                    FrozenSnapshot {
                        token: token.clone(),
                        cursor,
                        records,
                        expires,
                    },
                );
                self.page(service, connection, &token, 0)?
            }
            "snapshot_page" => {
                let token = args
                    .get("snapshot")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let index = id_field(args, "index")?;
                self.page(service, connection, token, index as usize)?
            }
            "snapshot_release" => {
                let token = args
                    .get("snapshot")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let removed = self
                    .snapshots
                    .get(&connection)
                    .is_some_and(|s| s.token == token);
                if removed {
                    self.snapshots.remove(&connection);
                }
                json!({"released":removed})
            }
            "subscribe" => {
                if self.subscriptions.contains_key(&connection) {
                    return Err("subscription_busy");
                }
                let after = args.get("after").ok_or("invalid_args")?;
                let boot = after
                    .get("boot_id")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                if boot != service.boot_id() {
                    return Err("instance_changed");
                }
                let seq = id_field(after, "seq")?;
                match owner.event_log().scan_after(seq, 1) {
                    Ok(_) => {}
                    Err(crate::events::EventError::Gap) => return Err("event_gap"),
                    Err(crate::events::EventError::Future) => return Err("invalid_cursor"),
                    Err(_) => return Err("event_error"),
                }
                let filter = args.get("filter").ok_or("invalid_args")?;
                let kinds = filter
                    .get("kinds")
                    .and_then(Value::as_array)
                    .ok_or("invalid_args")?;
                let targets = filter
                    .get("targets")
                    .and_then(Value::as_array)
                    .ok_or("invalid_args")?;
                if kinds.len() > 8 || targets.len() > 16 {
                    return Err("invalid_args");
                }
                let mut selected = Vec::new();
                for kind in kinds {
                    let name = kind.as_str().ok_or("invalid_args")?;
                    if ![
                        "signal",
                        "controller",
                        "reference",
                        "component",
                        "output",
                        "operation",
                        "host",
                    ]
                    .contains(&name)
                    {
                        return Err("invalid_args");
                    }
                    selected.push(name.to_string());
                }
                let selected_targets = targets
                    .iter()
                    .map(parse_filter_target)
                    .collect::<Result<Vec<_>, _>>()?;
                let token = self.issue_token(service.boot_id())?;
                self.subscriptions.insert(
                    connection,
                    Subscription {
                        token: token.clone(),
                        scan: seq,
                        kinds: selected,
                        targets: selected_targets,
                    },
                );
                json!({"subscription":token,"accepted_cursor":{"boot_id":service.boot_id(),"seq":seq.to_string()}})
            }
            "unsubscribe" => {
                let token = args
                    .get("subscription")
                    .and_then(Value::as_str)
                    .ok_or("invalid_args")?;
                let removed = self
                    .subscriptions
                    .get(&connection)
                    .is_some_and(|s| s.token == token);
                if removed {
                    self.subscriptions.remove(&connection);
                }
                json!({"removed":removed})
            }
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
            "component" => {
                let id = id_field(args, "component")?;
                let QueryResult::Component(snapshot) = owner
                    .query(Query::Component(ComponentId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let kind = owner
                    .component_catalog()
                    .iter()
                    .find(|(candidate, _)| candidate.get() == id)
                    .map(|(_, kind)| *kind)
                    .ok_or("unknown_component")?;
                json!({"component":id.to_string(),"instrument":snapshot.instrument.get().to_string(),"kind":kind,
                    "generation":snapshot.generation.to_string(),"revision":snapshot.revision.to_string(),
                    "state":match snapshot.state{ComponentState::Warming=>"warming",ComponentState::Ready=>"ready",ComponentState::Failed=>"failed"},
                    "good_steps":snapshot.good_steps,"pending":snapshot.pending.is_some(),"diagnostics":snapshot.diagnostics})
            }
            "controller" => {
                let id = id_field(args, "controller")?;
                let QueryResult::Controller(snap) = owner
                    .query(Query::Controller(ControllerId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let QueryResult::ControllerConfig(config) = owner
                    .query(Query::ControllerConfig(ControllerId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let mut view = controller_json(snap);
                view["config"] = json!({"input":{"instrument":config.input.instrument().get().to_string(),"parameter":config.input.parameter().get().to_string()},
                    "output":{"instrument":config.output.instrument().get().to_string(),"parameter":config.output.parameter().get().to_string()},
                    "reference":config.reference.get().to_string(),"pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
                        "output_min":config.pid.output_min,"output_max":config.pid.output_max},
                    "max_input_age_ns":nanos(config.max_input_age),"max_tick_gap_ns":nanos(config.max_tick_gap),
                    "lease_lifetime_ns":nanos(config.lease_lifetime),"proposal_ttl_ns":nanos(config.proposal_ttl),
                    "ema":{"time_constant_ns":nanos(config.ema.time_constant),"warmup_samples":config.ema.warmup_samples}});
                view
            }
            "describe" => {
                let id = id_field(args, "instrument")?;
                let QueryResult::Descriptor(descriptor) = owner
                    .query(Query::DescribeInstrument(InstrumentId::new(id)))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                let parameters: Vec<_> = descriptor.parameters.iter().map(parameter_json).collect();
                json!({"id":descriptor.id.get().to_string(),"name":descriptor.name,"parameters":parameters})
            }
            "output" => {
                let actuator = args.get("actuator").ok_or("invalid_args")?;
                let instrument = id_field(actuator, "instrument")?;
                let parameter = id_field(actuator, "parameter")?;
                let QueryResult::Output(snapshot) = owner
                    .query(Query::Output(ActuatorId::new(
                        InstrumentId::new(instrument),
                        ParameterId::new(parameter),
                    )))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                output_json(snapshot)
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
                let components: Vec<_> = owner
                    .component_catalog()
                    .iter()
                    .map(|(id, kind)| json!({"id":id.get().to_string(),"kind":kind}))
                    .collect();
                json!({"instruments":list,"controllers":[{"id":"1","kind":"native_pid"}],
                    "references":[{"id":"1","kind":"ramp"}],"components":components,
                    "outputs":[{"instrument":"1","parameter":lab_core::HEATER_POWER.get().to_string()}]})
            }
            _ => return Err("unsupported_operation"),
        };
        Ok(result)
    }

    fn issue_token(&mut self, boot: &str) -> Result<String, &'static str> {
        let counter = self.next_token;
        self.next_token = self.next_token.checked_add(1).ok_or("counter_exhausted")?;
        Ok(format!("{boot}:{counter}"))
    }
    fn page(
        &self,
        service: &ServiceHost,
        connection: u64,
        token: &str,
        index: usize,
    ) -> Result<Value, &'static str> {
        let snapshot = self.snapshots.get(&connection).ok_or("snapshot_expired")?;
        if snapshot.token != token {
            return Err("snapshot_expired");
        }
        if service.clock().now() >= snapshot.expires {
            return Err("snapshot_expired");
        }
        if index > snapshot.records.len() {
            return Err("invalid_args");
        }
        let mut page = Vec::new();
        let mut next = index;
        while next < snapshot.records.len() {
            let record = &snapshot.records[next];
            let bytes = serde_json::to_vec(record)
                .map_err(|_| "internal_error")?
                .len();
            let current: usize = page
                .iter()
                .map(|v: &Value| serde_json::to_vec(v).map(|a| a.len()).unwrap_or(8192))
                .sum();
            if current + bytes + 1024 > 8192 && !page.is_empty() {
                break;
            }
            if bytes + 1024 > 8192 {
                return Err("snapshot_capacity");
            }
            page.push(record.clone());
            next += 1;
        }
        Ok(
            json!({"snapshot":token,"cursor":{"boot_id":service.boot_id(),"seq":snapshot.cursor.to_string()},
            "records":page,"count":snapshot.records.len().to_string(),"next_index":(next<snapshot.records.len()).then(||next.to_string()),
            "expires_at":nanos(snapshot.expires)}),
        )
    }

    /// Offer at most four matching retained semantic events after scanning 32.
    /// A filtered scan emits progress so a client can advance its applied cursor.
    pub fn pump_events(&mut self, service: &ServiceHost, connection: u64) -> Vec<Value> {
        let Some(sub) = self.subscriptions.get_mut(&connection) else {
            return Vec::new();
        };
        let initial_scan = sub.scan;
        let events = match service.owner().event_log().scan_after(sub.scan, 32) {
            Ok(events) => events,
            Err(_) => {
                self.subscriptions.remove(&connection);
                return vec![
                    json!({"v":1,"type":"error","code":"event_gap","resync_required":true}),
                ];
            }
        };
        let mut offered = Vec::new();
        let mut scanned = sub.scan;
        for event in events {
            let seq = event["seq"]
                .as_str()
                .and_then(decimal_u64)
                .unwrap_or(scanned);
            scanned = seq;
            if (sub.kinds.is_empty()
                || event["kind"]
                    .as_str()
                    .is_some_and(|k| sub.kinds.iter().any(|s| s == k)))
                && (sub.targets.is_empty()
                    || sub
                        .targets
                        .iter()
                        .any(|t| event["kind"] == t.kind && event["target"] == t.target))
            {
                offered.push(event);
                if offered.len() >= 4 {
                    break;
                }
            }
        }
        sub.scan = scanned;
        if offered.is_empty() && scanned > initial_scan {
            offered.push(
                json!({"v":1,"type":"subscription_progress","subscription":sub.token,
            "boot_id":service.boot_id(),"seq":scanned.to_string()}),
            );
        }
        offered
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
        if service.is_stopping() && self.sessions.next_seq(&scope).ok() == Some(rid.seq) {
            return vec![error_reply(
                &msg,
                if matches!(&payload, Mutation::Shutdown) {
                    "shutdown_in_progress"
                } else {
                    "shutdown_before_execution"
                },
            )];
        }
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
        if matches!(&payload, Mutation::Shutdown) {
            match service.request_shutdown() {
                Ok(()) => {
                    self.pending_shutdown = Some(PendingShutdown {
                        connection,
                        scope,
                        seq: rid.seq,
                        msg,
                    });
                    return vec![accepted];
                }
                Err(error) => {
                    let failed = OperationState::Failed(domain_code(error).into());
                    self.sessions
                        .complete(&scope, rid.seq, failed.clone(), service.clock().now())
                        .expect("admitted shutdown record");
                    return vec![accepted, operation_reply(&msg, &rid, failed)];
                }
            }
        }
        let outcome = dispatch(service, payload, &rid).map_or_else(
            |error| OperationState::Failed(domain_code(error).into()),
            |value| OperationState::Completed(value.to_string()),
        );
        // The terminal record is committed even if the connection closes before delivery.
        self.sessions
            .complete(&scope, rid.seq, outcome.clone(), service.clock().now())
            .expect("bounded typed terminal record for admitted scope");
        let event_state = operation_state(outcome.clone());
        let published_at = service.clock().now();
        if service
            .owner_mut()
            .event_log_mut()
            .operation_terminal(published_at, &scope, rid.seq, event_state)
            .is_err()
        {
            let _ = service.request_shutdown();
        }
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
fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    rid: &WireRequestId,
) -> Result<Value, Error> {
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
            return Err(Error::InvalidConfiguration(
                "shutdown dispatch is host-only",
            ));
        }
    };
    match service
        .owner_mut()
        .command_with_cause(command, Some((rid.scope.clone(), rid.seq)))?
    {
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
fn parse_filter_target(value: &Value) -> Result<FilterTarget, &'static str> {
    let object = value.as_object().ok_or("invalid_args")?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    let target = match kind {
        "instrument" | "controller" | "reference" | "component" => {
            if object.len() != 2 || !object.contains_key("id") {
                return Err("invalid_args");
            }
            json!({"id":id_field(value,"id")?.to_string()})
        }
        "signal" | "output" => {
            if object.len() != 3
                || !object.contains_key("instrument")
                || !object.contains_key("parameter")
            {
                return Err("invalid_args");
            }
            json!({"instrument":id_field(value,"instrument")?.to_string(),"parameter":id_field(value,"parameter")?.to_string()})
        }
        _ => return Err("invalid_args"),
    };
    Ok(FilterTarget {
        kind: kind.into(),
        target,
    })
}
pub(crate) fn controller_json(s: ControllerSnapshot) -> Value {
    json!({"controller":s.id.get().to_string(),"state":state_name(s.state),
        "revision":s.config_revision.to_string(),"last_tick":s.last_tick.map(nanos),
        "latest_output":s.latest_output.map(|v|v.output)})
}
fn parameter_json(p: &ParameterDescriptor) -> Value {
    let spec = match &p.value_spec {
        ValueSpec::Float { min, max } => json!({"type":"float","min":min,"max":max}),
        ValueSpec::Integer { min, max } => {
            json!({"type":"integer","min":min.to_string(),"max":max.to_string()})
        }
        ValueSpec::Boolean => json!({"type":"boolean"}),
        ValueSpec::Text { max_bytes } => json!({"type":"text","max_bytes":max_bytes}),
        ValueSpec::Enum { choices } => json!({"type":"enum","choices":choices}),
    };
    json!({"id":p.id.get().to_string(),"name":p.name,"unit":{"id":p.unit.id(),"symbol":p.unit.symbol()},
        "value_spec":spec,"access":match p.access {AccessMode::ReadOnly=>"read_only",AccessMode::ReadWrite=>"read_write",AccessMode::WriteOnly=>"write_only"},
        "role":match p.role {ParameterRole::Measurement=>"measurement",ParameterRole::Configuration=>"configuration",ParameterRole::Actuator=>"actuator",ParameterRole::Action=>"action",ParameterRole::Diagnostic=>"diagnostic"},
        "write_effect":match p.write_effect {WriteEffect::None=>"none",WriteEffect::ConfigurationOnly=>"configuration_only",WriteEffect::OutputAffecting=>"output_affecting"},
        "signal":p.signal.map(|s|json!({"instrument":s.instrument().get().to_string(),"parameter":s.parameter().get().to_string()}))})
}
pub(crate) fn output_json(s: OutputSnapshot) -> Value {
    let owner = s.lease.map(|l| match l.owner() {
        OutputOwner::Manual(id) => json!({"kind":"manual","id":id.to_string()}),
        OutputOwner::Automatic(id) => json!({"kind":"automatic","id":id.to_string()}),
    });
    let observation = |o: Option<lab_core::output::OutputObservation>| {
        o.map(|v| json!({"value":v.value,"at":nanos(v.at)}))
    };
    json!({"state":match s.state {OutputState::Unverified=>"unverified",OutputState::SafePending=>"safe_pending",
            OutputState::Disarmed=>"disarmed",OutputState::ArmedManual=>"armed_manual",OutputState::ArmedAuto=>"armed_auto",OutputState::FaultLatched=>"fault_latched"},
        "owner":owner,"instance":s.lease.map(|l|l.instance().to_string()),"epoch":s.epoch.to_string(),"lease_expires_at":s.lease.map(|l|nanos(l.expires())),
        "fault_latched":s.fault_latched,"safe_confirmed":s.safe_confirmed,"pending":s.pending,
        "in_flight":s.in_flight.is_some(),"requested":s.requested,"sent":observation(s.sent),
        "acknowledged":observation(s.acknowledged),"readback":observation(s.readback),
        "outcome":s.outcome.map(|o|match o {DispatchOutcome::Acknowledged=>"acknowledged",DispatchOutcome::ReadbackVerified=>"readback_verified",
            DispatchOutcome::Failed=>"failed",DispatchOutcome::Ambiguous=>"ambiguous"})})
}
pub(crate) fn reference_json(id: u64, s: ReferenceSnapshot) -> Value {
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
pub(crate) fn state_name(s: ControllerState) -> &'static str {
    match s {
        ControllerState::Created => "created",
        ControllerState::Ready => "ready",
        ControllerState::Warming => "warming",
        ControllerState::Running => "running",
        ControllerState::Paused => "paused",
        ControllerState::Failed => "failed",
    }
}
pub(crate) fn quality_name(s: lab_core::SampleQuality) -> &'static str {
    match s {
        lab_core::SampleQuality::Good => "good",
        lab_core::SampleQuality::Unavailable => "unavailable",
    }
}
pub(crate) fn sample_value(sample: &lab_core::Sample) -> Value {
    match sample.value() {
        Some(lab_core::Value::Float(v)) => json!(v),
        _ => Value::Null,
    }
}
pub(crate) fn nanos(at: std::time::Duration) -> String {
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
        OperationState::FailedWithResult { code, detail } => json!({"state":"failed","code":code,
            "result":serde_json::from_str::<Value>(&detail).unwrap_or(Value::Null)}),
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
