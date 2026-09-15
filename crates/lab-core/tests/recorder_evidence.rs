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
}
struct Fake(Rc<RefCell<Wire>>);
impl ByteTransport for Fake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.bytes.extend_from_slice(bytes);
        let body = [15, 0, 6, 1];
        wire.readable.extend(body);
        wire.readable.push_back(crc(&body));
        Ok(bytes.len())
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
        self.0.borrow_mut().readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

fn fixture() -> (Runtime, ActuatorId) {
    let mut runtime = Runtime::new();
    runtime
        .register_transport(
            ResourceId::new(1),
            Box::new(Fake(Rc::new(RefCell::new(Wire::default())))),
        )
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
}
