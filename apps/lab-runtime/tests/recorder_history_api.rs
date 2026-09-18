//! Historical SQL is an Operation; page Query uses a bounded retained result.

use lab_core::{
    Command, InstrumentId, ParameterId, Sample, SignalId, Unit, Value as DomainValue,
    managed::CapturedInput, recording::RecordingFact,
};
use lab_runtime::{
    application::Application,
    host::{Clock, HostCore},
    recorder::{
        RecorderGap, RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, SqliteStore,
        WriterBarrier,
    },
    service::{ServiceHost, ServiceOptions},
    wire::{WireRequest, decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn frame(value: Value) -> WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

#[derive(Clone, Copy)]
struct FrozenClock(Duration);

impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-history-api-{suffix}.sqlite"))
}

#[test]
fn history_read_rejects_untrusted_fields_and_invalid_ranges_before_worker_admission() {
    let path = temporary_database();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello-invalid","op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let database = service.owner().recording_database_id().unwrap().to_owned();
    let boot = service.boot_id().to_owned();
    let base = json!({"mode":"measurements","database_id":database,"boot_id":boot,
        "run_id":{"boot_id":boot,"run_no":"1"},
        "signal":{"instrument":"1","parameter":"1"},
        "from_ns":"0","to_ns":"1000000000","max_records":8,"cursor":null});
    for field in ["sql", "path", "downsampling", "history_envelope"] {
        let mut args = base.clone();
        args[field] = json!("untrusted");
        let raw = encode_frame(&json!({"v":1,"msg_id":format!("wire-{field}"),
            "op":"history_read","request_id":{"scope":scope,"seq":"1"},
            "args":args}))
        .unwrap();
        assert!(
            decode_frame(&raw).is_err(),
            "{field} must fail at the wire boundary"
        );
    }
    for nested in ["run_id", "signal"] {
        let mut args = base.clone();
        args[nested]["sql"] = json!("untrusted");
        let raw = encode_frame(&json!({"v":1,"msg_id":format!("nested-{nested}"),
            "op":"history_read","request_id":{"scope":scope,"seq":"1"},
            "args":args}))
        .unwrap();
        assert!(
            decode_frame(&raw).is_err(),
            "{nested} must reject unknown nested fields"
        );
    }
    let mut unterminated = encode_frame(&json!({"v":1,"msg_id":"unterminated",
        "op":"history_read","request_id":{"scope":scope,"seq":"1"},
        "args":base.clone()}))
    .unwrap();
    unterminated.pop();
    assert!(decode_frame(&unterminated).is_err());
    assert!(decode_frame(&vec![b'x'; 16_385]).is_err());
    let mut cases = Vec::new();
    for (name, field, value) in [
        ("zero-limit", "max_records", json!(0)),
        ("large-limit", "max_records", json!(129)),
        ("equal-range", "to_ns", json!("0")),
        ("reverse-range", "from_ns", json!("2000000000")),
        ("bad-time", "from_ns", json!("01")),
        ("overflow-time", "to_ns", json!("18446744073709551616")),
        ("long-cursor", "cursor", json!("c".repeat(129))),
        ("unsupported-mode", "mode", json!("envelope")),
    ] {
        let mut args = base.clone();
        args[field] = value;
        cases.push((name, args));
    }
    let mut wrong_boot = base.clone();
    wrong_boot["run_id"]["boot_id"] = json!("ffffffffffffffffffffffffffffffff");
    cases.push(("mismatched-run", wrong_boot));
    let mut wrong_signal = base.clone();
    wrong_signal["signal"]["instrument"] = json!("-1");
    cases.push(("bad-signal", wrong_signal));
    for (seq, (name, args)) in cases.into_iter().enumerate() {
        let response = app.handle(
            &mut service,
            1,
            frame(json!({"v":1,"msg_id":format!("invalid-{name}"),
                "op":"history_read","request_id":{"scope":scope,"seq":(seq+1).to_string()},
                "args":args})),
        );
        assert_eq!(response.len(), 1, "{name}: {response:?}");
        assert_eq!(response[0]["type"], "error", "{name}: {response:?}");
    }
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_groups,
        0
    );
    service.request_shutdown().unwrap();
    let by = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < by);
        std::thread::yield_now();
    }
    drop(app);
    drop(service);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn restart_rejects_old_scope_event_and_history_cursor_but_pages_old_run_and_empty_range() {
    let path = temporary_database();
    let archive_boot = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
    let signal = SignalId::new(InstrumentId::new(211), lab_core::TEMPERATURE);
    let mut archive = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    archive.start_run("before restart").unwrap();
    let facts = (1..=2u64)
        .map(|sequence| RecordingFact::Measurement {
            sequence,
            sample: Sample::validated_good(
                signal,
                Unit::CELSIUS,
                Duration::from_secs(1),
                DomainValue::Float(10.0 + sequence as f64),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
            state_revision: (sequence == 2).then_some(1),
            lineage: (sequence == 2).then_some(CapturedInput {
                signal: SignalId::new(InstrumentId::new(212), lab_core::TEMPERATURE),
                value: 42.0,
                unit: Unit::CELSIUS,
                at: Duration::from_secs(1),
                freshness_at: Duration::from_secs(1),
                source_generation: 2,
                source_revision: 1,
                source_state_revision: Some(3),
            }),
        })
        .collect::<Vec<_>>();
    archive.append_facts(&facts).unwrap();
    archive.stop_run().unwrap();
    archive.finish_boot(Duration::from_secs(2)).unwrap();
    archive.close().unwrap();
    let text = path.to_string_lossy();
    let options = || {
        ServiceOptions::parse(&[
            "--serve",
            "--profile",
            "virtual-demo",
            "--port",
            "0",
            "--record-db",
            text.as_ref(),
        ])
        .unwrap()
    };
    let mut a = ServiceHost::startup(options()).unwrap();
    let a_boot = a.boot_id().to_owned();
    let database = a.owner().recording_database_id().unwrap().to_owned();
    let mut app_a = Application::new(a.boot_id()).unwrap();
    let hello_a = app_a.handle(
        &mut a,
        1,
        frame(json!({
            "v":1,"msg_id":"hello-a","op":"hello","args":{"scope":null}
        })),
    );
    let old_scope = hello_a[0]["result"]["scope"].as_str().unwrap().to_owned();
    let read_a = app_a.handle(
        &mut a,
        1,
        frame(json!({
            "v":1,"msg_id":"read-a","op":"history_read",
            "request_id":{"scope":old_scope,"seq":"1"},
            "args":{"mode":"measurements","database_id":database,"boot_id":archive_boot,
                "run_id":{"boot_id":archive_boot,"run_no":"1"},
                "signal":{"instrument":"211","parameter":"1"},
                "from_ns":"0","to_ns":"2000000000","max_records":1,"cursor":null}
        })),
    );
    assert_eq!(read_a[0]["state"], "accepted");
    let by = Instant::now() + Duration::from_secs(2);
    let result_a = loop {
        let result = app_a.poll_history(&mut a);
        if !result.is_empty() {
            break result;
        }
        assert!(Instant::now() < by);
        std::thread::yield_now();
    };
    assert_eq!(result_a[0].1["state"], "completed");
    let token = result_a[0].1["result"]["page_token"].clone();
    let page_a = app_a.handle(
        &mut a,
        1,
        frame(json!({
            "v":1,"msg_id":"page-a","op":"history_page","args":{"page_token":token}
        })),
    );
    let old_cursor = page_a[0]["result"]["next_cursor"].clone();
    assert!(old_cursor.is_string());
    a.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while a.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(app_a);
    drop(a);
    let mut b = ServiceHost::startup(options()).unwrap();
    assert_ne!(b.boot_id(), a_boot);
    assert_eq!(b.owner().recording_database_id(), Some(database.as_str()));
    let mut app_b = Application::new(b.boot_id()).unwrap();
    let old_hello = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"old-hello","op":"hello","args":{"scope":old_scope}
        })),
    );
    assert_eq!(old_hello[0]["code"], "instance_changed");
    let hello_b = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"hello-b","op":"hello","args":{"scope":null}
        })),
    );
    let scope = hello_b[0]["result"]["scope"].as_str().unwrap().to_owned();
    let stale_event = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"old-event","op":"subscribe",
            "args":{"after":{"boot_id":a_boot,"seq":"0"},
                "filter":{"kinds":[],"targets":[]}}
        })),
    );
    assert_eq!(stale_event[0]["code"], "instance_changed");
    let old_read = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"old-cursor","op":"history_read",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"mode":"measurements","database_id":database,"boot_id":archive_boot,
                "run_id":{"boot_id":archive_boot,"run_no":"1"},
                "signal":{"instrument":"211","parameter":"1"},
                "from_ns":"0","to_ns":"2000000000","max_records":1,"cursor":old_cursor}
        })),
    );
    assert_eq!(old_read[1]["code"], "history_cursor_expired");
    let mut sequence = 2;
    for (mode, from, to, expected) in [
        (
            "measurements",
            "2000000000",
            "3000000000",
            Vec::<u64>::new(),
        ),
        ("measurements", "0", "2000000000", vec![2, 3]),
    ] {
        let read = app_b.handle(
            &mut b,
            2,
            frame(json!({
                "v":1,"msg_id":format!("fresh-{sequence}"),"op":"history_read",
                "request_id":{"scope":scope,"seq":sequence.to_string()},
                "args":{"mode":mode,"database_id":database,"boot_id":archive_boot,
                    "run_id":{"boot_id":archive_boot,"run_no":"1"},
                    "signal":{"instrument":"211","parameter":"1"},
                    "from_ns":from,"to_ns":to,"max_records":8,"cursor":null}
            })),
        );
        assert_eq!(read[0]["state"], "accepted");
        let by = Instant::now() + Duration::from_secs(2);
        let result = loop {
            let result = app_b.poll_history(&mut b);
            if !result.is_empty() {
                break result;
            }
            assert!(Instant::now() < by);
            std::thread::yield_now();
        };
        assert_eq!(result[0].1["state"], "completed", "{result:?}");
        let token = result[0].1["result"]["page_token"].clone();
        let page = app_b.handle(
            &mut b,
            2,
            frame(json!({
                "v":1,"msg_id":"fresh-page","op":"history_page","args":{"page_token":token}
            })),
        );
        assert_eq!(page[0]["result"]["coverage"], "complete");
        assert_eq!(page[0]["result"]["next_cursor"], Value::Null);
        let ids = page[0]["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["record_seq"].as_str().unwrap().parse::<u64>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, expected);
        if mode == "measurements" && from == "0" {
            let rows = page[0]["result"]["rows"].as_array().unwrap();
            assert_eq!(rows[1]["state_revision"], "1");
            assert_eq!(rows[1]["lineage"]["source_generation"], "2");
            assert_eq!(rows[1]["lineage"]["source_revision"], "1");
            assert_eq!(rows[1]["lineage"]["source_state_revision"], "3");
            assert_eq!(rows[1]["lineage"]["value"], 42.0);
            assert_eq!(rows[1]["lineage"]["unit"], Unit::CELSIUS.id());
            assert_eq!(rows[0]["lineage"], Value::Null);
        }
        let release = app_b.handle(
            &mut b,
            2,
            frame(json!({
                "v":1,"msg_id":"release","op":"history_release","args":{"page_token":token}
            })),
        );
        assert_eq!(release[0]["type"], "result");
        sequence += 1;
    }
    let runs = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"runs","op":"history_read",
            "request_id":{"scope":scope,"seq":sequence.to_string()},
            "args":{"mode":"runs","database_id":database,"max_records":8,"cursor":null}
        })),
    );
    assert_eq!(runs[0]["state"], "accepted");
    let by = Instant::now() + Duration::from_secs(2);
    let result = loop {
        let result = app_b.poll_history(&mut b);
        if !result.is_empty() {
            break result;
        }
        assert!(Instant::now() < by);
        std::thread::yield_now();
    };
    let token = result[0].1["result"]["page_token"].clone();
    let page = app_b.handle(
        &mut b,
        2,
        frame(json!({
            "v":1,"msg_id":"run-page","op":"history_page","args":{"page_token":token}
        })),
    );
    assert_eq!(page[0]["result"]["runs"][0]["label"], "before restart");
    b.request_shutdown().unwrap();
    let close_by = Instant::now() + Duration::from_secs(4);
    while b.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < close_by);
        std::thread::yield_now();
    }
    drop(app_b);
    drop(b);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_archived_gap_page_keeps_loss_metadata_within_json_budget() {
    let path = temporary_database();
    let archive_boot = "79797979797979797979797979797979";
    let reason = "\0".repeat(512);
    let mut archive = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    archive.start_run("archive gap").unwrap();
    archive
        .append_facts(&[RecordingFact::Measurement {
            sequence: 6,
            sample: Sample::validated_good(
                SignalId::new(InstrumentId::new(1), lab_core::TEMPERATURE),
                Unit::CELSIUS,
                Duration::from_secs(1),
                DomainValue::Float(23.5),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
            state_revision: None,
            lineage: None,
        }])
        .unwrap();
    archive
        .fail_run(&RecorderGap {
            reason: reason.clone(),
            at: Duration::from_secs(2),
            first_missing_fact: Some(7),
            known_missing_count: Some(2),
            last_accepted_fact: Some(6),
        })
        .unwrap();
    drop(archive);
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status","op":"recording_status","args":{}})),
    );
    let database = status[0]["result"]["database_id"].clone();
    let read = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"read","op":"history_read",
            "request_id":{"scope":scope,"seq":"1"},
            "args":{"mode":"measurements","database_id":database,
                "boot_id":archive_boot,
                "run_id":{"boot_id":archive_boot,"run_no":"1"},
                "signal":{"instrument":"1","parameter":"1"},
                "from_ns":"0","to_ns":"3000000000",
                "max_records":8,"cursor":null}})),
    );
    assert_eq!(read[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        terminal = app.poll_history(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal[0].1["state"], "completed", "{terminal:?}");
    let token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"page","op":"history_page",
            "args":{"page_token":token}})),
    );
    assert_eq!(page[0]["result"]["coverage"], "gap");
    assert_eq!(page[0]["result"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(page[0]["result"]["loss"]["reason"], reason);
    assert_eq!(page[0]["result"]["loss"]["first_missing_fact_seq"], "7");
    assert_eq!(page[0]["result"]["loss"]["known_missing_count"], "2");
    assert!(serde_json::to_vec(&page[0]["result"]).unwrap().len() <= 8 * 1024);
    assert!(encode_frame(&page[0]).unwrap().len() <= 16_384);
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn history_cursor_expires_on_reconnect_and_explicit_ttl_without_silent_resume() {
    let path = temporary_database();
    let archive_boot = "89898989898989898989898989898989";
    let signal = SignalId::new(InstrumentId::new(1), lab_core::TEMPERATURE);
    let mut archive = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    archive.start_run("cursor archive").unwrap();
    archive
        .append_facts(
            &(1..=2u64)
                .map(|sequence| RecordingFact::Measurement {
                    sequence,
                    sample: Sample::validated_good(
                        signal,
                        Unit::CELSIUS,
                        Duration::from_secs(1),
                        DomainValue::Text("\0".repeat(1000)),
                    )
                    .unwrap(),
                    generation: 1,
                    revision: 1,
                    state_revision: None,
                    lineage: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    archive.stop_run().unwrap();
    archive.start_run("other archive interval").unwrap();
    archive.stop_run().unwrap();
    archive.finish_boot(Duration::from_secs(2)).unwrap();
    archive.close().unwrap();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}})),
    );
    let old_scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status","op":"recording_status","args":{}})),
    );
    let database = status[0]["result"]["database_id"].clone();
    let read = |app: &mut Application,
                service: &mut ServiceHost,
                connection: u64,
                scope: &str,
                sequence: u64,
                cursor: Value| {
        app.handle(
            service,
            connection,
            frame(
                json!({"v":1,"msg_id":format!("read-{connection}-{sequence}"),
                "op":"history_read","request_id":{"scope":scope,"seq":sequence.to_string()},
                "args":{"mode":"measurements","database_id":database,
                    "boot_id":archive_boot,
                    "run_id":{"boot_id":archive_boot,"run_no":"1"},
                    "signal":{"instrument":"1","parameter":"1"},
                    "from_ns":"0","to_ns":"2000000000",
                    "max_records":128,"cursor":cursor}}),
            ),
        )
    };
    assert_eq!(
        read(&mut app, &mut service, 1, &old_scope, 1, Value::Null)[0]["state"],
        "accepted"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let terminal = loop {
        let result = app.poll_history(&mut service);
        if !result.is_empty() {
            break result;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(terminal[0].1["state"], "completed");
    let token = terminal[0].1["result"]["page_token"].clone();
    let old_page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"old-page","op":"history_page",
            "args":{"page_token":token}})),
    );
    let old_id = old_page[0]["result"]["rows"][0]["record_seq"].clone();
    let old_cursor = old_page[0]["result"]["next_cursor"].clone();
    assert!(old_cursor.is_string());
    app.detach(&service, 1);
    let replacement_hello = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"hello-2","op":"hello","args":{"scope":null}})),
    );
    let new_scope = replacement_hello[0]["result"]["scope"]
        .as_str()
        .unwrap()
        .to_owned();
    let stale_page = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"stale-page","op":"history_page",
            "args":{"page_token":token}})),
    );
    assert_eq!(stale_page[0]["code"], "history_page_expired");
    let stale_read = read(&mut app, &mut service, 2, &new_scope, 1, old_cursor);
    assert_eq!(stale_read[1]["code"], "history_cursor_expired");
    assert_eq!(
        read(&mut app, &mut service, 2, &new_scope, 2, Value::Null)[0]["state"],
        "accepted"
    );
    let fresh = loop {
        let result = app.poll_history(&mut service);
        if !result.is_empty() {
            break result;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(fresh[0].1["state"], "completed");
    let new_token = fresh[0].1["result"]["page_token"].clone();
    let new_page = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"new-page","op":"history_page",
            "args":{"page_token":new_token}})),
    );
    assert_eq!(new_page[0]["result"]["rows"][0]["record_seq"], old_id);
    let expiring_cursor = new_page[0]["result"]["next_cursor"].clone();
    let release = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"release-new","op":"history_release",
            "args":{"page_token":new_token}})),
    );
    assert_eq!(release[0]["type"], "result");
    let mismatch = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"wrong-archive","op":"history_read",
            "request_id":{"scope":new_scope,"seq":"3"},
            "args":{"mode":"measurements","database_id":database,
                "boot_id":archive_boot,
                "run_id":{"boot_id":archive_boot,"run_no":"2"},
                "signal":{"instrument":"1","parameter":"1"},
                "from_ns":"0","to_ns":"2000000000",
                "max_records":128,"cursor":expiring_cursor}})),
    );
    assert_eq!(mismatch.len(), 2, "{mismatch:?}");
    assert_eq!(
        mismatch[1]["code"], "history_archive_mismatch",
        "{mismatch:?}"
    );
    let range_mismatch = app.handle(
        &mut service,
        2,
        frame(json!({"v":1,"msg_id":"wrong-range","op":"history_read",
            "request_id":{"scope":new_scope,"seq":"4"},
            "args":{"mode":"measurements","database_id":database,
                "boot_id":archive_boot,
                "run_id":{"boot_id":archive_boot,"run_no":"1"},
                "signal":{"instrument":"1","parameter":"1"},
                "from_ns":"1","to_ns":"2000000000",
                "max_records":128,"cursor":expiring_cursor}})),
    );
    assert_eq!(range_mismatch[1]["code"], "history_cursor_mismatch");
    app.expire_projections_at(service.clock().now() + Duration::from_secs(31));
    let expired = read(&mut app, &mut service, 2, &new_scope, 5, expiring_cursor);
    assert_eq!(expired[1]["code"], "history_cursor_expired");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn history_read_is_accepted_then_caches_one_bounded_raw_page_for_pure_query() {
    let path = temporary_database();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"h","op":"hello",
        "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let start = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"start",
        "op":"recording_start","request_id":{"scope":scope,"seq":"1"},
        "args":{"label":"history API"}})),
    );
    assert_eq!(start[0]["state"], "accepted");
    let activation_at = service.clock().now();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        service
            .owner_mut()
            .service(&FrozenClock(activation_at))
            .unwrap();
        let _ = app.poll_recording(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    let start_watermark = service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence;
    let at = service.clock().now() + Duration::from_millis(1);
    let plant = service.owner().plant_id();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at,
        })
        .unwrap();
    assert!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_records
            > 0
            || service
                .owner()
                .recording_status()
                .unwrap()
                .persisted_through_sequence
                > start_watermark,
        "refresh was not admitted into Recorder: {:?}",
        service.owner().recording_status()
    );
    // The measurement group was admitted before this history job. Both use the
    // Recorder worker's bounded FIFO, so completing the job is the authoritative
    // proof that the preceding group was committed before the SQL query ran.
    // Servicing the whole Runtime here would create unrelated periodic groups
    // while waiting and could make the test itself exhaust bounded ingress.
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"s",
        "op":"recording_status","args":{}})),
    );
    let database_id = status[0]["result"]["database_id"].clone();
    let run_id = status[0]["result"]["run_id"].clone();
    let read = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"read",
        "op":"history_read","request_id":{"scope":scope,"seq":"2"},
        "args":{"mode":"measurements","database_id":database_id,
            "boot_id":run_id["boot_id"],"run_id":run_id,
            "signal":{"instrument":"1","parameter":"1"},
            "from_ns":at.as_nanos().to_string(),
            "to_ns":(at+Duration::from_millis(1)).as_nanos().to_string(),
            "max_records":8,"cursor":null}})),
    );
    assert_eq!(read.len(), 1);
    assert_eq!(read[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        terminal = app.poll_history(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].1["state"], "completed");
    let token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"p",
        "op":"history_page","args":{"page_token":token}})),
    );
    assert_eq!(
        page[0]["result"]["rows"].as_array().unwrap().len(),
        1,
        "page={page:?}, status={status:?}, at={at:?}, owner={:?}",
        service.owner().recording_status()
    );
    assert_eq!(page[0]["result"]["rows"][0]["quality"], "good");
    let reject = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"bad",
        "op":"history_read","request_id":{"scope":scope,"seq":"3"},
        "args":{"mode":"measurements","database_id":database_id,
            "boot_id":run_id["boot_id"],"run_id":run_id,
            "signal":{"instrument":"1","parameter":"1"},"from_ns":"0",
            "to_ns":"10000000000","max_records":129,"cursor":null}})),
    );
    assert_eq!(reject[0]["type"], "error");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn disconnected_held_history_job_cannot_publish_into_reused_client_capacity() {
    // A disconnected job may finish after its connection slot is reused. Keeping
    // the orphan/cancellation fence prevents that late durable page from becoming
    // data owned by the new client generation.
    let path = temporary_database();
    let archive_boot = "91919191919191919191919191919191";
    let mut archive = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    archive.start_run("generation archive").unwrap();
    archive.stop_run().unwrap();
    archive.finish_boot(Duration::from_millis(1)).unwrap();
    archive.close().unwrap();

    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let database_id = worker.database_id().to_owned();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let clock = service.clock_copy();
    service.owner_mut().service(&clock).unwrap();
    service
        .owner_mut()
        .start_recording("held writer generation", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    let plant = service.owner().plant_id();
    service
        .owner_mut()
        .command(Command::RefreshMeasurement {
            instrument: plant,
            parameter: lab_core::TEMPERATURE,
            at: clock.now() + Duration::from_millis(1),
        })
        .unwrap();
    assert!(
        barrier.wait_until_reached(Duration::from_secs(2)),
        "SQLite fact stage was not held"
    );

    let mut app = Application::new(service.boot_id()).unwrap();
    let old_hello = app.handle(
        &mut service,
        101,
        frame(json!({"v":1,"msg_id":"old-hello","op":"hello","args":{"scope":null}})),
    );
    let old_scope = old_hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let old_read = app.handle(
        &mut service,
        101,
        frame(json!({
            "v":1,"msg_id":"old-read","op":"history_read",
            "request_id":{"scope":old_scope,"seq":"1"},
            "args":{"mode":"runs","database_id":database_id,
                "max_records":32,"cursor":null}
        })),
    );
    assert_eq!(old_read.len(), 1);
    assert_eq!(old_read[0]["state"], "accepted");
    app.detach(&service, 101);
    assert!(app.poll_history(&mut service).is_empty());

    let resumed = app.handle(
        &mut service,
        102,
        frame(json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":old_scope}})),
    );
    assert_eq!(resumed[0]["type"], "result");
    let terminal = app.handle(
        &mut service,
        102,
        frame(json!({"v":1,"msg_id":"old-status","op":"operation_status",
            "args":{"request_id":{"scope":old_scope,"seq":"1"}}})),
    );
    assert_eq!(terminal[0]["result"]["state"], "failed");
    assert_eq!(terminal[0]["result"]["code"], "client_disconnected");
    app.detach(&service, 102);

    let new_hello = app.handle(
        &mut service,
        202,
        frame(json!({"v":1,"msg_id":"new-hello","op":"hello","args":{"scope":null}})),
    );
    let new_scope = new_hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let new_read = app.handle(
        &mut service,
        202,
        frame(json!({
            "v":1,"msg_id":"new-read","op":"history_read",
            "request_id":{"scope":new_scope,"seq":"1"},
            "args":{"mode":"runs","database_id":database_id,
                "max_records":32,"cursor":null}
        })),
    );
    assert_eq!(new_read.len(), 1);
    assert_eq!(new_read[0]["state"], "accepted");

    barrier.release();
    let terminal = loop {
        let replies = app.poll_history(&mut service);
        if !replies.is_empty() {
            break replies;
        }
        assert!(
            Instant::now() < deadline,
            "replacement history job did not complete"
        );
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    };
    assert_eq!(terminal.len(), 1, "orphan result leaked: {terminal:?}");
    assert_eq!(terminal[0].0, 202);
    assert_eq!(terminal[0].1["state"], "completed");
    let page_token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        202,
        frame(json!({"v":1,"msg_id":"new-page","op":"history_page",
            "args":{"page_token":page_token}})),
    );
    assert_eq!(page.len(), 1);
    assert_eq!(page[0]["type"], "result");
    assert!(
        page[0]["result"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["run_id"]["boot_id"] == archive_boot),
        "unexpected runs page: {page:?}"
    );
    let stale = app.handle(
        &mut service,
        101,
        frame(json!({"v":1,"msg_id":"stale-page","op":"history_page",
            "args":{"page_token":page_token}})),
    );
    assert_eq!(stale[0]["code"], "hello_required");

    drop(app);
    drop(service);
    let remove_by = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < remove_by {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn continuation_capacity_is_bounded_and_disconnect_releases_its_slot() {
    let path = temporary_database();
    let archive_boot = "82828282828282828282828282828282";
    let mut archive = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    for run in 0..12 {
        archive.start_run(&format!("cursor run {run}")).unwrap();
        archive.stop_run().unwrap();
    }
    archive.finish_boot(Duration::from_millis(1)).unwrap();
    archive.close().unwrap();

    let worker = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    let database_id = worker.database_id().to_owned();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();

    let mut terminals = Vec::new();
    for connection in 1..=8u64 {
        let hello = app.handle(
            &mut service,
            connection,
            frame(json!({"v":1,"msg_id":format!("hello-{connection}"),
                "op":"hello","args":{"scope":null}})),
        );
        let scope = hello[0]["result"]["scope"].as_str().unwrap();
        let accepted = app.handle(
            &mut service,
            connection,
            frame(json!({"v":1,"msg_id":format!("read-{connection}"),
                "op":"history_read","request_id":{"scope":scope,"seq":"1"},
                "args":{"mode":"runs","database_id":database_id,
                    "max_records":1,"cursor":null}})),
        );
        assert_eq!(accepted[0]["state"], "accepted");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut completed = app.poll_history(&mut service);
            if let Some(terminal) = completed.pop() {
                assert!(completed.is_empty());
                terminals.push(terminal);
                break;
            }
            assert!(Instant::now() < deadline, "history job did not complete");
            std::thread::yield_now();
        }
    }
    for (connection, terminal) in terminals {
        assert_eq!(terminal["state"], "completed");
        let page = app.handle(
            &mut service,
            connection,
            frame(json!({"v":1,"msg_id":format!("page-{connection}"),
                "op":"history_page","args":{"page_token":terminal["result"]["page_token"]}})),
        );
        assert!(page[0]["result"]["next_cursor"].is_string());
    }

    let ninth = app.handle(
        &mut service,
        9,
        frame(json!({"v":1,"msg_id":"hello-9","op":"hello","args":{"scope":null}})),
    );
    let ninth_scope = ninth[0]["result"]["scope"].as_str().unwrap().to_owned();
    let full = app.handle(
        &mut service,
        9,
        frame(json!({"v":1,"msg_id":"read-9","op":"history_read",
            "request_id":{"scope":ninth_scope,"seq":"1"},
            "args":{"mode":"runs","database_id":database_id,
                "max_records":1,"cursor":null}})),
    );
    assert_eq!(full[1]["code"], "history_busy");

    app.detach(&service, 1);
    let released = app.handle(
        &mut service,
        9,
        frame(json!({"v":1,"msg_id":"read-9-retry","op":"history_read",
            "request_id":{"scope":ninth_scope,"seq":"2"},
            "args":{"mode":"runs","database_id":database_id,
                "max_records":1,"cursor":null}})),
    );
    assert_eq!(released.len(), 1);
    assert_eq!(released[0]["state"], "accepted");

    for connection in 2..=9 {
        app.detach(&service, connection);
    }
    assert!(app.poll_history(&mut service).is_empty());
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn run_discovery_operation_lists_archived_runs_after_service_reopen() {
    let path = temporary_database();
    let mut previous = lab_runtime::recorder::SqliteStore::open_with_boot(
        &path,
        "88888888888888888888888888888888",
    )
    .unwrap();
    previous.start_run("archive A").unwrap();
    previous.stop_run().unwrap();
    drop(previous);
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"h","op":"hello",
        "args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"s",
        "op":"recording_status","args":{}})),
    );
    let database_id = status[0]["result"]["database_id"].clone();
    let read = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"runs",
        "op":"history_read","request_id":{"scope":scope,"seq":"1"},
        "args":{"mode":"runs","database_id":database_id,"max_records":32,"cursor":null}})),
    );
    assert_eq!(read.len(), 1);
    assert_eq!(read[0]["state"], "accepted");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut terminal = Vec::new();
    while terminal.is_empty() && Instant::now() < deadline {
        terminal = app.poll_history(&mut service);
        std::thread::yield_now();
    }
    assert_eq!(terminal[0].1["state"], "completed");
    let token = terminal[0].1["result"]["page_token"].clone();
    let page = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"page",
        "op":"history_page","args":{"page_token":token}})),
    );
    assert_eq!(page[0]["result"]["runs"].as_array().unwrap().len(), 1);
    assert_eq!(page[0]["result"]["runs"][0]["label"], "archive A");
    drop(app);
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

#[test]
fn escaped_archived_values_page_through_public_api_with_bounded_complete_frames() {
    let path = temporary_database();
    let old_boot = "77777777777777777777777777777777";
    let instrument = InstrumentId::new(97);
    let parameter = ParameterId::new(23);
    let signal = SignalId::new(instrument, parameter);
    let mut archive = lab_runtime::recorder::SqliteStore::open_with_boot(&path, old_boot).unwrap();
    archive.start_run("escaped API archive").unwrap();
    archive
        .append_facts(
            &(1..=3u64)
                .map(|sequence| RecordingFact::Measurement {
                    sequence,
                    sample: Sample::validated_good(
                        signal,
                        Unit::CELSIUS,
                        Duration::from_secs(1),
                        DomainValue::Text("\0".repeat(1000)),
                    )
                    .unwrap(),
                    generation: 1,
                    revision: 1,
                    state_revision: None,
                    lineage: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    archive.stop_run().unwrap();
    archive.finish_boot(Duration::from_secs(2)).unwrap();
    archive.close().unwrap();
    let text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    let hello = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"hello",
        "op":"hello","args":{"scope":null}})),
    );
    let scope = hello[0]["result"]["scope"].as_str().unwrap().to_owned();
    let status = app.handle(
        &mut service,
        1,
        frame(json!({"v":1,"msg_id":"status",
        "op":"recording_status","args":{}})),
    );
    let database_id = status[0]["result"]["database_id"].clone();
    let mut cursor = Value::Null;
    let mut ids = Vec::new();
    for sequence in 1..=3u64 {
        let request = app.handle(
            &mut service,
            1,
            frame(json!({
                "v":1,"msg_id":format!("read-{sequence}"),"op":"history_read",
                "request_id":{"scope":scope,"seq":sequence.to_string()},
                "args":{"mode":"measurements","database_id":database_id,
                    "boot_id":old_boot,"run_id":{"boot_id":old_boot,"run_no":"1"},
                    "signal":{"instrument":instrument.get().to_string(),
                        "parameter":parameter.get().to_string()},
                    "from_ns":"0","to_ns":"2000000000","max_records":128,"cursor":cursor}
            })),
        );
        assert_eq!(request[0]["state"], "accepted");
        let deadline = Instant::now() + Duration::from_secs(2);
        let terminal = loop {
            let rows = app.poll_history(&mut service);
            if !rows.is_empty() {
                break rows;
            }
            assert!(
                Instant::now() < deadline,
                "history worker missed bounded completion"
            );
            std::thread::yield_now();
        };
        assert_eq!(terminal[0].1["state"], "completed", "{terminal:?}");
        let token = terminal[0].1["result"]["page_token"].clone();
        let reply = app.handle(
            &mut service,
            1,
            frame(json!({"v":1,
            "msg_id":format!("page-{sequence}"),"op":"history_page",
            "args":{"page_token":token}})),
        );
        let result = &reply[0]["result"];
        assert_eq!(result["rows"].as_array().unwrap().len(), 1);
        assert!(serde_json::to_vec(result).unwrap().len() <= 8 * 1024);
        assert!(encode_frame(&reply[0]).unwrap().len() <= 16_384);
        ids.push(result["rows"][0]["record_seq"].as_str().unwrap().to_owned());
        cursor = result["next_cursor"].clone();
        let released = app.handle(
            &mut service,
            1,
            frame(json!({"v":1,
            "msg_id":format!("release-{sequence}"),"op":"history_release",
            "args":{"page_token":token}})),
        );
        assert_eq!(released[0]["type"], "result");
    }
    assert!(cursor.is_null());
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.shutdown_step().unwrap().is_none() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    drop(app);
    drop(service);
    std::fs::remove_file(path).unwrap();
}
