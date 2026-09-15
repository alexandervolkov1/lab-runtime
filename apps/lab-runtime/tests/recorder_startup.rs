//! Recorder configuration is selected before readiness and never starts control.

use lab_runtime::{
    recorder::{RecordingPolicy, RecordingState},
    service::{ServiceHost, ServiceOptions},
};
use std::path::PathBuf;

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
