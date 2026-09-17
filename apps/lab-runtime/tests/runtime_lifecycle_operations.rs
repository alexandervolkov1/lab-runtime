//! C5/C8/C11 acceptance for three distinct Runtime lifecycle operations.

use lab_core::{
    Command, Query, QueryResult, SignalId, TEMPERATURE,
    control::{ControllerId, ControllerState},
    output::{ActuatorId, OutputCommand, OutputOwner, OutputResult, OutputState},
};
use lab_runtime::service::{LifecycleOperationError, ServiceHost, ServiceOptions};
use lab_runtime::{
    application::Application,
    wire::{decode_frame, encode_frame},
};
use serde_json::json;
use std::{fs, path::PathBuf};

fn temporary_path() -> PathBuf {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-operations-{suffix}.toml"))
}

fn deployment(name: &str, period: u64, initial: f64) -> String {
    format!(
        r#"schema_version=1
[runtime]
key="bench"
display_name="{name}"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="plant"
kind="thermal_plant"
display_name="{name} plant"
history_capacity=8
ambient_temperature=20.0
initial_temperature={initial}
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms={period}
"#
    )
}

fn controlled_deployment(kp: f64) -> String {
    format!(
        r#"schema_version=1
[runtime]
key="controlled"
display_name="Controlled"
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
key="pid"
input_instrument_id=51
input_parameter_id=1
output_instrument_id=51
output_parameter_id=2
reference_id=52
period_ms=100
ema_time_constant_ms=200
ema_warmup_samples=3
kp={kp}
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

fn request(value: serde_json::Value) -> lab_runtime::wire::WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

#[test]
fn c5_live_reload_commits_one_revision_and_uses_frozen_candidate_bytes() {
    let path = temporary_path();
    fs::write(&path, deployment("Old", 100, 22.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    fs::write(&path, deployment("New", 250, 22.0)).unwrap();

    let result = service.reload_configuration().unwrap();
    assert_eq!(result.revision, 2);
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);
    assert_eq!(
        service
            .loaded_configuration()
            .unwrap()
            .active()
            .toml_bytes(),
        deployment("New", 250, 22.0).as_bytes()
    );
    let QueryResult::Descriptor(descriptor) = service
        .owner()
        .query(Query::DescribeInstrument(lab_core::InstrumentId::new(41)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(descriptor.name, "New plant");
    fs::remove_file(path).unwrap();
}

#[test]
fn c8_c11_source_reload_and_model_restart_are_distinct_and_never_auto_start() {
    let path = temporary_path();
    fs::write(&path, deployment("Bench", 100, 22.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();

    assert_eq!(
        service.reload_managed_sources(),
        Err(LifecycleOperationError::NoManagedComponents)
    );
    let restarted = service.restart_models().unwrap();
    assert_eq!(restarted.models, 1);
    assert_eq!(restarted.generation, 2);
    assert_eq!(service.loaded_configuration().unwrap().revision(), 1);
    assert_eq!(
        service
            .owner()
            .query(Query::GetLatestSignal(SignalId::new(
                lab_core::InstrumentId::new(41),
                TEMPERATURE,
            )))
            .unwrap(),
        QueryResult::Latest(None)
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn c8_public_api_exposes_three_separate_deduplicated_operations() {
    let path = temporary_path();
    fs::write(&path, deployment("Bench", 100, 22.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let mut application = Application::new(service.boot_id()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap();
    let capabilities = hello[0]["result"]["capabilities"].as_array().unwrap();
    assert!(capabilities.contains(&json!("managed_component")));
    assert!(capabilities.contains(&json!("managed_transform")));
    assert!(!capabilities.contains(&json!("lua_source")));
    let operations = hello[0]["result"]["operations"].as_array().unwrap();
    assert!(operations.contains(&json!("reload_managed_sources")));
    assert!(operations.contains(&json!("restart_models")));
    assert!(!operations.contains(&json!("reload_managed_scripts")));
    let restart = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"r","op":"restart_models",
            "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(restart[0]["state"], "accepted");
    assert_eq!(restart[1]["state"], "completed");
    assert_eq!(restart[1]["result"]["generation"], "2");
    let known = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"again","op":"restart_models",
            "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(known.len(), 1);
    assert_eq!(known[0]["result"]["generation"], "2");
    fs::remove_file(path).unwrap();
}

#[test]
fn c5_public_stage_and_apply_are_distinct_revision_fenced_operations() {
    let path = temporary_path();
    fs::write(&path, deployment("Old", 100, 22.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let mut application = Application::new(service.boot_id()).unwrap();
    let hello = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap();
    fs::write(&path, deployment("Staged", 250, 22.0)).unwrap();

    let staged = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"s","op":"stage_configuration",
            "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(staged[1]["state"], "completed");
    let candidate = staged[1]["result"]["candidate_id"].as_str().unwrap();
    let base = staged[1]["result"]["base_revision"].as_str().unwrap();
    assert_eq!(service.loaded_configuration().unwrap().revision(), 1);

    let applied = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"a","op":"apply_configuration",
            "request_id":{"scope":scope,"seq":"2"},"args":{
                "candidate_id":candidate,"expected_revision":base}})),
    );
    assert_eq!(applied[1]["state"], "completed");
    assert_eq!(applied[1]["result"]["revision"], "2");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn c6_pid_reload_pauses_warming_control_proves_safe_and_never_rearms() {
    let path = temporary_path();
    fs::write(&path, controlled_deployment(3.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: lab_core::InstrumentId::new(51),
            parameter: TEMPERATURE,
            at: std::time::Duration::from_millis(1),
        })
        .unwrap();
    service
        .owner_mut()
        .command(Command::StartController {
            controller: ControllerId::new(53),
            at: std::time::Duration::from_millis(1),
        })
        .unwrap();
    fs::write(&path, controlled_deployment(4.5)).unwrap();

    assert_eq!(service.reload_configuration().unwrap().revision, 2);
    let QueryResult::Controller(controller) = service
        .owner()
        .query(Query::Controller(ControllerId::new(53)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(controller.state, ControllerState::Paused);
    assert!(controller.lease.is_none());
    let QueryResult::ControllerConfig(config) = service
        .owner()
        .query(Query::ControllerConfig(ControllerId::new(53)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(config.pid.kp, 4.5);
    let QueryResult::Output(output) = service
        .owner()
        .query(Query::Output(ActuatorId::new(
            lab_core::InstrumentId::new(51),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::Disarmed);
    assert!(output.safe_confirmed);
    assert!(output.lease.is_none());
    fs::remove_file(path).unwrap();
}

#[test]
fn c6_safe_profile_reload_revokes_manual_authority_without_rollback_rearm() {
    let path = temporary_path();
    fs::write(&path, controlled_deployment(3.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let actuator = ActuatorId::new(lab_core::InstrumentId::new(51), lab_core::HEATER_POWER);
    let lab_core::CommandResult::Output(OutputResult::Lease(old_lease)) = service
        .owner_mut()
        .command(Command::Output {
            actuator,
            at: std::time::Duration::ZERO,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(77),
                lifetime: std::time::Duration::from_millis(1000),
            },
        })
        .unwrap()
    else {
        panic!()
    };
    let replacement = controlled_deployment(3.0)
        .replace("max_lease_ms=2000", "max_lease_ms=1900")
        .replace("lease_lifetime_ms=2000", "lease_lifetime_ms=1800");
    fs::write(&path, replacement).unwrap();

    assert_eq!(service.reload_configuration().unwrap().revision, 2);
    let QueryResult::Output(output) = service.owner().query(Query::Output(actuator)).unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::Disarmed);
    assert!(output.safe_confirmed);
    assert!(output.lease.is_none());
    assert!(
        service
            .owner_mut()
            .command(Command::Output {
                actuator,
                at: std::time::Duration::from_millis(1),
                command: OutputCommand::Release(old_lease),
            })
            .is_err()
    );
    fs::remove_file(path).unwrap();
}
