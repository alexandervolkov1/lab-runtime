//! Public manual-power API: scope ownership, validation, deduplication and expiry.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::time::Duration;

#[path = "support/manual_output_physical.rs"]
mod physical;

struct Fixture {
    service: ServiceHost,
    app: Application,
    scope: String,
    seq: u64,
}
impl Fixture {
    fn new() -> Self {
        let mut service = ServiceHost::startup(
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap(),
        )
        .unwrap();
        let mut app = Application::new(service.boot_id()).unwrap();
        let hello = ask(
            &mut service,
            &mut app,
            1,
            json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
        );
        assert!(
            hello[0]["result"]["operations"]
                .as_array()
                .unwrap()
                .contains(&json!("output_write"))
        );
        Self {
            service,
            app,
            scope: hello[0]["result"]["scope"].as_str().unwrap().into(),
            seq: 1,
        }
    }
    fn query(&mut self, op: &str, args: Value) -> Value {
        ask(
            &mut self.service,
            &mut self.app,
            1,
            json!({"v":1,"msg_id":"q","op":op,"args":args}),
        )
        .remove(0)
    }
    fn output(&mut self) -> Value {
        self.query(
            "output",
            json!({"actuator":{"instrument":"1","parameter":"2"}}),
        )["result"]
            .clone()
    }
    fn args(&mut self) -> Value {
        let state = self.output();
        json!({"actuator":{"instrument":"1","parameter":"2"},"expected_instance":state["authority_instance"],"expected_epoch":state["epoch"]})
    }
    fn operation(&mut self, op: &str, args: Value) -> Vec<Value> {
        let seq = self.seq;
        self.seq += 1;
        ask(
            &mut self.service,
            &mut self.app,
            1,
            json!({"v":1,"msg_id":"m","op":op,"args":args,"request_id":{"scope":self.scope,"seq":seq.to_string()}}),
        )
    }
    fn acquire(&mut self) {
        let mut args = self.args();
        args["lease_lifetime_ns"] = json!("2000000000");
        let result = self.operation("output_acquire", args);
        assert_eq!(result[1]["state"], "completed", "{result:?}");
    }
}
fn ask(
    service: &mut ServiceHost,
    app: &mut Application,
    connection: u64,
    value: Value,
) -> Vec<Value> {
    app.handle(
        service,
        connection,
        decode_frame(&encode_frame(&value).unwrap()).unwrap(),
    )
}

#[test]
fn explicit_power_is_not_pid_reference_and_exact_retry_does_not_send_again() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.output()["manual_supported"], true);
    assert_eq!(fixture.output()["profile"]["safe_value"], 0.0);
    let reference = fixture.query("reference", json!({"reference":"1"}))["result"].clone();
    fixture.acquire();
    let mut args = fixture.args();
    args["value"] = json!(37.5);
    args["proposal_ttl_ns"] = json!("200000000");
    let seq = fixture.seq;
    let result = fixture.operation("output_write", args.clone());
    assert_eq!(result[0]["state"], "accepted");
    assert_eq!(result[1]["state"], "completed", "{result:?}");
    assert_eq!(result[1]["result"]["readback"]["value"], 37.5);
    let receipt = fixture.output()["manual_attempt"].clone();
    let retry = ask(
        &mut fixture.service,
        &mut fixture.app,
        1,
        json!({"v":1,"msg_id":"retry","op":"output_write","args":args,"request_id":{"scope":fixture.scope,"seq":seq.to_string()}}),
    );
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0]["result"], result[1]["result"]);
    assert_eq!(fixture.output()["manual_attempt"], receipt);
    assert_eq!(
        fixture.query("reference", json!({"reference":"1"}))["result"],
        reference
    );
    let args = fixture.args();
    let safe = fixture.operation("output_safe", args);
    assert_eq!(
        safe[1]["result"]["completion_basis"],
        "authority_revoked_safe_requested"
    );
    assert_eq!(safe[1]["result"]["output"]["safe_confirmed"], false);
    fixture
        .service
        .owner_mut()
        .command(lab_core::Command::ServiceSafety {
            at: Duration::from_secs(3),
        })
        .unwrap();
    assert_eq!(fixture.output()["safe_confirmed"], true);
    assert_eq!(fixture.output()["readback"]["value"], 0.0);
    assert_eq!(fixture.output()["manual_attempt"], receipt);
}

#[test]
fn invalid_values_unavailable_parameter_and_stale_fences_do_not_dispatch() {
    let mut fixture = Fixture::new();
    fixture.acquire();
    let before = fixture.output();
    let mut args = fixture.args();
    args["value"] = json!(101.0);
    args["proposal_ttl_ns"] = json!("100000000");
    let bad_range = fixture.operation("output_write", args.clone());
    assert_eq!(bad_range[1]["state"], "failed");
    assert_eq!(fixture.output(), before);
    args["expected_instance"] = json!("0");
    args["value"] = json!(20.0);
    assert_eq!(
        fixture.operation("output_write", args.clone())[1]["state"],
        "failed"
    );
    args["expected_instance"] = before["authority_instance"].clone();
    args["expected_epoch"] = json!("0");
    assert_eq!(
        fixture.operation("output_write", args.clone())[1]["state"],
        "failed"
    );
    args["expected_epoch"] = before["epoch"].clone();
    args["actuator"]["parameter"] = json!("1");
    assert_eq!(
        fixture.operation("output_write", args.clone())[1]["code"],
        "unknown_actuator"
    );
    args["actuator"]["parameter"] = json!("2");
    args["value"] = json!("20");
    let invalid = fixture.operation("output_write", args);
    assert_eq!(invalid[0]["accepted"], false);
    assert_eq!(invalid[0]["code"], "invalid_args");
    assert_eq!(fixture.output(), before);
}

#[test]
fn another_scope_and_pid_cannot_take_manual_output_and_disconnect_does_not_renew() {
    let mut fixture = Fixture::new();
    fixture.acquire();
    let second = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"h2","op":"hello","args":{"scope":null}}),
    )[0]["result"]["scope"]
        .clone();
    let args = fixture.args();
    let conflict = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"foreign","op":"output_safe","args":args,"request_id":{"scope":second,"seq":"1"}}),
    );
    assert_eq!(conflict[1]["code"], "busy");
    let start = fixture.operation("controller_start", json!({"controller":"1"}));
    assert_eq!(start[1]["state"], "failed");
    let before = fixture.output();
    fixture.app.detach(&fixture.service, 1);
    fixture
        .service
        .owner_mut()
        .command(lab_core::Command::ServiceSafety {
            at: Duration::from_secs(3),
        })
        .unwrap();
    let hello = ask(
        &mut fixture.service,
        &mut fixture.app,
        1,
        json!({"v":1,"msg_id":"reattach","op":"hello","args":{"scope":fixture.scope}}),
    );
    assert_eq!(hello[0]["type"], "result");
    let after = fixture.output();
    assert!(after["owner"].is_null());
    assert_eq!(after["safe_confirmed"], true);
    assert_ne!(after["epoch"], before["epoch"]);
}

#[test]
fn renewal_is_explicit_finite_and_idle_queries_do_not_extend_authority() {
    let mut fixture = Fixture::new();
    fixture.acquire();
    let original = fixture.output();
    let mut args = fixture.args();
    args["lease_lifetime_ns"] = json!("3000000000");
    assert_eq!(
        fixture.operation("output_renew", args.clone())[1]["state"],
        "failed"
    );
    assert_eq!(
        fixture.output()["lease_expires_at"],
        original["lease_expires_at"]
    );
    args["lease_lifetime_ns"] = json!("2000000000");
    assert_eq!(
        fixture.operation("output_renew", args)[1]["state"],
        "completed"
    );
    let renewed = fixture.output();
    assert!(
        renewed["lease_expires_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > original["lease_expires_at"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
    );
    assert_eq!(
        fixture.output()["lease_expires_at"],
        renewed["lease_expires_at"]
    );
    fixture
        .service
        .owner_mut()
        .command(lab_core::Command::ServiceSafety {
            at: Duration::from_secs(3),
        })
        .unwrap();
    assert!(fixture.output()["owner"].is_null());
}
