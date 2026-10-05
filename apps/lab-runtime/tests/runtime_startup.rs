//! Startup validation precedes readiness and leaves native control safely Ready.

use lab_core::{
    Query, QueryResult,
    control::ControllerState,
    output::{ActuatorId, OutputState},
};
use lab_runtime::host::HostCore;
use lab_runtime::service::{ServiceHost, ServiceOptions};

#[test]
fn documented_runtime_cli_forms_are_accepted_by_the_real_parser() {
    assert!(ServiceOptions::parse(&["--serve", "--config", "runtime.toml"]).is_ok());
    let recorder = std::env::temp_dir().join("runtime-cli-form.sqlite");
    let recorder = recorder.to_str().unwrap();
    assert!(
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--bind",
            "192.168.1.50",
            "--port",
            "0",
            "--record-db",
            recorder,
            "--record-policy",
            "best-effort",
            "--ws-port",
            "0",
            "--ws-origin",
            "http://127.0.0.1:3000",
        ])
        .is_ok()
    );
}

#[test]
fn profile_bind_defaults_to_ipv4_loopback_and_preserves_port_parsing() {
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "8765"]).unwrap();
    assert_eq!(options.bind_address(), std::net::Ipv4Addr::LOCALHOST);
    assert_eq!(options.port(), 8765);
}

#[test]
fn profile_accepts_explicit_numeric_ipv4_bind_and_rejects_invalid_bind() {
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--bind",
        "192.168.1.50",
        "--port",
        "8765",
    ])
    .unwrap();
    assert_eq!(options.bind_address().to_string(), "192.168.1.50");
    assert_eq!(options.port(), 8765);

    for bind in ["localhost", "192.168.1.999", "::1"] {
        assert!(
            ServiceOptions::parse(&[
                "--serve",
                "--profile",
                "virtual-demo",
                "--bind",
                bind,
                "--port",
                "8765",
            ])
            .is_err(),
            "invalid IPv4 bind was accepted: {bind}"
        );
    }
}

#[test]
fn startup_uses_the_explicit_tcp_bind_without_expanding_websocket() {
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--bind",
        "0.0.0.0",
        "--port",
        "0",
        "--ws-port",
        "0",
        "--ws-origin",
        "http://127.0.0.1:3000",
    ])
    .unwrap();
    let service = ServiceHost::startup(options).unwrap();
    assert_eq!(service.bound_address().ip().to_string(), "0.0.0.0");
    assert_eq!(
        service.websocket_bound_address().unwrap().ip().to_string(),
        "127.0.0.1"
    );
}

#[test]
fn trusted_virtual_profile_is_safe_and_ready_without_auto_start() {
    let host = HostCore::virtual_demo().unwrap();
    let QueryResult::Controller(controller) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!()
    };
    assert_eq!(controller.state, ControllerState::Ready);
    assert!(controller.lease.is_none());
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::Disarmed);
    assert!(output.safe_confirmed);
    assert_eq!(output.readback.unwrap().value, 0.0);
}

#[test]
fn service_cli_rejects_unknown_options_or_profile_before_runtime_activation() {
    assert!(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).is_ok()
    );
    for args in [
        vec!["--serve", "--profile", "unknown", "--port", "0"],
        vec!["--serve", "--profile", "virtual-demo", "--port", "bad"],
        vec![
            "--serve",
            "--profile",
            "virtual-demo",
            "--listen",
            "0.0.0.0",
        ],
        vec!["--serve", "--profile", "virtual-demo", "--port"],
        vec![
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--ws-port",
            "0",
        ],
        vec![
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--ws-origin",
            "http://127.0.0.1:3000",
        ],
        vec![
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--ws-port",
            "0",
            "--ws-origin",
            "*",
        ],
    ] {
        assert!(ServiceOptions::parse(&args).is_err());
    }
}

#[test]
fn optional_websocket_readiness_is_additive_and_loopback_only() {
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--ws-port",
        "0",
        "--ws-origin",
        "http://127.0.0.1:3000",
    ])
    .unwrap();
    let service = ServiceHost::startup(options).unwrap();
    let websocket = service.websocket_bound_address().unwrap();
    assert_eq!(websocket.ip().to_string(), "127.0.0.1");
    assert_ne!(websocket.port(), 0);
    let ready: serde_json::Value = serde_json::from_str(&service.ready_line()).unwrap();
    assert_eq!(ready["port"], service.bound_address().port());
    assert_eq!(ready["websocket"]["port"], websocket.port());
    assert_eq!(ready["websocket"]["path"], "/application/v1");
    assert_eq!(
        ready["websocket"]["subprotocol"],
        "lab-runtime.application.v1"
    );
}

#[test]
fn startup_binds_ephemeral_loopback_only_after_safe_ready_profile_and_has_new_boot_id() {
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let service = ServiceHost::startup(options).unwrap();
    assert_eq!(service.bound_address().ip().to_string(), "127.0.0.1");
    assert_ne!(service.bound_address().port(), 0);
    assert_eq!(service.boot_id().len(), 32);
    assert!(
        service
            .boot_id()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    let ready: serde_json::Value = serde_json::from_str(&service.ready_line()).unwrap();
    assert_eq!(ready["boot_id"], service.boot_id());
    assert_eq!(ready["port"], service.bound_address().port());
    assert_eq!(ready["state"], "ready");
    let prior_boot = service.boot_id().to_string();
    drop(service);
    let next = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    assert_ne!(next.boot_id(), prior_boot, "restart identity must change");
}

#[test]
fn occupied_loopback_bind_unwinds_native_startup_without_publishing_readiness() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    let text = port.to_string();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", &text]).unwrap();
    assert!(ServiceHost::startup(options).is_err());
    drop(occupied);
    let retry =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", &text]).unwrap();
    let service = ServiceHost::startup(retry).unwrap();
    assert_eq!(service.bound_address().port(), port);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&service.ready_line()).unwrap()["state"],
        "ready"
    );
}
