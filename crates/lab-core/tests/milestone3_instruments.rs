//! M3 acceptance for generic Metakon descriptors and descriptor-bound output units.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Query,
    QueryResult, Runtime, SignalId, Unit, Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal,
        OutputResult, SafeProfile,
    },
    transport::ResourceId,
};
use std::time::Duration;

fn data_config(id: u64, unit: Unit, expected_unit: Unit) -> MetakonInstrumentConfig {
    let instrument = InstrumentId::new(id);
    MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: instrument,
            name: format!("Flow device {id}"),
            parameters: vec![
                DataParameterDefinition {
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
                },
                DataParameterDefinition {
                    id: ParameterId::new(6),
                    name: "gas_flow".into(),
                    value_spec: ValueSpec::Float {
                        min: 0.0,
                        max: 100.0,
                    },
                    unit,
                    access: AccessMode::ReadWrite,
                    role: ParameterRole::Actuator,
                    write_effect: WriteEffect::OutputAffecting,
                    operation: KnownOperation::Output,
                    scale: 1.0,
                },
            ],
        },
        binding: MetakonBinding {
            resource: ResourceId::new(1),
            device: 15,
            channel: 0,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: Some(expected_unit),
            output_queue_ttl: Some(Duration::from_secs(1)),
            output_timeout: Some(Duration::from_millis(100)),
        },
        history_capacity: 4,
    }
}

fn output(
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

#[test]
fn custom_unit_is_discovered_and_used_by_the_normal_authority_path() {
    let sccm = Unit::new("sccm", "sccm").unwrap();
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterMetakon(data_config(10, sccm, sccm)))
        .unwrap();
    let QueryResult::Descriptor(descriptor) = runtime
        .query(Query::DescribeInstrument(InstrumentId::new(10)))
        .unwrap()
    else {
        panic!()
    };
    let actuator_descriptor = descriptor.parameter(ParameterId::new(6)).unwrap();
    assert_eq!(actuator_descriptor.unit.id(), "sccm");
    assert_eq!(actuator_descriptor.signal, None);

    let actuator = ActuatorId::new(InstrumentId::new(10), ParameterId::new(6));
    output(
        &mut runtime,
        actuator,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 5.0,
            max_lease: Duration::from_secs(2),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Readback,
        }),
        0,
    );
    output(&mut runtime, actuator, OutputCommand::RequestSafe, 0);
    let OutputResult::Dispatched(safe) =
        output(&mut runtime, actuator, OutputCommand::BeginDispatch, 0)
    else {
        panic!()
    };
    output(
        &mut runtime,
        actuator,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
        0,
    );
    let OutputResult::Lease(lease) = output(
        &mut runtime,
        actuator,
        OutputCommand::Acquire {
            owner: OutputOwner::Manual(1),
            lifetime: Duration::from_secs(1),
        },
        0,
    ) else {
        panic!()
    };
    assert!(
        runtime
            .command(Command::Output {
                actuator,
                command: OutputCommand::Propose(OutputProposal {
                    lease,
                    value: Value::Float(20.0),
                    unit: Unit::CELSIUS,
                    ttl: Duration::from_millis(100),
                }),
                at: Duration::ZERO,
            })
            .is_err()
    );
    assert_eq!(
        output(
            &mut runtime,
            actuator,
            OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(20.0),
                unit: sccm,
                ttl: Duration::from_millis(100),
            }),
            0,
        ),
        OutputResult::Queued
    );
}

#[test]
fn another_runtime_unit_needs_no_new_core_branch() {
    let rpm = Unit::new("rpm", "rpm").unwrap();
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterMetakon(data_config(20, rpm, rpm)))
        .unwrap();
    let QueryResult::Instruments(descriptors) = runtime.query(Query::Discover).unwrap() else {
        panic!()
    };
    assert_eq!(
        descriptors[0].parameter(ParameterId::new(6)).unwrap().unit,
        rpm
    );
    assert_eq!(
        descriptors[0]
            .parameter(ParameterId::new(1))
            .unwrap()
            .signal,
        Some(SignalId::new(InstrumentId::new(20), ParameterId::new(1)))
    );
}

#[test]
fn output_binding_unit_mismatch_is_atomic() {
    let sccm = Unit::new("sccm", "sccm").unwrap();
    let milliamp = Unit::new("mA", "mA").unwrap();
    let mut runtime = Runtime::new();
    assert!(
        runtime
            .command(Command::RegisterMetakon(data_config(30, sccm, milliamp)))
            .is_err()
    );
    assert_eq!(
        runtime.query(Query::Discover),
        Ok(QueryResult::Instruments(vec![]))
    );
}
