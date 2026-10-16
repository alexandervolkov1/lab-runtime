//! M9B.4 Reference and native-controller Application API contracts.

use lab_runtime::{
    application::Application,
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};

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
        Self {
            scope: hello[0]["result"]["scope"].as_str().unwrap().to_owned(),
            service,
            app,
            seq: 1,
        }
    }

    fn query(&mut self, op: &str, args: Value) -> Value {
        ask(
            &mut self.service,
            &mut self.app,
            1,
            json!({"v":1,"msg_id":format!("q-{}",self.seq),"op":op,"args":args}),
        )
        .remove(0)
    }

    fn operation(&mut self, op: &str, args: Value) -> Vec<Value> {
        let seq = self.seq;
        self.seq += 1;
        ask(
            &mut self.service,
            &mut self.app,
            1,
            json!({"v":1,"msg_id":format!("m-{seq}"),"op":op,
                "request_id":{"scope":self.scope,"seq":seq.to_string()},"args":args}),
        )
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
fn reference_identity_projection_configuration_and_event_are_coherent() {
    let mut fixture = Fixture::new();
    let discovery = fixture.query("discover", json!({}));
    let discovered = discovery["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["kind"] == "reference")
        .unwrap()["id"]["id"]
        .clone();
    let before = fixture.query("reference", json!({"reference":discovered}));
    assert_eq!(before["result"]["reference"], discovered);
    assert_eq!(before["result"]["status"], "valid");
    assert_eq!(before["result"]["configurable"], true);
    assert!(before["result"]["unit"]["id"].is_string());
    assert!(before["result"]["revision"].is_string());

    let cursor = json!({"boot_id":fixture.service.boot_id(),
        "seq":fixture.service.owner().event_log().latest_cursor().to_string()});
    let subscription = fixture.query(
        "subscribe",
        json!({"after":cursor,"filter":{"kinds":["reference"],"targets":[]}}),
    );
    assert_eq!(subscription["type"], "result");
    let revision = before["result"]["revision"].clone();
    let changed = fixture.operation(
        "reference_configure",
        json!({"reference":discovered,"expected_revision":revision,
            "kind":"fixed","value":42.5}),
    );
    assert_eq!(changed[0]["state"], "accepted");
    assert_eq!(changed[1]["state"], "completed");
    assert_eq!(changed[1]["result"]["revision"], "2");
    assert_eq!(changed[1]["result"]["value"], 42.5);
    let events = fixture.app.pump_events(&fixture.service, 1);
    let event = events
        .iter()
        .find(|event| event["kind"] == "reference")
        .unwrap();
    assert_eq!(event["target"]["id"], discovered);
    assert_eq!(event["data"]["reference"], discovered);
    assert_eq!(event["data"]["revision"], "2");
}

#[test]
fn invalid_reference_and_revision_conflict_are_atomic_structured_failures() {
    let mut fixture = Fixture::new();
    let before = fixture.query("reference", json!({"reference":"1"}))["result"].clone();
    let invalid = fixture.operation(
        "reference_configure",
        json!({"reference":"1","expected_revision":before["revision"],
            "kind":"ramp","target":80.0,"rate":-1.0}),
    );
    assert_eq!(invalid[1]["category"], "invalid_configuration");
    assert_eq!(
        fixture.query("reference", json!({"reference":"1"}))["result"],
        before
    );

    let conflict = fixture.operation(
        "reference_configure",
        json!({"reference":"1","expected_revision":"0",
            "kind":"fixed","value":33.0}),
    );
    assert_eq!(conflict[1]["code"], "revision_conflict");
    assert_eq!(conflict[1]["resync_required"], true);
    assert_eq!(
        fixture.query("reference", json!({"reference":"1"}))["result"],
        before
    );
}

#[test]
fn transitional_ramp_retune_keeps_its_pre_m9c_result_contract() {
    let mut fixture = Fixture::new();
    let before = fixture.query("reference", json!({"reference":"1"}));
    let retuned = fixture.operation(
        "reference_retune",
        json!({"reference":"1","expected_revision":before["result"]["revision"],
            "target":55.0,"rate":2.0}),
    );
    assert_eq!(retuned[1]["state"], "completed", "{retuned:?}");
    assert!(
        retuned[1]["result"]["committed_at"].is_string(),
        "{retuned:?}"
    );
}

#[test]
fn controller_projection_exposes_read_only_bindings_and_full_configuration() {
    let mut fixture = Fixture::new();
    let discovery = fixture.query("discover", json!({}));
    let id = discovery["result"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["kind"] == "controller")
        .unwrap()["id"]["id"]
        .clone();
    let controller = fixture.query("controller", json!({"controller":id}));
    let state = &controller["result"];
    assert_eq!(state["controller"], id);
    assert_eq!(state["kind"], "pid");
    assert_eq!(state["status"], "valid");
    assert_eq!(state["active"], false);
    assert!(state["bindings"]["input"]["instrument"].is_string());
    assert!(state["bindings"]["reference"]["id"].is_string());
    assert!(state["bindings"]["output"]["parameter"].is_string());
    assert!(state["config"]["pid"]["kp"].is_number());
    assert!(state["config"]["ema"]["unit"]["id"].is_string());
    assert!(state.get("lease").is_none());
}

#[test]
fn full_controller_configuration_round_trips_and_failures_are_atomic() {
    let mut fixture = Fixture::new();
    let before = fixture.query("controller", json!({"controller":"1"}))["result"].clone();
    let configured = fixture.operation(
        "controller_configure",
        json!({"controller":"1","expected_revision":before["revision"],
            "pid":{"kp":4.0,"ki":0.5,"kd":0.25,"output_min":0.0,"output_max":90.0},
            "ema":{"time_constant_ns":"300000000","warmup_samples":"2"},
            "max_input_age_ns":"600000000","max_tick_gap_ns":"600000000",
            "lease_lifetime_ns":"1000000000","proposal_ttl_ns":"100000000"}),
    );
    assert_eq!(configured[1]["state"], "completed");
    let after = fixture.query("controller", json!({"controller":"1"}))["result"].clone();
    assert_eq!(after["revision"], "2");
    assert_eq!(after["config"]["pid"]["kp"], 4.0);
    assert_eq!(after["config"]["pid"]["output_max"], 90.0);
    assert_eq!(after["config"]["ema"]["warmup_samples"], "2");
    assert_eq!(after["bindings"], before["bindings"]);

    let invalid = fixture.operation(
        "controller_configure_pid",
        json!({"controller":"1","expected_revision":"2",
            "pid":{"kp":4.0,"ki":0.5,"kd":0.25,"output_min":90.0,"output_max":0.0}}),
    );
    assert_eq!(invalid[1]["category"], "invalid_configuration");
    assert_eq!(
        fixture.query("controller", json!({"controller":"1"}))["result"],
        after
    );
    let conflict = fixture.operation(
        "controller_configure_pid",
        json!({"controller":"1","expected_revision":"1",
            "pid":{"kp":1.0,"ki":0.0,"kd":0.0,"output_min":0.0,"output_max":80.0}}),
    );
    assert_eq!(conflict[1]["code"], "revision_conflict");
}

#[test]
fn lifecycle_uses_runtime_safe_path_and_disconnect_does_not_undo_transition() {
    let mut fixture = Fixture::new();
    let cursor = json!({"boot_id":fixture.service.boot_id(),
        "seq":fixture.service.owner().event_log().latest_cursor().to_string()});
    let subscribed = fixture.query(
        "subscribe",
        json!({"after":cursor,"filter":{"kinds":["controller"],"targets":[]}}),
    );
    assert_eq!(subscribed["type"], "result");
    // Controller start validates input freshness against the authoritative
    // service clock. Refresh due native work immediately before the mutation so
    // workspace scheduling delay cannot age the fixture's startup sample.
    let clock = fixture.service.clock_copy();
    fixture.service.owner_mut().service(&clock).unwrap();
    let started = fixture.operation("controller_start", json!({"controller":"1"}));
    assert_eq!(started[1]["state"], "completed", "{started:?}");
    assert!(matches!(
        started[1]["result"]["state"].as_str(),
        Some("warming" | "running")
    ));
    let events = fixture.app.pump_events(&fixture.service, 1);
    let event = events
        .iter()
        .find(|event| event["kind"] == "controller")
        .unwrap();
    assert_eq!(event["target"]["id"], "1");
    assert_eq!(event["data"]["controller"], "1");
    assert_eq!(event["data"]["bindings"]["reference"]["id"], "1");
    fixture.app.detach(&fixture.service, 1);

    let hello = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"resume-scope","op":"hello","args":{"scope":fixture.scope}}),
    );
    assert_eq!(hello[0]["type"], "result");
    let current = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"current","op":"controller","args":{"controller":"1"}}),
    );
    assert_ne!(current[0]["result"]["state"], "ready");
    let pause = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"pause","op":"controller_pause",
            "request_id":{"scope":fixture.scope,"seq":fixture.seq.to_string()},
            "args":{"controller":"1"}}),
    );
    fixture.seq += 1;
    assert_eq!(pause[1]["result"]["state"], "paused");
    assert_eq!(pause[1]["result"]["active"], false);
    let invalid_pause = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"pause-again","op":"controller_pause",
            "request_id":{"scope":fixture.scope,"seq":fixture.seq.to_string()},
            "args":{"controller":"1"}}),
    );
    fixture.seq += 1;
    assert_eq!(invalid_pause[1]["code"], "invalid_state");
    let resumed = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"resume","op":"controller_resume",
            "request_id":{"scope":fixture.scope,"seq":fixture.seq.to_string()},
            "args":{"controller":"1"}}),
    );
    assert_eq!(resumed[1]["state"], "completed");
    assert!(resumed[1]["result"].get("lease").is_none());
}

#[test]
fn failed_reset_is_explicit_and_rejects_nonfailed_state() {
    let mut fixture = Fixture::new();
    let reset = fixture.operation("controller_reset_failed", json!({"controller":"1"}));
    assert_eq!(reset[0]["state"], "accepted");
    assert_eq!(reset[1]["code"], "invalid_state");
    assert_eq!(
        fixture.query("controller", json!({"controller":"1"}))["result"]["state"],
        "ready"
    );
}

#[test]
fn capability_and_wire_schema_expose_no_output_authority_path() {
    let mut fixture = Fixture::new();
    fixture.app.detach(&fixture.service, 1);
    let hello = ask(
        &mut fixture.service,
        &mut fixture.app,
        2,
        json!({"v":1,"msg_id":"hello2","op":"hello","args":{"scope":null}}),
    );
    let capabilities = hello[0]["result"]["capabilities"].as_array().unwrap();
    for name in [
        "reference_read_write",
        "controller_status",
        "controller_configuration",
        "controller_lifecycle",
    ] {
        assert!(
            capabilities
                .iter()
                .any(|capability| capability["name"] == name)
        );
    }
    let operations = hello[0]["result"]["operations"].as_array().unwrap();
    assert!(!operations.iter().any(|operation| operation == "set_output"));
    assert_eq!(hello[0]["result"]["limits"]["controller_result_records"], 1);
    let forged = json!({"v":1,"msg_id":"forged","op":"controller_start",
        "request_id":{"scope":hello[0]["result"]["scope"],"seq":"1"},
        "args":{"controller":"1","lease":"forged","acknowledged":true}});
    assert!(decode_frame(&encode_frame(&forged).unwrap()).is_err());
}
