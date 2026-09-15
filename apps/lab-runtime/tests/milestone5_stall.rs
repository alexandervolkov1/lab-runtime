//! A test-controlled blocked component worker cannot hold native control or safety.

use lab_core::{
    AccessMode, Command, InstrumentId, ParameterId, ParameterRole, Query, QueryResult, Runtime,
    SignalId, TEMPERATURE, Unit, ValueSpec, WriteEffect,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    managed::{
        ComponentDefinition, ComponentError, ComponentExecutor, ComponentId, ComponentKind,
        ComponentManifest, Correlation, Invocation, PlainData,
    },
    metakon::crc,
    output::{
        ActuatorId, EvidenceLevel, OutputCommand, OutputOwner, OutputResult, OutputState,
        SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    reference::{ReferenceConfig, ReferenceId},
    transport::{ByteTransport, ExecutorState, RecoveryStatus, ResourceId, TransportIoError},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};

struct BarrierExecutor {
    sender: SyncSender<Invocation>,
    finished: Receiver<lab_core::managed::ComponentCompletion>,
}

impl ComponentExecutor for BarrierExecutor {
    fn try_submit(&mut self, job: Invocation) -> Result<(), ComponentError> {
        self.sender.try_send(job).map_err(|_| ComponentError::Busy)
    }

    fn try_poll(&mut self) -> Option<lab_core::managed::ComponentCompletion> {
        self.finished.try_recv().ok()
    }

    fn try_cancel(&mut self, _: Correlation) {}

    fn try_expire(&mut self, _: Correlation) -> bool {
        false
    }
}

#[derive(Default)]
struct Wire {
    recoveries: usize,
    readable: Vec<u8>,
}
struct BadCrcTransport(Rc<RefCell<Wire>>);
impl ByteTransport for BadCrcTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut response = vec![1, 0, 1, 0, 0x44, 10, 0];
        response.push(crc(&response) ^ 1);
        self.0.borrow_mut().readable = response;
        Ok(bytes.len())
    }
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        bytes[..count].copy_from_slice(&wire.readable[..count]);
        wire.readable.drain(..count);
        Ok(count)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.recoveries += 1;
        wire.readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}

#[test]
fn worker_barrier_allows_renewed_native_ticks_bus_recovery_and_rust_safe_before_release() {
    let mut runtime = Runtime::new();
    let (jobs, receiver) = mpsc::sync_channel(1);
    let (reached, started) = mpsc::sync_channel(0);
    let (release, unblock) = mpsc::sync_channel(0);
    let (_results, completions) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let _job: Invocation = receiver.recv().unwrap();
        reached.send(()).unwrap();
        unblock.recv().unwrap();
    });
    runtime
        .install_component_executor(Box::new(BarrierExecutor {
            sender: jobs,
            finished: completions,
        }))
        .unwrap();
    let component = ComponentId::new(201);
    runtime
        .command(Command::StageComponent {
            definition: ComponentDefinition {
                manifest: ComponentManifest {
                    schema_version: 1,
                    id: component,
                    instrument: InstrumentId::new(201),
                    name: "blocked model".into(),
                    parameter: TEMPERATURE,
                    kind: ComponentKind::Source,
                    unit: Unit::CELSIUS,
                    min: 0.0,
                    max: 100.0,
                    warmup_samples: 1,
                    max_input_age: Duration::from_secs(2),
                    history_capacity: 8,
                },
                source: "return function(ctx) return ctx end".into(),
                config: PlainData::default(),
            },
            replaces: None,
            at: Duration::ZERO,
        })
        .unwrap();
    // This handshake makes all later checks happen while the worker is blocked.
    started.recv().unwrap();

    let id = InstrumentId::new(301);
    let target = ActuatorId::new(id, lab_core::HEATER_POWER);
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id,
            name: "independent virtual thermal plant".into(),
            history_capacity: 64,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: target,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(10),
                max_proposal_ttl: Duration::from_secs(2),
                required_evidence: EvidenceLevel::Readback,
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: target,
            command: OutputCommand::RequestSafe,
            at: Duration::ZERO,
        })
        .unwrap();
    let lab_core::CommandResult::Output(OutputResult::Dispatched(safe)) = runtime
        .command(Command::Output {
            actuator: target,
            command: OutputCommand::BeginDispatch,
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator: target,
            command: OutputCommand::Complete {
                dispatch_id: safe.id(),
                outcome: lab_core::output::DispatchOutcome::ReadbackVerified,
            },
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: ReferenceId::new(1),
            value: 60.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: ControllerId::new(42),
            input: SignalId::new(id, TEMPERATURE),
            output: target,
            reference: ReferenceId::new(1),
            ema: EmaConfig {
                time_constant: Duration::from_secs(1),
                warmup_samples: 1,
                unit: Unit::CELSIUS,
            },
            pid: PidConfig {
                kp: 3.0,
                ki: 0.4,
                kd: 0.2,
                output_min: 0.0,
                output_max: 100.0,
            },
            max_input_age: Duration::from_secs(2),
            max_tick_gap: Duration::from_secs(2),
            lease_lifetime: Duration::from_secs(5),
            proposal_ttl: Duration::from_secs(1),
        }))
        .unwrap();
    runtime
        .command(Command::PrepareController(ControllerId::new(42)))
        .unwrap();

    let manual_id = InstrumentId::new(303);
    let manual = ActuatorId::new(manual_id, lab_core::HEATER_POWER);
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: manual_id,
            name: "independent manual virtual plant".into(),
            history_capacity: 16,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: manual,
            command: OutputCommand::BindProfile(SafeProfile {
                min: 0.0,
                max: 100.0,
                safe_value: 0.0,
                max_lease: Duration::from_secs(10),
                max_proposal_ttl: Duration::from_secs(2),
                required_evidence: EvidenceLevel::Readback,
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: manual,
            command: OutputCommand::RequestSafe,
            at: Duration::ZERO,
        })
        .unwrap();
    let lab_core::CommandResult::Output(OutputResult::Dispatched(initial_safe)) = runtime
        .command(Command::Output {
            actuator: manual,
            command: OutputCommand::BeginDispatch,
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator: manual,
            command: OutputCommand::Complete {
                dispatch_id: initial_safe.id(),
                outcome: lab_core::output::DispatchOutcome::ReadbackVerified,
            },
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator: manual,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(9),
                lifetime: Duration::from_secs(2),
            },
            at: Duration::ZERO,
        })
        .unwrap();

    let wire = Rc::new(RefCell::new(Wire::default()));
    runtime
        .register_transport(ResourceId::new(7), Box::new(BadCrcTransport(wire.clone())))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: InstrumentId::new(70),
                name: "independent native bus".into(),
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
            },
            history_capacity: 4,
        }))
        .unwrap();

    for second in 0..=16 {
        let at = Duration::from_secs(second);
        runtime
            .command(Command::RefreshMeasurement {
                instrument: id,
                parameter: TEMPERATURE,
                at,
            })
            .unwrap();
        runtime
            .command(if second == 0 {
                Command::StartController {
                    controller: ControllerId::new(42),
                    at,
                }
            } else {
                Command::TickController {
                    controller: ControllerId::new(42),
                    at,
                }
            })
            .unwrap();
        runtime.command(Command::PollComponents { at }).unwrap();
    }
    let QueryResult::Controller(running) = runtime
        .query(Query::Controller(ControllerId::new(42)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(running.state, ControllerState::Running);
    let QueryResult::Output(renewed) = runtime.query(Query::Output(target)).unwrap() else {
        panic!()
    };
    assert!(
        renewed
            .lease
            .as_ref()
            .is_some_and(|lease| lease.expires() > Duration::from_secs(16))
    );

    let QueryResult::Output(manual_safe) = runtime.query(Query::Output(manual)).unwrap() else {
        panic!()
    };
    assert!(manual_safe.lease.is_none());
    assert!(manual_safe.safe_confirmed);
    assert_eq!(manual_safe.readback.unwrap().value, 0.0);

    runtime
        .command(Command::QueueMetakonRead {
            instrument: InstrumentId::new(70),
            parameter: ParameterId::new(1),
            at: Duration::from_secs(16),
            queue_ttl: Duration::from_secs(3),
            timeout: Duration::from_secs(1),
        })
        .unwrap();
    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_secs(17),
        })
        .unwrap();
    let QueryResult::Transport(recovering) =
        runtime.query(Query::Transport(ResourceId::new(7))).unwrap()
    else {
        panic!()
    };
    assert_eq!(recovering.state, ExecutorState::Recovering);
    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_secs(18),
        })
        .unwrap();
    assert_eq!(wire.borrow().recoveries, 1);

    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_secs(20),
        })
        .unwrap();
    let QueryResult::Controller(failed) = runtime
        .query(Query::Controller(ControllerId::new(42)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(failed.state, ControllerState::Failed);
    let QueryResult::Output(safe) = runtime.query(Query::Output(target)).unwrap() else {
        panic!()
    };
    assert_eq!(safe.state, OutputState::FaultLatched);
    assert!(safe.safe_confirmed);

    release.send(()).unwrap();
    worker.join().unwrap();
}
