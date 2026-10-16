//! Application admission records domain outcomes before any reply delivery.

use lab_core::reference::{ReferenceConfig, ReferenceId};
use lab_core::{Command, Unit};
use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::decode_frame,
};
use serde_json::{Value, json};
use std::time::Duration;

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
fn two_live_scopes_racing_on_one_revision_have_exactly_one_committed_retune() {
    let (mut service, mut app) = startup();
    let a = hello(&mut service, &mut app, 1);
    let b = hello(&mut service, &mut app, 2);
    let first = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"a","op":"reference_retune","request_id":{"scope":a,"seq":"1"},
            "args":{"reference":"1","expected_revision":"1","target":61.0,"rate":3.0}
        })),
    );
    let second = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"b","op":"reference_retune","request_id":{"scope":b,"seq":"1"},
            "args":{"reference":"1","expected_revision":"1","target":62.0,"rate":3.0}
        })),
    );
    assert_eq!(first[0]["state"], "accepted");
    assert_eq!(first[1]["state"], "completed");
    assert_eq!(second[0]["state"], "accepted");
    assert_eq!(second[1]["state"], "failed");
    let current = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"q","op":"reference","args":{"reference":"1"}
        })),
    );
    assert_eq!(current[0]["result"]["revision"], "2");
    assert_eq!(current[0]["result"]["target"], 61.0);
}

#[test]
fn retune_outcome_reports_the_registered_reference_identity_instead_of_a_profile_constant() {
    let (mut service, mut app) = startup();
    service
        .owner_mut()
        .command(Command::RegisterReference(ReferenceConfig::Ramp {
            id: ReferenceId::new(2),
            start: 20.0,
            target: 20.0,
            rate: 1.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        }))
        .unwrap();
    service
        .owner_mut()
        .event_log_mut()
        .track_reference(ReferenceId::new(2));
    let scope = hello(&mut service, &mut app, 1);
    let result = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"r2","op":"reference_retune","request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":"2","expected_revision":"1","target":45.0,"rate":2.0}
        })),
    );
    assert_eq!(result[1]["state"], "completed");
    assert_eq!(result[1]["result"]["reference"], "2");
}

#[test]
fn accepted_shutdown_barrier_rejects_other_clients_queued_start_before_any_output_lease() {
    let (mut service, mut app) = startup();
    let stopping_scope = hello(&mut service, &mut app, 1);
    let other_scope = hello(&mut service, &mut app, 2);
    let accepted = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"stop","op":"runtime_shutdown",
            "request_id":{"scope":stopping_scope,"seq":"1"},"args":{}
        })),
    );
    assert_eq!(accepted[0]["state"], "accepted");
    let start = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"late","op":"controller_start",
            "request_id":{"scope":other_scope,"seq":"1"},"args":{"controller":"1"}
        })),
    );
    assert_eq!(start[0]["code"], "shutdown_before_execution");
    assert_eq!(start[0]["accepted"], false);
    let current = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"q","op":"controller","args":{"controller":"1"}
        })),
    );
    assert_ne!(current[0]["result"]["state"], "running");
    assert_eq!(
        current[0]["result"]["latest_output"],
        serde_json::Value::Null
    );
}

#[test]
fn stopping_owner_refuses_new_hello_scope_and_reports_service_state_honestly() {
    let (mut service, mut app) = startup();
    let scope = hello(&mut service, &mut app, 1);
    let stop = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"stop","op":"runtime_shutdown",
            "request_id":{"scope":scope,"seq":"1"},"args":{}
        })),
    );
    assert_eq!(stop[0]["state"], "accepted");
    let late = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"late","op":"hello","args":{"scope":null}
        })),
    );
    assert_eq!(late[0]["code"], "shutdown_in_progress");
    assert_eq!(late[0]["accepted"], false);
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

#[test]
fn shutdown_is_accepted_first_and_terminal_only_after_safe_evidence_is_recorded() {
    let (mut service, mut app) = startup();
    let scope = hello(&mut service, &mut app, 1);
    let replies = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"stop","op":"runtime_shutdown",
        "request_id":{"scope":scope,"seq":"1"},"args":{}})),
    );
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["state"], "accepted");
    let state = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status","op":"operation_status",
        "args":{"request_id":{"scope":scope,"seq":"1"}}})),
    );
    assert_eq!(state[0]["result"]["state"], "accepted");
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break app.finish_shutdown(&mut service, status);
        }
        std::thread::yield_now();
    };
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    assert_eq!(terminal[0].1["result"]["safe_confirmed"], true);
    assert_eq!(
        terminal[0].1["result"]["outputs"][0]["safe_confirmed"],
        true
    );
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"later","op":"operation_status",
        "args":{"request_id":{"scope":scope,"seq":"1"}}})),
    );
    assert_eq!(status[0]["result"]["state"], "completed");
}
