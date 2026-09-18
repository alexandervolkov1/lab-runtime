//! M3 acceptance for extensible unit metadata and the pure Metakon codec.

use lab_core::{
    Unit, Value,
    metakon::{
        Address, CodecError, ExpectedRead, MetakonType, MetakonValue, TemperatureReading, crc,
        decode_ack, decode_read, encode_read, encode_scaled_i8, encode_write, scale_temperature,
    },
};

#[test]
fn custom_units_are_bounded_values_not_core_enum_variants() {
    let sccm = Unit::new("sccm", "sccm").unwrap();
    let milliamp = Unit::new("mA", "mA").unwrap();
    assert_eq!(sccm.id(), "sccm");
    assert_eq!(sccm.symbol(), "sccm");
    assert_ne!(sccm, milliamp);
    assert_eq!(Unit::new("sccm", "std cm3/min").unwrap(), sccm);

    assert!(Unit::new("", "x").is_err());
    assert!(Unit::new("blank id", "x").is_err());
    assert!(Unit::new("x", " ").is_err());
    assert!(Unit::new(&"x".repeat(33), "x").is_err());
    assert!(Unit::new("x", &"x".repeat(17)).is_err());
}

#[test]
fn built_in_units_use_the_same_identity_model() {
    assert_eq!(Unit::CELSIUS.id(), "degC");
    assert_eq!(Unit::CELSIUS.symbol(), "°C");
    assert_eq!(Unit::PERCENT, Unit::new("percent", "pct").unwrap());
    assert_ne!(Unit::PERCENT, Unit::UNITLESS);
}

#[test]
fn crc_and_request_frames_match_read_only_donor_vectors() {
    assert_eq!(crc(&[0x00]), 0x35);
    assert_eq!(crc(&[0x01]), 0x6b);
    assert_eq!(crc(&[0xff]), 0x00);

    assert_eq!(
        encode_read(Address::new(1, 0, 1)).unwrap().as_bytes(),
        &[0x01, 0x00, 0x01, 0x00, 0xa0]
    );
    assert_eq!(
        encode_read(Address::new(2, 0, 1)).unwrap().as_bytes(),
        &[0x02, 0x00, 0x01, 0x00, 0x28]
    );
    assert_eq!(
        encode_write(Address::new(15, 0, 2), MetakonValue::I16(1234))
            .unwrap()
            .as_bytes(),
        &[0x0f, 0x00, 0x02, 0x01, 0x04, 0xd2, 0x04, 0x7e]
    );
    assert_eq!(
        encode_write(Address::new(15, 0, 6), MetakonValue::I8(-25))
            .unwrap()
            .as_bytes(),
        &[0x0f, 0x00, 0x06, 0x01, 0x02, 0xe7, 0x2b]
    );
}

#[test]
fn read_and_ack_decoding_is_strict() {
    let expected = ExpectedRead::new(Address::new(1, 0, 1), MetakonType::I16, true, false);
    let frame = [0x01, 0x00, 0x01, 0x00, 0x44, 0xd2, 0x04, 0xf1];
    assert_eq!(decode_read(&frame, expected), Ok(MetakonValue::I16(1234)));
    let ack_address = Address::new(15, 0, 2);
    let ack = [0x0f, 0x00, 0x02, 0x01, 0xbe];
    assert_eq!(decode_ack(&ack, ack_address), Ok(()));
    assert_eq!(
        decode_ack(&ack[..4], ack_address),
        Err(CodecError::WrongLength)
    );
    let mut corrupt_ack_crc = ack;
    corrupt_ack_crc[4] ^= 1;
    assert_eq!(
        decode_ack(&corrupt_ack_crc, ack_address),
        Err(CodecError::BadCrc)
    );
    let mut wrong_ack_address = [0x0f, 0x00, 0x03, 0x01, 0];
    wrong_ack_address[4] = crc(&wrong_ack_address[..4]);
    assert_eq!(
        decode_ack(&wrong_ack_address, ack_address),
        Err(CodecError::WrongAddress)
    );

    for bad in [
        [0x02, 0x00, 0x01, 0x00, 0x44, 0xd2, 0x04, 0x79],
        [0x01, 0x01, 0x01, 0x00, 0x44, 0xd2, 0x04, 0x95],
        [0x01, 0x00, 0x02, 0x00, 0x44, 0xd2, 0x04, 0x82],
    ] {
        assert!(decode_read(&bad, expected).is_err());
    }
    let mut corrupt_crc = frame;
    corrupt_crc[7] ^= 1;
    assert_eq!(decode_read(&corrupt_crc, expected), Err(CodecError::BadCrc));
    assert_eq!(
        decode_read(&frame[..7], expected),
        Err(CodecError::WrongLength)
    );
    assert_eq!(
        decode_read(&[0; 39], expected),
        Err(CodecError::FrameTooLong)
    );
}

#[test]
fn measurement_scaling_and_output_representation_are_explicit() {
    assert_eq!(
        scale_temperature(MetakonValue::I16(1234), 0.1).unwrap(),
        TemperatureReading::Value(123.4)
    );
    assert_eq!(
        scale_temperature(MetakonValue::I16(i16::MIN), 0.1).unwrap(),
        TemperatureReading::SensorFault
    );
    assert!(scale_temperature(MetakonValue::I16(10_000), 0.1).is_err());
    assert!(scale_temperature(MetakonValue::I16(1), f64::NAN).is_err());

    assert_eq!(encode_scaled_i8(&Value::Float(-100.0), 1.0), Ok(-100));
    assert_eq!(encode_scaled_i8(&Value::Float(100.0), 1.0), Ok(100));
    assert!(encode_scaled_i8(&Value::Float(1.5), 1.0).is_err());
    assert!(encode_scaled_i8(&Value::Float(101.0), 1.0).is_err());
    assert!(encode_scaled_i8(&Value::Boolean(true), 1.0).is_err());
}

#[test]
fn register_six_output_values_have_exact_signed_i8_frames() {
    for (engineering, raw) in [
        (-100.0, -100_i8),
        (-10.0, -10_i8),
        (0.0, 0_i8),
        (10.0, 10_i8),
        (100.0, 100_i8),
    ] {
        assert_eq!(encode_scaled_i8(&Value::Float(engineering), 1.0), Ok(raw));
        let frame = encode_write(Address::new(5, 0, 6), MetakonValue::I8(raw)).unwrap();
        assert_eq!(&frame.as_bytes()[..6], &[5, 0, 6, 1, 2, raw as u8]);
        assert_eq!(frame.as_bytes()[6], crc(&frame.as_bytes()[..6]));
    }
    assert!(encode_scaled_i8(&Value::Float(-101.0), 1.0).is_err());
    assert!(encode_scaled_i8(&Value::Float(101.0), 1.0).is_err());
}
