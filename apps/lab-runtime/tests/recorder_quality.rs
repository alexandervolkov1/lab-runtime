//! Reopened rows preserve unavailable attempts and every current scalar kind.

use lab_core::{
    Command, InstrumentId, ParameterId, Runtime, Sample, SignalId, Unit, Value,
    plant::ThermalPlantConfig, recording::RecordingFact,
};
use lab_runtime::recorder::SqliteStore;
use std::{path::PathBuf, time::Duration};

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
