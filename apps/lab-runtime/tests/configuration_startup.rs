//! C1/C2 startup acceptance for `--serve --config` composition.

use lab_core::{
    Query, QueryResult,
    control::{ControllerId, ControllerState},
    output::{ActuatorId, OutputState},
    reference::{ReferenceId, ReferenceSnapshot},
};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

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

fn native_control_toml(port: u16) -> String {
    format!(
        r#"schema_version=1
[runtime]
key="configured-control"
display_name="Configured native control"
[server]
host="127.0.0.1"
port={port}
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=51
key="plant"
kind="thermal_plant"
display_name="Configured plant"
history_capacity=16
ambient_temperature=20.0
initial_temperature=20.0
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms=100
[[references]]
id=52
key="setpoint"
kind="ramp"
value=20.0
target=50.0
rate=2.0
unit_id="degC"
unit_symbol="°C"
[[safe_profiles]]
instrument_id=51
parameter_id=2
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
[[controllers]]
id=53
key="temperature-pid"
input_instrument_id=51
input_parameter_id=1
output_instrument_id=51
output_parameter_id=2
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
}

fn recorded_virtual_toml() -> String {
    virtual_toml("Relative path", 0).replace(
        "enabled=false\npolicy=\"best_effort\"",
        "enabled=true\npath=\"history.sqlite\"\npolicy=\"required\"",
    )
}

fn shutdown(mut service: ServiceHost) {
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if service.shutdown_step().unwrap().is_some() {
            return;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

fn remove_relative_fixture(directory: &Path) {
    for name in [
        "history.sqlite-shm",
        "history.sqlite-wal",
        "history.sqlite",
        "runtime.toml",
    ] {
        let path = directory.join(name);
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
    fs::remove_dir(directory).unwrap();
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
fn c1_relative_and_absolute_config_paths_resolve_the_same_recorder_database() {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    let current = std::env::current_dir().unwrap();
    let directory = current.join(format!(".lab-runtime-m8-relative-{suffix}"));
    fs::create_dir(&directory).unwrap();
    let absolute = directory.join("runtime.toml");
    let bytes = recorded_virtual_toml();
    fs::write(&absolute, &bytes).unwrap();
    let relative = absolute.strip_prefix(&current).unwrap();

    let relative_text = relative.to_string_lossy().into_owned();
    let relative_result = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--config", &relative_text]).unwrap(),
    );
    let relative_service = match relative_result {
        Ok(service) => service,
        Err(error) => {
            remove_relative_fixture(&directory);
            panic!("relative deployment path failed before equivalent startup: {error}")
        }
    };
    let relative_hash = relative_service
        .loaded_configuration()
        .unwrap()
        .active()
        .toml_hash();
    let database_id = relative_service
        .owner()
        .recording_database_id()
        .unwrap()
        .to_owned();
    shutdown(relative_service);

    let absolute_text = absolute.to_string_lossy().into_owned();
    let absolute_service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--config", &absolute_text]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        absolute_service
            .loaded_configuration()
            .unwrap()
            .active()
            .toml_hash(),
        relative_hash
    );
    assert_eq!(
        absolute_service.owner().recording_database_id(),
        Some(database_id.as_str())
    );
    assert_eq!(
        absolute_service
            .loaded_configuration()
            .unwrap()
            .active()
            .toml_bytes(),
        bytes.as_bytes()
    );
    shutdown(absolute_service);
    remove_relative_fixture(&directory);
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

#[test]
fn c1_c4_configured_native_graph_starts_ready_safe_and_disarmed() {
    let path = temporary_path("native-control", "toml");
    fs::write(&path, native_control_toml(0)).unwrap();
    let text = path.to_string_lossy().into_owned();

    let service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &text]).unwrap())
            .unwrap();
    let QueryResult::Reference(ReferenceSnapshot::Ramp { state, .. }) = service
        .owner()
        .query(Query::Reference(ReferenceId::new(52)))
        .unwrap()
    else {
        panic!("configured ramp Reference missing")
    };
    assert_eq!(state.current, 20.0);
    assert_eq!(state.target, 50.0);
    let QueryResult::Controller(controller) = service
        .owner()
        .query(Query::Controller(ControllerId::new(53)))
        .unwrap()
    else {
        panic!("configured controller missing")
    };
    assert_eq!(controller.state, ControllerState::Ready);
    let QueryResult::Output(output) = service
        .owner()
        .query(Query::Output(ActuatorId::new(
            lab_core::InstrumentId::new(51),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!("configured output missing")
    };
    assert_eq!(output.state, OutputState::Disarmed);
    assert!(output.lease.is_none());

    drop(service);
    fs::remove_file(path).unwrap();
}
