//! Acceptance tests written before the primary implementation.
//! Rename and atomic-validation intent: docs/implementation/MILESTONE_1_DESIGN.md §8.
use lab_core::*;
use std::time::Duration;

const ID: InstrumentId = InstrumentId::new(7);

fn setup(capacity: usize) -> Runtime {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime.command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: ID,
            name: "virtual lab".into(),
            history_capacity: capacity,
            base_temperature: 20.0,
            measurement_enabled: true,
        })),
        Ok(CommandResult::Registered(ID))
    );
    runtime
}

fn signal() -> SignalId {
    SignalId::new(ID, TEMPERATURE)
}

fn describe(runtime: &Runtime) -> InstrumentDescriptor {
    match runtime.query(Query::DescribeInstrument(ID)).unwrap() {
        QueryResult::Descriptor(value) => value,
        other => panic!("unexpected result: {other:?}"),
    }
}

fn state(runtime: &Runtime) -> InstrumentState {
    match runtime.query(Query::GetInstrumentState(ID)).unwrap() {
        QueryResult::State(value) => value,
        other => panic!("unexpected result: {other:?}"),
    }
}

fn window(runtime: &Runtime) -> Vec<Sample> {
    match runtime.query(Query::GetSignalWindow(signal())).unwrap() {
        QueryResult::Window(value) => value,
        other => panic!("unexpected result: {other:?}"),
    }
}

fn latest(runtime: &Runtime) -> Option<Sample> {
    match runtime.query(Query::GetLatestSignal(signal())).unwrap() {
        QueryResult::Latest(value) => value,
        other => panic!("unexpected result: {other:?}"),
    }
}

fn refresh(runtime: &mut Runtime, millis: u64) -> Result<CommandResult, Error> {
    runtime.command(Command::RefreshMeasurement {
        instrument: ID,
        parameter: TEMPERATURE,
        at: Duration::from_millis(millis),
    })
}

fn configure(
    runtime: &mut Runtime,
    parameter: ParameterId,
    value: Value,
) -> Result<CommandResult, Error> {
    runtime.command(Command::ConfigureParameter {
        instrument: ID,
        parameter,
        value,
    })
}

#[test]
fn discovery_is_generic_and_initial_observation_is_unknown() {
    let runtime = setup(4);
    let QueryResult::Instruments(instruments) = runtime.query(Query::Discover).unwrap() else {
        panic!("expected discovery")
    };
    assert_eq!(instruments.len(), 1);
    // This client knows descriptor fields, not a concrete instrument type.
    let display: Vec<_> = instruments
        .iter()
        .map(|instrument| {
            (
                instrument.id,
                instrument.name.clone(),
                instrument
                    .parameters
                    .iter()
                    .map(|parameter| (parameter.id, parameter.name.clone(), parameter.unit))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(display[0].0, ID);
    assert_eq!(display[0].2.len(), 4);
    assert!(latest(&runtime).is_none());
    assert!(window(&runtime).is_empty());
    let status = state(&runtime);
    assert_eq!(status.configured.len(), 2);
    assert!(status.observations[0].latest.is_none());
}

#[test]
fn descriptor_exposes_type_units_access_roles_and_side_effects() {
    let mut runtime = setup(4);
    let descriptor = describe(&runtime);
    let measurement = descriptor.parameter(TEMPERATURE).unwrap();
    assert_eq!(measurement.name, "temperature");
    assert_eq!(
        measurement.value_spec,
        ValueSpec::Float {
            min: -100.0,
            max: 110.0
        }
    );
    assert_eq!(measurement.unit, Unit::Celsius);
    assert_eq!(measurement.access, AccessMode::ReadOnly);
    assert_eq!(measurement.role, ParameterRole::Measurement);
    assert_eq!(measurement.write_effect, WriteEffect::None);
    assert_eq!(measurement.signal, Some(signal()));
    let output = descriptor.parameter(HEATER_POWER).unwrap();
    assert_eq!(output.role, ParameterRole::Actuator);
    assert_eq!(output.access, AccessMode::ReadWrite);
    assert_eq!(output.write_effect, WriteEffect::OutputAffecting);
    assert_eq!(output.unit, Unit::Percent);
    assert!(output.signal.is_none());
    let before = state(&runtime);
    assert_eq!(
        configure(&mut runtime, HEATER_POWER, Value::Float(10.0)),
        Err(Error::OperationNotAllowed(HEATER_POWER))
    );
    assert_eq!(state(&runtime), before);
    for parameter in &descriptor.parameters {
        parameter.validate_definition().unwrap();
    }
}

#[test]
fn rename_preserves_identity_configuration_and_samples() {
    let mut runtime = setup(4);
    refresh(&mut runtime, 1000).unwrap();
    let before = describe(&runtime);
    let old_state = state(&runtime);
    let old_window = window(&runtime);
    assert_eq!(
        runtime.command(Command::RenameInstrument {
            instrument: ID,
            name: "renamed".into()
        }),
        Ok(CommandResult::Renamed(ID))
    );
    let after = describe(&runtime);
    assert_eq!(after.id, before.id);
    assert_eq!(after.parameters, before.parameters);
    assert_eq!(after.name, "renamed");
    assert_eq!(state(&runtime), old_state);
    assert_eq!(window(&runtime), old_window);
}

#[test]
fn invalid_configuration_is_atomic_for_type_range_and_access() {
    let mut runtime = setup(4);
    refresh(&mut runtime, 1000).unwrap();
    let before = state(&runtime);
    let history = window(&runtime);
    let cases = [
        (
            BASE_TEMPERATURE,
            Value::Boolean(true),
            Error::WrongType {
                expected: ValueType::Float,
                actual: ValueType::Boolean,
            },
        ),
        (BASE_TEMPERATURE, Value::Float(101.0), Error::OutOfRange),
        (BASE_TEMPERATURE, Value::Float(f64::NAN), Error::NonFinite),
        (
            BASE_TEMPERATURE,
            Value::Float(f64::INFINITY),
            Error::NonFinite,
        ),
        (
            BASE_TEMPERATURE,
            Value::Float(f64::NEG_INFINITY),
            Error::NonFinite,
        ),
        (
            TEMPERATURE,
            Value::Float(1.0),
            Error::ReadOnlyParameter(TEMPERATURE),
        ),
        (
            HEATER_POWER,
            Value::Float(1.0),
            Error::OperationNotAllowed(HEATER_POWER),
        ),
        (
            ParameterId::new(99),
            Value::Float(1.0),
            Error::UnknownParameter {
                instrument: ID,
                parameter: ParameterId::new(99),
            },
        ),
    ];
    for (parameter, value, error) in cases {
        assert_eq!(configure(&mut runtime, parameter, value), Err(error));
        assert_eq!(state(&runtime), before);
        assert_eq!(window(&runtime), history);
    }
    let descriptor = describe(&runtime);
    assert!(matches!(
        runtime.command(Command::RenameInstrument {
            instrument: ID,
            name: "  ".into()
        }),
        Err(Error::InvalidConfiguration(_))
    ));
    assert_eq!(describe(&runtime), descriptor);
}

#[test]
fn queries_are_pure_owned_snapshots_and_do_not_advance_generator() {
    let mut runtime = setup(4);
    refresh(&mut runtime, 1250).unwrap();
    let before = state(&runtime);
    let history = window(&runtime);
    for _ in 0..100 {
        runtime.query(Query::Discover).unwrap();
        assert_eq!(state(&runtime), before);
        assert_eq!(window(&runtime), history);
        assert_eq!(latest(&runtime), history.last().cloned());
        let mut copy = describe(&runtime);
        copy.name.clear();
        copy.parameters.clear();
    }
    assert_eq!(describe(&runtime).parameters.len(), 4);
    let mut sample_copy = window(&runtime);
    sample_copy.clear();
    assert_eq!(window(&runtime), history);
}

#[test]
fn explicit_refresh_uses_supplied_time_and_configuration_preserves_old_observation() {
    let mut runtime = setup(4);
    assert!(latest(&runtime).is_none());
    refresh(&mut runtime, 0).unwrap();
    let old = latest(&runtime).unwrap();
    assert_eq!(old.value(), Some(&Value::Float(20.0)));
    assert_eq!(old.at(), Duration::ZERO);
    assert_eq!(old.unit(), Unit::Celsius);
    assert_eq!(old.signal(), signal());
    configure(&mut runtime, BASE_TEMPERATURE, Value::Float(25.0)).unwrap();
    assert_eq!(latest(&runtime), Some(old));
    refresh(&mut runtime, 2500).unwrap();
    let new = latest(&runtime).unwrap();
    assert_eq!(new.value(), Some(&Value::Float(27.5)));
    assert_eq!(new.at(), Duration::from_millis(2500));
    assert_eq!(new.quality(), SampleQuality::Good);
}

#[test]
fn measurement_failure_is_new_unavailable_observation_not_stale_success() {
    let mut runtime = setup(4);
    refresh(&mut runtime, 1000).unwrap();
    configure(&mut runtime, MEASUREMENT_ENABLED, Value::Boolean(false)).unwrap();
    assert_eq!(
        refresh(&mut runtime, 2000),
        Err(Error::MeasurementUnavailable {
            signal: signal(),
            reason: MeasurementFailure::Disabled
        })
    );
    let failed = latest(&runtime).unwrap();
    assert_eq!(failed.quality(), SampleQuality::Unavailable);
    assert_eq!(failed.failure(), Some(MeasurementFailure::Disabled));
    assert!(failed.value().is_none());
    assert_eq!(failed.at(), Duration::from_secs(2));
    assert_eq!(window(&runtime)[0].value(), Some(&Value::Float(21.0)));
    assert_eq!(window(&runtime).len(), 2);
    configure(&mut runtime, MEASUREMENT_ENABLED, Value::Boolean(true)).unwrap();
    assert_eq!(latest(&runtime), Some(failed));
    refresh(&mut runtime, 3000).unwrap();
    assert_eq!(latest(&runtime).unwrap().quality(), SampleQuality::Good);
    assert_eq!(latest(&runtime).unwrap().value(), Some(&Value::Float(23.0)));
}

#[test]
fn window_evicts_oldest_and_latest_matches_back_at_all_supported_small_bounds() {
    for capacity in [1, 2, 4, 16] {
        let mut runtime = setup(capacity);
        for tick in 0..1000 {
            refresh(&mut runtime, tick).unwrap();
            let samples = window(&runtime);
            assert_eq!(samples.len(), ((tick + 1) as usize).min(capacity));
            assert_eq!(latest(&runtime), samples.last().cloned());
        }
        assert_eq!(
            window(&runtime)[0].at(),
            Duration::from_millis(1000 - capacity as u64)
        );
    }
}

#[test]
fn identical_commands_and_clock_sequence_have_identical_results() {
    fn run() -> (
        InstrumentDescriptor,
        InstrumentState,
        Vec<Sample>,
        Vec<Result<CommandResult, Error>>,
    ) {
        let mut runtime = setup(3);
        let outcomes = vec![
            refresh(&mut runtime, 0),
            configure(&mut runtime, BASE_TEMPERATURE, Value::Float(12.0)),
            refresh(&mut runtime, 750),
            configure(&mut runtime, MEASUREMENT_ENABLED, Value::Boolean(false)),
            refresh(&mut runtime, 1500),
            configure(&mut runtime, MEASUREMENT_ENABLED, Value::Boolean(true)),
            refresh(&mut runtime, 2200),
        ];
        (
            describe(&runtime),
            state(&runtime),
            window(&runtime),
            outcomes,
        )
    }
    assert_eq!(run(), run());
}

#[test]
fn duplicate_ids_rejected_names_allowed_and_lookup_stable() {
    let mut runtime = setup(4);
    let config = VirtualInstrumentConfig {
        id: ID,
        name: "virtual lab".into(),
        history_capacity: 2,
        base_temperature: 5.0,
        measurement_enabled: true,
    };
    let before = runtime.query(Query::Discover).unwrap();
    assert_eq!(
        runtime.command(Command::RegisterVirtual(config.clone())),
        Err(Error::DuplicateInstrument(ID))
    );
    assert_eq!(runtime.query(Query::Discover).unwrap(), before);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: InstrumentId::new(3),
            ..config
        }))
        .unwrap();
    let QueryResult::Instruments(instruments) = runtime.query(Query::Discover).unwrap() else {
        panic!()
    };
    assert_eq!(instruments.len(), 2);
    assert_eq!(instruments[0].id, InstrumentId::new(3));
    assert_eq!(instruments[0].name, instruments[1].name);
    assert_eq!(
        describe(&runtime).parameter(BASE_TEMPERATURE).unwrap().id,
        BASE_TEMPERATURE
    );
}

#[test]
fn invalid_time_and_wrong_refresh_target_do_not_mutate_observations() {
    let mut runtime = setup(4);
    refresh(&mut runtime, 1000).unwrap();
    let before = state(&runtime);
    for time in [1000, 999, 0] {
        assert!(matches!(
            refresh(&mut runtime, time),
            Err(Error::NonMonotonicTime { .. })
        ));
        assert_eq!(state(&runtime), before);
    }
    for parameter in [HEATER_POWER, BASE_TEMPERATURE, MEASUREMENT_ENABLED] {
        assert_eq!(
            runtime.command(Command::RefreshMeasurement {
                instrument: ID,
                parameter,
                at: Duration::from_secs(2)
            }),
            Err(Error::OperationNotAllowed(parameter))
        );
        assert_eq!(state(&runtime), before);
    }
}

#[test]
fn unknown_identifiers_have_typed_errors() {
    let mut runtime = setup(4);
    let unknown = InstrumentId::new(999);
    for query in [
        Query::DescribeInstrument(unknown),
        Query::GetInstrumentState(unknown),
    ] {
        assert_eq!(runtime.query(query), Err(Error::UnknownInstrument(unknown)));
    }
    let unknown_signal = SignalId::new(ID, HEATER_POWER);
    for query in [
        Query::GetLatestSignal(unknown_signal),
        Query::GetSignalWindow(unknown_signal),
    ] {
        assert_eq!(
            runtime.query(query),
            Err(Error::UnknownSignal(unknown_signal))
        );
    }
    assert_eq!(
        runtime.command(Command::RenameInstrument {
            instrument: unknown,
            name: "x".into()
        }),
        Err(Error::UnknownInstrument(unknown))
    );
    assert_eq!(
        runtime.command(Command::RefreshMeasurement {
            instrument: ID,
            parameter: ParameterId::new(88),
            at: Duration::ZERO
        }),
        Err(Error::UnknownParameter {
            instrument: ID,
            parameter: ParameterId::new(88)
        })
    );
}

#[test]
fn invalid_registration_and_registry_bounds_are_atomic() {
    let mut runtime = Runtime::new();
    let config = VirtualInstrumentConfig {
        id: ID,
        name: "virtual".into(),
        history_capacity: 2,
        base_temperature: 20.0,
        measurement_enabled: true,
    };
    for bad in [
        VirtualInstrumentConfig {
            history_capacity: 0,
            ..config.clone()
        },
        VirtualInstrumentConfig {
            history_capacity: MAX_HISTORY_CAPACITY + 1,
            ..config.clone()
        },
        VirtualInstrumentConfig {
            name: " ".into(),
            ..config.clone()
        },
        VirtualInstrumentConfig {
            name: "x".repeat(MAX_NAME_BYTES + 1),
            ..config.clone()
        },
        VirtualInstrumentConfig {
            base_temperature: f64::NAN,
            ..config.clone()
        },
        VirtualInstrumentConfig {
            base_temperature: -101.0,
            ..config.clone()
        },
    ] {
        assert!(runtime.command(Command::RegisterVirtual(bad)).is_err());
        assert_eq!(
            runtime.query(Query::Discover).unwrap(),
            QueryResult::Instruments(vec![])
        );
    }
    for id in 0..MAX_INSTRUMENTS {
        runtime
            .command(Command::RegisterVirtual(VirtualInstrumentConfig {
                id: InstrumentId::new(id as u64),
                ..config.clone()
            }))
            .unwrap();
    }
    let before = runtime.query(Query::Discover).unwrap();
    assert!(matches!(
        runtime.command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: InstrumentId::new(1000),
            ..config
        })),
        Err(Error::InvalidConfiguration(_))
    ));
    assert_eq!(runtime.query(Query::Discover).unwrap(), before);
}

#[test]
fn large_duration_is_finite_and_each_instrument_has_independent_time() {
    let mut runtime = setup(4);
    runtime
        .command(Command::RefreshMeasurement {
            instrument: ID,
            parameter: TEMPERATURE,
            at: Duration::MAX,
        })
        .unwrap();
    let Some(Value::Float(value)) = latest(&runtime).unwrap().value().cloned() else {
        panic!()
    };
    assert!(value.is_finite() && (20.0..30.0).contains(&value));
    let second = InstrumentId::new(2);
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: second,
            name: "second".into(),
            history_capacity: 1,
            base_temperature: 0.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: second,
            parameter: TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
}
