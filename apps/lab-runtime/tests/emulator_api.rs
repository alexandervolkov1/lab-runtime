//! M9B.7 external-emulator publication and virtual-model lifecycle contracts.

use lab_core::{InstrumentId, Query, QueryResult, SignalId, TEMPERATURE};
use lab_runtime::{
    application::Application,
    recorder::RecordingState,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "lab-runtime-m9b7-{}-{}.toml",
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
key="m9b7"
display_name="M9B.7"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="external"
kind="virtual_measurement"
display_name="External virtual temperature"
history_capacity=16
base_temperature=22.0
measurement_enabled=true
external_publication=true
poll_period_ms=100
[[instruments]]
id=42
key="plant"
kind="thermal_plant"
display_name="Native thermal plant"
history_capacity=16
ambient_temperature=20.0
initial_temperature=21.0
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms=100
[[references]]
id=43
key="setpoint"
kind="fixed"
value=40.0
unit_id="degC"
unit_symbol="°C"
[[safe_profiles]]
instrument_id=42
parameter_id=2
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
[[controllers]]
id=44
key="pid"
input_instrument_id=41
input_parameter_id=1
output_instrument_id=42
output_parameter_id=2
reference_id=43
period_ms=100
ema_time_constant_ms=200
ema_warmup_samples=3
kp=2.0
ki=0.2
kd=0.0
output_min=0.0
output_max=100.0
max_input_age_ms=500
max_tick_gap_ms=500
lease_lifetime_ms=2000
proposal_ttl_ms=200
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
    let capabilities = hello[0]["result"]["capabilities"].as_array().unwrap();
    assert!(
        capabilities
            .iter()
            .any(|item| item["name"] == "emulator_publication")
    );
    assert!(
        capabilities
            .iter()
            .any(|item| item["name"] == "virtual_model_lifecycle")
    );
    assert!(
        !hello[0]["result"]["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("restart_models"))
    );
    (path, service, app, scope)
}

fn publish(
    app: &mut Application,
    service: &mut ServiceHost,
    scope: &str,
    seq: u64,
    instrument: u64,
    publication: (&str, Option<Value>),
    generation: u64,
) -> Vec<Value> {
    let (state, value) = publication;
    let mut args = json!({"signal":{"instrument":instrument.to_string(),"parameter":"1"},
        "state":state,"expected_generation":generation.to_string()});
    if let Some(value) = value {
        args["value"] = value;
    }
    app.handle(
        service,
        1,
        frame(
            json!({"v":1,"msg_id":format!("p-{seq}"),"op":"emulator_publish",
            "request_id":{"scope":scope,"seq":seq.to_string()},"args":args}),
        ),
    )
}

#[test]
fn discovery_publication_current_history_event_and_disconnect_share_one_signal() {
    let (path, mut service, mut app, scope) = start();
    let discovery = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"discover","op":"discover","args":{}})),
    );
    let records = discovery[0]["result"]["records"].as_array().unwrap();
    let external = records
        .iter()
        .find(|record| {
            record["kind"] == "signal" && record["id"] == json!({"instrument":"41","parameter":"1"})
        })
        .unwrap();
    assert_eq!(external["capabilities"]["emulator_publication"], true);
    assert_eq!(external["emulator"]["timing"], "runtime_receipt");
    assert_eq!(external["unit"]["id"], "degC");
    let plant = records
        .iter()
        .find(|record| {
            record["kind"] == "signal" && record["id"] == json!({"instrument":"42","parameter":"1"})
        })
        .unwrap();
    assert_eq!(plant["capabilities"]["emulator_publication"], false);
    assert_eq!(plant["emulator"], Value::Null);

    let cursor = service.owner().event_log().latest_cursor();
    let boot = service.boot_id().to_owned();
    let subscribed = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"sub","op":"subscribe","args":{
        "after":{"boot_id":boot,"seq":cursor.to_string()},
        "filter":{"kinds":["signal"],"targets":[]}}})),
    );
    assert_eq!(subscribed[0]["type"], "result");

    let good = publish(
        &mut app,
        &mut service,
        &scope,
        1,
        41,
        ("good", Some(json!(50.0))),
        1,
    );
    assert_eq!(good[0]["state"], "accepted");
    assert_eq!(good[1]["state"], "completed");
    assert_eq!(good[1]["result"]["signal"], external["id"]);
    let event = app.pump_events(&service, 1);
    assert!(event.iter().any(|item| item["kind"] == "signal"
        && item["target"] == external["id"]
        && item["data"]["value"] == 50.0));

    let current = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"latest","op":"latest",
        "args":{"signal":{"instrument":"41","parameter":"1"}}})),
    );
    assert_eq!(current[0]["result"]["value"], 50.0);
    assert_eq!(current[0]["result"]["quality"], "good");
    assert_eq!(current[0]["result"]["generation"], "1");
    let history = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"window","op":"measurement_window",
        "args":{"signal":{"instrument":"41","parameter":"1"},"max_records":16}})),
    );
    assert_eq!(history[0]["result"]["rows"][0]["value"], 50.0);
    let started = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"controller","op":"controller_start",
            "request_id":{"scope":scope,"seq":"2"},"args":{"controller":"44"}})),
    );
    assert_eq!(started[1]["state"], "completed");
    assert_eq!(started[1]["result"]["state"], "warming");

    app.detach(&service, 1);
    let QueryResult::Latest(Some(retained)) = service
        .owner()
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(41),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(retained.value(), Some(&lab_core::Value::Float(50.0)));
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    let QueryResult::Latest(Some(still_retained)) = service
        .owner()
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(41),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(still_retained.value(), Some(&lab_core::Value::Float(50.0)));
    drop(app);
    drop(service);
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn publication_validation_fault_recovery_and_generation_fence_are_atomic() {
    let (path, mut service, mut app, scope) = start();
    let rejected = publish(
        &mut app,
        &mut service,
        &scope,
        1,
        42,
        ("good", Some(json!(30.0))),
        1,
    );
    assert_eq!(rejected[1]["state"], "failed");
    let QueryResult::Latest(before) = service
        .owner()
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(42),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert!(before.is_none());

    let range = publish(
        &mut app,
        &mut service,
        &scope,
        2,
        41,
        ("good", Some(json!(5000.0))),
        1,
    );
    assert_eq!(range[1]["state"], "failed");
    let stale = publish(
        &mut app,
        &mut service,
        &scope,
        3,
        41,
        ("good", Some(json!(25.0))),
        2,
    );
    assert_eq!(stale[1]["code"], "invalid_configuration");
    let unavailable = publish(
        &mut app,
        &mut service,
        &scope,
        4,
        41,
        ("unavailable", None),
        1,
    );
    assert_eq!(unavailable[1]["state"], "completed");
    let current = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"u","op":"latest",
        "args":{"signal":{"instrument":"41","parameter":"1"}}})),
    );
    assert_eq!(current[0]["result"]["quality"], "unavailable");
    assert_eq!(current[0]["result"]["status"], "unavailable");
    assert_eq!(current[0]["result"]["failure"], "disabled");
    let recovered = publish(
        &mut app,
        &mut service,
        &scope,
        5,
        41,
        ("good", Some(json!(26.0))),
        1,
    );
    assert_eq!(recovered[1]["state"], "completed");
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn native_thermal_model_uses_generic_properties_and_targeted_restart() {
    let (path, mut service, mut app, scope) = start();
    let properties = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"props",
        "op":"configuration_properties","args":{}})),
    );
    let records = properties[0]["result"]["records"].as_array().unwrap();
    assert!(records.iter().any(
        |record| record["owner"] == json!({"kind":"instrument","id":"42"})
            && record["property"] == "time_constant_ms"
            && record["mutation_class"] == "reinitialize"
    ));
    let before = service
        .owner()
        .signal_generation(SignalId::new(InstrumentId::new(42), TEMPERATURE));
    let restarted = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"restart",
        "op":"virtual_models_restart","request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(restarted[1]["state"], "completed");
    assert_eq!(restarted[1]["result"]["models"], "1");
    assert_eq!(
        service
            .owner()
            .signal_generation(SignalId::new(InstrumentId::new(42), TEMPERATURE)),
        before + 1
    );
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn external_virtual_publication_reaches_durable_measurement_history() {
    let config = temporary_path();
    let database = config.with_extension("sqlite");
    let database_toml = database.to_string_lossy().replace('\\', "\\\\");
    let recorded = deployment().replace(
        "enabled=false\npolicy=\"best_effort\"",
        &format!("enabled=true\npath=\"{database_toml}\"\npolicy=\"best_effort\""),
    );
    fs::write(&config, recorded).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello",
        "op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let started = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"start",
        "op":"recording_start","request_id":{"scope":scope,"seq":"1"},
        "args":{"label":"external emulator"}})),
    );
    assert_eq!(started[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        app.poll_recording(&mut service);
        std::thread::yield_now();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .outstanding_groups
        != 0
    {
        assert!(Instant::now() < deadline);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status",
        "op":"recording_status","args":{}})),
    );
    let run_id = status[0]["result"]["run_id"].clone();
    let archive = status[0]["result"]["archive"]["id"].clone();
    let published = publish(
        &mut app,
        &mut service,
        &scope,
        2,
        41,
        ("good", Some(json!(55.0))),
        1,
    );
    assert_eq!(published[1]["state"], "completed");
    let deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .outstanding_groups
        != 0
    {
        assert!(Instant::now() < deadline);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let stopped = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"stop",
        "op":"recording_stop","request_id":{"scope":scope,"seq":"3"},
        "args":{"run_id":run_id}})),
    );
    assert_eq!(stopped[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut stop_replies = Vec::new();
    while service.owner().recording_status().unwrap().state != RecordingState::Idle {
        assert!(
            Instant::now() < deadline,
            "stop status {:?}, replies {stop_replies:?}",
            service.owner().recording_status()
        );
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        stop_replies.extend(app.poll_recording(&mut service));
        std::thread::yield_now();
    }
    let history = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"history",
        "op":"history_read","request_id":{"scope":scope,"seq":"4"},"args":{
            "mode":"measurements","database_id":archive,"boot_id":run_id["boot_id"],
            "run_id":run_id,"signal":{"instrument":"41","parameter":"1"},
            "from_ns":"0","to_ns":"18446744073709551615","max_records":16,"cursor":null}})),
    );
    assert_eq!(history[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let terminal = loop {
        if let Some((_, reply)) = app.poll_history(&mut service).into_iter().next() {
            break reply;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(terminal["state"], "completed");
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"page",
        "op":"history_page","args":{"page_token":terminal["result"]["page_token"]}})),
    );
    assert!(
        page[0]["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["value"] == json!({"kind":"float","value":55.0})),
        "page={page:?}"
    );
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(app);
    drop(service);
    for path in [
        &config,
        &database,
        &database.with_extension("sqlite-wal"),
        &database.with_extension("sqlite-shm"),
    ] {
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}
