//! A stalled SQLite worker must not halt native safety or BestEffort control.

use lab_core::{
    Command, Query, QueryResult,
    output::{ActuatorId, OutputState},
};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, WriterBarrier},
    server::run,
    service::{ServiceHost, ServiceOptions},
};
use serde_json::{Value as JsonValue, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-isolation-{suffix}.sqlite"))
}

fn remove_after_worker_close(path: PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&path).is_err() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!path.exists());
}

fn start(host: &mut HostCore) {
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("isolation", Duration::ZERO).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
}

#[test]
fn best_effort_native_pid_keeps_progressing_across_three_leases_while_writer_is_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    start(&mut host);
    let mut ticks = 0;
    for step in 1..=70 {
        let report = host
            .service(&FakeClock(Duration::from_millis(step * 100)))
            .unwrap();
        ticks += report.controller_ticks;
    }
    assert!(ticks > 30, "ticks={ticks}");
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Failed
    );
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::ArmedAuto);
    assert!(output.sent.is_some());
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
}

#[test]
fn required_progress_deadline_trips_native_output_even_with_writer_still_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    start(&mut host);
    host.service(&FakeClock(Duration::from_millis(100)))
        .unwrap();
    host.service(&FakeClock(Duration::from_secs(2))).unwrap();
    let QueryResult::Output(output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert!(output.fault_latched);
    assert!(output.lease.is_none());
    assert!(matches!(
        output.state,
        OutputState::SafePending | OutputState::FaultLatched
    ));
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
}

#[test]
fn real_socket_queries_and_native_pid_progress_across_three_leases_with_writer_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker_path = path.clone();
    let worker_barrier = barrier.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (ready_tx, ready_rx) = mpsc::channel();
    let join = thread::spawn(move || {
        let worker = RecorderWorker::open_with_barrier(
            &worker_path,
            RecorderLimits::default(),
            worker_barrier,
        )
        .unwrap();
        let mut host = HostCore::virtual_demo().unwrap();
        host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
            .unwrap();
        let options =
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap();
        let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        service
            .owner_mut()
            .start_recording("socket isolation", clock.now())
            .unwrap();
        let by = Instant::now() + Duration::from_secs(2);
        while service.owner().recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < by);
            service.owner_mut().service(&clock).unwrap();
            thread::yield_now();
        }
        let controller = service.owner().controller_id();
        service
            .owner_mut()
            .command(Command::StartController {
                controller,
                at: clock.now(),
            })
            .unwrap();
        ready_tx.send(service.bound_address()).unwrap();
        let terminal = run(service, stop_thread);
        assert!(
            terminal.is_err(),
            "failed BestEffort coverage requires incomplete shutdown"
        );
    });
    let address = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let reached_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < reached_by);
        thread::yield_now();
    }
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut client = BufReader::new(stream);
    let mut request = |value: JsonValue| -> JsonValue {
        let bytes = lab_runtime::wire::encode_frame(&value).unwrap();
        client.get_mut().write_all(&bytes).unwrap();
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
        serde_json::from_str(&line).unwrap()
    };
    assert_eq!(
        request(json!({"v":1,"msg_id":"hello","op":"hello",
        "args":{"scope":null}}))["type"],
        "result"
    );
    let query = |request: &mut dyn FnMut(JsonValue) -> JsonValue, id: &str| {
        let started = Instant::now();
        let reply = request(json!({"v":1,"msg_id":id,"op":"controller",
            "args":{"controller":"1"}}));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "socket Query waited on SQLite: {reply:?}"
        );
        assert_eq!(reply["type"], "result");
        reply
    };
    let first = query(&mut request, "before");
    thread::sleep(Duration::from_millis(6_300));
    let later = query(&mut request, "after");
    assert_eq!(later["result"]["state"], "running", "{later:?}");
    assert!(
        later["result"]["last_tick"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > first["result"]["last_tick"]
                .as_str()
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap()
    );
    let output_started = Instant::now();
    let output = request(json!({"v":1,"msg_id":"out","op":"output",
        "args":{"actuator":{"instrument":"1",
            "parameter":lab_core::HEATER_POWER.get().to_string()}}}));
    assert!(output_started.elapsed() < Duration::from_secs(1));
    assert_eq!(output["type"], "result", "{output:?}");
    assert_eq!(output["result"]["state"], "armed_auto");
    assert_eq!(output["result"]["owner"]["kind"], "automatic");
    assert!(
        output["result"]["lease_expires_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > later["result"]["last_tick"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
    );
    let reference_started = Instant::now();
    let reference = request(json!({"v":1,"msg_id":"ref","op":"reference",
        "args":{"reference":"1"}}));
    assert!(reference_started.elapsed() < Duration::from_secs(1));
    assert_eq!(reference["type"], "result");
    let health = request(json!({"v":1,"msg_id":"health","op":"recording_status","args":{}}));
    assert_eq!(health["result"]["state"], "failed", "{health:?}");
    barrier.release();
    stop.store(true, Ordering::Release);
    join.join().unwrap();
    remove_after_worker_close(path);
}
