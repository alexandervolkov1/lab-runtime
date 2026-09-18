use super::*;
use crate::configuration::{
    InstrumentDto, InstrumentPropertyAccess, InstrumentPropertyMutation, InstrumentPropertyType,
    InstrumentPropertyUnit,
};
use std::path::PathBuf;

fn projected(source: &impl InstrumentPropertySource) -> Vec<Value> {
    let mut records = Vec::new();
    project_instrument_properties(source, "7", &mut records);
    records
}

#[test]
fn existing_instrument_property_contract_is_preserved_by_neutral_projection() {
    let virtual_measurement = InstrumentDto::VirtualMeasurement {
        id: 41,
        key: "virtual".into(),
        display_name: "Virtual input".into(),
        history_capacity: 16,
        base_temperature: 22.5,
        measurement_enabled: true,
        external_publication: false,
        poll_period_ms: 100,
    };
    assert_eq!(
        projected(&virtual_measurement),
        vec![
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"41"},"property":"display_name"},"owner":{"kind":"instrument","id":"41"},"property":"display_name","value_type":"text","current":"Virtual input","unit":null,"access":"read_write","mutation_class":"live_safe","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"41"},"property":"poll_period_ms"},"owner":{"kind":"instrument","id":"41"},"property":"poll_period_ms","value_type":"integer","current":100,"unit":{"id":"ms","symbol":"ms"},"access":"read_write","mutation_class":"ordinary_live","constraints":{"minimum":1,"maximum":60000},"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"41"},"property":"base_temperature"},"owner":{"kind":"instrument","id":"41"},"property":"base_temperature","value_type":"number","current":22.5,"unit":null,"access":"deployment_only","mutation_class":"reinitialize","constraints":{"minimum":-100.0,"maximum":100.0},"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"41"},"property":"measurement_enabled"},"owner":{"kind":"instrument","id":"41"},"property":"measurement_enabled","value_type":"boolean","current":true,"unit":null,"access":"deployment_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"41"},"property":"external_publication"},"owner":{"kind":"instrument","id":"41"},"property":"external_publication","value_type":"boolean","current":false,"unit":null,"access":"read_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
        ]
    );

    let thermal_plant = InstrumentDto::ThermalPlant {
        id: 42,
        key: "plant".into(),
        display_name: "Thermal plant".into(),
        history_capacity: 32,
        ambient_temperature: 21.0,
        initial_temperature: 23.0,
        gain_per_percent: 0.5,
        time_constant_ms: 8_000,
        poll_period_ms: 250,
    };
    assert_eq!(
        projected(&thermal_plant),
        vec![
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"display_name"},"owner":{"kind":"instrument","id":"42"},"property":"display_name","value_type":"text","current":"Thermal plant","unit":null,"access":"read_write","mutation_class":"live_safe","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"poll_period_ms"},"owner":{"kind":"instrument","id":"42"},"property":"poll_period_ms","value_type":"integer","current":250,"unit":{"id":"ms","symbol":"ms"},"access":"read_write","mutation_class":"ordinary_live","constraints":{"minimum":1,"maximum":60000},"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"ambient_temperature"},"owner":{"kind":"instrument","id":"42"},"property":"ambient_temperature","value_type":"number","current":21.0,"unit":{"id":"degC","symbol":"°C"},"access":"deployment_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"initial_temperature"},"owner":{"kind":"instrument","id":"42"},"property":"initial_temperature","value_type":"number","current":23.0,"unit":{"id":"degC","symbol":"°C"},"access":"deployment_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"gain_per_percent"},"owner":{"kind":"instrument","id":"42"},"property":"gain_per_percent","value_type":"number","current":0.5,"unit":null,"access":"deployment_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"42"},"property":"time_constant_ms"},"owner":{"kind":"instrument","id":"42"},"property":"time_constant_ms","value_type":"number","current":8000,"unit":{"id":"ms","symbol":"ms"},"access":"deployment_only","mutation_class":"reinitialize","constraints":null,"revision":"7"}),
        ]
    );

    let metakon = InstrumentDto::Metakon {
        id: 43,
        key: "metakon".into(),
        definition: PathBuf::from("metakon.json"),
        resource_id: 1,
        address: 2,
        poll_period_ms: 500,
        queue_timeout_ms: 100,
        transaction_timeout_ms: 100,
    };
    assert_eq!(
        projected(&metakon),
        vec![
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"43"},"property":"poll_period_ms"},"owner":{"kind":"instrument","id":"43"},"property":"poll_period_ms","value_type":"integer","current":500,"unit":{"id":"ms","symbol":"ms"},"access":"read_write","mutation_class":"ordinary_live","constraints":{"minimum":1,"maximum":60000},"revision":"7"})
        ]
    );
}

struct ReferenceInstrumentProperties;

impl InstrumentPropertySource for ReferenceInstrumentProperties {
    fn instrument_id(&self) -> u64 {
        99
    }

    fn property_metadata(&self) -> Vec<InstrumentPropertyMetadata<'_>> {
        vec![InstrumentPropertyMetadata {
            id: "sample_period_ms",
            value_type: InstrumentPropertyType::Integer,
            current: InstrumentPropertyValue::Unsigned(20),
            unit: Some(InstrumentPropertyUnit {
                id: "ms",
                symbol: "ms",
            }),
            access: InstrumentPropertyAccess::ReadWrite,
            mutation: InstrumentPropertyMutation::OrdinaryLive,
            constraints: Some(InstrumentPropertyConstraints::Integer {
                minimum: 1,
                maximum: 1_000,
            }),
        }]
    }
}

#[test]
fn second_instrument_property_source_uses_generic_application_projection() {
    assert_eq!(
        projected(&ReferenceInstrumentProperties),
        vec![
            json!({"kind":"property","id":{"owner":{"kind":"instrument","id":"99"},"property":"sample_period_ms"},"owner":{"kind":"instrument","id":"99"},"property":"sample_period_ms","value_type":"integer","current":20,"unit":{"id":"ms","symbol":"ms"},"access":"read_write","mutation_class":"ordinary_live","constraints":{"minimum":1,"maximum":1000},"revision":"7"})
        ]
    );
}
