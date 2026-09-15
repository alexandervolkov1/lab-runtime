//! Application admission records domain outcomes before any reply delivery.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::decode_frame,
};
use serde_json::{Value, json};

fn startup() -> (ServiceHost, Application) {
    let service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let app = Application::new(service.boot_id()).unwrap();
    (service, app)
}
fn frame(value: Value) -> lab_runtime::wire::WireRequest {
    let bytes = lab_runtime::wire::encode_frame(&value).unwrap();
    decode_frame(&bytes).unwrap()
}
fn hello(service: &mut ServiceHost, app: &mut Application, connection: u64) -> String {
    let replies = app.handle(
        service,
        connection,
        frame(json!({
            "v":1,"msg_id":"hello","op":"hello","args":{"scope":null}
        })),
    );
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["type"], "result");
    replies[0]["result"]["scope"].as_str().unwrap().into()
}

#[test]
fn start_is_accepted_then_completed_at_warming_without_claiming_running_or_physical_evidence() {
    let (mut service, mut app) = startup();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    let scope = hello(&mut service, &mut app, 1);
    let replies = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"start","op":"controller_start",
            "request_id":{"scope":scope,"seq":"1"},"args":{"controller":"1"}
        })),
    );
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "completed");
    assert_eq!(replies[1]["result"]["state"], "warming");
    assert!(replies[1]["result"].get("ReadbackVerified").is_none());
    let query = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"q","op":"controller","args":{"controller":"1"}
        })),
    );
    assert_eq!(query[0]["result"]["state"], "warming");
    let status = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"s","op":"operation_status",
            "args":{"request_id":{"scope":scope,"seq":"1"}}
        })),
    );
    assert_eq!(status[0]["result"]["state"], "completed");
}

#[test]
fn invalid_domain_config_fails_after_acceptance_and_its_revision_does_not_change() {
    let (mut service, mut app) = startup();
    let scope = hello(&mut service, &mut app, 1);
    let result = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"bad","op":"reference_retune",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":"1","expected_revision":"1","target":40.0,"rate":0.0}
        })),
    );
    assert_eq!(result[0]["state"], "accepted");
    assert_eq!(result[1]["state"], "failed");
    let query = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"q","op":"reference","args":{"reference":"1"}
        })),
    );
    assert_eq!(query[0]["result"]["revision"], "1");
    let repeated = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"again","op":"reference_retune",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":"1","expected_revision":"1","target":40.0,"rate":0.0}
        })),
    );
    assert_eq!(repeated.len(), 1);
    assert_eq!(repeated[0]["state"], "failed");
}

#[test]
fn a_pure_query_does_not_refresh_or_advance_reference_without_the_scheduler() {
    let (mut service, mut app) = startup();
    hello(&mut service, &mut app, 1);
    let before = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"r1","op":"reference","args":{"reference":"1"}
        })),
    );
    let first_latest = app.handle(&mut service, 1, frame(json!({
        "v":1,"msg_id":"m1","op":"latest","args":{"signal":{"instrument":"1","parameter":lab_core::TEMPERATURE.get().to_string()}}
    })));
    let after = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"r2","op":"reference","args":{"reference":"1"}
        })),
    );
    let second_latest = app.handle(&mut service, 1, frame(json!({
        "v":1,"msg_id":"m2","op":"latest","args":{"signal":{"instrument":"1","parameter":lab_core::TEMPERATURE.get().to_string()}}
    })));
    assert_eq!(before[0]["result"], after[0]["result"]);
    assert_eq!(first_latest[0]["result"], second_latest[0]["result"]);
}
