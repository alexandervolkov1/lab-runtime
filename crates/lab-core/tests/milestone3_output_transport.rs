//! M3 acceptance for the final authority check at the actual byte boundary.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Query,
    QueryResult, Runtime, Unit, Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal,
        OutputResult, OutputState, SafeProfile,
    },
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

#[derive(Default)]
struct Wire {
    bytes: Vec<u8>,
    readable: VecDeque<u8>,
    write_limits: VecDeque<usize>,
    recoveries: usize,
}

struct Fake(Rc<RefCell<Wire>>);
impl ByteTransport for Fake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let limit = wire.write_limits.pop_front().unwrap_or(bytes.len());
        let count = bytes.len().min(limit);
        wire.bytes.extend_from_slice(&bytes[..count]);
        if count > 0 && wire.bytes.len().is_multiple_of(7) {
            let body = [15, 0, 6, 1];
            wire.readable.extend(body);
            wire.readable.push_back(crc(&body));
        }
        Ok(count)
    }
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        for slot in &mut bytes[..count] {
            *slot = wire.readable.pop_front().unwrap();
        }
        Ok(count)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.recoveries += 1;
        wire.readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

fn config(generation: u64, revision: u64) -> MetakonInstrumentConfig {
    let id = InstrumentId::new(50);
    MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id,
            name: "Metakon output".into(),
            parameters: vec![DataParameterDefinition {
                id: ParameterId::new(6),
                name: "power".into(),
                value_spec: ValueSpec::Float {
                    min: 0.0,
                    max: 100.0,
                },
                unit: Unit::PERCENT,
                access: AccessMode::ReadWrite,
                role: ParameterRole::Actuator,
                write_effect: WriteEffect::OutputAffecting,
                operation: KnownOperation::Output,
                scale: 1.0,
            }],
        },
        binding: MetakonBinding {
            resource: ResourceId::new(1),
            device: 15,
            channel: 0,
            binding_generation: generation,
            mapping_revision: revision,
            expected_output_unit: Some(Unit::PERCENT),
        },
        history_capacity: 2,
    }
}

fn command(
    runtime: &mut Runtime,
    actuator: ActuatorId,
    command: OutputCommand,
    ms: u64,
) -> OutputResult {
    let CommandResult::Output(result) = runtime
        .command(Command::Output {
            actuator,
            command,
            at: Duration::from_millis(ms),
        })
        .unwrap()
    else {
        panic!()
    };
    result
}

fn setup(
    write_limits: impl IntoIterator<Item = usize>,
) -> (Runtime, ActuatorId, Rc<RefCell<Wire>>) {
    let wire = Rc::new(RefCell::new(Wire {
        write_limits: write_limits.into_iter().collect(),
        ..Wire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport(ResourceId::new(1), Box::new(Fake(wire.clone())))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(config(1, 1)))
        .unwrap();
    let actuator = ActuatorId::new(InstrumentId::new(50), ParameterId::new(6));
    command(
        &mut runtime,
        actuator,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(10),
            max_proposal_ttl: Duration::from_secs(2),
            required_evidence: EvidenceLevel::Acknowledgement,
        }),
        0,
    );
    command(&mut runtime, actuator, OutputCommand::RequestSafe, 0);
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::ZERO,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports { at: Duration::ZERO })
        .unwrap();
    let OutputResult::Lease(lease) = command(
        &mut runtime,
        actuator,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(1),
            lifetime: Duration::from_secs(5),
        },
        1,
    ) else {
        panic!()
    };
    command(
        &mut runtime,
        actuator,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(75.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_secs(1),
        }),
        1,
    );
    (runtime, actuator, wire)
}

#[test]
fn revoke_after_queue_prevents_the_first_byte() {
    let (mut runtime, actuator, wire) = setup([]);
    wire.borrow_mut().bytes.clear();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(1),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    command(&mut runtime, actuator, OutputCommand::Trip, 2);
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    assert!(wire.borrow().bytes.is_empty());
}

#[test]
fn zero_byte_would_block_is_rechecked_before_retry() {
    let (mut runtime, actuator, wire) = setup([7, 0]);
    wire.borrow_mut().bytes.clear();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(1),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(1),
        })
        .unwrap();
    command(&mut runtime, actuator, OutputCommand::Trip, 2);
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    assert!(wire.borrow().bytes.is_empty());
}

#[test]
fn partial_write_is_not_retried_and_remains_unknown_until_recovery() {
    let (mut runtime, actuator, wire) = setup([7, 2, 0]);
    wire.borrow_mut().bytes.clear();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(1),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(2),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(1),
        })
        .unwrap();
    assert_eq!(wire.borrow().bytes.len(), 2);
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(3),
        })
        .unwrap();
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::SafePending);
    assert!(snapshot.in_flight.is_some());
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(4),
        })
        .unwrap();
    assert_eq!(wire.borrow().bytes.len(), 2);
    assert_eq!(wire.borrow().recoveries, 1);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.outcome, Some(DispatchOutcome::Ambiguous));
    assert!(snapshot.fault_latched);
}

#[test]
fn rebind_fences_queued_intent_even_with_equal_logical_ids() {
    let (mut runtime, actuator, wire) = setup([]);
    wire.borrow_mut().bytes.clear();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(1),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime
        .command(Command::RebindMetakon {
            instrument: InstrumentId::new(50),
            binding: config(2, 2).binding,
            at: Duration::from_millis(2),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    assert!(wire.borrow().bytes.is_empty());
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::Unverified);
}
