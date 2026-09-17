//! M9B.2 common protocol, error, capability, and operation-lifecycle contract.

use lab_runtime::{
    application::Application,
    protocol::{
        APPLICATION_API_VERSION, PROTOCOL_ID, PROTOCOL_VERSION, PublicError, supported_operations,
    },
    service::{ServiceHost, ServiceOptions},
    wire::{WireRequest, decode_frame, encode_frame},
};
use serde_json::{Value, json};

fn startup() -> (ServiceHost, Application) {
    let service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let app = Application::new(service.boot_id()).unwrap();
    (service, app)
}

fn frame(value: Value) -> WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

fn hello(service: &mut ServiceHost, app: &mut Application, connection: u64) -> Value {
    app.handle(
        service,
        connection,
        frame(json!({"v":PROTOCOL_VERSION,"msg_id":"hello","op":"hello",
            "args":{"scope":null}})),
    )
    .remove(0)
}

#[test]
fn hello_has_one_bounded_protocol_identity_and_authoritative_capabilities() {
    let (mut service, mut app) = startup();
    let hello = hello(&mut service, &mut app, 1);
    let result = &hello["result"];
    assert_eq!(result["protocol"]["id"], PROTOCOL_ID);
    assert_eq!(result["protocol"]["version"], PROTOCOL_VERSION);
    assert_eq!(
        result["application"]["api_version"],
        APPLICATION_API_VERSION
    );
    assert!(result["application"]["package_version"].is_string());
    assert!(result["limits"]["frame_bytes"].as_u64().unwrap() > 0);
    assert!(result["limits"]["error_message_bytes"].as_u64().unwrap() > 0);

    let operations = result["operations"].as_array().unwrap();
    let expected = supported_operations(service.protocol_features());
    assert_eq!(
        operations
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .unwrap(),
        expected
    );
    let capabilities = result["capabilities"].as_array().unwrap();
    assert!(capabilities.iter().all(|capability| {
        capability["name"].as_str().is_some_and(|name| {
            !name.contains("lua")
                && !name.contains("babashka")
                && !name.contains("sqlite")
                && !name.contains("raw_paged")
        }) && capability["version"].is_u64()
            && capability["stability"].is_string()
    }));
    assert!(operations.iter().any(|op| op == "recording_status"));
    assert!(!operations.iter().any(|op| op == "stage_configuration"));
}

#[test]
fn unknown_operation_is_a_structured_nonfatal_synchronous_rejection() {
    let (mut service, mut app) = startup();
    let hello = hello(&mut service, &mut app, 1);
    assert_eq!(hello["type"], "result");
    let unknown = frame(json!({"v":PROTOCOL_VERSION,"msg_id":"unknown",
        "op":"not_a_runtime_operation","args":{}}));
    let rejected = app.handle(&mut service, 1, unknown);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0]["type"], "error");
    assert_eq!(rejected[0]["accepted"], false);
    assert_eq!(rejected[0]["code"], "unsupported_operation");
    assert_eq!(rejected[0]["category"], "unsupported_operation");

    for removed in [
        "reload_managed_sources",
        "runtime_snapshot",
        "snapshot_page",
        "snapshot_release",
    ] {
        let response = app.handle(
            &mut service,
            1,
            frame(json!({"v":PROTOCOL_VERSION,"msg_id":removed,"op":removed,"args":{}})),
        );
        assert_eq!(response[0]["code"], "unsupported_operation", "{removed}");
    }

    let later = app.handle(
        &mut service,
        1,
        frame(json!({"v":PROTOCOL_VERSION,"msg_id":"later","op":"discover","args":{}})),
    );
    assert_eq!(later[0]["type"], "result");
}

#[test]
fn synchronous_rejection_and_async_failure_are_distinct_and_correlated() {
    let (mut service, mut app) = startup();
    let hello = hello(&mut service, &mut app, 1);
    let scope = hello["result"]["scope"].as_str().unwrap();

    let rejected = app.handle(
        &mut service,
        1,
        frame(
            json!({"v":PROTOCOL_VERSION,"msg_id":"bad-query","op":"latest",
            "args":{"signal":{"instrument":"999","parameter":"1"}}}),
        ),
    );
    assert_eq!(rejected[0]["type"], "error");
    assert_eq!(rejected[0]["accepted"], false);
    assert_eq!(rejected[0]["category"], "not_found");

    let failed = app.handle(
        &mut service,
        1,
        frame(json!({"v":PROTOCOL_VERSION,"msg_id":"bad-operation",
            "op":"reference_retune","request_id":{"scope":scope,"seq":"1"},
            "args":{"reference":"1","expected_revision":"1","target":40.0,"rate":0.0}})),
    );
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["state"], "accepted");
    assert_eq!(failed[0]["request_id"], failed[1]["request_id"]);
    assert_eq!(failed[1]["state"], "failed");
    assert_eq!(failed[1]["category"], "invalid_configuration");
    assert!(failed[1]["message"].as_str().unwrap().len() <= 256);
}

#[test]
fn public_errors_are_allowlisted_bounded_and_never_echo_internal_text() {
    let secret = "private Rust Debug path and arbitrary internal detail";
    let error = PublicError::from_internal_code(secret);
    assert_eq!(error.code(), "operation_failed");
    assert_eq!(error.category(), "operation_failed");
    assert!(!error.message().contains(secret));
    assert!(error.message().len() <= 256);
    assert!(PublicError::from_code("invalid_args").message().len() <= 256);
    assert!(PublicError::from_code("transport_unavailable").retryable());
}
