//! Manual delivery uses the same physical fixture as native-controller acceptance.

use super::*;
use lab_core::{
    CommandResult, Value,
    output::{ManualOutputCommand, OutputOwner},
    recording::{OutputStage, RecordingFact},
};

fn manual(
    runtime: &mut Runtime,
    command: ManualOutputCommand,
    at_ms: u64,
) -> lab_core::output::OutputSnapshot {
    let before = output(runtime);
    let CommandResult::ManualOutput(after) = runtime
        .command(Command::ManualOutput {
            actuator: actuator(),
            owner: 17,
            expected_instance: before.instance,
            expected_epoch: before.epoch,
            command,
            at: Duration::from_millis(at_ms),
        })
        .unwrap()
    else {
        panic!("manual output result")
    };
    after
}

fn acquire(runtime: &mut Runtime) {
    manual(
        runtime,
        ManualOutputCommand::Acquire {
            lifetime: Duration::from_secs(2),
        },
        2,
    );
}

fn write(runtime: &mut Runtime) {
    manual(
        runtime,
        ManualOutputCommand::Write {
            value: Value::Float(35.0),
            ttl: Duration::from_millis(50),
        },
        3,
    );
}

#[test]
fn manual_ack_is_not_readback_and_receipt_survives_safe_with_recorder_correlation() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    acquire(&mut runtime);
    runtime.enable_recording_facts();
    write(&mut runtime);
    let queued = output(&runtime).manual_attempt.unwrap();
    assert!(!queued.terminal);
    assert!(queued.sent.is_none());
    service(&mut runtime, 3);
    let ack = output(&runtime).manual_attempt.unwrap();
    assert!(!ack.terminal);
    assert!(ack.acknowledged.is_some());
    assert!(ack.readback.is_none());
    service(&mut runtime, 4);
    let receipt = output(&runtime).manual_attempt.unwrap();
    assert!(receipt.terminal);
    assert_eq!(receipt.outcome, Some(DispatchOutcome::ReadbackVerified));
    assert_eq!(receipt.readback.unwrap().value, 35.0);
    assert_eq!(wire.borrow().last_output, 35);
    manual(&mut runtime, ManualOutputCommand::Safe, 5);
    service(&mut runtime, 5);
    service(&mut runtime, 6);
    let safe = output(&runtime);
    assert!(safe.safe_confirmed);
    assert_eq!(safe.readback.unwrap().value, 0.0);
    assert_eq!(safe.manual_attempt, Some(receipt.clone()));
    let mut stages = Vec::new();
    for fact in runtime.take_recording_facts() {
        if let RecordingFact::Output {
            attempt_id, stage, ..
        } = fact
            && attempt_id == Some(receipt.id)
        {
            stages.push(stage);
        }
    }
    assert!(stages.contains(&OutputStage::Requested));
    assert!(stages.contains(&OutputStage::SendStarted));
    assert!(stages.contains(&OutputStage::Acknowledged));
    assert!(stages.contains(&OutputStage::ReadbackVerified));
}

#[test]
fn manual_transport_loss_distinguishes_no_send_and_unknown_effect_without_retry() {
    for after_send in [false, true] {
        let (mut runtime, wire) = setup();
        establish_safe(&mut runtime);
        acquire(&mut runtime);
        wire.borrow_mut().fail_before_first_write = !after_send;
        wire.borrow_mut().suppress_write_ack = after_send;
        write(&mut runtime);
        for at in 3..100 {
            service(&mut runtime, at);
        }
        let receipt = output(&runtime).manual_attempt.unwrap();
        assert!(receipt.terminal, "after_send={after_send}: {receipt:?}");
        assert_eq!(receipt.sent.is_some(), after_send);
        assert_eq!(
            receipt.outcome,
            Some(if after_send {
                DispatchOutcome::Ambiguous
            } else {
                DispatchOutcome::Failed
            })
        );
        assert!(receipt.acknowledged.is_none());
        assert!(receipt.readback.is_none());
        let normal_writes = wire
            .borrow()
            .frames
            .iter()
            .filter(|frame| frame.as_slice() == [15, 0, 6, 1, 2, 35, crc(&[15, 0, 6, 1, 2, 35])])
            .count();
        assert_eq!(normal_writes, usize::from(after_send));
        for at in 100..2_100 {
            service(&mut runtime, at);
        }
        assert!(output(&runtime).lease.is_none());
        assert_eq!(output(&runtime).manual_attempt, Some(receipt));
    }
}

#[test]
fn manual_readback_mismatch_retains_ack_and_reported_value_after_safe() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    acquire(&mut runtime);
    wire.borrow_mut().readback_offsets.push_back(-1);
    write(&mut runtime);
    for at in 3..20 {
        service(&mut runtime, at);
    }
    let receipt = output(&runtime).manual_attempt.unwrap();
    assert_eq!(receipt.outcome, Some(DispatchOutcome::Failed));
    assert_eq!(receipt.acknowledged.unwrap().value, 35.0);
    assert_eq!(receipt.reported_readback.unwrap().value, 34.0);
    assert_eq!(
        receipt.readback_failure,
        Some(OutputReadbackFailure::Mismatch)
    );
    assert!(receipt.readback.is_none());
    assert!(output(&runtime).safe_confirmed);
}

#[test]
fn manual_authority_conflicts_with_pid_and_explicit_safe_fences_queued_write() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    install_controller(&mut runtime);
    let before = output(&runtime);
    assert!(
        runtime
            .command(Command::ManualOutput {
                actuator: actuator(),
                owner: 17,
                expected_instance: before.instance,
                expected_epoch: before.epoch,
                command: ManualOutputCommand::Acquire {
                    lifetime: Duration::from_secs(1)
                },
                at: Duration::from_millis(2)
            })
            .is_err()
    );
    runtime
        .command(Command::PauseController {
            controller: CONTROLLER,
            at: Duration::from_millis(2),
        })
        .unwrap();
    service(&mut runtime, 2);
    service(&mut runtime, 3);
    manual(
        &mut runtime,
        ManualOutputCommand::Acquire {
            lifetime: Duration::from_secs(1),
        },
        4,
    );
    assert_eq!(
        output(&runtime).lease.unwrap().owner(),
        OutputOwner::Manual(17)
    );
    manual(
        &mut runtime,
        ManualOutputCommand::Write {
            value: Value::Float(35.0),
            ttl: Duration::from_millis(50),
        },
        5,
    );
    manual(&mut runtime, ManualOutputCommand::Safe, 5);
    for at in 5..100 {
        service(&mut runtime, at);
    }
    let receipt = output(&runtime).manual_attempt.unwrap();
    assert!(receipt.terminal);
    assert!(receipt.sent.is_none());
    assert!(
        !wire
            .borrow()
            .frames
            .iter()
            .any(|frame| frame.get(5) == Some(&35))
    );
}

#[test]
fn required_recording_gate_blocks_manual_write_but_preserves_safe_service() {
    let (mut runtime, wire) = setup();
    establish_safe(&mut runtime);
    acquire(&mut runtime);
    runtime.require_recording(Duration::from_millis(2));
    let before = output(&runtime);
    assert_eq!(
        runtime.command(Command::ManualOutput {
            actuator: actuator(),
            owner: 17,
            expected_instance: before.instance,
            expected_epoch: before.epoch,
            command: ManualOutputCommand::Write {
                value: Value::Float(35.0),
                ttl: Duration::from_millis(50)
            },
            at: Duration::from_millis(3)
        }),
        Err(lab_core::Error::RecordingUnavailable)
    );
    assert!(output(&runtime).manual_attempt.is_none());
    assert_eq!(wire.borrow().started_writes, 1);
    manual(&mut runtime, ManualOutputCommand::Safe, 4);
    service(&mut runtime, 4);
    service(&mut runtime, 5);
    assert!(output(&runtime).safe_confirmed);
    assert!(output(&runtime).lease.is_none());
}
