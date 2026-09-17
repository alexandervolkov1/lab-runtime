//! Committed host DTOs reveal descriptors/config/evidence without domain progress.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};

fn setup() -> (ServiceHost, Application) {
    let service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let app = Application::new(service.boot_id()).unwrap();
    (service, app)
}
fn ask(service: &mut ServiceHost, app: &mut Application, op: &str, args: Value) -> Value {
    let req =
        decode_frame(&encode_frame(&json!({"v":1,"msg_id":"q","op":op,"args":args})).unwrap())
            .unwrap();
    app.handle(service, 1, req).remove(0)
}
fn hello(service: &mut ServiceHost, app: &mut Application) {
    assert_eq!(
        ask(service, app, "hello", json!({"scope":null}))["type"],
        "result"
    );
}

#[test]
fn descriptor_and_controller_configuration_report_exact_roles_units_and_limits() {
    let (mut service, mut app) = setup();
    hello(&mut service, &mut app);
    let descriptor = ask(
        &mut service,
        &mut app,
        "describe",
        json!({"instrument":"1"}),
    );
    assert_eq!(descriptor["type"], "result");
    let parameters = descriptor["result"]["parameters"].as_array().unwrap();
    let temp = parameters
        .iter()
        .find(|p| {
            p["id"].as_str().and_then(|s| s.parse::<u64>().ok())
                == Some(lab_core::TEMPERATURE.get())
        })
        .unwrap();
    assert_eq!(temp["role"], "measurement");
    assert_eq!(temp["unit"]["id"], "degC");
    let heater = parameters
        .iter()
        .find(|p| {
            p["id"].as_str().and_then(|s| s.parse::<u64>().ok())
                == Some(lab_core::HEATER_POWER.get())
        })
        .unwrap();
    assert_eq!(heater["role"], "actuator");
    assert_eq!(heater["write_effect"], "output_affecting");
    let controller = ask(
        &mut service,
        &mut app,
        "controller",
        json!({"controller":"1"}),
    );
    assert_eq!(controller["result"]["config"]["pid"]["output_max"], 100.0);
    assert_eq!(
        controller["result"]["config"]["max_tick_gap_ns"],
        "500000000"
    );
    let discover = ask(&mut service, &mut app, "discover", json!({}));
    let components = discover["result"]["components"].as_array().unwrap();
    assert!(components.iter().any(|component| {
        component["implementation"] == "lua.v1" && component["kind"] == "source"
    }));
    assert!(components.iter().any(|component| {
        component["implementation"] == "native.moving_mean.v1" && component["kind"] == "transform"
    }));
}

#[test]
fn output_query_reports_rust_safe_readback_and_cannot_forge_authority() {
    let (mut service, mut app) = setup();
    hello(&mut service, &mut app);
    let before = ask(
        &mut service,
        &mut app,
        "output",
        json!({"actuator":{"instrument":"1","parameter":lab_core::HEATER_POWER.get().to_string()}}),
    );
    assert_eq!(before["result"]["state"], "disarmed");
    assert_eq!(before["result"]["safe_confirmed"], true);
    assert_eq!(before["result"]["readback"]["value"], 0.0);
    assert!(before["result"]["owner"].is_null());
    let after = ask(
        &mut service,
        &mut app,
        "output",
        json!({"actuator":{"instrument":"1","parameter":lab_core::HEATER_POWER.get().to_string()}}),
    );
    assert_eq!(before["result"], after["result"]);
}
