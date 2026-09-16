//! M7 facts must retain intermediate domain evidence inside one Core command.

use lab_core::{
    Command, CommandResult, Error, InstrumentId, Query, QueryResult, Runtime, SampleQuality,
    SignalId, Unit, Value, VirtualInstrumentConfig,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    output::{
        ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal,
        OutputResult, OutputState, SafeProfile,
    },
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    recording::{OutputStage, RecordingFact},
    reference::{ReferenceConfig, ReferenceId},
};
use std::time::Duration;

const PLANT: InstrumentId = InstrumentId::new(601);
const REFERENCE: ReferenceId = ReferenceId::new(602);
const CONTROLLER: ControllerId = ControllerId::new(603);

fn fixture() -> Runtime {
    fixture_with_warmup(1)
}

fn fixture_with_warmup(warmup_samples: usize) -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterThermalPlant(ThermalPlantConfig {
            id: PLANT,
            name: "Recorded plant".into(),
            history_capacity: 8,
            ambient_temperature: 20.0,
            initial_temperature: 20.0,
            gain_per_percent: 0.8,
            time_constant: Duration::from_secs(8),
        }))
        .unwrap();
    let output = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let command = |runtime: &mut Runtime, command| {
        runtime
            .command(Command::Output {
                actuator: output,
                command,
                at: Duration::ZERO,
            })
            .unwrap()
    };
    command(
        &mut runtime,
        OutputCommand::BindProfile(SafeProfile {
            min: 0.0,
            max: 100.0,
            safe_value: 0.0,
            max_lease: Duration::from_secs(5),
            max_proposal_ttl: Duration::from_secs(1),
            required_evidence: EvidenceLevel::Readback,
        }),
    );
    command(&mut runtime, OutputCommand::RequestSafe);
    let lab_core::CommandResult::Output(OutputResult::Dispatched(safe)) =
        command(&mut runtime, OutputCommand::BeginDispatch)
    else {
        panic!()
    };
    command(
        &mut runtime,
        OutputCommand::Complete {
            dispatch_id: safe.id(),
            outcome: DispatchOutcome::ReadbackVerified,
        },
    );
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: REFERENCE,
            value: 60.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::RegisterController(NativeControllerConfig {
            id: CONTROLLER,
            input: SignalId::new(PLANT, lab_core::TEMPERATURE),
            output,
            reference: REFERENCE,
            ema: EmaConfig {
                time_constant: Duration::from_secs(1),
                warmup_samples,
                unit: Unit::CELSIUS,
            },
            pid: PidConfig {
                kp: 3.0,
                ki: 0.4,
                kd: 0.2,
                output_min: 0.0,
                output_max: 100.0,
            },
            max_input_age: Duration::from_secs(2),
            max_tick_gap: Duration::from_secs(2),
            lease_lifetime: Duration::from_secs(5),
            proposal_ttl: Duration::from_secs(1),
        }))
        .unwrap();
    runtime
        .command(Command::PrepareController(CONTROLLER))
        .unwrap();
    runtime
}

#[test]
fn one_pid_tick_keeps_requested_sent_and_readback_as_separate_ordered_facts() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime.take_recording_facts();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    runtime.take_recording_facts();
    runtime
        .command(Command::TickController {
            controller: CONTROLLER,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    let stages: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output { stage, .. } => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        vec![
            OutputStage::Requested,
            OutputStage::Authorized,
            OutputStage::SendStarted,
            OutputStage::ReadbackVerified
        ]
    );
    let output_facts: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                attempt_id,
                dispatch_id,
                ..
            } => Some((*attempt_id, *dispatch_id)),
            _ => None,
        })
        .collect();
    assert_eq!(output_facts.len(), 4);
    assert!(output_facts[0].0.is_some());
    assert!(output_facts.iter().all(|fact| fact.0 == output_facts[0].0));
    assert_eq!(output_facts[0].1, None);
    assert!(output_facts[1].1.is_some());
    assert!(
        output_facts[1..]
            .iter()
            .all(|fact| fact.1 == output_facts[1].1)
    );
    assert!(
        facts
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence())
    );
}

#[test]
fn recording_fact_outbox_never_accumulates_while_disabled() {
    let mut runtime = fixture();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    assert!(runtime.take_recording_facts().is_empty());
}

#[test]
fn rejected_proposal_has_attempt_identity_without_an_invented_dispatch() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.enable_recording_facts();
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(17),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!("fixture did not grant a lease")
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(25.0),
                unit: Unit::CELSIUS,
                ttl: Duration::from_millis(100),
            }),
            at: Duration::ZERO,
        })
        .unwrap_err();
    let facts = runtime.take_recording_facts();
    let rejected: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage: OutputStage::RejectedBeforeSend,
                attempt_id,
                dispatch_id,
                ..
            } => Some((*attempt_id, *dispatch_id)),
            _ => None,
        })
        .collect();
    assert_eq!(rejected.len(), 1);
    assert!(rejected[0].0.is_some());
    assert_eq!(rejected[0].1, None);
}

#[test]
fn proposal_expired_at_exclusive_deadline_has_no_send_or_dispatch_fact() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.enable_recording_facts();
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(18),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!("fixture did not grant a lease")
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(30.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(100),
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::BeginDispatch,
            at: Duration::from_millis(100),
        })
        .unwrap_err();
    let output: Vec<_> = runtime
        .take_recording_facts()
        .into_iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                attempt_id,
                dispatch_id,
                ..
            } => Some((stage, attempt_id, dispatch_id)),
            _ => None,
        })
        .collect();
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].0, OutputStage::Requested);
    assert_eq!(output[1].0, OutputStage::ExpiredBeforeSend);
    assert_eq!(output[0].1, output[1].1);
    assert!(output[0].1.is_some());
    assert!(output.iter().all(|fact| fact.2.is_none()));
}

#[test]
fn revoking_a_queued_proposal_records_its_original_attempt_before_new_safe_intent() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.enable_recording_facts();
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(19),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!("fixture did not grant lease")
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(31.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(100),
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::RequestSafe,
            at: Duration::from_millis(1),
        })
        .unwrap();
    let output: Vec<_> = runtime
        .take_recording_facts()
        .into_iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                attempt_id,
                dispatch_id,
                authority_epoch,
                ..
            } => Some((stage, attempt_id, dispatch_id, authority_epoch)),
            _ => None,
        })
        .collect();
    assert_eq!(output.len(), 4);
    assert_eq!(output[0].0, OutputStage::Requested);
    assert_eq!(output[1].0, OutputStage::SupersededBeforeSend);
    assert_eq!(output[2].0, OutputStage::Revoked);
    assert_eq!(output[3].0, OutputStage::SafeRequested);
    assert_eq!(output[0].1, output[1].1);
    assert_ne!(output[1].1, output[3].1);
    assert_eq!(output[0].3, Some(lease.epoch()));
    assert_eq!(output[1].3, Some(lease.epoch()));
    assert!(output[3].3.unwrap() > lease.epoch());
    assert!(output.iter().all(|fact| fact.2.is_none()));
}

#[test]
fn manual_release_records_displaced_intent_and_distinct_safe_attempt() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.enable_recording_facts();
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(20),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!("fixture did not grant lease")
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(32.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(100),
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Release(lease),
            at: Duration::from_millis(1),
        })
        .unwrap();
    let output: Vec<_> = runtime
        .take_recording_facts()
        .into_iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                attempt_id,
                authority_epoch,
                ..
            } => Some((stage, attempt_id, authority_epoch)),
            _ => None,
        })
        .collect();
    assert_eq!(
        output.iter().map(|fact| fact.0).collect::<Vec<_>>(),
        [
            OutputStage::Requested,
            OutputStage::SupersededBeforeSend,
            OutputStage::Revoked,
            OutputStage::SafeRequested
        ]
    );
    assert_eq!(output[0].1, output[1].1);
    assert_ne!(output[0].1, output[3].1);
    assert_eq!(output[0].2, Some(lease.epoch()));
    assert_eq!(output[1].2, Some(lease.epoch()));
    assert!(output[3].2.unwrap() > lease.epoch());
}

#[test]
fn lease_watchdog_expiry_records_old_pending_attempt_and_new_safe_epoch() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.enable_recording_facts();
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(21),
                lifetime: Duration::from_millis(50),
            },
            at: Duration::ZERO,
        })
        .unwrap()
    else {
        panic!("fixture did not grant lease")
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(34.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(100),
            }),
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Tick,
            at: Duration::from_millis(50),
        })
        .unwrap();
    let output: Vec<_> = runtime
        .take_recording_facts()
        .into_iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                attempt_id,
                authority_epoch,
                ..
            } => Some((stage, attempt_id, authority_epoch)),
            _ => None,
        })
        .collect();
    assert_eq!(
        output.iter().map(|fact| fact.0).collect::<Vec<_>>(),
        [
            OutputStage::Requested,
            OutputStage::ExpiredBeforeSend,
            OutputStage::Revoked,
            OutputStage::SafeRequested
        ]
    );
    assert_eq!(output[0].1, output[1].1);
    assert_ne!(output[0].1, output[3].1);
    assert_eq!(output[1].2, Some(lease.epoch()));
    assert!(output[3].2.unwrap() > lease.epoch());
}

#[test]
fn required_outbox_overflow_closes_authority_before_the_next_ordinary_send() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.require_recording(Duration::ZERO);
    runtime
        .confirm_recording_start(Duration::ZERO, Duration::ZERO)
        .unwrap();
    runtime.enable_recording_facts();
    for _ in 0..=lab_core::recording::MAX_RECORDING_FACTS {
        runtime
            .command(Command::EvaluateReference {
                reference: REFERENCE,
                at: Duration::ZERO,
            })
            .unwrap();
    }
    assert!(runtime.recording_facts_overflowed());
    assert_eq!(
        runtime.recording_first_lost_fact(),
        Some((lab_core::recording::MAX_RECORDING_FACTS as u64) + 1)
    );
    assert_eq!(
        runtime.command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(7),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::ZERO,
        }),
        Err(Error::RecordingUnavailable)
    );
    let QueryResult::Output(output) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!("fixture output missing")
    };
    assert!(output.fault_latched);
    assert!(output.lease.is_none());
}

#[test]
fn good_then_unavailable_attempts_remain_distinct_recording_facts() {
    const VIRTUAL: InstrumentId = InstrumentId::new(604);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: VIRTUAL,
            name: "Failure fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_secs(1);
    runtime
        .command(Command::RefreshMeasurement {
            instrument: VIRTUAL,
            parameter: lab_core::TEMPERATURE,
            at,
        })
        .unwrap();
    runtime
        .command(Command::ConfigureParameter {
            instrument: VIRTUAL,
            parameter: lab_core::MEASUREMENT_ENABLED,
            value: Value::Boolean(false),
        })
        .unwrap();
    let failed_at = Duration::from_secs(2);
    assert!(
        runtime
            .command(Command::RefreshMeasurement {
                instrument: VIRTUAL,
                parameter: lab_core::TEMPERATURE,
                at: failed_at,
            })
            .is_err()
    );
    let facts = runtime.take_recording_facts();
    let samples: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Measurement { sample, .. } => Some(sample),
            _ => None,
        })
        .collect();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0].quality(), SampleQuality::Good);
    assert_eq!(samples[1].quality(), SampleQuality::Unavailable);
    assert_eq!(samples[0].at(), at);
    assert_eq!(samples[1].at(), failed_at);
    assert_eq!(samples[0].unit(), Unit::CELSIUS);
    assert_eq!(samples[1].unit(), Unit::CELSIUS);
    assert!(facts[0].sequence() < facts[1].sequence());
}

#[test]
fn required_recording_failure_fails_warming_controller_and_latches_output_without_rearm() {
    let mut runtime = fixture_with_warmup(2);
    runtime.require_recording(Duration::ZERO);
    assert_eq!(
        runtime.command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        }),
        Err(Error::RecordingUnavailable)
    );
    runtime
        .confirm_recording_start(Duration::ZERO, Duration::ZERO)
        .unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument: PLANT,
            parameter: lab_core::TEMPERATURE,
            at: Duration::ZERO,
        })
        .unwrap();
    runtime
        .command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::ZERO,
        })
        .unwrap();
    let QueryResult::Controller(warming) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    assert_eq!(warming.state, ControllerState::Warming);
    runtime.recording_failure(Duration::from_millis(10));
    let QueryResult::Controller(failed) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
    else {
        panic!()
    };
    assert_eq!(failed.state, ControllerState::Failed);
    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_millis(10),
        })
        .unwrap();
    let QueryResult::Output(output) = runtime
        .query(Query::Output(ActuatorId::new(
            PLANT,
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.state, OutputState::FaultLatched);
    assert!(output.lease.is_none());
    assert_eq!(
        runtime.command(Command::ResumeController {
            controller: CONTROLLER,
            at: Duration::from_millis(20),
        }),
        Err(Error::RecordingUnavailable)
    );
}

#[test]
fn required_gate_expires_at_exact_two_second_progress_deadline() {
    let mut runtime = fixture();
    runtime.require_recording(Duration::ZERO);
    runtime
        .confirm_recording_start(Duration::ZERO, Duration::ZERO)
        .unwrap();
    runtime
        .command(Command::ServiceSafety {
            at: Duration::from_millis(1999),
        })
        .unwrap();
    assert_eq!(
        runtime.command(Command::StartController {
            controller: CONTROLLER,
            at: Duration::from_secs(2),
        }),
        Err(Error::RecordingUnavailable)
    );
    let QueryResult::Output(output) = runtime
        .query(Query::Output(ActuatorId::new(
            PLANT,
            lab_core::HEATER_POWER,
        )))
        .unwrap()
    else {
        panic!()
    };
    assert!(output.fault_latched);
}

#[test]
fn direct_output_ack_has_separate_requested_authorized_send_and_ack_facts() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let at = Duration::from_millis(100);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(77),
                lifetime: Duration::from_secs(1),
            },
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(37.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(200),
            }),
            at,
        })
        .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(dispatch)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::BeginDispatch,
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Complete {
                dispatch_id: dispatch.id(),
                outcome: DispatchOutcome::Acknowledged,
            },
            at,
        })
        .unwrap();
    let stages: Vec<_> = runtime
        .take_recording_facts()
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output { stage, .. } => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        [
            OutputStage::Requested,
            OutputStage::Authorized,
            OutputStage::SendStarted,
            OutputStage::Acknowledged
        ]
    );
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert!(snapshot.acknowledged.is_some());
    assert!(snapshot.readback.is_none());
}

#[test]
fn late_old_dispatch_evidence_cannot_confirm_a_new_safe_epoch() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let at = Duration::from_millis(100);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(78),
                lifetime: Duration::from_secs(1),
            },
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Propose(OutputProposal {
                lease,
                value: Value::Float(37.0),
                unit: Unit::PERCENT,
                ttl: Duration::from_millis(200),
            }),
            at,
        })
        .unwrap();
    let CommandResult::Output(OutputResult::Dispatched(old)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::BeginDispatch,
            at,
        })
        .unwrap()
    else {
        panic!()
    };
    runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::RequestSafe,
            at: Duration::from_millis(101),
        })
        .unwrap();
    for outcome in [
        DispatchOutcome::Acknowledged,
        DispatchOutcome::ReadbackVerified,
    ] {
        let _ = runtime.command(Command::Output {
            actuator,
            command: OutputCommand::Complete {
                dispatch_id: old.id(),
                outcome,
            },
            at: Duration::from_millis(102),
        });
        let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
            panic!()
        };
        assert_eq!(snapshot.state, OutputState::SafePending);
        assert!(!snapshot.safe_confirmed);
        assert!(snapshot.epoch > lease.epoch());
    }
    let CommandResult::Output(OutputResult::Dispatched(safe)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::BeginDispatch,
            at: Duration::from_millis(103),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(safe.is_safe());
    assert!(safe.epoch() > old.epoch());
    assert!(
        runtime
            .command(Command::Output {
                actuator,
                command: OutputCommand::Complete {
                    dispatch_id: old.id(),
                    outcome: DispatchOutcome::ReadbackVerified,
                },
                at: Duration::from_millis(104),
            })
            .is_err()
    );
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.state, OutputState::SafePending);
    assert!(!snapshot.safe_confirmed);
    assert!(snapshot.acknowledged.is_none());
    assert!(snapshot.readback.is_none());
    let evidence: Vec<_> = runtime
        .take_recording_facts()
        .iter()
        .filter_map(|fact| match fact {
            RecordingFact::Output {
                stage,
                authority_epoch,
                dispatch_id,
                ..
            } => Some((*stage, *authority_epoch, *dispatch_id)),
            _ => None,
        })
        .collect();
    assert!(evidence.iter().any(|row| {
        row.0 == OutputStage::Acknowledged
            && row.1 == Some(lease.epoch())
            && row.2 == Some(old.id())
    }));
    assert!(
        evidence
            .iter()
            .all(|row| row.0 != OutputStage::SafeAcknowledged
                && row.0 != OutputStage::SafeReadbackVerified)
    );
}

#[test]
fn delayed_required_failure_notification_still_revokes_a_newer_manual_lease() {
    let mut runtime = fixture();
    runtime.require_recording(Duration::ZERO);
    runtime
        .confirm_recording_start(Duration::ZERO, Duration::ZERO)
        .unwrap();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let CommandResult::Output(OutputResult::Lease(_)) = runtime
        .command(Command::Output {
            actuator,
            command: OutputCommand::Acquire {
                owner: OutputOwner::Manual(88),
                lifetime: Duration::from_secs(1),
            },
            at: Duration::from_secs(1),
        })
        .unwrap()
    else {
        panic!()
    };
    runtime.recording_failure(Duration::ZERO); // stale owner notification
    let QueryResult::Output(output) = runtime.query(Query::Output(actuator)).unwrap() else {
        panic!()
    };
    assert!(output.lease.is_none());
    assert!(output.fault_latched);
}

#[test]
fn committed_pid_and_ramp_revisions_emit_configuration_facts_before_future_outputs() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    let pid = PidConfig {
        kp: 4.0,
        ki: 0.4,
        kd: 0.2,
        output_min: 0.0,
        output_max: 100.0,
    };
    runtime
        .command(Command::ConfigureControllerPid {
            controller: CONTROLLER,
            pid,
            expected_revision: 1,
        })
        .unwrap();
    let ramp = ReferenceId::new(605);
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Ramp {
            id: ramp,
            start: 20.0,
            target: 30.0,
            rate: 1.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        }))
        .unwrap();
    runtime
        .command(Command::RetuneRampReference {
            reference: ramp,
            expected_revision: 1,
            target: 40.0,
            rate: 2.0,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let facts = runtime.take_recording_facts();
    assert!(facts.iter().any(|fact| matches!(fact,
        RecordingFact::Controller {controller,config_revision:2,pid:Some(config),..}
            if *controller==CONTROLLER && *config==pid)));
    assert!(facts.iter().any(|fact| matches!(fact,
        RecordingFact::Reference {reference,revision:2,..} if *reference==ramp)));
    assert!(
        facts
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence())
    );
}
