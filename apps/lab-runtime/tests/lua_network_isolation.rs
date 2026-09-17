//! Standard service composition keeps real Lua observations separate from native authority.

use lab_core::managed::ComponentId;
use lab_core::{
    AccessMode, ParameterId, ParameterRole, Unit, ValueSpec, WriteEffect,
    instrument::{
        DataInstrumentDefinition, DataParameterDefinition, KnownOperation, MetakonBinding,
        MetakonInstrumentConfig,
    },
    transport::{ByteTransport, RecoveryStatus, ResourceId, TransportIoError},
};
use lab_core::{Command, InstrumentId, Query, QueryResult, SignalId, TEMPERATURE};
use lab_runtime::{
    application::Application,
    managed_executor::{ManagedExecutor, WorkerBarrier},
    recorder::{
        RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState,
        WriterBarrier as SqliteBarrier,
    },
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use lab_runtime::{
    host::{Clock, HostCore},
    server,
};
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;
static PROFILE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn wait_pool_release() {
    let until = Instant::now() + Duration::from_secs(2);
    while ManagedExecutor::active_workers() != 0 {
        assert!(
            Instant::now() < until,
            "prior fixed-worker pool did not unwind"
        );
        thread::yield_now();
    }
}
use std::time::{Duration, Instant};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

struct StalledMetakon {
    writes: Arc<AtomicUsize>,
    recoveries: Arc<AtomicUsize>,
}
impl ByteTransport for StalledMetakon {
    fn try_write(&mut self, _bytes: &[u8]) -> Result<usize, TransportIoError> {
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(0)
    }
    fn try_read(&mut self, _bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let n = self.recoveries.fetch_add(1, Ordering::AcqRel);
        Ok(if n == 0 {
            RecoveryStatus::Pending
        } else {
            RecoveryStatus::Complete
        })
    }
}
fn metakon_fixture() -> MetakonInstrumentConfig {
    MetakonInstrumentConfig {
        definition: DataInstrumentDefinition {
            schema_version: 1,
            id: InstrumentId::new(70),
            name: "Stalled Metakon read".into(),
            parameters: vec![DataParameterDefinition {
                id: ParameterId::new(1),
                name: "temperature".into(),
                value_spec: ValueSpec::Float {
                    min: -99.9,
                    max: 999.9,
                },
                unit: Unit::CELSIUS,
                access: AccessMode::ReadOnly,
                role: ParameterRole::Measurement,
                write_effect: WriteEffect::None,
                operation: KnownOperation::Temperature,
                scale: 0.1,
            }],
        },
        binding: MetakonBinding {
            resource: ResourceId::new(7),
            device: 1,
            channel: 0,
            binding_generation: 1,
            mapping_revision: 1,
            expected_output_unit: None,
        },
        history_capacity: 4,
    }
}

fn query(service: &mut ServiceHost, app: &mut Application, op: &str, args: Value) -> Value {
    let req =
        decode_frame(&encode_frame(&json!({"v":1,"msg_id":"q","op":op,"args":args})).unwrap())
            .unwrap();
    app.handle(service, 1, req).remove(0)
}
fn managed_step(host: &mut HostCore, id: u64, at: Duration) {
    host.command(Command::InvokeComponent {
        component: ComponentId::new(id),
        at,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        host.command(Command::PollComponents { at }).unwrap();
        let QueryResult::Component(s) = host.query(Query::Component(ComponentId::new(id))).unwrap()
        else {
            panic!("component query shape")
        };
        if s.pending.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "real Lua completion deadline");
        thread::yield_now();
    }
}

#[test]
fn standard_profile_prepares_lua_and_native_components_before_readiness_and_schedules_steps() {
    let _guard = PROFILE_TEST_LOCK.lock().unwrap();
    let mut service = ServiceHost::startup(
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap(),
    )
    .unwrap();
    let mut app = Application::new(service.boot_id()).unwrap();
    query(&mut service, &mut app, "hello", json!({"scope":null}));
    let discovery = query(&mut service, &mut app, "discover", json!({}));
    assert_eq!(
        discovery["result"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| record["kind"] == "component")
            .count(),
        2
    );
    let initial = query(
        &mut service,
        &mut app,
        "component",
        json!({"component":"201"}),
    );
    assert_eq!(initial["result"]["state"], "warming");
    let started = Instant::now();
    let mut progressed = false;
    while started.elapsed() < Duration::from_secs(2) {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        let source = query(
            &mut service,
            &mut app,
            "component",
            json!({"component":"201"}),
        );
        let latest = query(
            &mut service,
            &mut app,
            "latest",
            json!({"signal":{"instrument":"201","parameter":lab_core::TEMPERATURE.get().to_string()}}),
        );
        if source["result"]["state"] == "ready" && latest["result"]["quality"] == "good" {
            progressed = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        progressed,
        "real Lua Source did not publish a Good observation"
    );
    assert_eq!(
        query(
            &mut service,
            &mut app,
            "controller",
            json!({"controller":"1"})
        )["result"]["state"],
        "ready"
    );
}

#[test]
fn two_managed_slots_and_held_sqlite_leave_socket_native_renewal_and_m3_recovery_alive() {
    let _guard = PROFILE_TEST_LOCK.lock().unwrap();
    let barrier = Arc::new(WorkerBarrier::new());
    let sqlite_barrier = SqliteBarrier::held();
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    let database = std::env::temp_dir().join(format!("lab-runtime-m7-lua-sqlite-{suffix}.sqlite"));
    let writes = Arc::new(AtomicUsize::new(0));
    let recoveries = Arc::new(AtomicUsize::new(0));
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let service_stop = stop.clone();
    let worker_barrier = barrier.clone();
    let sqlite_worker_barrier = sqlite_barrier.clone();
    let worker_database = database.clone();
    let wire_writes = writes.clone();
    let wire_recoveries = recoveries.clone();
    let join = thread::spawn(move || {
        wait_pool_release();
        let mut host = HostCore::virtual_demo().unwrap();
        host.register_transport(
            ResourceId::new(7),
            Box::new(StalledMetakon {
                writes: wire_writes,
                recoveries: wire_recoveries,
            }),
        )
        .unwrap();
        host.command(Command::RegisterMetakon(metakon_fixture()))
            .unwrap();
        host.install_component_executor(Box::new(
            ManagedExecutor::new_with_barrier(worker_barrier.clone()).unwrap(),
        ))
        .unwrap();
        host.stage_standard_components(Duration::ZERO).unwrap();
        let init = Instant::now();
        while !host.source_component_initialized() {
            host.command(Command::PollComponents { at: Duration::ZERO })
                .unwrap();
            assert!(init.elapsed() < Duration::from_secs(2));
            thread::yield_now();
        }
        host.stage_standard_filter(Duration::ZERO).unwrap();
        while !host.standard_components_initialized() {
            host.command(Command::PollComponents { at: Duration::ZERO })
                .unwrap();
            assert!(init.elapsed() < Duration::from_secs(2));
            thread::yield_now();
        }
        host.command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::ZERO,
        })
        .unwrap();
        let first = Instant::now();
        while !matches!(
            host.query(Query::GetLatestSignal(SignalId::new(
                InstrumentId::new(201),
                TEMPERATURE
            )))
            .unwrap(),
            QueryResult::Latest(Some(_))
        ) {
            host.command(Command::PollComponents { at: Duration::ZERO })
                .unwrap();
            assert!(first.elapsed() < Duration::from_secs(2));
            thread::yield_now();
        }
        worker_barrier.enable();
        host.command(Command::InvokeComponent {
            component: ComponentId::new(202),
            at: Duration::ZERO,
        })
        .unwrap();
        host.command(Command::InvokeComponent {
            component: ComponentId::new(201),
            at: Duration::from_nanos(1),
        })
        .unwrap();
        host.command(Command::QueueMetakonRead {
            instrument: InstrumentId::new(70),
            parameter: ParameterId::new(1),
            at: Duration::from_nanos(1),
            queue_ttl: Duration::from_secs(1),
            timeout: Duration::from_millis(20),
        })
        .unwrap();
        let entered = Instant::now();
        while worker_barrier.entered() < 2 {
            assert!(entered.elapsed() < Duration::from_secs(2));
            thread::yield_now();
        }
        let recorder = RecorderWorker::open_with_barrier(
            &worker_database,
            RecorderLimits::default(),
            sqlite_worker_barrier,
        )
        .unwrap();
        host.attach_recorder(recorder, RecordingPolicy::BestEffort, Duration::ZERO)
            .unwrap();
        let mut service = ServiceHost::startup_from_trusted_host(
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap(),
            host,
        )
        .unwrap();
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        service
            .owner_mut()
            .start_recording("held SQLite with two Lua slots", clock.now())
            .unwrap();
        let started = Instant::now();
        while service.owner().recording_status().unwrap().state != RecordingState::Recording {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "Start SQL did not commit"
            );
            service.owner_mut().service(&clock).unwrap();
            thread::yield_now();
        }
        ready_tx.send(service.bound_address()).unwrap();
        let _ = server::run(service, service_stop);
    });
    let addr = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(barrier.entered(), 2);
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    stream
        .write_all(
            &encode_frame(&json!({"v":1,"msg_id":"h","op":"hello","args":{"scope":null}})).unwrap(),
        )
        .unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let hello: Value = serde_json::from_str(&line).unwrap();
    let scope = hello["result"]["scope"].as_str().unwrap();
    stream.write_all(&encode_frame(&json!({"v":1,"msg_id":"start","op":"controller_start","request_id":{"scope":scope,"seq":"1"},"args":{"controller":"1"}})).unwrap()).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let started = Instant::now();
    let sqlite_deadline = Instant::now() + Duration::from_secs(2);
    while !sqlite_barrier.reached() {
        assert!(
            Instant::now() < sqlite_deadline,
            "SQLite worker never reached held fact SQL"
        );
        thread::yield_now();
    }
    let mut observed = false;
    while started.elapsed() < Duration::from_secs(2) {
        stream
            .write_all(
                &encode_frame(
                    &json!({"v":1,"msg_id":"q","op":"controller","args":{"controller":"1"}}),
                )
                .unwrap(),
            )
            .unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let q: Value = serde_json::from_str(&line).unwrap();
        if q["result"]["state"] == "running" {
            observed = true;
            break;
        }
        thread::yield_now();
    }
    assert!(
        observed,
        "native loop stayed independent while both Lua slots were blocked"
    );
    let read_output = |stream: &mut TcpStream, reader: &mut BufReader<TcpStream>| -> Value {
        stream.write_all(&encode_frame(&json!({"v":1,"msg_id":"out","op":"output",
            "args":{"actuator":{"instrument":"1","parameter":lab_core::HEATER_POWER.get().to_string()}}})).unwrap()).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let first = read_output(&mut stream, &mut reader);
    assert_eq!(first["result"]["state"], "armed_auto");
    let initial_expires = first["result"]["lease_expires_at"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let instance = first["result"]["instance"].as_str().unwrap();
    let epoch = first["result"]["epoch"].clone();
    let renewal_deadline = Instant::now() + Duration::from_secs(2);
    let later = loop {
        let next = read_output(&mut stream, &mut reader);
        let expiry = next["result"]["lease_expires_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        if expiry >= initial_expires + 300_000_000 {
            break next;
        }
        assert!(
            Instant::now() < renewal_deadline,
            "native lease did not renew while both Lua workers stalled"
        );
        thread::yield_now();
    };
    assert_eq!(later["result"]["instance"], instance);
    assert_eq!(later["result"]["epoch"], epoch);
    assert_eq!(later["result"]["owner"], first["result"]["owner"]);
    assert_eq!(barrier.entered(), 2);
    assert!(sqlite_barrier.reached());
    assert!(
        writes.load(Ordering::Acquire) > 0,
        "real M3 executor never attempted its fake byte adapter"
    );
    assert!(
        recoveries.load(Ordering::Acquire) >= 2,
        "M3 deadline recovery did not progress during two-slot Lua stall"
    );
    stream
        .write_all(
            &encode_frame(&json!({"v":1,"msg_id":"snapshot","op":"runtime_snapshot","args":{}}))
                .unwrap(),
        )
        .unwrap();
    let mut snapshot_line = String::new();
    reader.read_line(&mut snapshot_line).unwrap();
    let snapshot: Value = serde_json::from_str(&snapshot_line).unwrap();
    let records = snapshot["result"]["records"].as_array().unwrap();
    assert!(
        records
            .iter()
            .any(|r| r["kind"] == "resource" && r["data"]["latest"]["outcome"] == "failed"),
        "resource records: {records:?}"
    );
    barrier.release();
    sqlite_barrier.release();
    stop.store(true, Ordering::Release);
    join.join().unwrap();
    let remove_by = Instant::now() + Duration::from_secs(2);
    while std::fs::remove_file(&database).is_err() && Instant::now() < remove_by {
        thread::yield_now();
    }
    assert!(!database.exists());
}

#[test]
fn managed_dependent_controller_fails_safe_while_independent_native_controller_continues() {
    let _guard = PROFILE_TEST_LOCK.lock().unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    wait_pool_release();
    let barrier = Arc::new(WorkerBarrier::new());
    host.install_component_executor(Box::new(
        ManagedExecutor::new_with_barrier(barrier.clone()).unwrap(),
    ))
    .unwrap();
    host.stage_standard_components(Duration::ZERO).unwrap();
    let init = Instant::now();
    while !host.source_component_initialized() {
        host.command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        assert!(init.elapsed() < Duration::from_secs(2));
        thread::yield_now();
    }
    host.stage_standard_filter(Duration::ZERO).unwrap();
    while !host.standard_components_initialized() {
        host.command(Command::PollComponents { at: Duration::ZERO })
            .unwrap();
        assert!(init.elapsed() < Duration::from_secs(2));
        thread::yield_now();
    }
    host.add_managed_dependent_fixture().unwrap();
    assert_eq!(host.configured_output_count(), 2);
    let independent = host
        .query(Query::Controller(lab_core::control::ControllerId::new(1)))
        .unwrap();
    let dependent = host
        .query(Query::Controller(lab_core::control::ControllerId::new(2)))
        .unwrap();
    assert!(
        matches!(independent,QueryResult::Controller(s) if s.state==lab_core::control::ControllerState::Ready)
    );
    assert!(
        matches!(dependent,QueryResult::Controller(s) if s.state==lab_core::control::ControllerState::Ready)
    );
    for at_ms in [0, 200, 400, 600, 800, 1000, 1200] {
        let at = Duration::from_millis(at_ms);
        managed_step(&mut host, 201, at);
        managed_step(&mut host, 202, at);
        if at_ms == 800 {
            host.command(Command::StartController {
                controller: lab_core::control::ControllerId::new(2),
                at,
            })
            .unwrap();
        }
        if at_ms == 1000 {
            host.command(Command::TickController {
                controller: lab_core::control::ControllerId::new(2),
                at,
            })
            .unwrap();
        }
        if at_ms >= 800 {
            host.command(Command::RefreshMeasurement {
                instrument: InstrumentId::new(1),
                parameter: TEMPERATURE,
                at,
            })
            .unwrap();
            if at_ms == 800 {
                host.command(Command::StartController {
                    controller: lab_core::control::ControllerId::new(1),
                    at,
                })
                .unwrap();
            } else {
                host.command(Command::TickController {
                    controller: lab_core::control::ControllerId::new(1),
                    at,
                })
                .unwrap();
            }
        }
    }
    let QueryResult::Controller(pre) = host
        .query(Query::Controller(lab_core::control::ControllerId::new(2)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(pre.state, lab_core::control::ControllerState::Running);
    // A fresh Source result is pending for Transform, while the next Source Step
    // occupies the other slot; both worker barriers must be entered together.
    managed_step(&mut host, 201, Duration::from_millis(1400));
    host.command(Command::RefreshMeasurement {
        instrument: InstrumentId::new(1),
        parameter: TEMPERATURE,
        at: Duration::from_millis(1400),
    })
    .unwrap();
    host.command(Command::TickController {
        controller: lab_core::control::ControllerId::new(1),
        at: Duration::from_millis(1400),
    })
    .unwrap();
    let QueryResult::Controller(native) = host
        .query(Query::Controller(lab_core::control::ControllerId::new(1)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(native.state, lab_core::control::ControllerState::Running);
    barrier.enable();
    host.command(Command::InvokeComponent {
        component: ComponentId::new(202),
        at: Duration::from_millis(1400),
    })
    .unwrap();
    host.command(Command::InvokeComponent {
        component: ComponentId::new(201),
        at: Duration::from_millis(1600),
    })
    .unwrap();
    let entered = Instant::now() + Duration::from_secs(2);
    while barrier.entered() < 2 {
        assert!(Instant::now() < entered);
        thread::yield_now();
    }
    assert_eq!(barrier.entered(), 2);
    host.command(Command::RefreshMeasurement {
        instrument: InstrumentId::new(1),
        parameter: TEMPERATURE,
        at: Duration::from_millis(1600),
    })
    .unwrap();
    host.command(Command::TickController {
        controller: lab_core::control::ControllerId::new(1),
        at: Duration::from_millis(1600),
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let failed = loop {
        host.command(Command::PollComponents {
            at: Duration::from_millis(1700),
        })
        .unwrap();
        let QueryResult::Controller(state) = host
            .query(Query::Controller(lab_core::control::ControllerId::new(2)))
            .unwrap()
        else {
            panic!()
        };
        if state.state == lab_core::control::ControllerState::Failed {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "managed failure did not propagate"
        );
        thread::yield_now();
    };
    let QueryResult::Controller(still_running) = host
        .query(Query::Controller(lab_core::control::ControllerId::new(1)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(failed.state, lab_core::control::ControllerState::Failed);
    assert!(failed.lease.is_none());
    assert_eq!(
        still_running.state,
        lab_core::control::ControllerState::Running
    );
    assert!(still_running.lease.is_some());
    let QueryResult::Latest(Some(unavailable)) = host
        .query(Query::GetLatestSignal(SignalId::new(
            InstrumentId::new(202),
            TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(unavailable.quality(), lab_core::SampleQuality::Unavailable);
    assert_eq!(barrier.entered(), 2);
    barrier.release();
}
