//! Version-1 NDJSON rejects bounded-resource and authority violations.

use lab_runtime::wire::{
    APPLICATION_JSON_LIMIT, FRAME_LIMIT, decode_application_json, decode_frame,
    decode_ndjson_frame, encode_application_json, encode_frame, encode_ndjson_frame,
};
use serde_json::json;

fn hello() -> Vec<u8> {
    b"{\"v\":1,\"msg_id\":\"h1\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n".to_vec()
}

#[test]
fn fragmented_safe_frame_and_crlf_decode_to_the_same_hello_request() {
    let request = decode_frame(&hello()).unwrap();
    assert_eq!(request.v, 1);
    assert_eq!(request.msg_id, "h1");
    assert_eq!(request.op, "hello");
    let mut crlf = hello();
    crlf.insert(crlf.len() - 1, b'\r');
    assert_eq!(decode_frame(&crlf).unwrap(), request);
    let encoded =
        encode_frame(&json!({"v":1,"msg_id":"h1","type":"result","result":{"ok":true}})).unwrap();
    assert!(encoded.ends_with(b"\n"));
    assert!(encoded.len() <= FRAME_LIMIT);
}

#[test]
fn application_json_and_ndjson_share_one_compatible_codec_at_the_exact_bounds() {
    let hello_frame = hello();
    let body = &hello_frame[..hello_frame.len() - 1];
    let application_request = decode_application_json(body).unwrap();
    assert_eq!(decode_ndjson_frame(&hello()).unwrap(), application_request);

    let value = json!({"v":1,"msg_id":"h1","type":"result","result":{"ok":true}});
    let body = encode_application_json(&value).unwrap();
    let frame = encode_ndjson_frame(&value).unwrap();
    assert_eq!(frame[..frame.len() - 1], body);
    assert_eq!(frame.last(), Some(&b'\n'));
    assert_eq!(frame, encode_frame(&value).unwrap());

    let request = &hello_frame[..hello_frame.len() - 1];
    let mut exact = vec![b' '; APPLICATION_JSON_LIMIT - request.len()];
    exact.extend_from_slice(request);
    assert_eq!(exact.len(), APPLICATION_JSON_LIMIT);
    assert_eq!(decode_application_json(&exact).unwrap().op, "hello");
    let mut exact_frame = exact.clone();
    exact_frame.push(b'\n');
    assert_eq!(exact_frame.len(), FRAME_LIMIT);
    assert_eq!(decode_ndjson_frame(&exact_frame).unwrap().op, "hello");

    exact.insert(0, b' ');
    assert_eq!(exact.len(), APPLICATION_JSON_LIMIT + 1);
    assert_eq!(
        decode_application_json(&exact).unwrap_err().code,
        "frame_too_large"
    );
}

#[test]
fn shared_application_json_codec_retains_strict_semantic_validation() {
    let duplicate_top =
        b"{\"v\":1,\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}";
    assert_eq!(
        decode_application_json(duplicate_top).unwrap_err().code,
        "duplicate_key"
    );
    let duplicate_nested =
        b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null,\"scope\":null}}";
    assert_eq!(
        decode_application_json(duplicate_nested).unwrap_err().code,
        "duplicate_key"
    );
    let mut invalid_utf8 = duplicate_top.to_vec();
    invalid_utf8[2] = 0xff;
    assert_eq!(
        decode_application_json(&invalid_utf8).unwrap_err().code,
        "invalid_utf8"
    );
    let invalid_request_id = b"{\"v\":1,\"msg_id\":\"x\",\"op\":\"shutdown\",\"request_id\":{\"scope\":\"s\",\"seq\":\"01\"},\"args\":{}}";
    assert_eq!(
        decode_application_json(invalid_request_id)
            .unwrap_err()
            .code,
        "invalid_id"
    );
    let invalid_args =
        b"{\"v\":1,\"msg_id\":\"x\",\"op\":\"hello\",\"args\":{\"scope\":null,\"extra\":true}}";
    assert_eq!(
        decode_application_json(invalid_args).unwrap_err().code,
        "invalid_args"
    );
    let exact_string = format!(
        "{{\"v\":1,\"msg_id\":\"x\",\"op\":\"unknown\",\"args\":{{\"x\":\"{}\"}}}}",
        "x".repeat(512)
    );
    assert!(decode_application_json(exact_string.as_bytes()).is_ok());
    let oversized_string = format!(
        "{{\"v\":1,\"msg_id\":\"x\",\"op\":\"unknown\",\"args\":{{\"x\":\"{}\"}}}}",
        "x".repeat(513)
    );
    assert_eq!(
        decode_application_json(oversized_string.as_bytes())
            .unwrap_err()
            .code,
        "string_too_large"
    );
}

#[test]
fn duplicate_keys_at_any_level_depth_and_value_budgets_are_rejected() {
    let duplicate_top =
        b"{\"v\":1,\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n";
    assert_eq!(
        decode_frame(duplicate_top).unwrap_err().code,
        "duplicate_key"
    );
    let duplicate_nested =
        b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null,\"scope\":null}}\n";
    assert_eq!(
        decode_frame(duplicate_nested).unwrap_err().code,
        "duplicate_key"
    );
    let mut deep = String::from("{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{");
    deep.push_str(&"[".repeat(17));
    deep.push_str(&"]".repeat(17));
    deep.push_str("}}\n");
    assert_eq!(
        decode_frame(deep.as_bytes()).unwrap_err().code,
        "json_depth"
    );
    let values = format!(
        "{{\"v\":1,\"msg_id\":\"h\",\"op\":\"discover\",\"args\":{{\"x\":[{}]}}}}\n",
        vec!["0"; 1025].join(",")
    );
    assert_eq!(
        decode_frame(values.as_bytes()).unwrap_err().code,
        "json_values"
    );
    // Commas alone undercount singleton values nested in many small objects.
    let singleton_objects = vec!["{\"x\":0}"; 600].join(",");
    let nested = format!(
        "{{\"v\":1,\"msg_id\":\"h\",\"op\":\"discover\",\"args\":{{\"extra\":[{singleton_objects}]}}}}\n"
    );
    assert!(nested.len() < FRAME_LIMIT);
    assert_eq!(
        decode_frame(nested.as_bytes()).unwrap_err().code,
        "json_values"
    );
}

#[test]
fn oversize_utf8_schema_version_and_known_output_commands_fail_closed() {
    let mut oversize = hello();
    oversize.splice(0..0, vec![b' '; FRAME_LIMIT]);
    assert_eq!(decode_frame(&oversize).unwrap_err().code, "frame_too_large");
    let oversized_result = json!({"records":vec!["x".repeat(512); 40]});
    assert_eq!(
        encode_frame(&oversized_result).unwrap_err().code,
        "frame_too_large"
    );
    let mut bad_utf8 = hello();
    bad_utf8.insert(4, 0xff);
    assert_eq!(decode_frame(&bad_utf8).unwrap_err().code, "invalid_utf8");
    let unknown_field =
        b"{\"v\":1,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null},\"extra\":1}\n";
    assert_eq!(
        decode_frame(unknown_field).unwrap_err().code,
        "unknown_field"
    );
    let wrong_version = b"{\"v\":2,\"msg_id\":\"h\",\"op\":\"hello\",\"args\":{\"scope\":null}}\n";
    assert_eq!(
        decode_frame(wrong_version).unwrap_err().code,
        "version_mismatch"
    );
    let direct_output = b"{\"v\":1,\"msg_id\":\"x\",\"op\":\"output_propose\",\"args\":{\"owner\":\"Automatic(1)\"}}\n";
    let bounded_unknown = decode_frame(direct_output).unwrap();
    assert_eq!(bounded_unknown.op, "output_propose");
    assert!(bounded_unknown.request_id.is_none());
    let fake_evidence = b"{\"v\":1,\"msg_id\":\"x\",\"op\":\"controller_start\",\"request_id\":{\"scope\":\"abc\",\"seq\":\"1\"},\"args\":{\"controller\":\"1\",\"ReadbackVerified\":true}}\n";
    assert_eq!(
        decode_frame(fake_evidence).unwrap_err().code,
        "invalid_args"
    );
}

#[test]
fn unknown_bounded_names_reach_application_rejection_without_raw_core_authority() {
    let request = decode_frame(&hello()).unwrap();
    assert_eq!(request.op, "hello");
    for prohibited in [
        "call",
        "output_acquire",
        "output_renew",
        "complete_dispatch",
        "raw_transport",
        "lua_deploy",
        "manual_write",
    ] {
        let frame = format!("{{\"v\":1,\"msg_id\":\"x\",\"op\":\"{prohibited}\",\"args\":{{}}}}\n");
        let request = decode_frame(frame.as_bytes()).unwrap();
        assert_eq!(request.op, prohibited);
        assert!(request.request_id.is_none());
    }
}
