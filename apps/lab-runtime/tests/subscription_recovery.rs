//! Frozen projection/cursor barrier and ring replay are owner-local, bounded state.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};

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
        json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().into();
    (service, app, scope)
}
fn ask(
    service: &mut ServiceHost,
    app: &mut Application,
    connection: u64,
    value: Value,
) -> Vec<Value> {
    let req = decode_frame(&encode_frame(&value).unwrap()).unwrap();
    app.handle(service, connection, req)
}

#[test]
fn retune_between_discovery_and_subscribe_is_replayed_and_pages_remain_frozen() {
    let (mut service, mut app, scope) = setup();
    let snapshot = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"snap","op":"discover","args":{}}),
    );
    assert_eq!(snapshot[0]["type"], "result");
    let token = snapshot[0]["result"]["projection"].as_str().unwrap();
    let cursor = json!({
        "boot_id": snapshot[0]["result"]["revision"]["boot_id"],
        "seq": snapshot[0]["result"]["revision"]["event_seq"]
    });
    let index = snapshot[0]["result"]["next_index"].as_str().unwrap_or("0");
    let frozen_page = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"page","op":"discovery_page","args":{"projection":token,"index":index}}),
    );
    let mutation = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"retune","op":"reference_retune",
        "request_id":{"scope":scope,"seq":"1"},"args":{"reference":"1","expected_revision":"1","target":60.0,"rate":4.0}}),
    );
    assert_eq!(mutation[1]["state"], "completed");
    let after_page = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"page2","op":"discovery_page","args":{"projection":token,"index":index}}),
    );
    assert_eq!(
        frozen_page[0]["result"]["records"],
        after_page[0]["result"]["records"]
    );
    let sub = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"sub","op":"subscribe","args":{"after":cursor,
        "filter":{"kinds":[],"targets":[]}}}),
    );
    assert_eq!(sub[0]["type"], "result");
    let offered = app.pump_events(&service, 1);
    assert!(
        offered
            .iter()
            .any(|e| e["kind"] == "reference" && e["data"]["revision"] == "2")
    );
}

#[test]
fn ring_overrun_rejects_old_cursor_with_gap_and_filtered_scan_reports_progress() {
    let (mut service, mut app, scope) = setup();
    let boot = service.boot_id().to_string();
    let sub = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"filter","op":"subscribe",
        "args":{"after":{"boot_id":boot,"seq":"0"},"filter":{"kinds":["output"],"targets":[]}}}),
    );
    assert_eq!(sub[0]["type"], "result");
    let first = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"retune","op":"reference_retune",
        "request_id":{"scope":scope,"seq":"1"},"args":{"reference":"1","expected_revision":"1","target":60.0,"rate":4.0}}),
    );
    assert_eq!(first[1]["state"], "completed");
    let progress = app.pump_events(&service, 1);
    assert_eq!(progress[0]["type"], "subscription_progress");
    assert!(progress[0]["seq"].as_str().unwrap().parse::<u64>().unwrap() > 0);
    ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"un","op":"unsubscribe","args":{"subscription":sub[0]["result"]["subscription"]}}),
    );
    for n in 2..=520u64 {
        let result = ask(
            &mut service,
            &mut app,
            1,
            json!({"v":1,"msg_id":"many","op":"reference_retune",
            "request_id":{"scope":scope,"seq":n.to_string()},"args":{"reference":"1","expected_revision":n.to_string(),
                "target":30.0+(n%2) as f64,"rate":3.0}}),
        );
        assert_eq!(result[1]["state"], "completed");
    }
    let gap = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"gap","op":"subscribe",
        "args":{"after":{"boot_id":boot,"seq":"0"},"filter":{"kinds":[],"targets":[]}}}),
    );
    assert_eq!(gap[0]["code"], "event_gap");
    assert_eq!(gap[0]["resync_required"], true);
    assert!(
        gap[0]["oldest"]["seq"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn eviction_during_an_installed_replay_reports_gap_and_abandons_that_subscription() {
    let (mut service, mut app, scope) = setup();
    let boot = service.boot_id().to_string();
    let sub = ask(
        &mut service,
        &mut app,
        1,
        json!({
            "v":1,"msg_id":"sub","op":"subscribe",
            "args":{"after":{"boot_id":boot,"seq":"0"},"filter":{"kinds":[],"targets":[]}}
        }),
    );
    assert_eq!(sub[0]["type"], "result");
    for seq in 1..=520u64 {
        let retune = ask(
            &mut service,
            &mut app,
            1,
            json!({
                "v":1,"msg_id":"many","op":"reference_retune",
                "request_id":{"scope":scope,"seq":seq.to_string()},
                "args":{"reference":"1","expected_revision":seq.to_string(),
                "target":30.0+(seq%2) as f64,"rate":3.0}
            }),
        );
        assert_eq!(retune[1]["state"], "completed");
    }
    let gap = app.pump_events(&service, 1);
    assert_eq!(gap.len(), 1);
    assert_eq!(gap[0]["code"], "event_gap");
    assert_eq!(gap[0]["resync_required"], true);
    assert!(app.pump_events(&service, 1).is_empty());
}

#[test]
fn expired_projection_and_previous_boot_cursor_or_scope_require_explicit_resync() {
    let (mut first, mut app, scope) = setup();
    let snap = ask(
        &mut first,
        &mut app,
        1,
        json!({"v":1,"msg_id":"snap","op":"discover","args":{}}),
    );
    let token = snap[0]["result"]["projection"].as_str().unwrap();
    app.expire_snapshots_at(std::time::Duration::from_secs(6));
    let page = ask(
        &mut first,
        &mut app,
        1,
        json!({"v":1,"msg_id":"page","op":"discovery_page","args":{"projection":token,"index":"0"}}),
    );
    assert_eq!(page[0]["code"], "snapshot_expired");
    let old_boot = first.boot_id().to_string();
    drop(app);
    drop(first);
    let mut second = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    assert_ne!(old_boot, second.boot_id());
    let mut fresh = Application::new(second.boot_id()).unwrap();
    let old_scope = ask(
        &mut second,
        &mut fresh,
        2,
        json!({"v":1,"msg_id":"old","op":"hello","args":{"scope":scope}}),
    );
    assert_eq!(old_scope[0]["code"], "instance_changed");
    ask(
        &mut second,
        &mut fresh,
        2,
        json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}}),
    );
    let old_cursor = ask(
        &mut second,
        &mut fresh,
        2,
        json!({"v":1,"msg_id":"oldcur","op":"subscribe",
        "args":{"after":{"boot_id":old_boot,"seq":"0"},"filter":{"kinds":[],"targets":[]}}}),
    );
    assert_eq!(old_cursor[0]["code"], "instance_changed");
}

#[test]
fn typed_target_filter_delivers_only_matching_events_and_advances_over_others() {
    let (mut service, mut app, scope) = setup();
    let boot = service.boot_id().to_string();
    let sub = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"filter","op":"subscribe",
        "args":{"after":{"boot_id":boot,"seq":"0"},"filter":{"kinds":[],"targets":[{"kind":"reference","id":"1"}]}}}),
    );
    assert_eq!(sub[0]["type"], "result");
    let retune = ask(
        &mut service,
        &mut app,
        1,
        json!({"v":1,"msg_id":"retune","op":"reference_retune",
        "request_id":{"scope":scope,"seq":"1"},"args":{"reference":"1","expected_revision":"1","target":63.0,"rate":4.0}}),
    );
    assert_eq!(retune[1]["state"], "completed");
    let offered = app.pump_events(&service, 1);
    assert!(offered.iter().any(|e| e["kind"] == "reference"));
    assert!(offered.iter().all(|e| e["kind"] != "operation"));
}
