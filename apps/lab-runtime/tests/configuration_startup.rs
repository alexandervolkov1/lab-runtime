//! C1/C2 startup acceptance for `--serve --config` composition.

use lab_core::{Query, QueryResult};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{fs, path::PathBuf};

fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-{label}-{suffix}.{extension}"))
}

fn virtual_toml(name: &str, port: u16) -> String {
    format!(
        r#"schema_version=1
[runtime]
key="configured-bench"
display_name="{name}"
[server]
host="127.0.0.1"
port={port}
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="temperature"
kind="virtual_measurement"
display_name="{name} sensor"
history_capacity=8
base_temperature=23.5
measurement_enabled=true
poll_period_ms=50
"#
    )
}

#[test]
fn c1_configured_startup_uses_explicit_graph_and_remains_disarmed() {
    let path = temporary_path("startup", "toml");
    fs::write(&path, virtual_toml("Configured", 0)).unwrap();
    let text = path.to_string_lossy().into_owned();
    let options = ServiceOptions::parse(&["--serve", "--config", &text]).unwrap();

    let service = ServiceHost::startup(options).unwrap();
    let QueryResult::Instruments(instruments) = service.owner().query(Query::Discover).unwrap()
    else {
        panic!()
    };
    assert_eq!(instruments.len(), 1);
    assert_eq!(instruments[0].id.get(), 41);
    assert_eq!(instruments[0].name, "Configured sensor");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 1);
    assert_eq!(
        service
            .loaded_configuration()
            .unwrap()
            .active()
            .toml_bytes(),
        fs::read(&path).unwrap()
    );
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn c2_invalid_config_fails_before_listener_binding_and_mixed_cli_is_rejected() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let path = temporary_path("invalid", "toml");
    fs::write(
        &path,
        virtual_toml("Invalid", port).replace("schema_version=1", "schema_version=9"),
    )
    .unwrap();
    let text = path.to_string_lossy().into_owned();
    let options = ServiceOptions::parse(&["--serve", "--config", &text]).unwrap();
    assert!(ServiceHost::startup(options).is_err());
    let rebound = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(
        rebound.is_ok(),
        "invalid validation must not consume listener"
    );
    assert!(
        ServiceOptions::parse(&["--serve", "--config", &text, "--profile", "virtual-demo"])
            .is_err()
    );
    drop(rebound);
    fs::remove_file(path).unwrap();
}
