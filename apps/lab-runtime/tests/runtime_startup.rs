//! Startup validation precedes readiness and leaves native control safely Ready.

use lab_core::{
    Query, QueryResult,
    control::ControllerState,
    output::{ActuatorId, OutputState},
};
use lab_runtime::host::HostCore;
use lab_runtime::service::{ServiceHost, ServiceOptions};

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
fn service_cli_rejects_unsafe_bind_or_unknown_profile_before_runtime_activation() {
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
    ] {
        assert!(ServiceOptions::parse(&args).is_err());
    }
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
fn occupied_loopback_bind_unwinds_real_lua_startup_without_publishing_readiness() {
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
