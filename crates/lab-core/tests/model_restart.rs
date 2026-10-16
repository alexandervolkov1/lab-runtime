//! C11 acceptance for a fenced native virtual-model restart.

use lab_core::{
    Command, CommandResult, InstrumentId, Query, QueryResult, Runtime, SignalId, TEMPERATURE,
    plant::ThermalPlantConfig,
};
use std::time::Duration;

const PLANT: InstrumentId = InstrumentId::new(17);

fn config(initial_temperature: f64) -> ThermalPlantConfig {
    ThermalPlantConfig {
        id: PLANT,
        name: "restartable plant".into(),
        history_capacity: 8,
        ambient_temperature: 20.0,
        initial_temperature,
        gain_per_percent: 0.8,
        time_constant: Duration::from_secs(8),
    }
}

#[test]
fn c11_restart_advances_generation_and_old_observation_is_not_current() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(config(31.0)))
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();

    let result = runtime
        .command(Command::RestartThermalPlant {
            instrument: PLANT,
            config: config(24.0),
            expected_generation: 1,
            at: Duration::from_secs(2),
        })
        .unwrap();
    assert_eq!(
        result,
        CommandResult::ModelRestarted {
            instrument: PLANT,
            generation: 2
        }
    );
    assert_eq!(
        runtime
            .query(Query::GetLatestSignal(SignalId::new(PLANT, TEMPERATURE)))
            .unwrap(),
        QueryResult::Latest(None)
    );

    let CommandResult::MeasurementRefreshed(sample) = runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: TEMPERATURE,
            at: Duration::from_secs(3),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(24.0)));
}

#[test]
fn c11_stale_generation_rejects_without_resetting_current_observation() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(config(31.0)))
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();

    assert!(
        runtime
            .command(Command::RestartThermalPlant {
                instrument: PLANT,
                config: config(24.0),
                expected_generation: 9,
                at: Duration::from_secs(2),
            })
            .is_err()
    );
    let QueryResult::Latest(Some(sample)) = runtime
        .query(Query::GetLatestSignal(SignalId::new(PLANT, TEMPERATURE)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(sample.value(), Some(&lab_core::Value::Float(31.0)));
}
