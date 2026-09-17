//! M9B.6 semantic configuration and generic property contracts.

use lab_core::{InstrumentId, Query, QueryResult, SignalId, TEMPERATURE};
use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn temporary_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "lab-runtime-m9b6-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn deployment() -> &'static str {
    r#"schema_version=1
[runtime]
key="m9b6"
display_name="M9B.6"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="input"
kind="virtual_measurement"
display_name="Input"
history_capacity=16
base_temperature=22.0
poll_period_ms=100
[[managed_components]]
id=42
instrument_id=42
key="mean"
display_name="Moving mean"
implementation="native.moving_mean.v1"
input_instrument_id=41
period_ms=100
config={window=3}
"#
}

fn frame(value: Value) -> lab_runtime::wire::WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

fn start() -> (PathBuf, ServiceHost, Application, String) {
    let path = temporary_path();
    fs::write(&path, deployment()).unwrap();
    let arg = path.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    (path, service, app, scope)
}

#[test]
fn generic_projection_exposes_native_component_properties_without_special_operations() {
    let (path, mut service, mut app, scope) = start();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status","op":"configuration_status","args":{}})),
    );
    assert_eq!(status[0]["result"]["revision"], "1");
    assert_eq!(status[0]["result"]["source"]["kind"], "deployment_file");
    assert_eq!(
        status[0]["result"]["source"]["automatically_persisted"],
        false
    );

    let properties = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"properties","op":"configuration_properties","args":{}})),
    );
    let records = properties[0]["result"]["records"].as_array().unwrap();
    let window = records
        .iter()
        .find(|record| {
            record["owner"] == json!({"kind":"component","id":"42"})
                && record["property"] == "window"
        })
        .unwrap();
    assert_eq!(window["current"], 3);
    assert_eq!(window["constraints"], json!({"minimum":2,"maximum":64}));
    assert_eq!(window["mutation_class"], "reinitialize");
    assert_eq!(window["access"], "deployment_only");
    let operations = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"bad","op":"hello","args":{"scope":null}})),
    )[0]["result"]["operations"]
        .as_array()
        .unwrap()
        .clone();
    assert!(operations.contains(&json!("property_configure")));
    assert!(!operations.iter().any(|operation| {
        operation
            .as_str()
            .is_some_and(|name| name.contains("moving_mean") || name == "set_component_window")
    }));
    let deployment_only = app.handle(
        &mut service,
        1,
        frame(
            json!({"v":1,"msg_id":"deployment-only","op":"property_configure",
            "request_id":{"scope":scope,"seq":"1"},"args":{
                "target":{"kind":"component","id":"42"},"property":"window",
                "value":4,"expected_revision":"1"}}),
        ),
    );
    assert_eq!(deployment_only[1]["code"], "invalid_configuration");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 1);
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn live_property_commit_is_revision_fenced_atomic_and_does_not_replace_generation() {
    let (path, mut service, mut app, scope) = start();
    let signal = SignalId::new(InstrumentId::new(41), TEMPERATURE);
    let generation = service.owner().signal_generation(signal);
    let cursor = service.owner().event_log().latest_cursor();
    let boot = service.boot_id().to_owned();
    let subscribed = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"subscribe","op":"subscribe","args":{
            "after":{"boot_id":boot,"seq":cursor.to_string()},
            "filter":{"kinds":["configuration"],"targets":[]}}})),
    );
    assert_eq!(subscribed[0]["type"], "result");
    let configured = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"configure","op":"property_configure",
            "request_id":{"scope":scope,"seq":"1"},"args":{
                "target":{"kind":"instrument","id":"41"},"property":"poll_period_ms",
                "value":250,"expected_revision":"1"}})),
    );
    assert_eq!(configured[0]["state"], "accepted");
    assert_eq!(configured[1]["state"], "completed");
    assert_eq!(configured[1]["result"]["revision"], "2");
    assert_eq!(
        configured[1]["result"]["persisted_to_deployment_source"],
        false
    );
    assert_eq!(service.owner().signal_generation(signal), generation);
    let events = app.pump_events(&service, 1);
    let event = events
        .iter()
        .find(|event| event["kind"] == "configuration")
        .unwrap();
    assert_eq!(event["target"]["id"], "runtime");
    assert_eq!(event["data"]["revision"], "2");
    assert_eq!(event["data"]["runtime_overrides"], 1);
    let properties = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"after","op":"configuration_properties","args":{}})),
    );
    let poll = properties[0]["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["owner"] == json!({"kind":"instrument","id":"41"})
                && record["property"] == "poll_period_ms"
        })
        .unwrap();
    assert_eq!(poll["current"], 250);
    assert_eq!(poll["revision"], "2");

    let conflict = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"conflict","op":"property_configure",
            "request_id":{"scope":scope,"seq":"2"},"args":{
                "target":{"kind":"instrument","id":"41"},"property":"poll_period_ms",
                "value":300,"expected_revision":"1"}})),
    );
    assert_eq!(conflict[1]["code"], "revision_conflict");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);

    let invalid = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"invalid","op":"property_configure",
            "request_id":{"scope":scope,"seq":"3"},"args":{
                "target":{"kind":"instrument","id":"41"},"property":"poll_period_ms",
                "value":0,"expected_revision":"2"}})),
    );
    assert_eq!(invalid[1]["code"], "invalid_configuration");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);
    let QueryResult::Descriptor(descriptor) = service
        .owner()
        .query(Query::DescribeInstrument(InstrumentId::new(41)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(descriptor.name, "Input");
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn configuration_state_is_runtime_owned_across_client_disconnect() {
    let (path, mut service, mut app, scope) = start();
    let configured = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"configure","op":"property_configure",
            "request_id":{"scope":scope,"seq":"1"},"args":{
                "target":{"kind":"instrument","id":"41"},"property":"display_name",
                "value":"Renamed input","expected_revision":"1"}})),
    );
    assert_eq!(configured[1]["state"], "completed");
    app.detach(&service, 1);
    let resumed = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":scope}})),
    );
    assert_eq!(resumed[0]["result"]["next_seq"], "2");
    let status = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"operation","op":"operation_status",
            "args":{"request_id":{"scope":scope,"seq":"1"}}})),
    );
    assert_eq!(status[0]["result"]["state"], "completed");
    let QueryResult::Descriptor(descriptor) = service
        .owner()
        .query(Query::DescribeInstrument(InstrumentId::new(41)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(descriptor.name, "Renamed input");
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}
