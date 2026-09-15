//! Public lifecycle uses the same deduplication and committed-start barrier.

use lab_runtime::{
    application::Application,
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, WriterBarrier},
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
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-api-{suffix}.sqlite"))
}

#[test]
fn recording_start_returns_accepted_before_durable_completion_and_duplicate_does_not_restart() {
    let path = temporary_database();
    let path_text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        path_text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"h","op":"hello","args":{"scope":null}
        })),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"s","op":"recording_status","args":{}
        })),
    );
    assert_eq!(status[0]["result"]["state"], "idle");
    let request = json!({
        "v":1,"msg_id":"start","op":"recording_start",
        "request_id":{"scope":scope,"seq":"1"},"args":{"label":"API fixture"}
    });
    let accepted = app.handle(&mut service, 1, frame(request.clone()));
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["state"], "accepted");
    let duplicate = app.handle(&mut service, 1, frame(request));
    assert_eq!(duplicate.len(), 1);
    assert_eq!(duplicate[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        terminal = app.poll_recording(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    let status = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"s2","op":"recording_status","args":{}
        })),
    );
    assert_eq!(status[0]["result"]["state"], "recording");
    let run_id = status[0]["result"]["run_id"].clone();
    let stop = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"stop","op":"recording_stop",
            "request_id":{"scope":scope,"seq":"2"},"args":{"run_id":run_id}
        })),
    );
    assert_eq!(stop.len(), 1);
    assert_eq!(stop[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut stop_terminal = Vec::new();
    while stop_terminal.is_empty() && Instant::now() < deadline {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        stop_terminal = app.poll_recording(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(stop_terminal.len(), 1);
    assert_eq!(stop_terminal[0].1["state"], "completed");
    let closed_status = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"s3","op":"recording_status","args":{}
        })),
    );
    assert_eq!(closed_status[0]["result"]["state"], "idle");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn accepted_retune_and_terminal_outcome_survive_client_disconnect_and_sqlite_reopen() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let serving_boot = service.boot_id().to_owned();
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("operation audit", now)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    let initial_watermark = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"hello","op":"hello","args":{"scope":null}
        })),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let reference = service.owner().reference_id().get().to_string();
    let outcome = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"retune","op":"reference_retune",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":reference,"expected_revision":"1",
                "target":55.0,"rate":1.0}
        })),
    );
    assert_eq!(outcome.len(), 2);
    assert_eq!(outcome[0]["state"], "accepted");
    assert_eq!(outcome[1]["state"], "completed");
    let duplicate = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"retune-again","op":"reference_retune",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":reference,"expected_revision":"1",
                "target":55.0,"rate":1.0}
        })),
    );
    assert_eq!(duplicate.len(), 1);
    assert_eq!(duplicate[0]["state"], "completed");
    let reached_deadline = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() && Instant::now() < reached_deadline {
        std::thread::yield_now();
    }
    assert!(
        barrier.reached(),
        "writer never entered held operation commit"
    );
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .persisted_through_sequence,
        initial_watermark,
        "domain completion must precede held durability"
    );
    app.detach(&service, 1);
    barrier.release();
    let receipt_deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .outstanding_groups
        > 0
        && Instant::now() < receipt_deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_groups,
        0,
        "accepted operation prefix must receipt before adding shutdown facts"
    );
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(
        terminal.recorder_flushed,
        "terminal={terminal:?}, recorder={:?}",
        service.owner().recording_status()
    );
    drop(app);
    drop(service);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let phases: Vec<(String, String, String)> = archive
        .prepare("SELECT phase,command,request_seq FROM operation_events ORDER BY record_seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        phases,
        vec![
            ("accepted".into(), "reference_retune".into(), "1".into()),
            ("completed".into(), "reference_retune".into(), "1".into())
        ]
    );
    let archived_boot: String = archive
        .query_row(
            "SELECT lower(hex(boot_id)) FROM operation_events LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived_boot, serving_boot);
    let mapped: Vec<(Option<i64>, Option<String>)> = archive
        .prepare(
            "SELECT r.wall_estimate_us,r.wall_basis FROM records r JOIN operation_events o
                  ON o.boot_id=r.boot_id AND o.record_seq=r.record_seq ORDER BY r.record_seq",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(mapped.len(), 2);
    assert!(
        mapped
            .iter()
            .all(|(wall, basis)| wall.is_some() && basis.as_deref() == Some("boot_anchor"))
    );
    drop(archive);
    std::fs::remove_file(path).unwrap();
}
