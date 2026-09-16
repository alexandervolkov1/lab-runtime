//! C5/C8/C11 acceptance for three distinct Runtime lifecycle operations.

use lab_core::{Query, QueryResult, SignalId, TEMPERATURE};
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
fn c8_c11_script_reload_and_model_restart_are_distinct_and_never_auto_start() {
    let path = temporary_path();
    fs::write(&path, deployment("Bench", 100, 22.0)).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();

    assert_eq!(
        service.reload_managed_scripts(),
        Err(LifecycleOperationError::NoManagedComponents)
    );
    let restarted = service.restart_virtual_models().unwrap();
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
    let restart = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"r","op":"restart_virtual_models",
            "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(restart[0]["state"], "accepted");
    assert_eq!(restart[1]["state"], "completed");
    assert_eq!(restart[1]["result"]["generation"], "2");
    let known = application.handle(
        &mut service,
        1,
        request(json!({"v":1,"msg_id":"again","op":"restart_virtual_models",
            "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(known.len(), 1);
    assert_eq!(known[0]["result"]["generation"], "2");
    fs::remove_file(path).unwrap();
}
