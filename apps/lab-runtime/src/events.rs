//! A bounded host projection records selected committed facts after each unit.
//!
//! Sequence is publication order, not a durable history or physical timestamp.
//! A pure query never calls observe. Ring pressure evicts oldest records without
//! pinning them for frozen API projections or subscribers.

use crate::application::projections::{
    controller_projection_json, nanos, output_json, reference_json,
};
use crate::measurements::sample_json;
use lab_core::control::ControllerId;
use lab_core::managed::{ComponentId, ComponentState};
use lab_core::output::ActuatorId;
use lab_core::reference::ReferenceId;
use lab_core::{ParameterRole, Query, QueryResult, Runtime, SignalId};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

/// Fixed maximum retained semantic records; eviction never blocks producers.
pub const EVENT_RING_LIMIT: usize = 1024;
/// Fixed maximum encoded bytes for one semantic record.
pub const EVENT_SIZE_LIMIT: usize = 4096;

/// Ring cursor rejection: old history was lost or a future sequence was offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventError {
    /// Cursor predates the first retained event's predecessor.
    Gap,
    /// Cursor is ahead of this process's last committed publication.
    Future,
    /// A sequence cannot advance without wrapping into an older identity.
    Exhausted,
    /// A public fact would exceed the fixed event record size.
    Oversized,
}

#[derive(Clone)]
enum Target {
    Signal(SignalId),
    Controller(ControllerId),
    Reference(ReferenceId),
    Output(ActuatorId),
    Component(ComponentId),
}
impl Target {
    fn key(&self) -> String {
        match self {
            Self::Signal(s) => format!("signal:{}:{}", s.instrument().get(), s.parameter().get()),
            Self::Controller(id) => format!("controller:{}", id.get()),
            Self::Reference(id) => format!("reference:{}", id.get()),
            Self::Output(a) => format!("output:{}:{}", a.instrument().get(), a.parameter().get()),
            Self::Component(id) => format!("component:{}", id.get()),
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Self::Signal(_) => "signal",
            Self::Controller(_) => "controller",
            Self::Reference(_) => "reference",
            Self::Output(_) => "output",
            Self::Component(_) => "component",
        }
    }
    fn id(&self) -> Value {
        match self {
            Self::Signal(s) => {
                json!({"instrument":s.instrument().get().to_string(),"parameter":s.parameter().get().to_string()})
            }
            Self::Controller(id) => json!({"id":id.get().to_string()}),
            Self::Reference(id) => json!({"id":id.get().to_string()}),
            Self::Output(a) => {
                json!({"instrument":a.instrument().get().to_string(),"parameter":a.parameter().get().to_string()})
            }
            Self::Component(id) => json!({"id":id.get().to_string()}),
        }
    }
    fn query(&self, runtime: &Runtime) -> Option<Value> {
        match self {
            Self::Signal(s) => match runtime.query(Query::GetLatestSignal(*s)).ok()? {
                QueryResult::Latest(Some(sample)) => {
                    let generation = match runtime
                        .query(Query::GetInstrumentState(s.instrument()))
                        .ok()
                    {
                        Some(QueryResult::State(state)) => state.generation,
                        _ => 1,
                    };
                    Some(sample_json(&sample, generation))
                }
                QueryResult::Latest(None) => Some(json!({
                    "signal":{"instrument":s.instrument().get().to_string(),
                        "parameter":s.parameter().get().to_string()},
                    "value":Value::Null,"quality":"unavailable","status":"not_observed",
                    "failure":"not_observed","observed_at_ns":Value::Null,
                    "source_at_ns":Value::Null,"generation":"1"
                })),
                _ => None,
            },
            Self::Controller(id) => match runtime.query(Query::Controller(*id)).ok()? {
                QueryResult::Controller(snapshot) => {
                    let QueryResult::ControllerConfig(config) =
                        runtime.query(Query::ControllerConfig(*id)).ok()?
                    else {
                        return None;
                    };
                    Some(controller_projection_json(snapshot, config))
                }
                _ => None,
            },
            Self::Reference(id) => match runtime.query(Query::Reference(*id)).ok()? {
                QueryResult::Reference(s) => Some(reference_json(id.get(), s)),
                _ => None,
            },
            Self::Output(id) => match runtime.query(Query::Output(*id)).ok()? {
                QueryResult::Output(s) => Some(output_json(s)),
                _ => None,
            },
            Self::Component(id) => match runtime.query(Query::Component(*id)).ok()? {
                QueryResult::Component(s) => Some(
                    json!({"component":id.get().to_string(),"instrument":s.instrument.get().to_string(),
                    "generation":s.generation.to_string(),"revision":s.revision.to_string(),
                    "state":match s.state{ComponentState::Warming=>"warming",ComponentState::Ready=>"ready",ComponentState::Failed=>"failed"},
                    "good_steps":s.good_steps,"pending":s.pending.is_some(),"diagnostics":s.diagnostics}),
                ),
                _ => None,
            },
        }
    }
}

/// Owner-local current facts and the last 1,024 bounded semantic publications.
pub struct EventLog {
    boot_id: String,
    sequence: u64,
    targets: Vec<Target>,
    facts: BTreeMap<String, Value>,
    ring: VecDeque<Value>,
}
impl EventLog {
    /// Capture a no-event baseline from registered bounded public identities.
    pub fn new(
        runtime: &Runtime,
        controllers: &[ControllerId],
        references: &[ReferenceId],
        boot_id: &str,
    ) -> Self {
        let mut targets = Vec::new();
        if let Ok(QueryResult::Instruments(instruments)) = runtime.query(Query::Discover) {
            for instrument in instruments {
                for p in instrument.parameters {
                    if let Some(signal) = p.signal {
                        targets.push(Target::Signal(signal));
                    }
                    if p.role == ParameterRole::Actuator {
                        targets.push(Target::Output(ActuatorId::new(instrument.id, p.id)));
                    }
                }
            }
        }
        targets.extend(controllers.iter().copied().map(Target::Controller));
        targets.extend(references.iter().copied().map(Target::Reference));
        targets.sort_by_key(Target::key);
        let facts = targets
            .iter()
            .filter_map(|t| t.query(runtime).map(|v| (t.key(), v)))
            .collect();
        Self {
            boot_id: boot_id.into(),
            sequence: 0,
            targets,
            facts,
            ring: VecDeque::new(),
        }
    }

    /// Reset process identity before readiness, preserving no old process cursor.
    pub fn set_boot_id(&mut self, boot_id: &str) {
        self.boot_id = boot_id.into();
        self.sequence = 0;
        self.ring.clear();
    }
    /// Track a trusted staged component before asynchronous init can commit.
    pub fn track_component(&mut self, id: ComponentId) {
        if self
            .targets
            .iter()
            .any(|t| matches!(t,Target::Component(existing) if *existing==id))
        {
            return;
        }
        self.targets.push(Target::Component(id));
        self.targets.sort_by_key(Target::key);
    }
    /// Track a trusted native controller registered after initial composition.
    pub fn track_controller(&mut self, id: ControllerId) {
        if !self
            .targets
            .iter()
            .any(|t| matches!(t,Target::Controller(existing) if *existing==id))
        {
            self.targets.push(Target::Controller(id));
            self.targets.sort_by_key(Target::key);
        }
    }
    /// Track a trusted Reference registered after initial composition.
    pub fn track_reference(&mut self, id: ReferenceId) {
        if !self
            .targets
            .iter()
            .any(|t| matches!(t,Target::Reference(existing) if *existing==id))
        {
            self.targets.push(Target::Reference(id));
            self.targets.sort_by_key(Target::key);
        }
    }
    fn track_new_signals(&mut self, runtime: &Runtime) {
        if let Ok(QueryResult::Instruments(instruments)) = runtime.query(Query::Discover) {
            for instrument in instruments {
                for p in instrument.parameters {
                    if let Some(signal) = p.signal
                        && !self
                            .targets
                            .iter()
                            .any(|t| matches!(t,Target::Signal(existing) if *existing==signal))
                    {
                        self.targets.push(Target::Signal(signal));
                    }
                    if p.role == ParameterRole::Actuator {
                        let actuator = ActuatorId::new(instrument.id, p.id);
                        if !self
                            .targets
                            .iter()
                            .any(|t| matches!(t,Target::Output(existing) if *existing==actuator))
                        {
                            self.targets.push(Target::Output(actuator));
                        }
                    }
                }
            }
        }
        self.targets.sort_by_key(Target::key);
    }
    /// Freshest committed publication sequence, zero before the first event.
    pub const fn latest_cursor(&self) -> u64 {
        self.sequence
    }
    /// Earliest cursor from which complete replay remains possible.
    pub fn oldest_cursor(&self) -> u64 {
        self.ring
            .front()
            .and_then(|v| v["seq"].as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .and_then(|seq| seq.checked_sub(1))
            .unwrap_or(self.sequence)
    }
    /// Process-local identity required for cursor verification.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }
    /// Owned current public facts for a frozen projection, in stable kind/ID order.
    pub fn projection_records(&self) -> Vec<Value> {
        self.targets
            .iter()
            .filter_map(|t| {
                self.facts
                    .get(&t.key())
                    .map(|data| json!({"kind":t.kind(),"target":t.id(),"data":data}))
            })
            .collect()
    }
    /// Compare complete selected committed state after exactly one domain unit.
    pub fn observe(
        &mut self,
        runtime: &Runtime,
        at: Duration,
        cause: Option<(&str, u64)>,
    ) -> Result<(), EventError> {
        self.track_new_signals(runtime);
        let targets = self.targets.clone();
        for target in targets {
            let Some(data) = target.query(runtime) else {
                continue;
            };
            let key = target.key();
            if self.facts.get(&key) == Some(&data) {
                continue;
            }
            let previous = self.facts.get(&key);
            match &target {
                Target::Signal(signal)
                    if data["quality"] == "unavailable"
                        && data["failure"] == "transport"
                        && previous.is_none_or(|old| {
                            old["quality"] != "unavailable" || old["failure"] != "transport"
                        }) =>
                {
                    tracing::warn!(
                        event = "physical_measurement_unavailable",
                        instrument = signal.instrument().get(),
                        parameter = signal.parameter().get(),
                        "physical measurement became unavailable because of transport"
                    );
                }
                Target::Signal(signal)
                    if data["quality"] == "good"
                        && previous.is_some_and(|old| {
                            old["quality"] == "unavailable" && old["failure"] == "transport"
                        }) =>
                {
                    tracing::info!(
                        event = "physical_measurement_recovered",
                        instrument = signal.instrument().get(),
                        parameter = signal.parameter().get(),
                        "physical measurement recovered with a fresh Good observation"
                    );
                }
                Target::Controller(id) if data["state"] == "failed" => tracing::warn!(
                    event = "controller_failed",
                    controller = id.get(),
                    "controller entered Failed; input health does not rearm output authority"
                ),
                Target::Output(actuator)
                    if data["outcome"] == "ambiguous"
                        || data["readback_failure"] == "mismatch"
                        || data["readback_failure"] == "unavailable" =>
                {
                    tracing::warn!(
                        event = "physical_output_unconfirmed",
                        instrument = actuator.instrument().get(),
                        parameter = actuator.parameter().get(),
                        state = data["state"].as_str().unwrap_or("unknown"),
                        outcome = data["outcome"].as_str().unwrap_or("unknown"),
                        readback = data["readback_failure"].as_str().unwrap_or("unknown"),
                        "physical output is unconfirmed; requested, sent, ACK, readback, and physical effect remain distinct"
                    );
                }
                Target::Output(actuator) if data["safe_confirmed"] == true => tracing::info!(
                    event = "physical_output_safe_readback_verified",
                    instrument = actuator.instrument().get(),
                    parameter = actuator.parameter().get(),
                    "safe register readback verified; independent physical effect is not proven"
                ),
                Target::Output(actuator) if data["state"] == "safe_pending" => tracing::warn!(
                    event = "physical_output_safe_obligation",
                    instrument = actuator.instrument().get(),
                    parameter = actuator.parameter().get(),
                    "safe obligation is pending and is not permission to blindly resend"
                ),
                Target::Output(actuator) => tracing::debug!(
                    event = "physical_output_state",
                    instrument = actuator.instrument().get(),
                    parameter = actuator.parameter().get(),
                    state = data["state"].as_str().unwrap_or("unknown"),
                    "physical output state changed"
                ),
                _ => {}
            }
            self.append(at, target.kind(), target.id(), data.clone(), cause)?;
            self.facts.insert(key, data);
        }
        Ok(())
    }
    /// Publish a terminal operation record after resulting state facts.
    pub fn operation_terminal(
        &mut self,
        at: Duration,
        scope: &str,
        seq: u64,
        data: Value,
    ) -> Result<(), EventError> {
        self.append(
            at,
            "operation",
            json!({"scope":scope}),
            data,
            Some((scope, seq)),
        )
    }
    /// Publish the bounded owner lifecycle fact before a terminal operation.
    pub fn host_state(&mut self, at: Duration, state: &str, data: Value) -> Result<(), EventError> {
        self.append(
            at,
            "host",
            json!({"id":"runtime"}),
            json!({"state":state,"status":data}),
            None,
        )
    }
    /// Publish a semantic Recorder lifecycle snapshot, never worker diagnostics.
    pub fn recorder_state(
        &mut self,
        at: Duration,
        archive_id: Option<&str>,
        data: Value,
    ) -> Result<(), EventError> {
        self.append(at, "recorder", json!({"id":archive_id}), data, None)
    }
    /// Publish one semantic resource snapshot after an explicit lifecycle change.
    pub fn resource_state(&mut self, at: Duration, id: u64, data: Value) -> Result<(), EventError> {
        self.append(at, "resource", json!({"id":id.to_string()}), data, None)
    }
    /// Publish current configuration/candidate state after a lifecycle operation.
    pub fn configuration_state(&mut self, at: Duration, data: Value) -> Result<(), EventError> {
        self.append(at, "configuration", json!({"id":"runtime"}), data, None)
    }
    fn append(
        &mut self,
        at: Duration,
        kind: &str,
        target: Value,
        data: Value,
        cause: Option<(&str, u64)>,
    ) -> Result<(), EventError> {
        let sequence = self.sequence.checked_add(1).ok_or(EventError::Exhausted)?;
        let record = json!({"v":1,"type":"event","boot_id":self.boot_id,"seq":sequence.to_string(),"published_at":nanos(at),
            "kind":kind,"target":target,"data":data,"request_id":cause.map(|(scope,seq)|json!({"scope":scope,"seq":seq.to_string()}))});
        if crate::wire::encode_frame(&record)
            .map_err(|_| EventError::Oversized)?
            .len()
            > EVENT_SIZE_LIMIT + 1
        {
            return Err(EventError::Oversized);
        }
        self.sequence = sequence;
        if self.ring.len() == EVENT_RING_LIMIT {
            self.ring.pop_front();
        }
        self.ring.push_back(record);
        Ok(())
    }
    /// Retrieve up to limit retained events after a cursor; no silent jump on gap.
    pub fn scan_after(&self, cursor: u64, limit: usize) -> Result<Vec<Value>, EventError> {
        if cursor > self.sequence {
            return Err(EventError::Future);
        }
        if cursor < self.oldest_cursor() {
            return Err(EventError::Gap);
        }
        Ok(self
            .ring
            .iter()
            .filter(|v| {
                v["seq"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|n| n > cursor)
            })
            .take(limit.min(32))
            .cloned()
            .collect())
    }
    /// Test and bounded-diagnostic view of the retained replay window.
    pub fn after(&self, cursor: u64) -> Result<Vec<Value>, EventError> {
        self.scan_after(cursor, 32)
    }
}
