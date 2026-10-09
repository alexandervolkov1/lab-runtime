//! Application admission owns the whole Reference audit budget before side effects.

use lab_core::Command;
use lab_runtime::{
    application::Application,
    host::{Clock, HostCore},
    recorder::{RecorderLimits, RecorderWorker, RecordingPolicy, RecordingState, WriterBarrier},
    service::{ServiceHost, ServiceOptions},
    wire::{decode_frame, encode_frame},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

// The virtual composition uses the process-wide bounded ManagedExecutor. Only
// these fixtures share it; wire clients still have independent scopes/identities.
static FIXTURE: Mutex<()> = Mutex::new(());
struct Frozen(Duration);
impl Clock for Frozen {
    fn now(&self) -> Duration {
        self.0
    }
}
struct Fixture {
    service: ServiceHost,
    app: Application,
    barrier: WriterBarrier,
    path: PathBuf,
    now: Duration,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.barrier.release();
    }
}
impl Fixture {
    fn new(policy: RecordingPolicy, limits: RecorderLimits) -> Self {
        Self::with_sql_fault(policy, limits, false)
    }
    fn with_sql_fault(policy: RecordingPolicy, limits: RecorderLimits, fail_commit: bool) -> Self {
        let guard = FIXTURE.lock().unwrap_or_else(|p| p.into_inner());
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let id: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("recorder-admission-{id}.sqlite"));
        if fail_commit {
            drop(lab_runtime::recorder::SqliteStore::open(&path).unwrap());
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE admission_gate(id INTEGER PRIMARY KEY);
                CREATE TABLE admission_probe(id INTEGER PRIMARY KEY, gate_id INTEGER NOT NULL,
                FOREIGN KEY(gate_id) REFERENCES admission_gate(id) DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER fail_admission_completion AFTER INSERT ON operation_events
                WHEN NEW.phase='completed' BEGIN INSERT INTO admission_probe(gate_id) VALUES(999); END;").unwrap();
        }
        let barrier = WriterBarrier::held();
        let worker = RecorderWorker::open_with_barrier(&path, limits, barrier.clone()).unwrap();
        let mut host = HostCore::virtual_demo().unwrap();
        host.attach_recorder(worker, policy, Duration::ZERO)
            .unwrap();
        let options =
            ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"])
                .unwrap();
        let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
        let now = service.clock_copy().now();
        service.owner_mut().service(&Frozen(now)).unwrap();
        service
            .owner_mut()
            .start_recording("admission", now)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while service.owner().recording_status().unwrap().state != RecordingState::Recording {
            assert!(Instant::now() < deadline);
            service.owner_mut().service(&Frozen(now)).unwrap();
            std::thread::yield_now();
        }
        let app = Application::new(service.boot_id()).unwrap();
        Self {
            service,
            app,
            barrier,
            path,
            now,
            _guard: guard,
        }
    }
    fn call(&mut self, connection: u64, value: Value) -> Vec<Value> {
        self.app.handle(
            &mut self.service,
            connection,
            decode_frame(&encode_frame(&value).unwrap()).unwrap(),
        )
    }
    fn hello(&mut self, connection: u64) -> String {
        self.call(
            connection,
            json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
        )[0]["result"]["scope"]
            .as_str()
            .unwrap()
            .into()
    }
    fn facts(&mut self, count: u64) {
        for i in 0..count {
            self.service
                .owner_mut()
                .command(Command::RefreshMeasurement {
                    instrument: lab_core::InstrumentId::new(1),
                    parameter: lab_core::TEMPERATURE,
                    at: self.now + Duration::from_nanos(i + 1),
                })
                .unwrap();
        }
    }
    fn retune(&mut self, connection: u64, scope: &str, seq: u64, revision: u64) -> Vec<Value> {
        self.call(connection, json!({"v":1,"msg_id":"retune","op":"reference_retune",
            "request_id":{"scope":scope,"seq":seq.to_string()},
            "args":{"reference":"1","expected_revision":revision.to_string(),"target":50.0+revision as f64,"rate":2.0}}))
    }
    fn wait(&mut self, predicate: impl Fn(&lab_runtime::recorder::RecordingStatus) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let _ = self.service.owner_mut().recording_activation_committed();
            let status = self.service.owner().recording_status().unwrap();
            if predicate(status) {
                return;
            }
            assert!(Instant::now() < deadline, "{status:?}");
            std::thread::yield_now();
        }
    }
    fn seal(&mut self) -> rusqlite::Connection {
        self.barrier.release();
        self.wait(|s| s.outstanding_groups == 0);
        if self.service.owner().recording_status().unwrap().state == RecordingState::Recording {
            let now = self.service.clock_copy().now();
            self.service.owner_mut().stop_recording_at(now).unwrap();
            self.wait(|s| s.state == RecordingState::Idle);
        }
        self.service.owner_mut().finish_recorder().unwrap();
        self.wait(|s| s.worker_closed);
        assert!(
            self.service
                .owner()
                .recording_status()
                .unwrap()
                .terminal_seal_committed
        );
        let db = rusqlite::Connection::open(&self.path).unwrap();
        assert_eq!(
            db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap(),
            "ok"
        );
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        db
    }
}
fn count(db: &rusqlite::Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn three_acquisition_groups_and_reference_have_complete_audit() {
    let mut f = Fixture::new(RecordingPolicy::Required, RecorderLimits::default());
    let scope = f.hello(1);
    f.facts(3);
    let replies = f.retune(1, &scope, 1, 1);
    assert_eq!(replies[0]["state"], "accepted");
    assert_eq!(replies[1]["state"], "completed");
    assert_eq!(
        f.service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_groups,
        5
    );
    let db = f.seal();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM operation_events"), 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
}

#[test]
fn two_scopes_with_two_acquisition_groups_both_retain_terminal_evidence() {
    let mut f = Fixture::new(RecordingPolicy::Required, RecorderLimits::default());
    let a = f.hello(1);
    let b = f.hello(2);
    f.facts(2);
    assert_eq!(f.retune(1, &a, 1, 1)[1]["state"], "completed");
    assert_eq!(f.retune(2, &b, 1, 2)[1]["state"], "completed");
    assert_eq!(
        f.service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_groups,
        6
    );
    let db = f.seal();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM operation_events"), 4);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
}

#[test]
fn failed_required_recorder_rejects_before_admission_but_best_effort_is_explicit() {
    for policy in [RecordingPolicy::Required, RecordingPolicy::BestEffort] {
        let mut f = Fixture::new(
            policy,
            RecorderLimits {
                groups: 4,
                ..RecorderLimits::default()
            },
        );
        let scope = f.hello(1);
        f.facts(5);
        assert_eq!(
            f.service.owner().recording_status().unwrap().state,
            RecordingState::Failed
        );
        let replies = f.retune(1, &scope, 1, 1);
        let state = f.call(
            1,
            json!({"v":1,"msg_id":"status","op":"recording_status","args":{}}),
        );
        assert_eq!(state[0]["result"]["state"], "failed");
        assert_eq!(state[0]["result"]["accepting_facts"], false);
        if policy == RecordingPolicy::Required {
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0]["code"], "recording_unavailable");
            assert_eq!(replies[0]["accepted"], false);
        } else {
            assert_eq!(replies[1]["state"], "completed");
        }
        f.app.detach(&f.service, 1);
        let resumed = f.call(
            2,
            json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":scope}}),
        );
        assert_eq!(
            resumed[0]["result"]["next_seq"],
            if policy == RecordingPolicy::Required {
                "1"
            } else {
                "2"
            }
        );
        let reference = f.call(
            2,
            json!({"v":1,"msg_id":"reference","op":"reference","args":{"reference":"1"}}),
        );
        assert_eq!(
            reference[0]["result"]["revision"],
            if policy == RecordingPolicy::Required {
                "1"
            } else {
                "2"
            }
        );
        let db = f.seal();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 0);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 1);
    }
}

#[test]
fn protected_credit_handles_nine_ordinary_groups_two_clients_and_exact_busy_identity() {
    for policy in [RecordingPolicy::Required, RecordingPolicy::BestEffort] {
        let mut f = Fixture::new(policy, RecorderLimits::default());
        let a = f.hello(1);
        let b = f.hello(2);
        let c = f.hello(3);
        f.facts(9);
        assert_eq!(f.retune(1, &a, 1, 1)[1]["state"], "completed");
        assert_eq!(f.retune(2, &b, 1, 2)[1]["state"], "completed");
        let before = f.service.owner().recording_status().unwrap().clone();
        assert_eq!(before.outstanding_groups, 13);
        assert_eq!(before.outstanding_records, 15);
        assert_eq!(before.activation_generation, 1);
        assert_eq!(before.persisted_through_sequence, 2);
        let busy = f.retune(3, &c, 1, 3);
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0]["code"], "busy");
        assert_eq!(busy[0]["accepted"], false);
        assert_eq!(f.retune(1, &a, 1, 1)[0]["state"], "completed");
        assert_eq!(f.retune(1, &a, 1, 2)[0]["code"], "request_conflict");
        assert_eq!(f.retune(3, &c, 2, 3)[0]["code"], "sequence_gap");
        assert_eq!(*f.service.owner().recording_status().unwrap(), before);
        f.app.detach(&f.service, 3);
        let resumed = f.call(
            4,
            json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":c}}),
        );
        assert_eq!(resumed[0]["result"]["next_seq"], "1");
        f.barrier.release();
        f.wait(|s| s.outstanding_groups == 0);
        // Explicit client resubmission after a known not-admitted Busy. Runtime
        // never repeats a command; the same mutation identity is still available.
        assert_eq!(f.retune(4, &c, 1, 3)[1]["state"], "completed");
        let db = f.seal();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM operation_events"), 6);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 3);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM measurements"), 9);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
        drop(db);
        let mut reopened = RecorderWorker::open(&f.path, RecorderLimits::default()).unwrap();
        assert_eq!(reopened.poll().state, RecordingState::Idle);
        reopened.request_finish().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reopened.poll().worker_closed {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(reopened.poll().terminal_seal_committed);
    }
}

#[test]
fn reduced_shared_capacity_rejects_whole_operation_without_gap_or_sequence_consumption() {
    let mut f = Fixture::new(
        RecordingPolicy::Required,
        RecorderLimits {
            groups: 4,
            ..RecorderLimits::default()
        },
    );
    let scope = f.hello(1);
    f.facts(3);
    let before = f.service.owner().recording_status().unwrap().clone();
    let reply = f.retune(1, &scope, 1, 1);
    assert_eq!(reply.len(), 1);
    assert_eq!(reply[0]["code"], "busy");
    assert_eq!(*f.service.owner().recording_status().unwrap(), before);
    f.app.detach(&f.service, 1);
    let resumed = f.call(
        2,
        json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":scope}}),
    );
    assert_eq!(resumed[0]["result"]["next_seq"], "1");
    let db = f.seal();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM operation_events"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
}

#[test]
fn genuine_ordinary_overload_remains_failed_with_durable_gap() {
    let mut f = Fixture::new(RecordingPolicy::Required, RecorderLimits::default());
    f.facts(10);
    let status = f.service.owner().recording_status().unwrap();
    assert_eq!(status.state, RecordingState::Failed);
    assert_eq!(status.outstanding_groups, 9);
    assert_eq!(status.coverage, "gap");
    let db = f.seal();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM measurements"), 9);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 1);
}

#[test]
fn real_sql_commit_failure_keeps_only_the_committed_prefix_and_no_false_credit() {
    for policy in [RecordingPolicy::Required, RecordingPolicy::BestEffort] {
        let mut f = Fixture::with_sql_fault(policy, RecorderLimits::default(), true);
        let scope = f.hello(1);
        f.facts(3);
        assert_eq!(f.retune(1, &scope, 1, 1)[1]["state"], "completed");
        f.barrier.release();
        f.wait(|s| s.state == RecordingState::Failed && s.worker_closed);
        let failed = f.service.owner().recording_status().unwrap();
        assert_eq!(failed.outstanding_groups, 1);
        assert_eq!(failed.outstanding_records, 2);
        assert_eq!(failed.persisted_through_sequence, 6);
        assert_eq!(failed.activation_generation, 1);
        assert!(!failed.terminal_seal_committed);
        let saved = failed.clone();
        // Repeated owner polls cannot release a rolled-back transaction.
        for _ in 0..4 {
            let _ = f.service.owner_mut().recording_activation_committed();
        }
        assert_eq!(*f.service.owner().recording_status().unwrap(), saved);
        let second = f.retune(1, &scope, 2, 2);
        if policy == RecordingPolicy::Required {
            assert_eq!(second.len(), 1);
            assert_eq!(second[0]["code"], "recording_unavailable");
            assert_eq!(second[0]["accepted"], false);
        } else {
            assert_eq!(second[1]["state"], "completed");
        }
        let reference = f.call(
            1,
            json!({"v":1,"msg_id":"after-failure","op":"reference",
            "args":{"reference":"1"}}),
        );
        assert_eq!(
            reference[0]["result"]["revision"],
            if policy == RecordingPolicy::Required {
                "2"
            } else {
                "3"
            }
        );
        f.app.detach(&f.service, 1);
        let resumed = f.call(
            2,
            json!({"v":1,"msg_id":"resume","op":"hello","args":{"scope":scope}}),
        );
        assert_eq!(
            resumed[0]["result"]["next_seq"],
            if policy == RecordingPolicy::Required {
                "2"
            } else {
                "3"
            }
        );
        let db = rusqlite::Connection::open(&f.path).unwrap();
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM operation_events WHERE phase='accepted'"
            ),
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM operation_events WHERE phase='completed'"
            ),
            0
        );
        assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 0);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM measurements"), 3);
        assert_eq!(
            db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap(),
            "ok"
        );
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
    }
}

#[test]
fn configure_and_failed_domain_command_preserve_both_audits_across_two_scopes() {
    let mut f = Fixture::new(RecordingPolicy::Required, RecorderLimits::default());
    let a = f.hello(1);
    let b = f.hello(2);
    f.facts(3);
    let configured = f.call(
        1,
        json!({"v":1,"msg_id":"configure","op":"reference_configure",
        "request_id":{"scope":a,"seq":"1"},
        "args":{"reference":"1","expected_revision":"1","kind":"fixed","value":24.0}}),
    );
    assert_eq!(configured[0]["state"], "accepted");
    assert_eq!(configured[1]["state"], "completed");
    let failed = f.retune(2, &b, 1, 999);
    assert_eq!(failed[0]["state"], "accepted");
    assert_eq!(failed[1]["state"], "failed");
    assert_eq!(
        f.service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_groups,
        7
    );
    assert_eq!(
        f.service
            .owner()
            .recording_status()
            .unwrap()
            .outstanding_records,
        8
    );
    let db = f.seal();
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM operation_events WHERE phase='accepted'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM operation_events WHERE phase='completed'"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM operation_events WHERE phase='failed'"
        ),
        1
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
}

#[test]
fn bounded_measurement_bursts_do_not_starve_two_reference_clients() {
    let mut f = Fixture::new(RecordingPolicy::Required, RecorderLimits::default());
    let a = f.hello(1);
    let b = f.hello(2);
    f.barrier.release();
    for cycle in 0..32 {
        f.now = f.service.clock_copy().now();
        f.facts(4);
        assert_eq!(
            f.retune(1, &a, cycle + 1, cycle * 2 + 1)[1]["state"],
            "completed"
        );
        assert_eq!(
            f.retune(2, &b, cycle + 1, cycle * 2 + 2)[1]["state"],
            "completed"
        );
        f.wait(|s| s.outstanding_groups == 0);
        assert_eq!(
            f.service.owner().recording_status().unwrap().state,
            RecordingState::Recording
        );
    }
    let db = f.seal();
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM operation_events WHERE phase='completed'"
        ),
        64
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM reference_events"), 64);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM measurements"), 128);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM gaps"), 0);
}
