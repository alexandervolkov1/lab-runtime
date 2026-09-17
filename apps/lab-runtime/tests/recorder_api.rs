//! M9B.5 semantic Recorder Application API contracts.

use lab_runtime::{
    application::Application,
    recorder::RecordingState,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn frame(value: Value) -> lab_runtime::wire::WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

fn temporary_database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "lab-runtime-m9b5-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn recorded_service(path: &Path) -> ServiceHost {
    let text = path.to_string_lossy();
    ServiceHost::startup(
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--record-db",
            text.as_ref(),
        ])
        .unwrap(),
    )
    .unwrap()
}

fn hello(service: &mut ServiceHost, app: &mut Application, connection: u64) -> Value {
    app.handle(
        service,
        connection,
        frame(json!({"v":1,"msg_id":format!("hello-{connection}"),
            "op":"hello","args":{"scope":null}})),
    )
    .remove(0)
}

fn progress(
    service: &mut ServiceHost,
    app: &mut Application,
    wanted: RecordingState,
) -> Vec<(u64, Value)> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut replies = Vec::new();
    while service.owner().recording_status().unwrap().state != wanted {
        assert!(
            Instant::now() < deadline,
            "wanted {wanted:?}, status {:?}",
            service.owner().recording_status()
        );
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        replies.extend(app.poll_recording(service));
        std::thread::yield_now();
    }
    replies.extend(app.poll_recording(service));
    replies
}

fn drain_ingress(service: &mut ServiceHost) {
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
}

fn shutdown_and_remove(mut service: ServiceHost, path: PathBuf) {
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn unconfigured_status_is_explicit_and_control_is_not_advertised() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = hello(&mut service, &mut app, 1);
    let operations = hello["result"]["operations"].as_array().unwrap();
    assert!(operations.iter().any(|op| op == "recording_status"));
    assert!(!operations.iter().any(|op| op == "recording_start"));
    let capabilities = hello["result"]["capabilities"].as_array().unwrap();
    assert!(
        capabilities
            .iter()
            .any(|cap| cap["name"] == "recording_status")
    );
    assert!(
        !capabilities
            .iter()
            .any(|cap| cap["name"] == "recording_control")
    );
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status","op":"recording_status","args":{}})),
    );
    let state = &status[0]["result"];
    assert_eq!(state["configured"], false);
    assert_eq!(state["available"], false);
    assert_eq!(state["state"], "unconfigured");
    assert_eq!(state["failure"]["code"], "recorder_not_configured");
}

#[test]
fn lifecycle_projection_events_and_sealed_terminal_share_run_identity() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = temporary_database();
    let mut service = recorded_service(&path);
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = hello(&mut service, &mut app, 1);
    let scope = hello["result"]["scope"].as_str().unwrap().to_owned();
    let capabilities = hello["result"]["capabilities"].as_array().unwrap();
    for capability in [
        "recording_status",
        "recording_control",
        "measurement_history",
    ] {
        assert!(capabilities.iter().any(|cap| cap["name"] == capability));
    }
    let initial = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"initial","op":"recording_status","args":{}})),
    );
    assert_eq!(initial[0]["result"]["state"], "idle");
    assert_eq!(initial[0]["result"]["accepting_facts"], false);
    assert!(initial[0]["result"]["active_run"].is_null());
    let cursor = json!({"boot_id":service.boot_id(),
        "seq":service.owner().event_log().latest_cursor().to_string()});
    let subscribed = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"sub","op":"subscribe",
            "args":{"after":cursor,"filter":{"kinds":["recorder"],"targets":[]}}})),
    );
    assert_eq!(subscribed[0]["type"], "result");
    let started = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"start","op":"recording_start",
            "request_id":{"scope":scope,"seq":"1"},"args":{"label":"M9B.5"}})),
    );
    assert_eq!(started[0]["state"], "accepted");
    let terminal = progress(&mut service, &mut app, RecordingState::Recording);
    assert!(
        terminal
            .iter()
            .any(|(_, reply)| reply["state"] == "completed")
    );
    let active = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"active","op":"recording_status","args":{}})),
    );
    let state = &active[0]["result"];
    assert_eq!(state["configured"], true);
    assert_eq!(state["accepting_facts"], true);
    assert!(state["active_run"]["run_no"].is_string());
    assert!(state["active_interval"]["interval_no"].is_string());
    assert_eq!(state["active_run"], state["run_id"]);
    let run_id = state["run_id"].clone();
    let archive_id = state["archive"]["id"].clone();
    drain_ingress(&mut service);
    let duplicate = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"start-again","op":"recording_start",
            "request_id":{"scope":scope,"seq":"2"},"args":{"label":"duplicate"}})),
    );
    assert_eq!(duplicate[1]["code"], "invalid_state");
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    drain_ingress(&mut service);
    let events = app.pump_events(&service, 1);
    let recording_event = events
        .iter()
        .rev()
        .find(|event| event["kind"] == "recorder" && event["data"]["state"] == "recording")
        .unwrap();
    assert_eq!(recording_event["target"]["id"], archive_id);
    assert_eq!(recording_event["data"]["active_run"], run_id);

    let stopped = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"stop","op":"recording_stop",
            "request_id":{"scope":scope,"seq":"3"},"args":{"run_id":run_id}})),
    );
    assert_eq!(stopped[0]["state"], "accepted");
    let stop_terminal = progress(&mut service, &mut app, RecordingState::Idle);
    let stopped = stop_terminal
        .iter()
        .find_map(|(_, reply)| (reply["state"] == "completed").then_some(reply))
        .unwrap();
    assert_eq!(stopped["result"]["run_id"], run_id);
    assert_eq!(
        stopped["result"]["completion"]["accepted_facts_drained"],
        true
    );
    assert_eq!(stopped["result"]["completion"]["interval_sealed"], true);
    assert_eq!(stopped["result"]["completion"]["run_sealed"], true);
    assert_eq!(
        stopped["result"]["completion"]["transaction_committed"],
        true
    );
    assert_eq!(stopped["result"]["completion"]["writer_closed"], false);
    assert_eq!(
        stopped["result"]["completion"]["archive_boot_sealed"],
        false
    );
    let invalid_stop = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"stop-again","op":"recording_stop",
            "request_id":{"scope":scope,"seq":"4"},"args":{"run_id":run_id}})),
    );
    assert_eq!(invalid_stop[1]["code"], "invalid_state");
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Idle
    );

    let runs = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"runs","op":"history_read",
            "request_id":{"scope":scope,"seq":"5"},
            "args":{"mode":"runs","database_id":archive_id,
                "max_records":32,"cursor":null}})),
    );
    assert_eq!(runs[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let history_terminal = loop {
        let replies = app.poll_history(&mut service);
        if let Some((_, reply)) = replies.into_iter().next() {
            break reply;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    let token = history_terminal["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"page","op":"history_page",
            "args":{"page_token":token}})),
    );
    assert_eq!(page[0]["result"]["runs"][0]["run_id"], run_id);
    drop(app);
    shutdown_and_remove(service, path);
}

#[test]
fn disconnect_never_owns_start_stop_and_reconnect_rebuilds_current_state() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = temporary_database();
    let mut service = recorded_service(&path);
    let mut app = Application::new(service.boot_id()).unwrap();
    let first = hello(&mut service, &mut app, 1);
    let scope = first["result"]["scope"].as_str().unwrap().to_owned();
    let accepted = app.handle(
        &mut service,
        1,
        frame(json!({
        "v":1,"msg_id":"start","op":"recording_start",
        "request_id":{"scope":scope,"seq":"1"},"args":{"label":"detached"}})),
    );
    assert_eq!(accepted[0]["state"], "accepted");
    app.detach(&service, 1);
    assert!(progress(&mut service, &mut app, RecordingState::Recording).is_empty());
    let second = app.handle(
        &mut service,
        2,
        frame(json!({
        "v":1,"msg_id":"hello-2","op":"hello","args":{"scope":scope}})),
    );
    assert_eq!(second[0]["type"], "result");
    let current = app.handle(
        &mut service,
        2,
        frame(json!({
        "v":1,"msg_id":"current","op":"recording_status","args":{}})),
    );
    assert_eq!(current[0]["result"]["state"], "recording");
    let run_id = current[0]["result"]["run_id"].clone();
    drain_ingress(&mut service);
    let stop = app.handle(
        &mut service,
        2,
        frame(json!({
        "v":1,"msg_id":"stop","op":"recording_stop",
        "request_id":{"scope":scope,"seq":"2"},"args":{"run_id":run_id}})),
    );
    assert_eq!(stop[0]["state"], "accepted");
    app.detach(&service, 2);
    assert!(progress(&mut service, &mut app, RecordingState::Idle).is_empty());
    let third = hello(&mut service, &mut app, 3);
    assert_eq!(third["type"], "result");
    let idle = app.handle(
        &mut service,
        3,
        frame(json!({
        "v":1,"msg_id":"idle","op":"recording_status","args":{}})),
    );
    assert_eq!(idle[0]["result"]["state"], "idle");
    assert_eq!(idle[0]["result"]["accepting_facts"], false);
    drop(app);
    shutdown_and_remove(service, path);
}
