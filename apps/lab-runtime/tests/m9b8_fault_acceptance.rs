//! M9B.8 reconnect, resynchronization, pressure, and client-isolation acceptance.

use lab_core::{InstrumentId, Query, QueryResult, SignalId, TEMPERATURE};
use lab_runtime::{
    application::Application,
    server::run,
    service::{ServiceHost, ServiceOptions},
    sessions::{Admission, Mutation, OperationState, SessionStore},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

static NEXT_CONFIG: AtomicU64 = AtomicU64::new(1);

fn temporary_config() -> PathBuf {
    std::env::temp_dir().join(format!(
        "lab-runtime-m9b8-{}-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_CONFIG.fetch_add(1, Ordering::Relaxed)
    ))
}

fn deployment() -> &'static str {
    r#"schema_version=1
[runtime]
key="m9b8"
display_name="M9B.8 acceptance"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="external"
kind="virtual_measurement"
display_name="External"
history_capacity=16
base_temperature=22.0
measurement_enabled=true
external_publication=true
poll_period_ms=10
[[instruments]]
id=42
key="plant"
kind="thermal_plant"
display_name="Plant"
history_capacity=16
ambient_temperature=20.0
initial_temperature=21.0
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms=10
[[references]]
id=43
key="setpoint"
kind="ramp"
value=40.0
target=40.0
rate=2.0
unit_id="degC"
unit_symbol="°C"
[[safe_profiles]]
instrument_id=42
parameter_id=2
min=0.0
max=100.0
safe_value=0.0
max_lease_ms=2000
max_proposal_ttl_ms=200
required_evidence="readback"
[[controllers]]
id=44
key="pid"
input_instrument_id=41
input_parameter_id=1
output_instrument_id=42
output_parameter_id=2
reference_id=43
period_ms=10
ema_time_constant_ms=20
ema_warmup_samples=3
kp=2.0
ki=0.2
kd=0.0
output_min=0.0
output_max=100.0
max_input_age_ms=500
max_tick_gap_ms=500
lease_lifetime_ms=2000
proposal_ttl_ms=200
"#
}

fn frame(value: Value) -> lab_runtime::wire::WireRequest {
    decode_frame(&encode_frame(&value).unwrap()).unwrap()
}

fn ask(
    app: &mut Application,
    service: &mut ServiceHost,
    connection: u64,
    value: Value,
) -> Vec<Value> {
    app.handle(service, connection, frame(value))
}

fn hello(app: &mut Application, service: &mut ServiceHost, connection: u64) -> Value {
    ask(
        app,
        service,
        connection,
        json!({"v":1,"msg_id":format!("hello-{connection}"),
            "op":"hello","args":{"scope":null}}),
    )[0]
    .clone()
}

fn setup() -> (PathBuf, ServiceHost, Application) {
    let path = temporary_config();
    fs::write(&path, deployment()).unwrap();
    let argument = path.to_string_lossy().into_owned();
    let service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &argument]).unwrap())
            .unwrap();
    let app = Application::new(service.boot_id()).unwrap();
    (path, service, app)
}

#[test]
fn reconnect_rebuilds_authoritative_state_and_exact_replay_never_reapplies() {
    let (path, mut service, mut app) = setup();
    let first = hello(&mut app, &mut service, 1);
    let scope = first["result"]["scope"].as_str().unwrap().to_owned();
    let boot = service.boot_id().to_owned();

    for cycle in 0..32 {
        let cursor = service.owner().event_log().latest_cursor().to_string();
        let sub = ask(
            &mut app,
            &mut service,
            1,
            json!({"v":1,"msg_id":format!("sub-{cycle}"),"op":"subscribe",
                "args":{"after":{"boot_id":boot,"seq":cursor},
                    "filter":{"kinds":[],"targets":[]}}}),
        );
        let token = sub[0]["result"]["subscription"].clone();
        let unsubscribed = ask(
            &mut app,
            &mut service,
            1,
            json!({"v":1,"msg_id":format!("unsub-{cycle}"),"op":"unsubscribe",
                "args":{"subscription":token}}),
        );
        assert_eq!(unsubscribed[0]["result"]["removed"], true);
    }

    let mutation = json!({"v":1,"msg_id":"retune","op":"reference_retune",
        "request_id":{"scope":scope,"seq":"1"},
        "args":{"reference":"43","expected_revision":"1","target":45.0,"rate":3.0}});
    let committed = ask(&mut app, &mut service, 1, mutation.clone());
    assert_eq!(committed[1]["state"], "completed");
    assert_eq!(committed[1]["result"]["revision"], "2");
    app.detach(&service, 1);

    let resumed = ask(
        &mut app,
        &mut service,
        2,
        json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":scope}}),
    );
    assert_eq!(resumed[0]["result"]["next_seq"], "2");
    let replay = ask(&mut app, &mut service, 2, mutation);
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0]["state"], "completed");

    for (msg, op, args) in [
        ("discover", "discover", json!({})),
        ("measurements", "measurements_current", json!({})),
        ("reference", "reference", json!({"reference":"43"})),
        ("controller", "controller", json!({"controller":"44"})),
        ("recorder", "recording_status", json!({})),
        ("configuration", "configuration_status", json!({})),
        ("properties", "configuration_properties", json!({})),
        ("snapshot", "runtime_snapshot", json!({})),
    ] {
        let response = ask(
            &mut app,
            &mut service,
            2,
            json!({"v":1,"msg_id":msg,"op":op,"args":args}),
        );
        assert_eq!(response[0]["type"], "result", "{op}: {response:?}");
    }
    let reference = ask(
        &mut app,
        &mut service,
        2,
        json!({"v":1,"msg_id":"reference-after","op":"reference",
            "args":{"reference":"43"}}),
    );
    assert_eq!(reference[0]["result"]["revision"], "2");

    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn gap_resync_and_emulator_pressure_preserve_native_progress_and_client_isolation() {
    let (path, mut service, mut app) = setup();
    let slow = hello(&mut app, &mut service, 1);
    let slow_scope = slow["result"]["scope"].as_str().unwrap().to_owned();
    let healthy = hello(&mut app, &mut service, 2);
    let healthy_scope = healthy["result"]["scope"].as_str().unwrap().to_owned();
    let boot = service.boot_id().to_owned();
    let subscribed = ask(
        &mut app,
        &mut service,
        1,
        json!({"v":1,"msg_id":"slow-sub","op":"subscribe",
            "args":{"after":{"boot_id":boot,"seq":"0"},
                "filter":{"kinds":[],"targets":[]}}}),
    );
    assert_eq!(subscribed[0]["type"], "result");

    let plant = SignalId::new(InstrumentId::new(42), TEMPERATURE);
    let initial_generation = service.owner().signal_generation(plant);
    for seq in 1..=600u64 {
        let state = if seq % 7 == 0 {
            json!({"signal":{"instrument":"41","parameter":"1"},
                "state":"unavailable","expected_generation":"1"})
        } else {
            json!({"signal":{"instrument":"41","parameter":"1"},
                "state":"good","value":20.0+(seq%50) as f64,
                "expected_generation":"1"})
        };
        let publication = ask(
            &mut app,
            &mut service,
            2,
            json!({"v":1,"msg_id":format!("publish-{seq}"),"op":"emulator_publish",
                "request_id":{"scope":healthy_scope,"seq":seq.to_string()},"args":state}),
        );
        assert_eq!(publication[1]["state"], "completed");
        if seq % 25 == 0 {
            let clock = service.clock_copy();
            service.owner_mut().service(&clock).unwrap();
            let current = ask(
                &mut app,
                &mut service,
                2,
                json!({"v":1,"msg_id":format!("current-{seq}"),
                    "op":"measurements_current","args":{}}),
            );
            assert_eq!(current[0]["type"], "result");
        }
    }
    assert_eq!(service.owner().signal_generation(plant), initial_generation);
    let gap = app.pump_events(&service, 1);
    assert_eq!(gap[0]["code"], "event_gap");
    assert_eq!(gap[0]["resync_required"], true);

    let current = ask(
        &mut app,
        &mut service,
        1,
        json!({"v":1,"msg_id":"resync-current","op":"measurements_current","args":{}}),
    );
    assert_eq!(current[0]["type"], "result");
    let history = ask(
        &mut app,
        &mut service,
        1,
        json!({"v":1,"msg_id":"resync-history","op":"measurement_window",
            "args":{"signal":{"instrument":"41","parameter":"1"},"max_records":128}}),
    );
    assert_eq!(history[0]["type"], "result");
    assert!(history[0]["result"]["rows"].as_array().unwrap().len() <= 16);
    let fresh_cursor = service.owner().event_log().latest_cursor().to_string();
    let fresh = ask(
        &mut app,
        &mut service,
        1,
        json!({"v":1,"msg_id":"fresh-sub","op":"subscribe",
            "args":{"after":{"boot_id":boot,"seq":fresh_cursor},
                "filter":{"kinds":["signal"],"targets":[]}}}),
    );
    assert_eq!(fresh[0]["type"], "result");

    let physical = ask(
        &mut app,
        &mut service,
        1,
        json!({"v":1,"msg_id":"not-writable","op":"emulator_publish",
            "request_id":{"scope":slow_scope,"seq":"1"},"args":{
                "signal":{"instrument":"42","parameter":"1"},"state":"good",
                "value":90.0,"expected_generation":initial_generation.to_string()}}),
    );
    assert_eq!(physical[1]["state"], "failed");
    let QueryResult::Latest(plant_latest) = service
        .owner()
        .query(Query::GetLatestSignal(plant))
        .unwrap()
    else {
        panic!()
    };
    assert!(plant_latest.is_some());
    assert!(
        service
            .owner()
            .output_safe_records()
            .iter()
            .all(|record| { record["authority"]["owner"] != "external_emulator" })
    );

    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

#[test]
fn session_and_global_pending_limits_are_finite_and_recover_after_completion() {
    const BOOT: &str = "1234567890abcdef1234567890abcdef";
    let mut sessions = SessionStore::new(BOOT).unwrap();
    let mutation = |reference| Mutation::RetuneRamp {
        reference,
        expected_revision: 1,
        target: 40.0,
        rate: 2.0,
    };
    let mut scopes = Vec::new();
    for connection in 1..=9u64 {
        scopes.push(
            sessions
                .open(None, connection, Duration::ZERO)
                .unwrap()
                .scope,
        );
    }
    for (index, scope) in scopes.iter().take(8).enumerate() {
        for seq in 1..=8 {
            assert_eq!(
                sessions.admit(scope, seq, mutation(index as u64 + 1), Duration::ZERO),
                Admission::Accepted
            );
        }
        assert_eq!(
            sessions.admit(scope, 9, mutation(index as u64 + 1), Duration::ZERO),
            Admission::Busy
        );
    }
    assert_eq!(
        sessions.admit(&scopes[8], 1, mutation(9), Duration::ZERO),
        Admission::Busy
    );
    sessions
        .complete(
            &scopes[0],
            1,
            OperationState::Completed("done".into()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        sessions.admit(&scopes[8], 1, mutation(9), Duration::ZERO),
        Admission::Accepted
    );
}

#[test]
fn malformed_and_semantically_invalid_clients_leave_a_valid_client_usable() {
    assert!(decode_frame(b"{not-json}\n").is_err());
    assert!(decode_frame(&vec![b'x'; 16_385]).is_err());
    let (path, mut service, mut app) = setup();
    hello(&mut app, &mut service, 1);
    hello(&mut app, &mut service, 2);
    for request in [
        json!({"v":1,"msg_id":"unknown","op":"unknown","args":{}}),
        json!({"v":1,"msg_id":"wrong","op":"latest","args":{
            "signal":{"instrument":"999","parameter":"1"}}}),
        json!({"v":1,"msg_id":"enum","op":"subscribe","args":{
            "after":{"boot_id":service.boot_id(),"seq":"0"},
            "filter":{"kinds":["internal_tick"],"targets":[]}}}),
    ] {
        let rejected = ask(&mut app, &mut service, 1, request);
        assert_eq!(rejected[0]["type"], "error");
    }
    app.detach(&service, 1);
    let healthy = ask(
        &mut app,
        &mut service,
        2,
        json!({"v":1,"msg_id":"healthy","op":"discover","args":{}}),
    );
    assert_eq!(healthy[0]["type"], "result");
    drop(app);
    drop(service);
    fs::remove_file(path).unwrap();
}

fn socket_send(reader: &mut BufReader<TcpStream>, value: Value) -> Value {
    reader
        .get_mut()
        .write_all(&encode_frame(&value).unwrap())
        .unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(!line.is_empty(), "server closed before its bounded reply");
    serde_json::from_str(&line).unwrap()
}

fn socket_connect(address: std::net::SocketAddr) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    BufReader::new(stream)
}

#[test]
fn real_server_isolates_a_dead_bad_client_and_shuts_down_after_reconnect() {
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let join = thread::spawn(move || {
        let host = ServiceHost::startup(
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap(),
        )
        .unwrap();
        ready_tx.send(host.bound_address()).unwrap();
        run(host, flag).unwrap();
    });
    let address = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let mut bad = socket_connect(address);
    bad.get_mut().write_all(b"{not-json}\n").unwrap();
    let mut rejection = String::new();
    bad.read_line(&mut rejection).unwrap();
    assert!(rejection.contains("invalid_json"));
    drop(bad);

    let mut healthy = socket_connect(address);
    assert_eq!(
        socket_send(
            &mut healthy,
            json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}})
        )["type"],
        "result"
    );
    let first = socket_send(
        &mut healthy,
        json!({"v":1,"msg_id":"first","op":"latest",
            "args":{"signal":{"instrument":"1","parameter":"1"}}}),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let later = loop {
        let value = socket_send(
            &mut healthy,
            json!({"v":1,"msg_id":"later","op":"latest",
                "args":{"signal":{"instrument":"1","parameter":"1"}}}),
        );
        if value["result"]["observed_at"] != first["result"]["observed_at"] {
            break value;
        }
        assert!(
            Instant::now() < deadline,
            "native measurement did not progress"
        );
        thread::yield_now();
    };
    assert_eq!(later["type"], "result");
    drop(healthy);

    let mut reconnected = socket_connect(address);
    assert_eq!(
        socket_send(
            &mut reconnected,
            json!({"v":1,"msg_id":"hello-new","op":"hello","args":{"scope":null}})
        )["type"],
        "result"
    );
    assert_eq!(
        socket_send(
            &mut reconnected,
            json!({"v":1,"msg_id":"discover","op":"discover","args":{}})
        )["type"],
        "result"
    );
    drop(reconnected);
    stop.store(true, Ordering::Release);
    join.join().unwrap();
}
