use lab_core::*;

#[test]
fn every_value_variant_is_checked_without_coercion() {
    let cases = [
        (
            ValueSpec::Float {
                min: -1.0,
                max: 1.0,
            },
            Value::Float(0.5),
        ),
        (ValueSpec::Integer { min: -2, max: 2 }, Value::Integer(2)),
        (ValueSpec::Boolean, Value::Boolean(false)),
        (ValueSpec::Text { max_bytes: 8 }, Value::Text("°C".into())),
        (
            ValueSpec::Enum {
                choices: vec!["off".into(), "on".into()],
            },
            Value::Enum("on".into()),
        ),
    ];
    for (spec, valid) in &cases {
        assert_eq!(spec.validate(valid), Ok(()));
        assert_eq!(spec.value_type(), valid.value_type());
        for (_, other) in &cases {
            if valid.value_type() != other.value_type() {
                assert_eq!(
                    spec.validate(other),
                    Err(Error::WrongType {
                        expected: valid.value_type(),
                        actual: other.value_type()
                    })
                );
            }
        }
    }
}

#[test]
fn inclusive_numeric_bounds_and_nonfinite_values_are_validated() {
    let spec = ValueSpec::Float {
        min: -1.0,
        max: 1.0,
    };
    for value in [-1.0, 0.0, 1.0] {
        assert_eq!(spec.validate(&Value::Float(value)), Ok(()));
    }
    for value in [-1.01, 1.01] {
        assert_eq!(spec.validate(&Value::Float(value)), Err(Error::OutOfRange));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(spec.validate(&Value::Float(value)), Err(Error::NonFinite));
    }
    let integer = ValueSpec::Integer {
        min: i64::MIN,
        max: i64::MAX,
    };
    for value in [i64::MIN, 0, i64::MAX] {
        assert_eq!(integer.validate(&Value::Integer(value)), Ok(()));
    }
    assert_eq!(
        ValueSpec::Integer { min: 1, max: 2 }.validate(&Value::Integer(0)),
        Err(Error::OutOfRange)
    );
}

#[test]
fn text_enum_and_invalid_specs_are_bounded() {
    assert_eq!(
        ValueSpec::Text { max_bytes: 2 }.validate(&Value::Text("°".into())),
        Ok(())
    );
    assert_eq!(
        ValueSpec::Text { max_bytes: 1 }.validate(&Value::Text("°".into())),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        ValueSpec::Enum {
            choices: vec!["yes".into()]
        }
        .validate(&Value::Enum("no".into())),
        Err(Error::OutOfRange)
    );
    for spec in [
        ValueSpec::Float { min: 2.0, max: 1.0 },
        ValueSpec::Float {
            min: f64::NAN,
            max: 1.0,
        },
        ValueSpec::Float {
            min: 0.0,
            max: f64::INFINITY,
        },
        ValueSpec::Integer { min: 2, max: 1 },
        ValueSpec::Text {
            max_bytes: MAX_TEXT_BYTES + 1,
        },
        ValueSpec::Enum { choices: vec![] },
        ValueSpec::Enum {
            choices: vec!["x".into(), "x".into()],
        },
        ValueSpec::Enum {
            choices: vec![" ".into()],
        },
        ValueSpec::Enum {
            choices: vec!["x".repeat(MAX_TEXT_BYTES + 1)],
        },
        ValueSpec::Enum {
            choices: (0..=MAX_ENUM_CHOICES).map(|n| n.to_string()).collect(),
        },
    ] {
        assert!(matches!(
            spec.validate_definition(),
            Err(Error::InvalidConfiguration(_))
        ));
    }
}

#[test]
fn units_and_descriptor_semantics_are_explicit() {
    let descriptor = ParameterDescriptor {
        id: ParameterId::new(1),
        name: "value".into(),
        value_spec: ValueSpec::Boolean,
        unit: Unit::Unitless,
        access: AccessMode::ReadWrite,
        role: ParameterRole::Configuration,
        write_effect: WriteEffect::ConfigurationOnly,
        signal: None,
    };
    descriptor.validate_definition().unwrap();
    let mut invalid = descriptor.clone();
    invalid.unit = Unit::Celsius;
    assert!(matches!(
        invalid.validate_definition(),
        Err(Error::InvalidConfiguration(_))
    ));
    invalid = descriptor.clone();
    invalid.access = AccessMode::ReadOnly;
    assert!(invalid.validate_definition().is_err());
    invalid = descriptor.clone();
    invalid.role = ParameterRole::Actuator;
    assert!(invalid.validate_definition().is_err());
    assert_eq!(Unit::Celsius.symbol(), "°C");
    assert_eq!(Unit::Percent.symbol(), "%");
    assert_eq!(Unit::Pascal.symbol(), "Pa");
    assert_eq!(Unit::Unitless.symbol(), "1");
}

#[test]
fn errors_are_typed_and_displayable() {
    fn accepts_error(_: &dyn std::error::Error) {}
    let error = Error::WrongType {
        expected: ValueType::Float,
        actual: ValueType::Integer,
    };
    accepts_error(&error);
    assert!(error.to_string().contains("Float"));
    assert!(error.to_string().contains("Integer"));
}
