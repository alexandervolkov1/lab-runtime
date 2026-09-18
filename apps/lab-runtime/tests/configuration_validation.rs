//! C1-C4 acceptance for bounded, side-effect-free deployment validation.

use lab_core::{Query, QueryResult};
use lab_runtime::configuration::{
    ArtifactReader, ConfigurationError, MAX_RUNTIME_TOML_BYTES, MAX_RUNTIME_TOML_DEPTH,
    MAX_RUNTIME_TOML_VALUES, load_runtime_toml, parse_runtime_toml,
};
use lab_runtime::host::HostCore;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const READ_ONLY_DEFINITION: &[u8] = br#"{
  "schema_version": 1,
  "profile": "metakon-5x3-v1",
  "id": 11,
  "name": "Metakon temperature",
  "parameters": [
    {
      "id": 1,
      "name": "channel_type",
      "value_type": "integer",
      "unit": {"id": "1", "symbol": "1"},
      "min": 0,
      "max": 255,
      "role": "diagnostic",
      "access": "read_only",
      "operation": "channel_type",
      "scale": 1,
      "write_effect": "none"
    },
    {
      "id": 2,
      "name": "temperature",
      "value_type": "float",
      "unit": {"id": "degC", "symbol": "C"},
      "min": -99.9,
      "max": 999.9,
      "role": "measurement",
      "access": "read_only",
      "operation": "temperature",
      "scale": 0.1,
      "write_effect": "none"
    }
  ]
}"#;

fn physical_config(definition: &str) -> Vec<u8> {
    format!(
        r#"schema_version = 1
[runtime]
key = "metakon-bench"
display_name = "Read-only bench"
[server]
host = "127.0.0.1"
port = 7420
[recording]
enabled = true
path = "history.sqlite"
policy = "required"
[[resources]]
id = 1
key = "temperature-bus"
kind = "windows_com_read_only"
port = "COM3"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
read_timeout_ms = 50
write_timeout_ms = 50
open_timeout_ms = 2000
recovery_timeout_ms = 2000
[[instruments]]
id = 11
key = "furnace-temperature"
kind = "metakon"
definition = "{definition}"
resource_id = 1
address = 1
poll_period_ms = 1000
"#
    )
    .into_bytes()
}

#[derive(Default)]
struct MemoryReader {
    files: BTreeMap<PathBuf, Vec<u8>>,
    reads: usize,
}

impl ArtifactReader for MemoryReader {
    fn read(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
        self.reads += 1;
        let bytes = self
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| ConfigurationError::artifact("missing artifact"))?;
        if bytes.len() > maximum {
            return Err(ConfigurationError::artifact("artifact exceeds limit"));
        }
        Ok(bytes)
    }
}

fn reader_with_definition(base: &Path, name: &str, bytes: &[u8]) -> MemoryReader {
    let mut reader = MemoryReader::default();
    reader.files.insert(base.join(name), bytes.to_vec());
    reader
}

fn native_control_config(reference_unit: &str, output_parameter: u64) -> Vec<u8> {
    format!(
        r#"schema_version=1
[runtime]
key="native-control"
display_name="Native control"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=51
key="plant"
kind="thermal_plant"
display_name="Plant"
history_capacity=16
ambient_temperature=20.0
initial_temperature=20.0
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms=100
[[references]]
id=52
key="setpoint"
kind="fixed"
value=40.0
unit_id="{reference_unit}"
unit_symbol="u"
[[safe_profiles]]
instrument_id=51
parameter_id={output_parameter}
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
[[controllers]]
id=53
key="pid"
input_instrument_id=51
input_parameter_id=1
output_instrument_id=51
output_parameter_id={output_parameter}
reference_id=52
period_ms=100
ema_time_constant_ms=200
ema_warmup_samples=3
kp=3.0
ki=0.4
kd=0.2
output_min=0.0
output_max=100.0
max_input_age_ms=500
max_tick_gap_ms=500
lease_lifetime_ms=2000
proposal_ttl_ms=200
"#
    )
    .into_bytes()
}

#[test]
fn c1_valid_runtime_toml_is_deterministic_and_freezes_exact_bytes() {
    let base = Path::new("C:/lab/config");
    let bytes = physical_config("metakon.json");
    let mut first_reader = reader_with_definition(base, "metakon.json", READ_ONLY_DEFINITION);
    let mut second_reader = reader_with_definition(base, "metakon.json", READ_ONLY_DEFINITION);

    let first = parse_runtime_toml(&bytes, base, &mut first_reader).unwrap();
    let second = parse_runtime_toml(&bytes, base, &mut second_reader).unwrap();

    assert_eq!(first.effective(), second.effective());
    assert_eq!(first.toml_bytes(), bytes);
    assert_eq!(first.toml_hash(), second.toml_hash());
    assert_eq!(first.artifacts().len(), 1);
    assert_eq!(first.artifacts()[0].bytes(), READ_ONLY_DEFINITION);
    assert_eq!(first_reader.reads, 1);

    let mut reformatted = b"# exact bytes are provenance\r\n".to_vec();
    reformatted.extend_from_slice(&bytes);
    let mut third_reader = reader_with_definition(base, "metakon.json", READ_ONLY_DEFINITION);
    let third = parse_runtime_toml(&reformatted, base, &mut third_reader).unwrap();
    assert_eq!(first.effective(), third.effective());
    assert_ne!(first.toml_hash(), third.toml_hash());
}

#[test]
fn c1_reordered_object_tables_have_one_canonical_effective_graph() {
    fn config(order: [u64; 2]) -> Vec<u8> {
        let instrument = |id: u64| {
            format!(
                r#"[[instruments]]
id={id}
key="sensor-{id}"
kind="virtual_measurement"
display_name="Sensor {id}"
history_capacity=8
base_temperature=20.0
measurement_enabled=true
poll_period_ms=100
"#
            )
        };
        format!(
            r#"schema_version=1
[runtime]
key="ordered"
display_name="Ordered"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
{}{}
"#,
            instrument(order[0]),
            instrument(order[1])
        )
        .into_bytes()
    }
    let base = Path::new("C:/lab/config");
    let mut first_reader = MemoryReader::default();
    let mut second_reader = MemoryReader::default();
    let first = parse_runtime_toml(&config([20, 10]), base, &mut first_reader).unwrap();
    let second = parse_runtime_toml(&config([10, 20]), base, &mut second_reader).unwrap();
    assert_eq!(first.effective(), second.effective());
    assert_ne!(first.toml_hash(), second.toml_hash());
}

#[test]
fn c1_repository_example_deployments_parse_with_the_production_loader() {
    let examples =
        std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples"))
            .unwrap();
    for name in [
        "runtime.virtual.toml",
        "runtime.metakon-read-only.toml",
        "runtime.metakon-513-com5.toml",
        "runtime.metakon-513-com5-output.toml",
    ] {
        let deployment = load_runtime_toml(&examples.join(name)).unwrap();
        assert!(!deployment.toml_bytes().is_empty());
    }
}

#[test]
fn c2_structurally_invalid_candidate_reads_no_artifacts() {
    let base = Path::new("C:/lab/config");
    let mut bytes = physical_config("metakon.json");
    let needle = b"resource_id = 1";
    let at = bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap();
    bytes.splice(at..at + needle.len(), b"resource_id = 9".iter().copied());
    let mut reader = reader_with_definition(base, "metakon.json", READ_ONLY_DEFINITION);

    let error = parse_runtime_toml(&bytes, base, &mut reader).unwrap_err();

    assert!(error.to_string().contains("unknown resource"));
    assert_eq!(reader.reads, 0);
}

#[test]
fn c3_unknown_duplicate_bom_and_oversize_inputs_are_rejected_boundedly() {
    let base = Path::new("C:/lab/config");
    for bytes in [
        b"schema_version=1\nunknown=true\n".to_vec(),
        b"schema_version=1\nschema_version=1\n".to_vec(),
        [b"\xef\xbb\xbf".as_slice(), b"schema_version=1\n"].concat(),
        vec![b' '; MAX_RUNTIME_TOML_BYTES + 1],
    ] {
        let mut reader = MemoryReader::default();
        assert!(parse_runtime_toml(&bytes, base, &mut reader).is_err());
        assert_eq!(reader.reads, 0);
    }
}

#[test]
fn c3_nesting_value_count_and_numeric_overflow_are_rejected_before_artifact_reads() {
    let base = Path::new("C:/lab/config");
    let nested = format!(
        "schema_version=1\nunknown={}0{}\n",
        "[".repeat(MAX_RUNTIME_TOML_DEPTH + 1),
        "]".repeat(MAX_RUNTIME_TOML_DEPTH + 1)
    );
    let assignments = (0..=MAX_RUNTIME_TOML_VALUES)
        .map(|index| format!("v{index}=0\n"))
        .collect::<String>();
    let numeric_overflow = physical_config("metakon.json")
        .windows(b"port = 7420".len())
        .position(|window| window == b"port = 7420")
        .map(|at| {
            let mut bytes = physical_config("metakon.json");
            bytes.splice(
                at..at + b"port = 7420".len(),
                b"port = 999999999999999999999999999999999999"
                    .iter()
                    .copied(),
            );
            bytes
        })
        .unwrap();

    for bytes in [
        nested.into_bytes(),
        assignments.into_bytes(),
        numeric_overflow,
    ] {
        let mut reader = MemoryReader::default();
        assert!(parse_runtime_toml(&bytes, base, &mut reader).is_err());
        assert_eq!(reader.reads, 0);
    }
}

fn virtual_graph(instrument_count: u64) -> Vec<u8> {
    let mut document = String::from(
        r#"schema_version=1
[runtime]
key="maximum-graph"
display_name="Maximum graph"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
"#,
    );
    for id in 1..=instrument_count {
        document.push_str(&format!(
            r#"[[instruments]]
id={id}
key="sensor-{id}"
kind="virtual_measurement"
display_name="Sensor {id}"
history_capacity=8
base_temperature=20.0
measurement_enabled=true
poll_period_ms=100
"#,
        ));
    }
    document.into_bytes()
}

#[test]
fn c4_largest_admitted_instrument_graph_composes_and_the_next_object_is_rejected() {
    let base = Path::new("C:/lab/config");
    let mut reader = MemoryReader::default();
    let deployment = parse_runtime_toml(&virtual_graph(64), base, &mut reader).unwrap();
    let host = HostCore::configured_native(&deployment).unwrap();
    let QueryResult::Instruments(instruments) = host.query(Query::Discover).unwrap() else {
        panic!("discovery result missing")
    };
    assert_eq!(instruments.len(), 64);

    let mut reader = MemoryReader::default();
    let error = parse_runtime_toml(&virtual_graph(65), base, &mut reader).unwrap_err();
    assert!(error.to_string().contains("object limit"));
    assert_eq!(reader.reads, 0);
}

#[test]
fn c4_unqualified_physical_output_and_duplicate_com_binding_are_rejected() {
    let base = Path::new("C:/lab/config");
    let writable = br#"{
      "schema_version":1,"profile":"metakon-5x3-v1","id":11,"name":"Output",
      "parameters":[{"id":6,"name":"power","value_type":"float",
      "unit":{"id":"percent","symbol":"%"},"min":0,"max":100,
      "role":"actuator","access":"write_only","operation":"output",
      "scale":1,"write_effect":"output_affecting"}]}
    "#;
    let mut writable_reader = reader_with_definition(base, "metakon.json", writable);
    let error = parse_runtime_toml(&physical_config("metakon.json"), base, &mut writable_reader)
        .unwrap_err();
    assert!(error.to_string().contains("windows_com"));

    let mut duplicate = physical_config("metakon.json");
    duplicate.extend_from_slice(
        br#"
[[resources]]
id = 2
key = "other-bus"
kind = "windows_com_read_only"
port = "com3"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
read_timeout_ms = 50
write_timeout_ms = 50
open_timeout_ms = 2000
recovery_timeout_ms = 2000
"#,
    );
    let mut duplicate_reader = reader_with_definition(base, "metakon.json", READ_ONLY_DEFINITION);
    let error = parse_runtime_toml(&duplicate, base, &mut duplicate_reader).unwrap_err();
    assert!(error.to_string().contains("duplicate COM"));
    assert_eq!(duplicate_reader.reads, 0);
}

#[test]
fn c3_relative_deployment_resolution_does_not_admit_parent_traversal() {
    let base = Path::new("C:/lab/config");
    let mut reader = MemoryReader::default();
    let error = parse_runtime_toml(
        &physical_config("../definitions/metakon.json"),
        base,
        &mut reader,
    )
    .unwrap_err();
    assert!(error.to_string().contains("parent traversal"));
    assert_eq!(reader.reads, 0);
}

#[test]
fn c4_native_graph_unit_and_output_role_mismatches_reject_before_artifact_reads() {
    let base = Path::new("C:/lab/config");
    for bytes in [
        native_control_config("degF", 2),
        native_control_config("degC", 1),
    ] {
        let mut reader = MemoryReader::default();
        let error = parse_runtime_toml(&bytes, base, &mut reader).unwrap_err();
        assert!(
            error.to_string().contains("unit") || error.to_string().contains("output"),
            "unexpected diagnostic: {error}"
        );
        assert_eq!(reader.reads, 0);
    }
}
