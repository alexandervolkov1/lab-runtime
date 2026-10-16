//! Trusted M3 ACK capture cannot be promoted to readback or physical proof.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Runtime, Unit,
    Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal, OutputResult,
        SafeProfile,
    },
    recording::{OutputEvidenceSource, OutputStage, RecordingFact},
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
        if count == 7 && count == bytes.len() {
            let body = [15, 0, 6, 1];
            wire.readable.extend(body);
            wire.readable.push_back(crc(&body));
        }
        Ok(count)
    }
    fn try_read(&mut self, buffer: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = buffer.len().min(wire.readable.len());
        for byte in &mut buffer[..count] {
            *byte = wire.readable.pop_front().unwrap();
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

fn fixture_with_limits(
    write_limits: impl IntoIterator<Item = usize>,
) -> (Runtime, ActuatorId, Rc<RefCell<Wire>>) {
    let mut runtime = Runtime::new();
    let wire = Rc::new(RefCell::new(Wire {
        write_limits: write_limits.into_iter().collect(),
        ..Wire::default()
    }));
    runtime
        .register_transport(ResourceId::new(1), Box::new(Fake(wire.clone())))
        .unwrap();
    let instrument = InstrumentId::new(711);
    let parameter = ParameterId::new(6);
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "ACK fixture".into(),
                parameters: vec![DataParameterDefinition {
                    id: parameter,
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
                binding_generation: 1,
                mapping_revision: 1,
                expected_output_unit: Some(Unit::PERCENT),
                output_queue_ttl: Some(Duration::from_secs(1)),
                output_timeout: Some(Duration::from_millis(100)),
            },
            history_capacity: 2,
        }))
        .unwrap();
    let actuator = ActuatorId::new(instrument, parameter);
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(5),
                max_proposal_ttl: Duration::from_secs(1),
                required_evidence: EvidenceLevel::Acknowledgement,
            }),
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::ZERO,
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
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
    (runtime, actuator, wire)
}

fn fixture() -> (Runtime, ActuatorId) {
    let (runtime, actuator, _) = fixture_with_limits([]);
    (runtime, actuator)
}

#[test]
fn actual_metakon_ack_is_trusted_protocol_ack_without_readback_fact() {
    let (mut runtime, actuator) = fixture();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(1),
                lifetime: Duration::from_secs(1),
            },
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(40.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(200),
            }),
        })
        .unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime.command(Command::PollTransports { at }).unwrap();
    let facts = runtime.take_recording_facts();
    let stages: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output { stage, .. } => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        [
            OutputStage::Requested,
            OutputStage::Authorized,
            OutputStage::SendStarted,
            OutputStage::Acknowledged
        ]
    );
    assert!(facts.iter().any(|fact| matches!(
        fact,
        RecordingFact::Output {
            stage: OutputStage::Acknowledged,
            source: OutputEvidenceSource::TransportProtocol,
            ..
        }
    )));
    assert!(!stages.contains(&OutputStage::ReadbackVerified));
    let correlations: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                attempt_id,
                dispatch_id,
                ..
            } => Some((*attempt_id, *dispatch_id)),
            _ => None,
        })
        .collect();
    assert!(correlations[0].0.is_some());
    assert!(correlations.iter().all(|pair| pair.0 == correlations[0].0));
    assert_eq!(correlations[0].1, None);
    assert!(correlations[1].1.is_some());
    assert!(
        correlations[1..]
            .iter()
            .all(|pair| pair.1 == correlations[1].1)
    );
    for fact in &facts {
        if let RecordingFact::Output {
            unit,
            authority_epoch,
            resource,
            binding_generation,
            mapping_revision,
            ..
        } = fact
        {
            assert_eq!(*unit, Some(Unit::PERCENT));
            assert_eq!(*authority_epoch, Some(lease.epoch()));
            assert_eq!(*resource, Some(ResourceId::new(1)));
            assert_eq!(*binding_generation, Some(1));
            assert_eq!(*mapping_revision, Some(1));
        }
    }
}

#[test]
fn partial_metakon_write_records_uncertainty_and_recovery_without_ack_or_readback() {
    let (mut runtime, actuator, wire) = fixture_with_limits([7, 2, 0]);
    wire.borrow_mut().bytes.clear();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(1),
                lifetime: Duration::from_secs(1),
            },
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(75.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(200),
            }),
        })
        .unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(2),
        })
        .unwrap();
    runtime.command(Command::PollTransports { at }).unwrap();
    assert_eq!(wire.borrow().bytes.len(), 2, "actual prefix was sent");
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(3),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(4),
        })
        .unwrap();
    assert_eq!(wire.borrow().recoveries, 1);
    assert_eq!(
        wire.borrow().bytes.len(),
        2,
        "ordinary effect was not retried"
    );
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::from_millis(5),
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(5),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(5),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(6),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(7),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    let output: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                attempt_id,
                dispatch_id,
                authority_epoch,
                source,
                ..
            } => Some((*stage, *attempt_id, *dispatch_id, *authority_epoch, *source)),
            _ => None,
        })
        .collect();
    assert!(output.iter().any(|row| row.0 == OutputStage::Requested));
    assert!(output.iter().any(|row| row.0 == OutputStage::Authorized));
    assert!(output.iter().any(|row| row.0 == OutputStage::SendStarted));
    assert!(
        output
            .iter()
            .any(|row| row.0 == OutputStage::TransportUncertain)
    );
    assert!(output.iter().any(|row| row.0 == OutputStage::Ambiguous));
    assert!(
        output.iter().any(|row| row.0 == OutputStage::SafeRequested),
        "{output:?}"
    );
    assert!(
        output
            .iter()
            .any(|row| row.0 == OutputStage::SafeSendStarted),
        "{output:?}"
    );
    assert!(
        output
            .iter()
            .any(|row| row.0 == OutputStage::SafeAcknowledged),
        "{output:?}"
    );
    assert!(!output.iter().any(|row| matches!(
        row.0,
        OutputStage::Acknowledged | OutputStage::ReadbackVerified
    )));
    let ordinary = output
        .iter()
        .find(|row| row.0 == OutputStage::Requested)
        .unwrap();
    assert!(ordinary.1.is_some());
    assert_eq!(ordinary.3, Some(lease.epoch()));
    assert!(
        output
            .iter()
            .filter(|row| matches!(
                row.0,
                OutputStage::Authorized
                    | OutputStage::SendStarted
                    | OutputStage::TransportUncertain
                    | OutputStage::Ambiguous
            ))
            .all(|row| row.1 == ordinary.1 && row.2.is_some() && row.3 == ordinary.3)
    );
    let safe = output
        .iter()
        .find(|row| row.0 == OutputStage::SafeRequested)
        .unwrap();
    assert_ne!(safe.1, ordinary.1);
    assert_ne!(safe.3, ordinary.3);
    assert!(
        output
            .iter()
            .filter(|row| matches!(
                row.0,
                OutputStage::SafeSendStarted | OutputStage::SafeAcknowledged
            ))
            .all(|row| row.1 == safe.1 && row.3 == safe.3 && row.2.is_some())
    );
}
