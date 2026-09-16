//! C1-C4 acceptance for bounded, side-effect-free deployment validation.

use lab_runtime::configuration::{
    ArtifactReader, ConfigurationError, MAX_RUNTIME_TOML_BYTES, parse_runtime_toml,
};
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
fn c4_physical_output_definition_and_duplicate_com_binding_are_rejected() {
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
    assert!(error.to_string().contains("read-only"));

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
