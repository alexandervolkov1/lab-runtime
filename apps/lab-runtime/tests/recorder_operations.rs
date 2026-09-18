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

#[derive(Clone, Copy)]
struct FrozenClock(Duration);

impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-api-{suffix}.sqlite"))
}

#[test]
fn start_acceptance_commits_before_held_terminal_then_pause_from_new_client_reopens_once() {
    let path = temporary_database();
    let barrier = WriterBarrier::held_terminal_operation_after_acceptance();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    service
        .owner_mut()
        .start_recording("start/pause operation", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let prefix = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let mut app = Application::new(service.boot_id()).unwrap();
    let scope_a = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"h-a","op":"hello","args":{"scope":null}
        })),
    )[0]["result"]["scope"]
        .as_str()
        .unwrap()
        .to_owned();
    let start = json!({"v":1,"msg_id":"start","op":"controller_start",
        "request_id":{"scope":scope_a,"seq":"1"},"args":{"controller":"1"}});
    let replies = app.handle(&mut service, 1, frame(start));
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "completed");
    assert_eq!(replies[1]["result"]["state"], "warming");
    let by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < by, "terminal SQL did not reach held stage");
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .persisted_through_sequence
            > prefix,
        "accepted command must commit before held terminal"
    );
    app.detach(&service, 1);
    barrier.release();
    let receipt_by = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .outstanding_records
        != 0
    {
        assert!(Instant::now() < receipt_by);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let scope_b = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"h-b","op":"hello","args":{"scope":null}
        })),
    )[0]["result"]["scope"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(scope_a, scope_b);
    let pause = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"pause","op":"controller_pause",
            "request_id":{"scope":scope_b,"seq":"1"},"args":{"controller":"1"}
        })),
    );
    assert_eq!(pause.len(), 2);
    assert_eq!(pause[0]["state"], "accepted");
    assert_eq!(pause[1]["state"], "completed");
    let duplicate = app.handle(
        &mut service,
        2,
        frame(json!({
            "v":1,"msg_id":"pause-retry","op":"controller_pause",
            "request_id":{"scope":scope_b,"seq":"1"},"args":{"controller":"1"}
        })),
    );
    assert_eq!(duplicate.len(), 1);
    assert_eq!(duplicate[0]["state"], "completed");
    service.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    let terminal = loop {
        if let Some(result) = service.shutdown_step().unwrap() {
            break result;
        }
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    };
    assert!(terminal.recorder_flushed);
    drop(app);
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut statement = db
        .prepare(
            "SELECT request_scope,request_seq,command,phase FROM operation_events
         WHERE command IN ('controller_start','controller_pause') ORDER BY record_seq",
        )
        .unwrap();
    let rows: Vec<(String, String, String, String)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            (
                scope_a.clone(),
                "1".into(),
                "controller_start".into(),
                "accepted".into()
            ),
            (
                scope_a,
                "1".into(),
                "controller_start".into(),
                "completed".into()
            ),
            (
                scope_b.clone(),
                "1".into(),
                "controller_pause".into(),
                "accepted".into()
            ),
            (
                scope_b,
                "1".into(),
                "controller_pause".into(),
                "completed".into()
            ),
        ]
    );
    drop(statement);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn annotation_completion_reports_pending_ingress_before_a_held_sqlite_commit() {
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
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("annotation", now)
        .unwrap();
    let ready_by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < ready_by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let initial = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let mut app = Application::new(service.boot_id()).unwrap();
    let scope = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"h","op":"hello","args":{"scope":null}
        })),
    )[0]["result"]["scope"]
        .as_str()
        .unwrap()
        .to_owned();
    for invalid_args in [
        json!({"name":"n".repeat(65),"data":{}}),
        json!({"name":"n","data":"\0".repeat(400)}),
        json!({"name":"n","data":vec![0;65]}),
        json!({"name":"n","data":{"a":{"b":{"c":{"d":1}}}}}),
    ] {
        let rejected = app.handle(
            &mut service,
            1,
            frame(json!({
                "v":1,"msg_id":"bad","op":"experiment_annotate",
                "request_id":{"scope":scope,"seq":"1"},"args":invalid_args
            })),
        );
        assert_eq!(rejected[0]["type"], "error");
        assert_eq!(rejected[0]["accepted"], false);
    }
    for injected in ["origin", "ack", "safe_evidence", "readback_verified"] {
        let mut args = json!({"name":"n","data":{}});
        args[injected] = json!(true);
        let decoded = decode_frame(
            &encode_frame(&json!({
                "v":1,"msg_id":"injected","op":"experiment_annotate",
                "request_id":{"scope":scope,"seq":"1"},"args":args
            }))
            .unwrap(),
        );
        assert!(
            decoded.is_err(),
            "untrusted {injected} field passed wire validation"
        );
    }
    let request = json!({"v":1,"msg_id":"a","op":"experiment_annotate",
        "request_id":{"scope":scope,"seq":"1"},
        "args":{"name":"checkpoint","data":{"temperature":42,"note":"operator"}}});
    let replies = app.handle(&mut service, 1, frame(request.clone()));
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "completed");
    assert_eq!(replies[1]["result"]["durability"], "pending");
    let reserved = replies[1]["result"]["record_seq"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(reserved > initial);
    let duplicate = app.handle(&mut service, 1, frame(request));
    assert_eq!(duplicate.len(), 1);
    assert_eq!(duplicate[0]["result"]["record_seq"], reserved.to_string());
    let held_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < held_by);
        std::thread::yield_now();
    }
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .persisted_through_sequence,
        initial
    );
    barrier.release();
    let database = service.owner().recording_database_id().unwrap().to_owned();
    let history = app.handle(
        &mut service,
        1,
        frame(json!({
            "v":1,"msg_id":"history","op":"history_read",
            "request_id":{"scope":scope,"seq":"2"},
            "args":{"mode":"runs","database_id":database,"max_records":8,"cursor":null}
        })),
    );
    assert_eq!(history[0]["state"], "accepted");
    let history_by = Instant::now() + Duration::from_secs(2);
    let history_result = loop {
        let result = app.poll_history(&mut service);
        if !result.is_empty() {
            break result;
        }
        assert!(Instant::now() < history_by);
        std::thread::yield_now();
    };
    assert_eq!(history_result[0].1["state"], "completed");
    service.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(3);
    let terminal = loop {
        if let Some(terminal) = service.shutdown_step().unwrap() {
            break terminal;
        }
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    };
    assert!(terminal.recorder_flushed);
    drop(app);
    drop(service);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let (kind, origin, payload): (String, String, Vec<u8>) = archive
        .query_row(
            "SELECT kind,origin,payload FROM records WHERE record_seq=?1",
            [reserved.to_be_bytes().to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(kind, "annotation");
    assert_eq!(origin, "local_client");
    let data: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(data["actor_scope"], scope);
    assert_eq!(data["name"], "checkpoint");
    assert_eq!(data["data"]["temperature"], 42);
    let history_audits: i64 = archive
        .query_row(
            "SELECT COUNT(*) FROM operation_events WHERE command='history_read'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        history_audits, 0,
        "historical selection must not audit itself recursively"
    );
    drop(archive);
    std::fs::remove_file(&path).unwrap();
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
    let start_at = service.clock().now();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        service.owner_mut().service(&FrozenClock(start_at)).unwrap();
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
    assert_eq!(stop.len(), 1, "stop={stop:?}");
    assert_eq!(stop[0]["state"], "accepted");
    let stop_at = service.clock().now();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut stop_terminal = Vec::new();
    while stop_terminal.is_empty() && Instant::now() < deadline {
        service.owner_mut().service(&FrozenClock(stop_at)).unwrap();
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
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.shutdown_step().unwrap().is_none() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(service.shutdown_step().unwrap().unwrap().recorder_flushed);
    drop(app);
    drop(service);
    let archive = rusqlite::Connection::open(&path).unwrap();
    let lifecycle: Vec<(String,String)> = archive.prepare(
        "SELECT command,phase FROM operation_events WHERE command IN ('recording_start','recording_stop') ORDER BY record_seq"
    ).unwrap().query_map([],|row|Ok((row.get(0)?,row.get(1)?))).unwrap()
        .map(|row|row.unwrap()).collect();
    assert_eq!(
        lifecycle,
        [
            ("recording_start".into(), "accepted".into()),
            ("recording_start".into(), "completed".into()),
            ("recording_stop".into(), "accepted".into()),
            ("recording_stop".into(), "completed".into()),
        ]
    );
    let boundary: Vec<u8> = archive
        .query_row(
            "SELECT payload FROM records WHERE kind='boundary_snapshot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let boundary: serde_json::Value = serde_json::from_slice(&boundary).unwrap();
    assert_eq!(boundary["pending_operations"][0]["scope"], scope);
    assert_eq!(boundary["pending_operations"][0]["request_seq"], "1");
    let stop_seal: Vec<u8> = archive
        .query_row(
            "SELECT payload FROM records WHERE kind='interval_seal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let stop_seal: serde_json::Value = serde_json::from_slice(&stop_seal).unwrap();
    assert_eq!(stop_seal["pending_operations"][0]["scope"], scope);
    assert_eq!(stop_seal["pending_operations"][0]["request_seq"], "2");
    assert_eq!(stop_seal["coverage"], "complete");
    assert!(stop_seal["accepted_prefix_through_seq"].as_str().is_some());
    drop(archive);
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
