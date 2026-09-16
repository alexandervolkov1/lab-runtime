//! A stalled SQLite worker must not halt native safety or BestEffort control.

use lab_core::{
    AccessMode, Command, CommandResult, InstrumentId, ParameterId, ParameterRole, Query,
    QueryResult, Unit, Value, ValueSpec, WriteEffect,
    control::ControllerState,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    metakon::crc,
    output::{
        ActuatorId, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal, OutputResult,
        OutputState, SafeProfile,
    },
    transport::{ByteTransport, ExecutorState, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, WriterBarrier},
    server::run,
    service::{ServiceHost, ServiceOptions},
};
use serde_json::{Value as JsonValue, json};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    rc::Rc,
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

#[derive(Default)]
struct M3Wire {
    written: Vec<u8>,
    readable: VecDeque<u8>,
    write_limits: VecDeque<usize>,
    recoveries: usize,
    recovery_pending: bool,
}

struct M3Transport(Rc<RefCell<M3Wire>>);
impl ByteTransport for M3Transport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let limit = wire.write_limits.pop_front().unwrap_or(bytes.len());
        let count = bytes.len().min(limit);
        wire.written.extend_from_slice(&bytes[..count]);
        if count == 7 && count == bytes.len() {
            let body = [15, 0, 6, 1];
            wire.readable.extend(body);
            wire.readable.push_back(crc(&body));
        }
        Ok(count)
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        let count = bytes.len().min(wire.readable.len());
        for byte in &mut bytes[..count] {
            *byte = wire.readable.pop_front().unwrap();
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let mut wire = self.0.borrow_mut();
        wire.recoveries += 1;
        if wire.recovery_pending {
            Ok(RecoveryStatus::Pending)
        } else {
            wire.readable.clear();
            Ok(RecoveryStatus::Complete)
        }
    }
}

fn install_m3_output(
    host: &mut HostCore,
    resource: ResourceId,
    instrument: InstrumentId,
    wire: Rc<RefCell<M3Wire>>,
) -> ActuatorId {
    let parameter = ParameterId::new(6);
    let actuator = ActuatorId::new(instrument, parameter);
    host.register_transport(resource, Box::new(M3Transport(Rc::clone(&wire))))
        .unwrap();
    host.command(Command::RegisterMetakon(MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: instrument,
            name: "held-writer M3 output".into(),
            parameters: vec![DataParameterDefinition {
                id: parameter,
                name: "power".into(),
                value_spec: ValueSpec::Float {
                    min: 0.0,
                    max: 100.0,
                },
                unit: Unit::PERCENT,
                access: AccessMode::ReadWrite,
                role: ParameterRole::Actuator,
                write_effect: WriteEffect::OutputAffecting,
                operation: KnownOperation::Output,
                scale: 1.0,
            }],
        },
        binding: MetakonBinding {
            resource,
            device: 15,
            channel: 0,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: Some(Unit::PERCENT),
        },
        history_capacity: 2,
    }))
    .unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(1),
            max_proposal_ttl: Duration::from_millis(200),
            required_evidence: EvidenceLevel::Acknowledgement,
        }),
    })
    .unwrap();
    host.track_trusted_output(actuator).unwrap();
    host.command(Command::Output {
        actuator,
        at: Duration::ZERO,
        command: OutputCommand::RequestSafe,
    })
    .unwrap();
    host.command(Command::QueueMetakonOutput {
        actuator,
        at: Duration::ZERO,
        queue_ttl: Duration::from_secs(1),
        timeout: Duration::from_millis(20),
    })
    .unwrap();
    host.command(Command::PollTransports { at: Duration::ZERO })
        .unwrap();
    let QueryResult::Output(output) = host.query(Query::Output(actuator)).unwrap() else {
        panic!("M3 output query changed kind")
    };
    assert!(output.safe_confirmed);
    wire.borrow_mut().written.clear();
    actuator
}

fn queue_manual_m3(host: &mut HostCore, actuator: ActuatorId, at: Duration, value: f64) {
    let CommandResult::Output(OutputResult::Lease(lease)) = host
        .command(Command::Output {
            actuator,
            at,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(77),
                lifetime: Duration::from_millis(200),
            },
        })
        .unwrap()
    else {
        panic!("manual M3 lease absent")
    };
    host.command(Command::Output {
        actuator,
        at,
        command: OutputCommand::Propose(OutputProposal {
            lease,
            value: Value::Float(value),
            unit: Unit::PERCENT,
            ttl: Duration::from_millis(100),
        }),
    })
    .unwrap();
    host.command(Command::QueueMetakonOutput {
        actuator,
        at,
        queue_ttl: Duration::from_millis(200),
        timeout: Duration::from_millis(20),
    })
    .unwrap();
}

#[test]
fn best_effort_m3_reaches_first_byte_while_sqlite_writer_is_held() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let wire = Rc::new(RefCell::new(M3Wire {
        write_limits: [7].into_iter().collect(),
        ..M3Wire::default()
    }));
    let mut host = HostCore::virtual_demo().unwrap();
    let actuator = install_m3_output(
        &mut host,
        ResourceId::new(31),
        InstrumentId::new(731),
        Rc::clone(&wire),
    );
    wire.borrow_mut().write_limits = [1].into_iter().collect();
    host.attach_recorder(worker, RecordingPolicy::BestEffort, Duration::ZERO)
        .unwrap();
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("BestEffort M3 first byte", Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    queue_manual_m3(&mut host, actuator, Duration::from_millis(1), 65.0);
    while !barrier.reached() {
        assert!(Instant::now() < deadline, "SQLite fact stage was not held");
        std::thread::yield_now();
    }
    host.command(Command::PollTransports {
        at: Duration::from_millis(1),
    })
    .unwrap();
    assert_eq!(
        wire.borrow().written.len(),
        1,
        "M3 first byte was blocked by SQLite"
    );
    assert!(
        barrier.reached(),
        "storage must still be held at the send boundary"
    );
    let QueryResult::Transport(resource) =
        host.query(Query::Transport(ResourceId::new(31))).unwrap()
    else {
        panic!("transport query changed kind")
    };
    assert_eq!(resource.state, ExecutorState::InFlight);
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
}

#[test]
fn required_deadline_safes_virtual_output_but_keeps_partial_m3_recovery_unknown() {
    let path = temporary_database();
    let barrier = WriterBarrier::held();
    let worker =
        RecorderWorker::open_with_barrier(&path, RecorderLimits::default(), barrier.clone())
            .unwrap();
    let wire = Rc::new(RefCell::new(M3Wire {
        write_limits: [7].into_iter().collect(),
        ..M3Wire::default()
    }));
    let mut host = HostCore::virtual_demo().unwrap();
    let m3 = install_m3_output(
        &mut host,
        ResourceId::new(32),
        InstrumentId::new(732),
        Rc::clone(&wire),
    );
    {
        let mut wire = wire.borrow_mut();
        wire.write_limits = [2].into_iter().collect();
        wire.recovery_pending = true;
    }
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    host.service(&FakeClock(Duration::ZERO)).unwrap();
    host.start_recording("Required multi-output recovery", Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        host.service(&FakeClock(Duration::ZERO)).unwrap();
        std::thread::yield_now();
    }
    host.command(Command::StartController {
        controller: host.controller_id(),
        at: Duration::ZERO,
    })
    .unwrap();
    queue_manual_m3(&mut host, m3, Duration::from_millis(1), 70.0);
    while !barrier.reached() {
        assert!(Instant::now() < deadline, "SQLite fact stage was not held");
        std::thread::yield_now();
    }
    host.command(Command::PollTransports {
        at: Duration::from_millis(1),
    })
    .unwrap();
    assert_eq!(wire.borrow().written.len(), 2);

    host.command(Command::ServiceSafety {
        at: Duration::from_secs(2),
    })
    .unwrap();
    host.command(Command::ServiceSafety {
        at: Duration::from_millis(2_001),
    })
    .unwrap();
    let QueryResult::Controller(controller) =
        host.query(Query::Controller(host.controller_id())).unwrap()
    else {
        panic!("controller query changed kind")
    };
    assert_eq!(controller.state, ControllerState::Failed);
    let QueryResult::Output(virtual_output) = host
        .query(Query::Output(ActuatorId::new(
            host.plant_id(),
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!("virtual output query changed kind")
    };
    assert!(virtual_output.fault_latched);
    assert!(virtual_output.safe_confirmed);
    assert!(virtual_output.lease.is_none());
    let QueryResult::Output(physical_output) = host.query(Query::Output(m3)).unwrap() else {
        panic!("M3 output query changed kind")
    };
    assert!(physical_output.fault_latched);
    assert!(!physical_output.safe_confirmed);
    assert!(physical_output.lease.is_none());
    assert_eq!(
        wire.borrow().written.len(),
        2,
        "partial ordinary bytes were retried"
    );
    assert!(wire.borrow().recoveries >= 1);
    let QueryResult::Transport(resource) =
        host.query(Query::Transport(ResourceId::new(32))).unwrap()
    else {
        panic!("transport query changed kind")
    };
    assert_eq!(resource.state, ExecutorState::Recovering);
    assert!(
        barrier.reached(),
        "safety must progress while SQLite remains held"
    );
    barrier.release();
    drop(host);
    remove_after_worker_close(path);
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

#[test]
fn nonreading_history_client_cannot_block_native_service_while_sqlite_is_held() {
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
        let database = worker.database_id().to_owned();
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
            .start_recording("slow history peer", clock.now())
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
        ready_tx.send((service.bound_address(), database)).unwrap();
        assert!(run(service, stop_thread).is_err());
    });
    let (address, database) = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let reached_by = Instant::now() + Duration::from_secs(2);
    while !barrier.reached() {
        assert!(Instant::now() < reached_by);
        thread::yield_now();
    }
    let slow = TcpStream::connect(address).unwrap();
    slow.set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut slow = BufReader::new(slow);
    slow.get_mut()
        .write_all(
            &lab_runtime::wire::encode_frame(&json!({"v":1,"msg_id":"slow-hello",
            "op":"hello","args":{"scope":null}}))
            .unwrap(),
        )
        .unwrap();
    let mut line = String::new();
    slow.read_line(&mut line).unwrap();
    let hello: JsonValue = serde_json::from_str(&line).unwrap();
    let scope = hello["result"]["scope"].as_str().unwrap();
    let mut requests = Vec::new();
    for sequence in 1..=32 {
        requests.extend(
            lab_runtime::wire::encode_frame(&json!({
                "v":1,"msg_id":format!("slow-{sequence}"),"op":"history_read",
                "request_id":{"scope":scope,"seq":sequence.to_string()},
                "args":{"mode":"runs","database_id":database,"max_records":8,"cursor":null}
            }))
            .unwrap(),
        );
    }
    slow.get_mut().write_all(&requests).unwrap();
    // From here this socket never reads its history replies.
    let probe = TcpStream::connect(address).unwrap();
    probe
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut probe = BufReader::new(probe);
    let mut query = |value: JsonValue| -> JsonValue {
        probe
            .get_mut()
            .write_all(&lab_runtime::wire::encode_frame(&value).unwrap())
            .unwrap();
        let mut line = String::new();
        probe.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    assert_eq!(
        query(json!({"v":1,"msg_id":"probe-hello","op":"hello",
            "args":{"scope":null}}))["type"],
        "result"
    );
    let before = query(json!({"v":1,"msg_id":"probe-before","op":"controller",
        "args":{"controller":"1"}}));
    assert_eq!(before["type"], "result");
    thread::sleep(Duration::from_millis(6_300));
    let started = Instant::now();
    let after = query(json!({"v":1,"msg_id":"probe-after","op":"controller",
        "args":{"controller":"1"}}));
    assert!(started.elapsed() < Duration::from_secs(1), "{after:?}");
    assert_eq!(after["result"]["state"], "running", "{after:?}");
    assert!(
        after["result"]["last_tick"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > before["result"]["last_tick"]
                .as_str()
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap()
    );
    let health = query(
        json!({"v":1,"msg_id":"probe-health","op":"recording_status",
        "args":{}}),
    );
    assert_eq!(health["type"], "result");
    assert_eq!(health["result"]["state"], "failed");
    drop(probe);
    drop(slow);
    barrier.release();
    stop.store(true, Ordering::Release);
    join.join().unwrap();
    remove_after_worker_close(path);
}
