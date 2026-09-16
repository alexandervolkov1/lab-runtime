//! C5/C17 acceptance for exact loaded configuration provenance.

use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-{label}-{suffix}.{extension}"))
}

#[test]
fn c5_exact_loaded_runtime_toml_is_durable_after_path_mutation() {
    let config = temporary_path("provenance", "toml");
    let database = temporary_path("provenance", "sqlite");
    let database_toml = database.to_string_lossy().replace('\\', "\\\\");
    let original = format!(
        r#"schema_version=1
[runtime]
key="provenance"
display_name="Frozen provenance"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=true
path="{database_toml}"
policy="best_effort"
[[instruments]]
id=41
key="temperature"
kind="virtual_measurement"
display_name="Frozen sensor"
history_capacity=8
base_temperature=23.5
measurement_enabled=true
poll_period_ms=50
"#
    );
    fs::write(&config, &original).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    fs::write(&config, "not the loaded deployment").unwrap();

    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if service.shutdown_step().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let stored: Vec<u8> = connection
        .query_row(
            "SELECT content FROM provenance_content WHERE kind='runtime_toml'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, original.as_bytes());
    drop(connection);
    fs::remove_file(config).unwrap();
    fs::remove_file(database).unwrap();
}
