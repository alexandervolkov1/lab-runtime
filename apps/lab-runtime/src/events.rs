//! A bounded host projection records selected committed facts after each unit.
//!
//! Sequence is publication order, not a durable history or physical timestamp.
//! A pure query never calls observe. Ring pressure evicts oldest records without
//! pinning them for snapshots or subscribers.

use crate::application::{
    controller_json, nanos, output_json, quality_name, reference_json, sample_value,
};
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
                QueryResult::Latest(Some(sample)) => Some(
                    json!({"value":sample_value(&sample),"quality":quality_name(sample.quality()),
                "observed_at":nanos(sample.at()),"freshness_at":nanos(sample.freshness_at()),
                "unit":{"id":sample.unit().id(),"symbol":sample.unit().symbol()}}),
                ),
                QueryResult::Latest(None) => Some(Value::Null),
                _ => None,
            },
            Self::Controller(id) => match runtime.query(Query::Controller(*id)).ok()? {
                QueryResult::Controller(s) => Some(controller_json(s)),
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
    /// Owned current public facts for a frozen snapshot, in stable kind/ID order.
    pub fn snapshot_records(&self) -> Vec<Value> {
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
            self.facts.insert(key, data.clone());
            self.append(at, target.kind(), target.id(), data, cause)?;
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
        if serde_json::to_vec(&record)
            .map_err(|_| EventError::Oversized)?
            .len()
            > EVENT_SIZE_LIMIT
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
