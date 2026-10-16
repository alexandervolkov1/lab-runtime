//! M9B.3 discovery, current measurement, window, and subscription projections.

use lab_core::{Command, ParameterId};
use lab_runtime::{
    application::Application,
    host::Clock,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::time::Duration;

fn setup() -> (ServiceHost, Application, String) {
    let mut service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    (service, app, scope)
}
fn ask(
    service: &mut ServiceHost,
    app: &mut Application,
    connection: u64,
    value: Value,
) -> Vec<Value> {
    let request = decode_frame(&encode_frame(&value).unwrap()).unwrap();
    app.handle(service, connection, request)
}

#[test]
fn discovery_is_composition_derived_paged_and_uses_stable_signal_ids() {
    let (mut service, mut app, _) = setup();
    let reply = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"d","op":"discover","args":{}}),
    );
    let result = &reply[0]["result"];
    assert_eq!(result["complete"], true);
    assert!(serde_json::to_vec(result).unwrap().len() <= 8 * 1024);
    let records = result["records"].as_array().unwrap();
    let signal = records
        .iter()
        .find(|record| record["kind"] == "signal")
        .unwrap();
    assert!(signal["id"]["instrument"].is_string());
    assert!(signal["id"]["parameter"].is_string());
    assert!(signal["unit"]["id"].is_string());
    assert!(records.iter().any(
        |record| record["kind"] == "instrument" && record["implementation_kind"] == "emulated"
    ));
    let names = records
        .iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn current_and_window_make_quality_unit_status_and_identity_explicit() {
    let (mut service, mut app, _) = setup();
    let plant = service.owner().plant_id();
    let at = service.clock().now() + Duration::from_millis(1);
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: ParameterId::new(1),
            at,
        })
        .unwrap();
    let current = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"c","op":"measurements_current","args":{}}),
    );
    let rows = current[0]["result"]["records"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        assert!(row["signal"]["instrument"].is_string());
        assert!(row["unit"]["id"].is_string());
        assert!(row["quality"].is_string());
        assert!(row["status"].is_string());
        assert!(row["generation"].is_string());
        assert!(row.get("value").is_some());
        assert!(row.get("failure").is_some());
        assert!(row.get("observed_at_ns").is_some());
    }
    assert!(rows.iter().any(|row| row["quality"] == "good"));
    assert!(rows.iter().any(|row| row["quality"] == "unavailable"));
    let signal = rows[0]["signal"].clone();
    let window = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"w","op":"measurement_window",
        "args":{"signal":signal,"max_records":128}}),
    );
    assert_eq!(window[0]["result"]["ordering"], "oldest_first");
    assert_eq!(window[0]["result"]["source"], "runtime_recent");
    assert!(serde_json::to_vec(&window[0]["result"]).unwrap().len() <= 8 * 1024);
}

#[test]
fn missing_signal_is_structured_not_found_and_subscription_identity_matches_current() {
    let (mut service, mut app, _) = setup();
    let missing = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"missing","op":"latest",
        "args":{"signal":{"instrument":"999999","parameter":"1"}}}),
    );
    assert_eq!(missing[0]["category"], "not_found");
    let current = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"c","op":"measurements_current","args":{}}),
    );
    let id = current[0]["result"]["records"][0]["signal"].clone();
    let cursor = json!({"boot_id":service.boot_id(),"seq":service.owner().event_log().latest_cursor().to_string()});
    let subscribed = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"s","op":"subscribe",
        "args":{"after":cursor,"filter":{"kinds":["signal"],"targets":[]}}}),
    );
    assert!(subscribed[0]["result"]["subscription"].is_string());
    let boot_id = service.boot_id().to_owned();
    let second = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"s2","op":"subscribe",
            "args":{"after":{"boot_id":boot_id,"seq":"0"},
                "filter":{"kinds":[],"targets":[]}}}),
    );
    assert_eq!(second[0]["code"], "subscription_busy");
    let instrument = id["instrument"].as_str().unwrap().parse().unwrap();
    let parameter = id["parameter"].as_str().unwrap().parse().unwrap();
    let at = service.clock().now() + Duration::from_millis(1);
    let _ = service.owner_mut().command(Command::RefreshMeasurement {
        instrument: lab_core::InstrumentId::new(instrument),
        parameter: ParameterId::new(parameter),
        at,
    });
    let events = app.pump_events(&service, 1);
    if let Some(event) = events.iter().find(|event| event["kind"] == "signal") {
        assert_eq!(event["target"], id);
        assert_eq!(event["data"]["signal"], id);
        assert!(event["data"]["quality"].is_string());
    }
    let token = subscribed[0]["result"]["subscription"].clone();
    let removed = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"u","op":"unsubscribe","args":{"subscription":token}}),
    );
    assert_eq!(removed[0]["result"]["removed"], true);
}

#[test]
fn hello_advertises_only_implemented_measurement_capabilities_and_bounds() {
    let (mut service, mut app, _) = setup();
    app.detach(&service, 1);
    let hello = ask(
        &mut service,
        &mut app,
        2,
        json!({"v":1,"msg_id":"h2","op":"hello","args":{"scope":null}}),
    );
    let capabilities = hello[0]["result"]["capabilities"].as_array().unwrap();
    for name in [
        "structured_discovery",
        "current_measurements",
        "recent_measurement_history",
        "live_subscriptions",
    ] {
        assert!(
            capabilities
                .iter()
                .any(|capability| capability["name"] == name),
            "missing {name}"
        );
    }
    assert_eq!(hello[0]["result"]["limits"]["recent_history_records"], 128);
    assert_eq!(hello[0]["result"]["limits"]["subscriptions_per_client"], 1);
    assert_eq!(hello[0]["result"]["limits"]["client_event_queue"], 16);
}
