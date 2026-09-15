//! Recorder configuration is selected before readiness and never starts control.

use lab_runtime::{
    application::Application,
    host::Clock,
    recorder::{RecordingPolicy, RecordingState},
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-startup-{suffix}.sqlite"))
}

#[test]
fn database_option_defaults_to_required_and_invalid_candidates_do_not_start() {
    let path = temporary_database();
    let text = path.to_string_lossy();
    let args = [
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ];
    let options = ServiceOptions::parse(&args).unwrap();
    assert_eq!(
        options.recording().unwrap().policy,
        RecordingPolicy::Required
    );
    assert!(
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--record-policy",
            "best-effort"
        ])
        .is_err()
    );
    assert!(
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--record-db",
            "relative.sqlite"
        ])
        .is_err()
    );
    assert!(
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--record-db",
            text.as_ref(),
            "--record-policy",
            "unknown"
        ])
        .is_err()
    );
    let service = ServiceHost::startup(options).unwrap();
    let status = service.owner().recording_status().unwrap();
    assert_eq!(status.state, RecordingState::Idle);
    drop(service);
    // Detached startup worker releases its connection after the owner drops.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn public_recording_status_reports_cached_limits_quota_and_actual_wal_health() {
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
    let at = service.clock().now();
    service
        .owner_mut()
        .start_recording("status health", at)
        .unwrap();
    let by = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < by);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = decode_frame(
        &encode_frame(&json!({
            "v":1,"msg_id":"health-hello","op":"hello","args":{"scope":null}
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(app.handle(&mut service, 1, hello)[0]["type"], "result");
    let frame = decode_frame(
        &encode_frame(&json!({
            "v":1,"msg_id":"storage-health","op":"recording_status","args":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let response = app.handle(&mut service, 1, frame);
    assert_eq!(response[0]["type"], "result");
    let health = &response[0]["result"];
    assert_eq!(health["limits"]["records"], 1024);
    assert_eq!(health["limits"]["groups"], 4);
    assert_eq!(health["limits"]["bytes"], 4 * 1024 * 1024);
    assert_eq!(
        health["main_quota_bytes"],
        (1024u64 * 1024 * 1024).to_string()
    );
    assert_eq!(
        health["wal_threshold_bytes"],
        (16u64 * 1024 * 1024).to_string()
    );
    let reported = health["wal_bytes"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let sidecar = std::fs::metadata(path.with_extension("sqlite-wal"))
        .unwrap()
        .len();
    assert!(reported > 0);
    assert_eq!(
        reported, sidecar,
        "status must report actual worker-observed WAL bytes"
    );
    assert!(
        health["main_logical_bytes"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    service.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(app);
    drop(service);
    std::fs::remove_file(path).unwrap();
}
