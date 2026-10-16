//! Deterministic completion admission under a held writer and genuine overload.

use super::*;
use crate::recorder::{RecorderLimits, WriterBarrier};
use lab_core::reference::ReferenceId;
use std::{path::PathBuf, time::Instant};

fn wait(host: &mut HostCore, predicate: impl Fn(&RecordingStatus) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        host.poll_recorder(Duration::from_millis(10));
        if predicate(host.recording_status().unwrap()) {
            return;
        }
        assert!(Instant::now() < deadline, "{:?}", host.recording_status());
        std::thread::yield_now();
    }
}

struct Release(WriterBarrier);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn fixture() -> (HostCore, PathBuf, Release) {
    fixture_with_commit_failure(false)
}

fn fixture_with_commit_failure(fail_commit: bool) -> (HostCore, PathBuf, Release) {
    fixture_with_barrier(fail_commit, WriterBarrier::held())
}

fn fixture_with_barrier(fail_commit: bool, barrier: WriterBarrier) -> (HostCore, PathBuf, Release) {
    fixture_with_limits(
        fail_commit,
        barrier,
        RecorderLimits {
            groups: 4,
            ..RecorderLimits::default()
        },
    )
}

fn fixture_with_limits(
    fail_commit: bool,
    barrier: WriterBarrier,
    limits: RecorderLimits,
) -> (HostCore, PathBuf, Release) {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let path = std::env::temp_dir().join(format!("reference-completion-{suffix}.sqlite"));
    if fail_commit {
        drop(crate::recorder::SqliteStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE completion_gate(id INTEGER PRIMARY KEY);
            CREATE TABLE completion_probe(id INTEGER PRIMARY KEY,gate_id INTEGER NOT NULL,
              FOREIGN KEY(gate_id) REFERENCES completion_gate(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_completion AFTER INSERT ON operation_events
              WHEN NEW.phase='completed'
              BEGIN INSERT INTO completion_probe(gate_id) VALUES(999); END;",
        )
        .unwrap();
    }
    let mut worker = RecorderWorker::open_with_barrier(&path, limits, barrier.clone()).unwrap();
    worker.set_periodic_time_for_test(Duration::ZERO);
    let mut host = HostCore::virtual_demo().unwrap();
    host.attach_recorder(worker, RecordingPolicy::Required, Duration::ZERO)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !host.recording_activation_committed().unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    host.start_recording("completion", Duration::ZERO).unwrap();
    wait(&mut host, |s| s.state == RecordingState::Recording);
    (host, path, Release(barrier))
}

#[test]
fn completion_credit_remains_charged_after_commit_until_receipt() {
    let (mut host, path, release) =
        fixture_with_barrier(false, WriterBarrier::held_after_fact_commit());
    retune(&mut host, 1).unwrap();
    assert!(release.0.wait_until_reached(Duration::from_secs(3)));
    host.poll_recorder(Duration::from_millis(10));
    let status = host.recording_status().unwrap();
    // The accepted audit has its own receipt. The committed completion has not.
    assert_eq!(status.persisted_through_sequence, 3);
    assert_eq!(status.confirmed_submission, Some(Duration::from_millis(3)));
    assert_eq!(status.outstanding_groups, 1);
    assert_eq!(status.outstanding_records, 2);
    assert_eq!(status.activation_generation, 1);
    release.0.release();
    wait(&mut host, |s| s.outstanding_groups == 0);
    assert_eq!(
        host.recording_status().unwrap().persisted_through_sequence,
        5
    );
    for _ in 0..3 {
        host.poll_recorder(Duration::from_millis(10));
        assert_eq!(host.recording_status().unwrap().outstanding_records, 0);
    }
    close(&mut host);
    drop(host);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM operation_events", [], |r| r.get(0))
            .unwrap(),
        2
    );
}

fn audit(phase: &'static str, at: u64) -> OperationRecord {
    OperationRecord {
        scope: "completion".into(),
        request_seq: 1,
        command: "reference_retune",
        phase,
        data: "{}".into(),
        outcome_basis: if phase == "accepted" {
            "application_admission"
        } else {
            "domain_result"
        },
        at: Duration::from_millis(at),
    }
}

fn pending_fact(host: &mut HostCore, at: u64) {
    host.command(Command::EvaluateReference {
        reference: ReferenceId::new(1),
        at: Duration::from_millis(at),
    })
    .unwrap();
}

#[test]
fn separate_fact_reservations_backpressure_cancel_and_transfer_exact_credit() {
    let (mut host, _path, release) = fixture();
    let worker = host.recorder.as_mut().unwrap();
    let tokens: Vec<_> = (0..4)
        .map(|_| worker.reserve_fact_group().unwrap().unwrap())
        .collect();
    assert_eq!(worker.reserve_fact_group().unwrap(), None);
    let status = worker.poll();
    assert_eq!(status.state, RecordingState::Recording);
    assert_eq!(status.outstanding_groups, 4);
    assert_eq!(status.outstanding_records, 1024);
    assert_eq!(status.persisted_through_sequence, 2);
    worker.cancel_fact_group(tokens[3]).unwrap();
    assert!(worker.cancel_fact_group(tokens[3]).is_err());
    let replacement = worker.reserve_fact_group().unwrap().unwrap();
    assert!(replacement > tokens[3]);
    for token in [tokens[1], tokens[2], replacement] {
        worker.cancel_fact_group(token).unwrap();
    }
    let fact = lab_core::recording::RecordingFact::Measurement {
        sequence: 1,
        sample: lab_core::Sample::validated_good(
            SignalId::new(InstrumentId::new(7), lab_core::TEMPERATURE),
            lab_core::Unit::CELSIUS,
            Duration::from_millis(1),
            lab_core::Value::Float(20.0),
        )
        .unwrap(),
        generation: 2,
        revision: 3,
        state_revision: None,
        lineage: None,
    };
    worker
        .admit_reserved_facts(tokens[0], vec![fact], Duration::from_millis(1))
        .unwrap();
    let status = worker.poll();
    assert_eq!(status.outstanding_groups, 1);
    assert_eq!(status.outstanding_records, 1);
    assert_eq!(status.persisted_through_sequence, 2);
    assert_eq!(status.activation_generation, 1);
    release.0.release();
    wait(&mut host, |s| s.outstanding_groups == 0);
    assert_eq!(
        host.recording_status().unwrap().persisted_through_sequence,
        3
    );
    close(&mut host);
}

fn retune(host: &mut HostCore, expected_revision: u64) -> Result<CommandResult, Error> {
    if !host.reserve_reference_operation(audit("accepted", 3))? {
        return Err(Error::RecordingUnavailable);
    }
    host.record_operation(audit("accepted", 3));
    let reservation = host.prepare_reference_completion(
        "completion",
        1,
        "reference_retune",
        Duration::from_millis(3),
    );
    let result = reservation.and_then(|()| {
        host.command(Command::RetuneRampReference {
            reference: ReferenceId::new(1),
            expected_revision,
            target: 51.0,
            rate: 2.0,
            at: Duration::from_millis(4),
        })
    });
    host.record_operation(audit(
        if result.is_ok() {
            "completed"
        } else {
            "failed"
        },
        5,
    ));
    result
}

fn close(host: &mut HostCore) {
    host.stop_recording_at(Duration::from_millis(10)).unwrap();
    wait(host, |s| s.state == RecordingState::Idle);
    host.finish_recorder().unwrap();
    wait(host, |s| s.worker_closed);
    assert!(host.recording_status().unwrap().terminal_seal_committed);
    assert_eq!(host.recording_status().unwrap().outstanding_groups, 0);
}

#[test]
fn reference_completion_preserves_terminal_with_two_held_facts_and_four_group_limit() {
    let (mut host, path, release) = fixture();
    pending_fact(&mut host, 1);
    pending_fact(&mut host, 2);
    retune(&mut host, 1).unwrap();
    let status = host.recording_status().unwrap();
    assert_eq!(status.state, RecordingState::Recording);
    assert_eq!(status.outstanding_groups, 4);
    assert_eq!(status.outstanding_records, 5);
    assert_eq!(status.persisted_through_sequence, 2);
    assert_eq!(status.confirmed_submission, Some(Duration::ZERO));
    release.0.release();
    wait(&mut host, |s| s.outstanding_groups == 0);
    assert_eq!(
        host.recording_status().unwrap().persisted_through_sequence,
        7
    );
    assert_eq!(host.recording_status().unwrap().activation_generation, 1);
    close(&mut host);
    let db = rusqlite::Connection::open(&path).unwrap();
    let audits: String = db.query_row("SELECT group_concat(phase, ',') FROM operation_events WHERE command='reference_retune'", [], |r| r.get(0)).unwrap();
    assert_eq!(audits, "accepted,completed");
    assert_eq!(
        db.query_row::<f64, _, _>(
            "SELECT target FROM reference_events ORDER BY record_seq DESC LIMIT 1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        51.0
    );
    assert_eq!(
        db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
    drop(db);
    drop(host);
    let mut reopened = RecorderWorker::open(&path, RecorderLimits::default()).unwrap();
    assert_eq!(reopened.poll().state, RecordingState::Idle);
    reopened.request_finish().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !reopened.poll().worker_closed {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn failed_reference_command_cancels_only_unused_fact_reservation() {
    let (mut host, path, release) = fixture();
    pending_fact(&mut host, 1);
    pending_fact(&mut host, 2);
    assert!(retune(&mut host, 999).is_err());
    assert_eq!(host.recording_status().unwrap().outstanding_groups, 4);
    assert_eq!(host.recording_status().unwrap().outstanding_records, 4);
    release.0.release();
    wait(&mut host, |s| s.outstanding_groups == 0);
    close(&mut host);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT group_concat(phase, ',') FROM operation_events",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        "accepted,failed"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM reference_events", [], |r| r.get(0))
            .unwrap(),
        2
    );
}

#[test]
fn required_failure_observed_at_final_command_poll_blocks_reference_effect() {
    let (mut host, _path, release) = fixture();
    assert!(
        host.reserve_reference_operation(audit("accepted", 3))
            .unwrap()
    );
    host.record_operation(audit("accepted", 3));
    host.prepare_reference_completion(
        "completion",
        1,
        "reference_retune",
        Duration::from_millis(3),
    )
    .unwrap();
    host.recorder.as_mut().unwrap().fail_with_gap(RecorderGap {
        reason: "failure after Application token check".into(),
        at: Duration::from_millis(3),
        first_missing_fact: None,
        known_missing_count: Some(1),
        last_accepted_fact: None,
    });
    // The cached host status is deliberately still Recording. Only the final
    // command poll observes failure, after the Application preflight succeeded.
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    assert_eq!(
        host.command(Command::RetuneRampReference {
            reference: ReferenceId::new(1),
            expected_revision: 1,
            target: 51.0,
            rate: 2.0,
            at: Duration::from_millis(4),
        }),
        Err(Error::RecordingUnavailable)
    );
    let QueryResult::Reference(ReferenceSnapshot::Ramp { revision, .. }) =
        host.query(Query::Reference(ReferenceId::new(1))).unwrap()
    else {
        panic!("Reference")
    };
    assert_eq!(revision, 1);
    host.record_operation(audit("failed", 5));
    release.0.release();
    wait(&mut host, |s| {
        s.outstanding_groups == 0 && s.failure_persisted
    });
    host.finish_recorder().unwrap();
    wait(&mut host, |s| s.worker_closed);
    assert!(host.recording_status().unwrap().terminal_seal_committed);
}

#[test]
fn real_reference_completion_overload_stays_failed_with_durable_gap() {
    let (mut host, path, release) = fixture();
    for at in 1..=5 {
        pending_fact(&mut host, at);
    }
    // A fifth ordinary fact exhausts the explicitly selected four-group fault
    // profile. Required Reference admission must reject the already failed store.
    assert_eq!(retune(&mut host, 1), Err(Error::RecordingUnavailable));
    let QueryResult::Reference(ReferenceSnapshot::Ramp { revision, .. }) =
        host.query(Query::Reference(ReferenceId::new(1))).unwrap()
    else {
        panic!("ramp Reference")
    };
    assert_eq!(revision, 1);
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Failed
    );
    assert_eq!(host.recording_status().unwrap().coverage, "gap");
    assert!(!host.runtime.required_recording_open());
    release.0.release();
    wait(&mut host, |s| {
        s.failure_persisted && s.outstanding_groups == 0
    });
    host.finish_recorder().unwrap();
    wait(&mut host, |s| s.worker_closed);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM operation_events WHERE phase='completed'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn reference_completion_sql_failure_rolls_back_fact_terminal_and_receipt() {
    let (mut host, path, release) = fixture_with_commit_failure(true);
    pending_fact(&mut host, 1);
    pending_fact(&mut host, 2);
    retune(&mut host, 1).unwrap();
    release.0.release();
    wait(&mut host, |s| {
        s.state == RecordingState::Failed && s.worker_closed
    });
    let status = host.recording_status().unwrap();
    assert_eq!(status.persisted_through_sequence, 5);
    assert_eq!(status.outstanding_groups, 1);
    assert_eq!(status.outstanding_records, 2);
    assert_eq!(status.confirmed_submission, Some(Duration::from_millis(3)));
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM reference_events", [], |r| r.get(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT group_concat(phase, ',') FROM operation_events",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        "accepted"
    );
    assert_eq!(
        db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn m18_review_reference_admission_drains_prior_mixed_measurement_group() {
    let (mut host, path, release) = fixture();
    // Model the Core outbox at an owner handoff, before admission drains it.
    host.runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(1),
        })
        .unwrap();
    host.runtime
        .command(Command::EvaluateReference {
            reference: REFERENCE,
            at: Duration::from_millis(2),
        })
        .unwrap();
    retune(&mut host, 1).unwrap();
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    assert!(host.runtime.take_recording_facts().is_empty());
    release.0.release();
    wait(&mut host, |s| s.outstanding_groups == 0);
    close(&mut host);
    drop(host);
    let db = rusqlite::Connection::open(path).unwrap();
    let count = |table: &str| {
        db.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count("measurements"), 1);
    assert_eq!(count("reference_events"), 2);
    assert_eq!(count("operation_events"), 2);
}

#[test]
fn m18_review_reference_completion_must_preserve_safety_facts_from_late_receipt() {
    late_reference_receipt(false);
}

#[test]
fn late_receipt_at_final_reference_dispatch_preserves_safety_and_failed_audit() {
    late_reference_receipt(true);
}

fn late_reference_receipt(at_final_dispatch: bool) {
    let (mut host, path, release) = fixture_with_limits(
        false,
        WriterBarrier::held_after_fact_commit(),
        RecorderLimits::default(),
    );
    // The owner can be descheduled across the Required progress deadline.
    // No scheduler/transport call occurs within the Reference operation below.
    pending_fact(&mut host, 1);
    assert!(release.0.wait_until_reached(Duration::from_secs(3)));
    let late = lab_core::recording::REQUIRED_PROGRESS_AGE + Duration::from_millis(1);
    let accepted = audit("accepted", late.as_millis() as u64);
    assert!(host.reserve_reference_operation(accepted.clone()).unwrap());
    host.record_operation(accepted);
    assert!(host.runtime.required_recording_open());
    if at_final_dispatch {
        // Publish the current owner-turn time without a Core safety service:
        // scheduler safety and receipt polling are distinct bounded opportunities.
        host.last_now = late;
        host.prepare_reference_completion("completion", 1, "reference_retune", late)
            .unwrap();
        assert!(host.runtime.required_recording_open());
    }
    release.0.release();
    let deadline = Instant::now() + Duration::from_secs(3);
    // Observe the writer directly to fix receipt arrival before preparation,
    // without prematurely executing the HostCore safety side effect.
    while host
        .recorder
        .as_mut()
        .unwrap()
        .poll()
        .persisted_through_sequence
        < 4
    {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let prepared = if at_final_dispatch {
        Ok(())
    } else {
        host.prepare_reference_completion("completion", 1, "reference_retune", late)
    };
    assert_eq!(
        host.recording_status().unwrap().state,
        RecordingState::Recording
    );
    let result = prepared.and_then(|()| {
        host.command(Command::RetuneRampReference {
            reference: REFERENCE,
            expected_revision: 1,
            target: 51.0,
            rate: 2.0,
            at: late,
        })
    });
    assert!(!host.runtime.required_recording_open());
    let captured = host.recording_status().unwrap().clone();
    assert_eq!(result, Err(Error::RecordingUnavailable));
    let QueryResult::Reference(ReferenceSnapshot::Ramp { revision, .. }) =
        host.query(Query::Reference(REFERENCE)).unwrap()
    else {
        panic!("Reference")
    };
    assert_eq!(
        revision, 1,
        "Required closure must precede the Reference effect"
    );
    host.record_operation(audit(
        if result.is_ok() {
            "completed"
        } else {
            "failed"
        },
        2002,
    ));
    wait(&mut host, |s| {
        s.outstanding_groups == 0 && (s.state != RecordingState::Failed || s.failure_persisted)
    });
    if host.recording_status().unwrap().state == RecordingState::Failed {
        host.finish_recorder().unwrap();
        wait(&mut host, |s| s.worker_closed);
    } else {
        // The fixed path keeps SQLite healthy after the Required safety trip.
        // Resolve the real virtual output safety transition before stopping;
        // the old failing path used Failed shutdown and never reached this step.
        host.command(Command::ServiceSafety {
            at: Duration::from_millis(2003),
        })
        .unwrap();
        host.stop_recording_at(Duration::from_millis(2004)).unwrap();
        wait(&mut host, |s| s.state == RecordingState::Idle);
        host.finish_recorder().unwrap();
        wait(&mut host, |s| s.worker_closed);
    }
    let final_status = host.recording_status().unwrap().clone();
    drop(host);
    let db = rusqlite::Connection::open(&path).unwrap();
    let terminal_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operation_events WHERE phase IN ('completed','failed')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!(
        "review late receipt: result={result:?}, captured={captured:?}, final={final_status:?}, terminals={terminal_count}, sqlite={}",
        path.display()
    );
    assert_ne!(
        captured.first_error.as_deref(),
        Some("Reference completion fact bound violated"),
        "Required progress polling produced safety facts inside the Reference completion window"
    );
    assert_eq!(
        terminal_count, 1,
        "accepted operation must retain its terminal audit"
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT group_concat(phase, ',') FROM operation_events ORDER BY record_seq",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        "accepted,failed"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM gaps", [], |r| r.get(0))
            .unwrap(),
        0
    );
    assert_eq!(final_status.outstanding_groups, 0);
    assert_eq!(final_status.outstanding_records, 0);
    assert_eq!(final_status.outstanding_bytes, 0);
    let accepted: Vec<u8> = db
        .query_row(
            "SELECT record_seq FROM operation_events WHERE phase='accepted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let revoked: Vec<u8> = db
        .query_row(
            "SELECT record_seq FROM output_events WHERE stage='revoked'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let failed: Vec<u8> = db
        .query_row(
            "SELECT record_seq FROM operation_events WHERE phase='failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        accepted < revoked && revoked < failed,
        "safety facts must precede the reserved terminal in the FIFO"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM reference_events", [], |r| r.get(0))
            .unwrap(),
        1,
        "only the earlier evaluation, no rejected Reference revision"
    );
    assert_eq!(
        db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn m18_review_capture_error_must_not_orphan_removed_probe_reservation() {
    let (mut host, path, release) = fixture();
    let token = host
        .recorder
        .as_mut()
        .unwrap()
        .reserve_fact_group()
        .unwrap()
        .unwrap();
    let signal = SignalId::new(PLANT, lab_core::TEMPERATURE);
    host.probe_fact_reservations.insert(signal, token);
    assert!(
        host.reserve_reference_operation(audit("accepted", 3))
            .unwrap()
    );
    host.record_operation(audit("accepted", 3));
    host.prepare_reference_completion(
        "completion",
        1,
        "reference_retune",
        Duration::from_millis(3),
    )
    .unwrap();
    // Deliberately violate the synchronous Application owner ordering. This is
    // capture-error cleanup coverage, NOT evidence of concurrent probe production.
    host.runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_millis(4),
        })
        .unwrap();
    host.admit_recording_facts(Duration::from_millis(4));
    assert!(host.probe_fact_reservations.is_empty());
    assert_eq!(
        host.recording_status().unwrap().first_error.as_deref(),
        Some("Reference completion fact bound violated")
    );
    host.record_operation(audit("failed", 5));
    host.cancel_configuration_activation(None).unwrap();
    release.0.release();
    wait(&mut host, |s| s.failure_persisted);
    let charged = host.recording_status().unwrap().clone();
    assert_eq!(
        (
            charged.outstanding_groups,
            charged.outstanding_records,
            charged.outstanding_bytes
        ),
        (0, 0, 0),
        "no orphan credit after the failure seal receipt"
    );
    // The test retained the otherwise lost identity only to clean up its fixture.
    let orphan = host
        .recorder
        .as_mut()
        .unwrap()
        .cancel_fact_group(token)
        .is_ok();
    host.finish_recorder().unwrap();
    wait(&mut host, |s| s.worker_closed);
    eprintln!(
        "review capture cleanup: orphan={orphan}, before_manual_cleanup={charged:?}, sqlite={}",
        path.display()
    );
    assert!(
        !orphan,
        "capture removed the owner token but neither cancelled nor transferred its credit"
    );
}
