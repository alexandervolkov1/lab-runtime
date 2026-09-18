//! M3 acceptance for native Metakon read mapping and protocol-failure recovery.

use lab_core::{
    AccessMode, Command, InstrumentId, MeasurementFailure, ParameterId, ParameterRole, Query,
    QueryResult, Runtime, SampleQuality, Unit, Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    transport::{
        ByteTransport, ExecutorState, RecoveryStatus, ResourceId, TransportError, TransportIoError,
        TransportShutdown,
    },
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

#[derive(Default)]
struct ReadWire {
    writes: Vec<u8>,
    write_errors: VecDeque<TransportIoError>,
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
    recoveries: usize,
    recovery_status: Option<RecoveryStatus>,
    shutdowns: usize,
}

struct Fake(Rc<RefCell<ReadWire>>);
impl ByteTransport for Fake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        if let Some(error) = wire.write_errors.pop_front() {
            return Err(error);
        }
        wire.writes.extend_from_slice(bytes);
        if let Some(response) = wire.responses.pop_front() {
            wire.readable.extend(response);
        }
        Ok(bytes.len())
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
        Ok(wire.recovery_status.unwrap_or(RecoveryStatus::Complete))
    }
    fn try_shutdown(&mut self) -> TransportShutdown {
        self.0.borrow_mut().shutdowns += 1;
        TransportShutdown::Complete
    }
}

fn response(raw: i16) -> Vec<u8> {
    let [low, high] = raw.to_le_bytes();
    let mut bytes = vec![1, 0, 1, 0, 0x44, low, high];
    bytes.push(crc(&bytes));
    bytes
}

fn config() -> MetakonInstrumentConfig {
    MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: InstrumentId::new(70),
            name: "Native Metakon temperature".into(),
            parameters: vec![DataParameterDefinition {
                id: ParameterId::new(1),
                name: "temperature".into(),
                value_spec: ValueSpec::Float {
                    min: -99.9,
                    max: 999.9,
                },
                unit: Unit::CELSIUS,
                access: AccessMode::ReadOnly,
                role: ParameterRole::Measurement,
                write_effect: WriteEffect::None,
                operation: KnownOperation::Temperature,
                scale: 0.1,
            }],
        },
        binding: MetakonBinding {
            resource: ResourceId::new(7),
            device: 1,
            channel: 0,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: None,
            output_queue_ttl: None,
            output_timeout: None,
        },
        history_capacity: 4,
    }
}

fn queue_and_poll(runtime: &mut Runtime, milliseconds: u64) {
    runtime
        .command(Command::QueueMetakonRead {
            instrument: InstrumentId::new(70),
            parameter: ParameterId::new(1),
            at: Duration::from_millis(milliseconds),
            queue_ttl: Duration::from_millis(50),
            timeout: Duration::from_millis(20),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(milliseconds),
        })
        .unwrap();
}

#[test]
fn valid_and_sensor_fault_responses_become_typed_samples() {
    let wire = Rc::new(RefCell::new(ReadWire {
        responses: VecDeque::from([response(1234), response(i16::MIN)]),
        ..ReadWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport(ResourceId::new(7), Box::new(Fake(wire)))
        .unwrap();
    runtime.command(Command::RegisterMetakon(config())).unwrap();
    let signal = lab_core::SignalId::new(InstrumentId::new(70), ParameterId::new(1));

    queue_and_poll(&mut runtime, 1);
    let QueryResult::Latest(Some(sample)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.quality(), SampleQuality::Good);
    assert_eq!(sample.value(), Some(&Value::Float(123.4)));

    queue_and_poll(&mut runtime, 2);
    let QueryResult::Latest(Some(sample)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.quality(), SampleQuality::Unavailable);
    assert_eq!(sample.failure(), Some(MeasurementFailure::SensorFault));
    assert_eq!(sample.value(), None);
}

#[test]
fn bad_crc_and_late_bytes_require_recovery_before_a_new_read() {
    let mut bad = response(10);
    bad[7] ^= 1;
    bad.extend(response(999));
    let wire = Rc::new(RefCell::new(ReadWire {
        responses: VecDeque::from([bad, response(20)]),
        ..ReadWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport(ResourceId::new(7), Box::new(Fake(wire.clone())))
        .unwrap();
    runtime.command(Command::RegisterMetakon(config())).unwrap();
    let signal = lab_core::SignalId::new(InstrumentId::new(70), ParameterId::new(1));

    queue_and_poll(&mut runtime, 1);
    let QueryResult::Transport(snapshot) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.state, ExecutorState::Recovering);
    let QueryResult::Latest(Some(sample)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.failure(), Some(MeasurementFailure::Transport));

    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    assert_eq!(wire.borrow().recoveries, 1);
    queue_and_poll(&mut runtime, 3);
    let QueryResult::Latest(Some(sample)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.value(), Some(&Value::Float(2.0)));
}

#[test]
fn protocol_recovery_pending_is_finite_and_does_not_advance_generation() {
    let mut bad = response(10);
    bad[7] ^= 1;
    let wire = Rc::new(RefCell::new(ReadWire {
        responses: VecDeque::from([bad]),
        recovery_status: Some(RecoveryStatus::Pending),
        ..ReadWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport_with_recovery_timeout(
            ResourceId::new(7),
            Box::new(Fake(wire)),
            Duration::from_millis(5),
        )
        .unwrap();
    runtime.command(Command::RegisterMetakon(config())).unwrap();
    runtime.enable_recording_facts();

    queue_and_poll(&mut runtime, 1);
    let QueryResult::Transport(recovering) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(recovering.state, ExecutorState::Recovering);
    assert_eq!(recovering.generation, 1);
    assert!(matches!(
        runtime.command(Command::QueueMetakonRead {
            instrument: InstrumentId::new(70),
            parameter: ParameterId::new(1),
            at: Duration::from_millis(2),
            queue_ttl: Duration::from_millis(50),
            timeout: Duration::from_millis(20),
        }),
        Err(lab_core::Error::Transport(
            TransportError::ResourceUnavailable
        ))
    ));

    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(5),
        })
        .unwrap();
    let QueryResult::Transport(before_deadline) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(before_deadline.state, ExecutorState::Recovering);

    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(6),
        })
        .unwrap();
    let QueryResult::Transport(offline) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(offline.state, ExecutorState::Offline);
    assert_eq!(offline.generation, 1);
}

#[test]
fn shutdown_from_protocol_recovery_initiates_adapter_retirement() {
    let mut bad = response(10);
    bad[7] ^= 1;
    let wire = Rc::new(RefCell::new(ReadWire {
        responses: VecDeque::from([bad]),
        recovery_status: Some(RecoveryStatus::Pending),
        ..ReadWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport_with_recovery_timeout(
            ResourceId::new(7),
            Box::new(Fake(wire.clone())),
            Duration::from_millis(5),
        )
        .unwrap();
    runtime.command(Command::RegisterMetakon(config())).unwrap();
    queue_and_poll(&mut runtime, 1);

    assert_eq!(
        runtime
            .shutdown_transport(ResourceId::new(7), Duration::from_millis(2))
            .unwrap(),
        TransportShutdown::Complete
    );
    assert_eq!(wire.borrow().shutdowns, 1);
    let QueryResult::Transport(snapshot) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.state, ExecutorState::Offline);
    assert_eq!(snapshot.generation, 1);
}

#[test]
fn clean_transaction_recovery_retry_can_publish_new_good_after_unavailable() {
    let wire = Rc::new(RefCell::new(ReadWire {
        write_errors: VecDeque::from([TransportIoError::Disconnected]),
        responses: VecDeque::from([response(250)]),
        ..ReadWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport_with_recovery_timeout(
            ResourceId::new(7),
            Box::new(Fake(wire)),
            Duration::from_millis(5),
        )
        .unwrap();
    runtime.command(Command::RegisterMetakon(config())).unwrap();
    let signal = lab_core::SignalId::new(InstrumentId::new(70), ParameterId::new(1));

    queue_and_poll(&mut runtime, 1);
    let QueryResult::Latest(Some(unavailable)) =
        runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(unavailable.quality(), SampleQuality::Unavailable);

    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(2),
        })
        .unwrap();
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(3),
        })
        .unwrap();
    let QueryResult::Latest(Some(good)) = runtime.query(Query::GetLatestSignal(signal)).unwrap()
    else {
        panic!()
    };
    assert_eq!(good.quality(), SampleQuality::Good);
    assert_eq!(good.value(), Some(&Value::Float(25.0)));
    let QueryResult::Transport(snapshot) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.generation, 2);
}
