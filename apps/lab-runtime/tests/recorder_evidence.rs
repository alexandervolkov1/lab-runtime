//! Real SQLite retains the original M3 byte attempt through partial I/O and recovery.

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
    plant::ThermalPlantConfig,
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::recorder::SqliteStore;
use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc, time::Duration};

#[derive(Default)]
struct Wire {
    bytes: Vec<u8>,
    readable: VecDeque<u8>,
    limits: VecDeque<usize>,
    recoveries: usize,
    reply_on_write: bool,
}
struct Fake(Rc<RefCell<Wire>>);
impl ByteTransport for Fake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let limit = wire.limits.pop_front().unwrap_or(bytes.len());
        let count = bytes.len().min(limit);
        wire.bytes.extend_from_slice(&bytes[..count]);
        if wire.reply_on_write && count == 7 && count == bytes.len() {
            let body = [15, 0, 6, 1];
            wire.readable.extend(body);
            wire.readable.push_back(crc(&body));
        }
        Ok(count)
    }
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
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

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-partial-m3-{suffix}.sqlite"))
}

fn setup() -> (Runtime, ActuatorId, Rc<RefCell<Wire>>) {
    let wire = Rc::new(RefCell::new(Wire {
        limits: [7, 2, 0].into_iter().collect(),
        reply_on_write: true,
        ..Wire::default()
    }));
    let mut runtime = Runtime::new();
    let resource = ResourceId::new(1);
    let instrument = InstrumentId::new(712);
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    runtime
        .register_transport(resource, Box::new(Fake(wire.clone())))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "partial byte archive".into(),
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
                resource,
                device: 15,
                channel: 0,
                binding_generation: 1,
                mapping_revision: 1,
                expected_output_unit: Some(Unit::PERCENT),
            },
            history_capacity: 2,
        }))
        .unwrap();
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
    wire.borrow_mut().bytes.clear();
    (runtime, actuator, wire)
}

#[test]
fn partial_m3_attempt_reopens_as_old_epoch_ambiguous_before_distinct_safe_ack() {
    let path = temporary_database();
    let (mut runtime, actuator, wire) = setup();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("partial M3 archive").unwrap();
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
        panic!("safe ACK did not permit manual lease")
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
    assert_eq!(wire.borrow().bytes.len(), 2);
    for ms in [3, 4] {
        runtime
            .command(Command::PollTransports {
                at: Duration::from_millis(ms),
            })
            .unwrap();
    }
    assert_eq!(wire.borrow().recoveries, 1);
    assert_eq!(wire.borrow().bytes.len(), 2, "ordinary prefix not retried");
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
    for ms in [5, 6, 7] {
        runtime
            .command(Command::PollTransports {
                at: Duration::from_millis(ms),
            })
            .unwrap();
    }
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_millis(8)).unwrap();
    store.close().unwrap();
    let reopened = SqliteStore::open_with_boot(&path, "abababababababababababababababab").unwrap();
    reopened.close().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    type Row = (
        String,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        String,
    );
    let rows: Vec<Row> = db
        .prepare("SELECT stage,attempt_id,dispatch_id,authority_epoch,resource_id,generation,revision,evidence_source FROM output_events ORDER BY record_seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let stages: Vec<_> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        stages,
        [
            "requested",
            "authorized",
            "send_started",
            "transport_uncertain",
            "ambiguous",
            "revoked",
            "safe_requested",
            "safe_send_started",
            "safe_acknowledged"
        ]
    );
    let old_attempt = rows[0].1.clone();
    let old_dispatch = rows[1].2.clone();
    assert!(old_attempt.is_some() && old_dispatch.is_some());
    for (index, row) in rows.iter().take(5).enumerate() {
        assert_eq!(row.1, old_attempt);
        assert_eq!(
            row.2,
            if index == 0 {
                None
            } else {
                old_dispatch.clone()
            }
        );
        assert_eq!(row.3, Some(lease.epoch().to_be_bytes().to_vec()));
        assert_eq!(row.4, Some(1u64.to_be_bytes().to_vec()));
        assert_eq!(row.5, Some(1u64.to_be_bytes().to_vec()));
        assert_eq!(row.6, Some(1u64.to_be_bytes().to_vec()));
    }
    assert_eq!(rows[4].7, "transport_protocol");
    assert_ne!(rows[6].1, old_attempt);
    assert_ne!(rows[6].3, rows[4].3);
    assert_eq!(rows[8].1, rows[6].1);
    assert_eq!(rows[8].3, rows[6].3);
    assert_eq!(rows[8].7, "transport_protocol");
    assert!(!stages.contains(&"acknowledged"));
    assert!(!stages.contains(&"readback_verified"));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn late_virtual_ack_reopens_under_its_old_epoch_without_safe_evidence() {
    let path = temporary_database();
    let mut runtime = Runtime::new();
    let instrument = InstrumentId::new(713);
    let actuator = ActuatorId::new(instrument, lab_core::HEATER_POWER);
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: instrument,
            name: "late virtual ACK".into(),
            history_capacity: 2,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    let output = |runtime: &mut Runtime, command, at| {
        runtime.command(Command::Output {
            actuator,
            command,
            at,
        })
    };
    output(
        &mut runtime,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(5),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Readback,
        }),
        Duration::ZERO,
    )
    .unwrap();
    output(&mut runtime, OutputCommand::RequestSafe, Duration::ZERO).unwrap();
    let CommandResult::Output(OutputResult::Dispatched(initial_safe)) =
        output(&mut runtime, OutputCommand::BeginDispatch, Duration::ZERO).unwrap()
    else {
        panic!()
    };
    output(
        &mut runtime,
        OutputCommand::Complete {
            dispatch_id: initial_safe.id(),
            outcome: lab_core::output::DispatchOutcome::ReadbackVerified,
        },
        Duration::ZERO,
    )
    .unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = output(
        &mut runtime,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(713),
            lifetime: Duration::from_secs(1),
        },
        at,
    )
    .unwrap() else {
        panic!()
    };
    output(
        &mut runtime,
        OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(37.0),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(200),
        }),
        at,
    )
    .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(old)) =
        output(&mut runtime, OutputCommand::BeginDispatch, at).unwrap()
    else {
        panic!()
    };
    output(
        &mut runtime,
        OutputCommand::RequestSafe,
        Duration::from_millis(2),
    )
    .unwrap();
    output(
        &mut runtime,
        OutputCommand::Complete {
            dispatch_id: old.id(),
            outcome: lab_core::output::DispatchOutcome::Acknowledged,
        },
        Duration::from_millis(3),
    )
    .unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("late virtual ACK archive").unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_millis(4)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    archive.close().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let (epoch, dispatch): (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT authority_epoch,dispatch_id FROM output_events WHERE stage='acknowledged'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(epoch, lease.epoch().to_be_bytes());
    let (instance, sequence) = old.id().diagnostic_parts();
    let expected_dispatch = [instance.to_be_bytes(), sequence.to_be_bytes()].concat();
    assert_eq!(dispatch, expected_dispatch);
    let safe_evidence: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM output_events WHERE stage IN ('safe_acknowledged','safe_readback_verified')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(safe_evidence, 0);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn late_m3_ack_after_safe_epoch_reopens_only_as_old_ordinary_evidence() {
    let path = temporary_database();
    let (mut runtime, actuator, wire) = setup();
    runtime.enable_recording_facts();
    let at = Duration::from_millis(1);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(2),
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
    wire.borrow_mut().limits.clear();
    wire.borrow_mut().reply_on_write = false;
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at,
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    runtime.command(Command::PollTransports { at }).unwrap();
    assert_eq!(wire.borrow().bytes.len(), 7);
    runtime
        .command(Command::Output {
            actuator,
            at: Duration::from_millis(2),
            command: OutputCommand::RequestSafe,
        })
        .unwrap();
    let body = [15, 0, 6, 1];
    wire.borrow_mut().readable.extend(body);
    wire.borrow_mut().readable.push_back(crc(&body));
    runtime
        .command(Command::PollTransports {
            at: Duration::from_millis(3),
        })
        .unwrap();
    let lab_core::QueryResult::Output(after_old) =
        runtime.query(lab_core::Query::Output(actuator)).unwrap()
    else {
        panic!()
    };
    assert_eq!(after_old.state, lab_core::output::OutputState::SafePending);
    assert!(!after_old.safe_confirmed);
    assert!(after_old.epoch > lease.epoch());
    wire.borrow_mut().reply_on_write = true;
    runtime
        .command(Command::QueueMetakonOutput {
            actuator,
            at: Duration::from_millis(4),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(100),
        })
        .unwrap();
    for ms in [4, 5] {
        runtime
            .command(Command::PollTransports {
                at: Duration::from_millis(ms),
            })
            .unwrap();
    }
    let facts = runtime.take_recording_facts();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("late M3 ACK archive").unwrap();
    store.append_facts(&facts).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_millis(6)).unwrap();
    store.close().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let ordinary: Vec<Vec<u8>> = db
        .prepare("SELECT authority_epoch FROM output_events WHERE stage='acknowledged'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ordinary, vec![lease.epoch().to_be_bytes().to_vec()]);
    let safe: Vec<Vec<u8>> = db
        .prepare("SELECT authority_epoch FROM output_events WHERE stage='safe_acknowledged'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(safe.len(), 1);
    assert_ne!(safe[0], ordinary[0]);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
