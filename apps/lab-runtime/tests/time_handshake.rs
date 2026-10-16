//! Read-only calendar metadata is boot-stable even with Recorder disabled.
use lab_runtime::{
    application::Application,
    host::{Clock, SystemClock},
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};

fn setup() -> (ServiceHost, Application) {
    let service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let app = Application::new(service.boot_id()).unwrap();
    (service, app)
}
fn hello(service: &mut ServiceHost, app: &mut Application, connection: u64) -> Value {
    let request = decode_frame(
        &encode_frame(&json!({"v":1,"msg_id":"h","op":"hello",
        "args":{"scope":null}}))
        .unwrap(),
    )
    .unwrap();
    let result = app.handle(service, connection, request).remove(0);
    assert_eq!(result["type"], "result");
    assert!(encode_frame(&result).unwrap().len() <= 16_384);
    result["result"].clone()
}
#[test]
fn handshake_anchor_does_not_depend_on_recording_and_changes_only_with_boot() {
    let (mut service, mut app) = setup();
    assert!(service.owner().recording_status().is_none());
    let first = hello(&mut service, &mut app, 1);
    let second = hello(&mut service, &mut app, 2);
    assert_eq!(first["time_anchor"], second["time_anchor"]);
    assert_eq!(first["boot_id"], first["time_anchor"]["boot_id"]);
    assert_eq!(first["time_anchor"]["source"], "system_clock");
    assert!(first["time_anchor"]["utc_accuracy_ns"].is_null());
    assert_eq!(first["next_seq"], "1");
    let anchor = service.clock().time_anchor().unwrap();
    assert!(anchor.after() <= service.clock().now());
    assert_eq!(
        anchor.wall_us().unwrap().to_string(),
        first["time_anchor"]["utc_unix_us"]
    );
    drop(app);
    drop(service);
    let (mut reboot, mut other) = setup();
    let third = hello(&mut reboot, &mut other, 1);
    assert_ne!(first["boot_id"], third["boot_id"]);
    assert_eq!(third["boot_id"], third["time_anchor"]["boot_id"]);
}
#[test]
fn clock_copies_share_the_measurement_origin_and_fixed_anchor() {
    let clock = SystemClock::new();
    let copy = clock;
    assert_eq!(clock.time_anchor(), copy.time_anchor());
    let anchor = clock.time_anchor().unwrap();
    assert_eq!(
        anchor.estimate_us(anchor.after()).unwrap(),
        anchor.wall_us().unwrap()
    );
    assert_eq!(
        anchor
            .estimate_us(anchor.after() + std::time::Duration::from_secs(763))
            .unwrap(),
        anchor.wall_us().unwrap() + 763_000_000
    );
    assert!(copy.now() >= anchor.after());
}
