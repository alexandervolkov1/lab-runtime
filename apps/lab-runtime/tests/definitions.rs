//! Strict host JSON definition acceptance.

use lab_runtime::definition::{DefinitionError, parse_definition_json};

fn fixture(unit_id: &str, symbol: &str, id: u64) -> String {
    format!(
        r#"{{
          "schema_version": 1,
          "profile": "metakon-5x3-v1",
          "id": {id},
          "name": "Configured flow device",
          "parameters": [
            {{
              "id": 1, "name": "temperature", "value_type": "float",
              "unit": {{"id": "degC", "symbol": "°C"}},
              "min": -99.9, "max": 999.9, "role": "measurement",
              "access": "read_only", "operation": "temperature", "scale": 0.1,
              "write_effect": "none"
            }},
            {{
              "id": 6, "name": "flow", "value_type": "float",
              "unit": {{"id": "{unit_id}", "symbol": "{symbol}"}},
              "min": 0.0, "max": 100.0, "role": "actuator",
              "access": "read_write", "operation": "output", "scale": 1.0,
              "write_effect": "output_affecting"
            }}
          ]
        }}"#
    )
}

#[test]
fn two_custom_units_load_without_core_variants() {
    let sccm = parse_definition_json(&fixture("sccm", "sccm", 1)).unwrap();
    let rpm = parse_definition_json(&fixture("rpm", "rpm", 2)).unwrap();
    assert_eq!(sccm.parameters[1].unit.id(), "sccm");
    assert_eq!(rpm.parameters[1].unit.id(), "rpm");
}

#[test]
fn unknown_duplicate_and_overnested_json_are_rejected() {
    let unknown = fixture("sccm", "sccm", 1).replace(
        "\"schema_version\": 1,",
        "\"schema_version\": 1, \"surprise\": true,",
    );
    assert!(matches!(
        parse_definition_json(&unknown),
        Err(DefinitionError::InvalidJson(_))
    ));

    let duplicate = fixture("sccm", "sccm", 1).replace(
        "\"schema_version\": 1,",
        "\"schema_version\": 1, \"schema_version\": 1,",
    );
    assert!(matches!(
        parse_definition_json(&duplicate),
        Err(DefinitionError::InvalidJson(_))
    ));

    assert!(matches!(
        parse_definition_json(r#"[[[[[[[[[0]]]]]]]]]"#),
        Err(DefinitionError::TooDeep)
    ));
}

#[test]
fn raw_mappings_and_metadata_disguises_have_no_schema_path() {
    let raw = fixture("sccm", "sccm", 1)
        .replace("\"operation\": \"output\"", "\"operation\": \"raw_write\"");
    assert!(parse_definition_json(&raw).is_err());

    let disguised =
        fixture("sccm", "sccm", 1).replace("\"role\": \"actuator\"", "\"role\": \"measurement\"");
    assert!(matches!(
        parse_definition_json(&disguised),
        Err(DefinitionError::InvalidDomain(_))
    ));
}
