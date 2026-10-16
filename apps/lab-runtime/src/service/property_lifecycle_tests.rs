//! Property activation failures retain the Recorder reservation/receipt fences.

use super::*;
use crate::{
    application::Application,
    recorder::{RecordingState, WriterBarrier},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::time::Duration;

fn fixture(
    limits: RecorderLimits,
    barrier: Option<WriterBarrier>,
    fail_commit: bool,
) -> (ServiceHost, PathBuf) {
    let directory = crate::test_support::temporary_database("property-fence").with_extension("");
    std::fs::create_dir(&directory).unwrap();
    let config = directory.join("runtime.virtual.toml");
    std::fs::write(
        &config,
        include_str!("../../../../examples/runtime.virtual.toml"),
    )
    .unwrap();
    let deployment = load_runtime_toml(&config).unwrap();
    let mut host = HostCore::configured_native(&deployment).unwrap();
    let database = directory.join("history.sqlite");
    if fail_commit {
        drop(crate::recorder::SqliteStore::open(&database).unwrap());
        let db = rusqlite::Connection::open(&database).unwrap();
        // A deferred foreign key fails COMMIT after every activation INSERT ran.
        // Install before the worker takes its exclusive SQLite ownership.
        db.execute_batch("CREATE TABLE property_gate(id INTEGER PRIMARY KEY);
            CREATE TABLE property_fail(gate_id INTEGER REFERENCES property_gate(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER property_commit_fail AFTER INSERT ON records
            WHEN NEW.kind='configuration_lifecycle'
            BEGIN INSERT INTO property_fail(gate_id) VALUES(999); END;").unwrap();
    }
    let worker = match barrier {
        Some(barrier) => RecorderWorker::open_with_barrier(&database, limits, barrier),
        None => RecorderWorker::open(&database, limits),
    }
    .unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    // Trusted fixture composition only; production startup owns this same lifecycle.
    service.deployment = Some(DeploymentLifecycle::new(deployment));
    (service, database)
}

fn application_property(service: &mut ServiceHost) -> Vec<Value> {
    let mut app = Application::new(service.boot_id()).unwrap();
    let request = |value| decode_frame(&encode_frame(&value).unwrap()).unwrap();
    let hello = app.handle(
        service,
        1,
        request(json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}})),
    );
    app.handle(service, 1, request(json!({"v":1,"msg_id":"property","op":"property_configure",
        "request_id":{"scope":hello[0]["result"]["scope"],"seq":"1"},
        "args":{"target":{"kind":"instrument","id":"1"},"property":"poll_period_ms","value":200,"expected_revision":"1"}})))
}

fn start_recording(service: &mut ServiceHost) {
    let clock = service.clock_copy();
    service
        .owner_mut()
        .start_recording("property receipt fence", clock.now())
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(std::time::Instant::now() < deadline);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
}

fn assert_required_admission_closed(service: &mut ServiceHost) {
    let controller = service.owner().controller_id();
    let at = service.clock_copy().now();
    assert_eq!(
        service
            .owner_mut()
            .command(lab_core::Command::StartController { controller, at }),
        Err(lab_core::Error::RecordingUnavailable),
        "a failed activation must keep Required control admission closed"
    );
}

#[test]
fn property_reservation_exhaustion_never_commits_configuration() {
    // A valid trusted limit too small for the lifecycle charge. The existing
    // bounded reservation loop must fail before owner commit, without bypass.
    let (mut service, database) = fixture(
        RecorderLimits {
            bytes: 256,
            ..RecorderLimits::default()
        },
        None,
        false,
    );
    let replies = application_property(&mut service);
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "failed");
    assert_eq!(replies[1]["code"], "operation_failed");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 1);
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .activation_generation,
        1
    );
    let properties = crate::configuration_api::property_records(&service).unwrap();
    assert_eq!(
        properties
            .iter()
            .find(|r| r["property"] == "poll_period_ms")
            .unwrap()["current"],
        100
    );
    drop(service);
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM configurations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
}

#[test]
fn property_sql_commit_failure_is_failed_without_a_durable_generation_or_rearm() {
    let (mut service, database) = fixture(RecorderLimits::default(), None, true);
    start_recording(&mut service);
    let replies = application_property(&mut service);
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "failed", "{replies:?}");
    assert_eq!(replies[1]["code"], "operation_failed");
    // Owner commit preceded the failed durable activation; it is never reported
    // as a Completed mutation or inferred to have rolled back from SQL failure.
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);
    let status = service.owner().recording_status().unwrap();
    assert_eq!(status.state, RecordingState::Failed);
    assert_eq!(status.activation_generation, 1);
    assert!(
        status.outstanding_records > 0,
        "unconfirmed credit stays charged"
    );
    assert_required_admission_closed(&mut service);
    drop(service);
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM configurations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
}

#[test]
fn property_queued_activation_without_receipt_never_returns_completed() {
    let barrier = WriterBarrier::held();
    struct Release(WriterBarrier);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    let _release = Release(barrier.clone());
    let (mut service, _) = fixture(RecorderLimits::default(), Some(barrier.clone()), false);
    start_recording(&mut service);
    // Earlier facts/audit occupy the existing writer gate ahead of Activation.
    // This makes reservation/owner commit succeed but withholds the SQL receipt.
    let replies = application_property(&mut service);
    assert!(barrier.reached());
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "failed");
    assert_eq!(replies[1]["code"], "operation_failed");
    assert_eq!(service.loaded_configuration().unwrap().revision(), 2);
    let status = service.owner().recording_status().unwrap();
    assert_eq!(status.activation_generation, 1);
    assert!(status.outstanding_groups > 0);
    assert_required_admission_closed(&mut service);
    barrier.release();
    // A late SQL receipt cannot turn the cached Failed outcome into Completed,
    // or reopen Required admission after the operation's bounded wait expired.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let now = service.clock_copy().now();
        service.owner_mut().poll_recorder_for_test(now);
        if service
            .owner()
            .recording_status()
            .unwrap()
            .activation_generation
            == 2
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_required_admission_closed(&mut service);
    drop(service);
}
