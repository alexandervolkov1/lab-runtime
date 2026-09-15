//! Reopened rows preserve unavailable attempts and every current scalar kind.

use lab_core::{
    AccessMode, Command, InstrumentId, ParameterId, ParameterRole, Runtime, Sample, SignalId, Unit,
    Value, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    plant::ThermalPlantConfig,
    recording::RecordingFact,
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::recorder::SqliteStore;
use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc, time::Duration};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-quality-{suffix}.sqlite"))
}

#[test]
fn same_publication_time_failure_preserves_both_original_facts_after_reopen() {
    let path = temporary_database();
    let instrument = InstrumentId::new(818);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: instrument,
            name: "same-time invalidation".into(),
            history_capacity: 1,
            ambient_temperature: 20.0,
            gain_per_percent: 1.0,
            initial_temperature: 20.0,
            time_constant: Duration::from_secs(1),
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_secs(1);
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at,
        })
        .unwrap();
    runtime
        .command(Command::InjectPlantMeasurementFailure { instrument, at })
        .unwrap();
    let facts = runtime.take_recording_facts();
    assert_eq!(facts.len(), 2);
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("equal-time quality").unwrap();
    store.append_facts(&facts).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(2)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let rows = archive
        .read_measurements(instrument, lab_core::TEMPERATURE, 8)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].quality, "good");
    assert_eq!(rows[1].quality, "unavailable");
    assert!(rows[0].value.is_some());
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[0].published_at, at);
    assert_eq!(rows[1].published_at, at);
    assert!(rows[0].record_sequence < rows[1].record_sequence);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[derive(Default)]
struct SensorWire {
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
}
struct SensorFake(Rc<RefCell<SensorWire>>);
impl ByteTransport for SensorFake {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
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
        self.0.borrow_mut().readable.clear();
        Ok(RecoveryStatus::Complete)
    }
}
fn sensor_response(raw: i16) -> Vec<u8> {
    let [low, high] = raw.to_le_bytes();
    let mut bytes = vec![1, 0, 1, 0, 0x44, low, high];
    bytes.push(lab_core::metakon::crc(&bytes));
    bytes
}

#[test]
fn native_sensor_sentinel_reopens_as_unavailable_without_reusing_previous_good() {
    let path = temporary_database();
    let instrument = InstrumentId::new(820);
    let parameter = ParameterId::new(1);
    let resource = ResourceId::new(7);
    let wire = Rc::new(RefCell::new(SensorWire {
        responses: VecDeque::from([sensor_response(1234), sensor_response(i16::MIN)]),
        ..SensorWire::default()
    }));
    let mut runtime = Runtime::new();
    runtime
        .register_transport(resource, Box::new(SensorFake(wire)))
        .unwrap();
    runtime
        .command(Command::RegisterMetakon(MetakonInstrumentConfig {
            definition: DataInstrumentDefinition {
                schema_version: 1,
                id: instrument,
                name: "native sensor fault archive".into(),
                parameters: vec![DataParameterDefinition {
                    id: parameter,
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
                resource,
                device: 1,
                channel: 0,
                binding_generation: 1,
                mapping_revision: 1,
                expected_output_unit: None,
            },
            history_capacity: 1,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("sensor sentinel").unwrap();
    for milliseconds in 1..=2u64 {
        let at = Duration::from_millis(milliseconds);
        runtime
            .command(Command::QueueMetakonRead {
                instrument,
                parameter,
                at,
                queue_ttl: Duration::from_millis(50),
                timeout: Duration::from_millis(20),
            })
            .unwrap();
        runtime.command(Command::PollTransports { at }).unwrap();
        store.append_facts(&runtime.take_recording_facts()).unwrap();
    }
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_millis(3)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let rows = archive.read_measurements(instrument, parameter, 8).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value, Some(Value::Float(123.4)));
    assert_eq!(rows[0].quality, "good");
    assert_eq!(rows[1].quality, "unavailable");
    assert_eq!(rows[1].failure.as_deref(), Some("SensorFault"));
    assert_eq!(rows[1].value, None);
    assert!(rows[0].record_sequence < rows[1].record_sequence);
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn all_scalar_kinds_and_delayed_publication_reopen_without_coercion() {
    let path = temporary_database();
    let instrument = InstrumentId::new(819);
    let signal = SignalId::new(instrument, ParameterId::new(21));
    let values = [
        Value::Float(12.5),
        Value::Integer(i64::MIN),
        Value::Integer(i64::MAX),
        Value::Boolean(false),
        Value::Text("°C text".into()),
        Value::Enum("ready".into()),
    ];
    let facts: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let at = Duration::from_secs(index as u64 + 1);
            RecordingFact::Measurement {
                sequence: index as u64 + 1,
                sample: Sample::validated_good(signal, Unit::CELSIUS, at, value.clone()).unwrap(),
                generation: 1,
                revision: 1,
            }
        })
        .chain(std::iter::once(RecordingFact::Measurement {
            sequence: 7,
            sample: Sample::derived_good(
                signal,
                Unit::CELSIUS,
                Duration::from_secs(10),
                Duration::from_secs(2),
                Value::Float(44.0),
            )
            .unwrap(),
            generation: 1,
            revision: 2,
        }))
        .collect();
    assert!(
        Sample::validated_good(
            signal,
            Unit::CELSIUS,
            Duration::ZERO,
            Value::Float(f64::NAN)
        )
        .is_err()
    );
    let mut store = SqliteStore::open(&path).unwrap();
    store.start_run("all kinds").unwrap();
    store.append_facts(&facts).unwrap();
    store.stop_run().unwrap();
    store.finish_boot(Duration::from_secs(11)).unwrap();
    store.close().unwrap();
    let archive = SqliteStore::open(&path).unwrap();
    let rows = archive
        .read_measurements(instrument, ParameterId::new(21), 16)
        .unwrap();
    assert_eq!(rows.len(), 7);
    for (row, expected) in rows.iter().zip(values.iter()) {
        assert_eq!(row.value.as_ref(), Some(expected));
        assert_eq!(row.quality, "good");
    }
    assert_eq!(rows[6].value, Some(Value::Float(44.0)));
    assert_eq!(rows[6].published_at, Duration::from_secs(10));
    drop(archive);
    let db = rusqlite::Connection::open(&path).unwrap();
    let observed: Vec<u8> = db.query_row(
        "SELECT observed_at FROM measurements WHERE record_seq=(SELECT record_seq FROM records WHERE fact_seq=?1)",
        [7u64.to_be_bytes().as_slice()],|row|row.get(0)).unwrap();
    assert_eq!(
        observed,
        (Duration::from_secs(2).as_nanos() as u64).to_be_bytes()
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
