//! Startup validation precedes readiness and leaves native control safely Ready.

use lab_core::{
    Query, QueryResult,
    control::ControllerState,
    output::{ActuatorId, OutputState},
};
use lab_runtime::host::HostCore;

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
