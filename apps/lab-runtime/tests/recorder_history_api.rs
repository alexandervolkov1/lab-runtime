//! Historical SQL is an Operation; page Query uses a bounded retained result.

use lab_core::Command;
use lab_runtime::{
    application::Application,
    host::Clock,
    service::{ServiceHost, ServiceOptions},
    wire::{WireRequest, decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn frame(value: Value) -> WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}
fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-history-api-{suffix}.sqlite"))
}

#[test]
fn history_read_is_accepted_then_caches_one_bounded_raw_page_for_pure_query() {
    let path = temporary_database();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"h","op":"hello",
        "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let start = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"start",
        "op":"recording_start","request_id":{"scope":scope,"seq":"1"},
        "args":{"label":"history API"}})),
    );
    assert_eq!(start[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().run_no.is_none() && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        let _ = app.poll_recording(&mut service);
        std::thread::yield_now();
    }
    let start_watermark = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let at = service.clock().now() + Duration::from_millis(1);
    let plant = service.owner().plant_id();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at,
        })
        .unwrap();
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_records
            > 0
            || service
                .owner()
                .recording_status()
                .unwrap()
                .persisted_through_sequence
                > start_watermark,
        "refresh was not admitted into Recorder: {:?}",
        service.owner().recording_status()
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence
        <= start_watermark
        && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"s",
        "op":"recording_status","args":{}})),
    );
    let database_id = status[0]["result"]["database_id"].clone();
    let run_id = status[0]["result"]["run_id"].clone();
    let read = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"read",
        "op":"history_read","request_id":{"scope":scope,"seq":"2"},
        "args":{"mode":"measurements","database_id":database_id,
            "boot_id":run_id["boot_id"],"run_id":run_id,
            "signal":{"instrument":"1","parameter":"1"},
            "from_ns":"0","to_ns":"10000000000","max_records":8,"cursor":null}})),
    );
    assert_eq!(read.len(), 1);
    assert_eq!(read[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        terminal = app.poll_history(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    let token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"p",
        "op":"history_page","args":{"page_token":token}})),
    );
    assert_eq!(
        page[0]["result"]["rows"].as_array().unwrap().len(),
        1,
        "page={page:?}, status={status:?}, at={at:?}, owner={:?}",
        service.owner().recording_status()
    );
    assert_eq!(page[0]["result"]["rows"][0]["quality"], "good");
    let reject = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"bad",
        "op":"history_read","request_id":{"scope":scope,"seq":"3"},
        "args":{"mode":"measurements","database_id":database_id,
            "boot_id":run_id["boot_id"],"run_id":run_id,
            "signal":{"instrument":"1","parameter":"1"},"from_ns":"0",
            "to_ns":"10000000000","max_records":129,"cursor":null}})),
    );
    assert_eq!(reject[0]["type"], "error");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn run_discovery_operation_lists_archived_runs_after_service_reopen() {
    let path = temporary_database();
    let mut previous = lab_runtime::recorder::SqliteStore::open_with_boot(
        &path,
        "88888888888888888888888888888888",
    )
    .unwrap();
    previous.start_run("archive A").unwrap();
    previous.stop_run().unwrap();
    drop(previous);
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"h","op":"hello",
        "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"s",
        "op":"recording_status","args":{}})),
    );
    let database_id = status[0]["result"]["database_id"].clone();
    let read = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"runs",
        "op":"history_read","request_id":{"scope":scope,"seq":"1"},
        "args":{"mode":"runs","database_id":database_id,"max_records":32,"cursor":null}})),
    );
    assert_eq!(read.len(), 1);
    assert_eq!(read[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        terminal = app.poll_history(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal[0].1["state"], "completed");
    let token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"page",
        "op":"history_page","args":{"page_token":token}})),
    );
    assert_eq!(page[0]["result"]["runs"].as_array().unwrap().len(), 1);
    assert_eq!(page[0]["result"]["runs"][0]["label"], "archive A");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}
