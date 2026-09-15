//! Historical SQL is an Operation; page Query uses a bounded retained result.

use lab_core::{
    Command, InstrumentId, ParameterId, Sample, SignalId, Unit, Value as DomainValue,
    recording::RecordingFact,
};
use lab_runtime::{
    application::Application,
    host::Clock,
    recorder::{RecorderGap, SqliteStore},
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
fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-history-api-{suffix}.sqlite"))
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
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().run_no.is_none() && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        let _ = app.poll_recording(&mut service);
        std::thread::yield_now();
    }
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
    let deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .unwrap()
        .persisted_through_sequence
        <= start_watermark
        && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
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
