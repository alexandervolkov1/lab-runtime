//! Standard service composition keeps real Lua observations separate from native authority.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn query(service: &mut ServiceHost, app: &mut Application, op: &str, args: Value) -> Value {
    let req =
        decode_frame(&encode_frame(&json!({"v":1,"msg_id":"q","op":op,"args":args})).unwrap())
            .unwrap();
    app.handle(service, 1, req).remove(0)
}

#[test]
fn standard_profile_prepares_two_real_lua_components_before_readiness_and_schedules_steps() {
    let mut service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    query(&mut service, &mut app, "hello", json!({"scope":null}));
    let discovery = query(&mut service, &mut app, "discover", json!({}));
    assert_eq!(
        discovery["result"]["components"].as_array().unwrap().len(),
        2
    );
    let initial = query(
        &mut service,
        &mut app,
        "component",
        json!({"component":"201"}),
    );
    assert_eq!(initial["result"]["state"], "warming");
    let started = Instant::now();
    let mut progressed = false;
    while started.elapsed() < Duration::from_secs(2) {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        let source = query(
            &mut service,
            &mut app,
            "component",
            json!({"component":"201"}),
        );
        let latest = query(
            &mut service,
            &mut app,
            "latest",
            json!({"signal":{"instrument":"201","parameter":lab_core::TEMPERATURE.get().to_string()}}),
        );
        if source["result"]["state"] == "ready" && latest["result"]["quality"] == "good" {
            progressed = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        progressed,
        "real Lua Source did not publish a Good observation"
    );
    assert_eq!(
        query(
            &mut service,
            &mut app,
            "controller",
            json!({"controller":"1"})
        )["result"]["state"],
        "ready"
    );
}
