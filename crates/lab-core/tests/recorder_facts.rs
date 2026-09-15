//! M7 facts must retain intermediate domain evidence inside one Core command.

use lab_core::{
    Command, CommandResult, Error, InstrumentId, Query, QueryResult, Runtime, SampleQuality, SignalId, Unit, Value,
    VirtualInstrumentConfig,
    control::{ControllerId, ControllerState, NativeControllerConfig, PidConfig},
    output::{ActuatorId, DispatchOutcome, EvidenceLevel, OutputCommand, OutputOwner, OutputProposal,
        OutputResult, OutputState, SafeProfile},
    plant::ThermalPlantConfig,
    processing::EmaConfig,
    recording::{RecordingFact, OutputStage},
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
    runtime.command(Command::RegisterThermalPlant(ThermalPlantConfig {
        id: PLANT, name: "Recorded plant".into(), history_capacity: 8,
        ambient_temperature: 20.0, initial_temperature: 20.0,
        gain_per_percent: 0.8, time_constant: Duration::from_secs(8),
    })).unwrap();
    let output = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let command = |runtime: &mut Runtime, command| runtime.command(Command::Output {
        actuator: output, command, at: Duration::ZERO,
    }).unwrap();
    command(&mut runtime, OutputCommand::BindProfile(SafeProfile {
        min: 0.0, max: 100.0, safe_value: 0.0,
        max_lease: Duration::from_secs(5),
        max_proposal_ttl: Duration::from_secs(1),
        required_evidence: EvidenceLevel::Readback,
    }));
    command(&mut runtime, OutputCommand::RequestSafe);
    let lab_core::CommandResult::Output(OutputResult::Dispatched(safe)) =
        command(&mut runtime, OutputCommand::BeginDispatch) else { panic!() };
    command(&mut runtime, OutputCommand::Complete {
        dispatch_id: safe.id(), outcome: DispatchOutcome::ReadbackVerified,
    });
    runtime.command(Command::RegisterReference(ReferenceConfig::Fixed {
        id: REFERENCE, value: 60.0, unit: Unit::CELSIUS,
    })).unwrap();
    runtime.command(Command::RegisterController(NativeControllerConfig {
        id: CONTROLLER,
        input: SignalId::new(PLANT, lab_core::TEMPERATURE), output,
        reference: REFERENCE,
        ema: EmaConfig { time_constant: Duration::from_secs(1), warmup_samples, unit: Unit::CELSIUS },
        pid: PidConfig { kp: 3.0, ki: 0.4, kd: 0.2, output_min: 0.0, output_max: 100.0 },
        max_input_age: Duration::from_secs(2),
        max_tick_gap: Duration::from_secs(2),
        lease_lifetime: Duration::from_secs(5),
        proposal_ttl: Duration::from_secs(1),
    })).unwrap();
    runtime.command(Command::PrepareController(CONTROLLER)).unwrap();
    runtime
}

#[test]
fn one_pid_tick_keeps_requested_sent_and_readback_as_separate_ordered_facts() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    runtime.command(Command::RefreshMeasurement {
        instrument: PLANT, parameter: lab_core::TEMPERATURE, at: Duration::ZERO,
    }).unwrap();
    runtime.command(Command::StartController { controller: CONTROLLER, at: Duration::ZERO }).unwrap();
    runtime.take_recording_facts();
    runtime.command(Command::RefreshMeasurement {
        instrument: PLANT, parameter: lab_core::TEMPERATURE, at: Duration::from_secs(1),
    }).unwrap();
    runtime.take_recording_facts();
    runtime.command(Command::TickController {
        controller: CONTROLLER, at: Duration::from_secs(1),
    }).unwrap();
    let facts = runtime.take_recording_facts();
    let stages: Vec<_> = facts.iter().filter_map(|fact| match fact {
        RecordingFact::Output { stage, .. } => Some(*stage), _ => None,
    }).collect();
    assert_eq!(stages, vec![OutputStage::Requested, OutputStage::Authorized,
        OutputStage::SendStarted, OutputStage::ReadbackVerified]);
    assert!(facts.windows(2).all(|pair| pair[0].sequence() < pair[1].sequence()));
}

#[test]
fn recording_fact_outbox_never_accumulates_while_disabled() {
    let mut runtime = fixture();
    runtime.command(Command::RefreshMeasurement {
        instrument: PLANT, parameter: lab_core::TEMPERATURE, at: Duration::ZERO,
    }).unwrap();
    assert!(runtime.take_recording_facts().is_empty());
}

#[test]
fn required_outbox_overflow_closes_authority_before_the_next_ordinary_send() {
    let mut runtime = fixture();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    runtime.require_recording(Duration::ZERO);
    runtime.confirm_recording_start(Duration::ZERO, Duration::ZERO).unwrap();
    runtime.enable_recording_facts();
    for _ in 0..=lab_core::recording::MAX_RECORDING_FACTS {
        runtime.command(Command::EvaluateReference {
            reference: REFERENCE, at: Duration::ZERO,
        }).unwrap();
    }
    assert!(runtime.recording_facts_overflowed());
    assert_eq!(runtime.recording_first_lost_fact(),
        Some((lab_core::recording::MAX_RECORDING_FACTS as u64) + 1));
    assert_eq!(runtime.command(Command::Output {
        actuator, command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(7), lifetime: Duration::from_secs(1),
        }, at: Duration::ZERO,
    }), Err(Error::RecordingUnavailable));
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
    runtime.command(Command::RegisterVirtual(VirtualInstrumentConfig {
        id: VIRTUAL, name: "Failure fixture".into(), history_capacity: 1,
        base_temperature: 20.0, measurement_enabled: true,
    })).unwrap();
    runtime.enable_recording_facts();
    let at = Duration::from_secs(1);
    runtime.command(Command::RefreshMeasurement {
        instrument: VIRTUAL, parameter: lab_core::TEMPERATURE, at,
    }).unwrap();
    runtime.command(Command::ConfigureParameter {
        instrument: VIRTUAL, parameter: lab_core::MEASUREMENT_ENABLED,
        value: Value::Boolean(false),
    }).unwrap();
    let failed_at = Duration::from_secs(2);
    assert!(runtime.command(Command::RefreshMeasurement {
        instrument: VIRTUAL, parameter: lab_core::TEMPERATURE, at: failed_at,
    }).is_err());
    let facts = runtime.take_recording_facts();
    let samples: Vec<_> = facts.iter().filter_map(|fact| match fact {
        RecordingFact::Measurement { sample, .. } => Some(sample), _ => None,
    }).collect();
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
    assert_eq!(runtime.command(Command::StartController {
        controller: CONTROLLER, at: Duration::ZERO,
    }), Err(Error::RecordingUnavailable));
    runtime.confirm_recording_start(Duration::ZERO, Duration::ZERO).unwrap();
    runtime.command(Command::RefreshMeasurement {
        instrument: PLANT, parameter: lab_core::TEMPERATURE, at: Duration::ZERO,
    }).unwrap();
    runtime.command(Command::StartController {
        controller: CONTROLLER, at: Duration::ZERO,
    }).unwrap();
    let QueryResult::Controller(warming) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
        else { panic!() };
    assert_eq!(warming.state, ControllerState::Warming);
    runtime.recording_failure(Duration::from_millis(10));
    let QueryResult::Controller(failed) = runtime.query(Query::Controller(CONTROLLER)).unwrap()
        else { panic!() };
    assert_eq!(failed.state, ControllerState::Failed);
    runtime.command(Command::ServiceSafety { at: Duration::from_millis(10) }).unwrap();
    let QueryResult::Output(output) = runtime.query(Query::Output(ActuatorId::new(
        PLANT, lab_core::HEATER_POWER,
    ))).unwrap() else { panic!() };
    assert_eq!(output.state, OutputState::FaultLatched);
    assert!(output.lease.is_none());
    assert_eq!(runtime.command(Command::ResumeController {
        controller: CONTROLLER, at: Duration::from_millis(20),
    }), Err(Error::RecordingUnavailable));
}

#[test]
fn required_gate_expires_at_exact_two_second_progress_deadline() {
    let mut runtime = fixture();
    runtime.require_recording(Duration::ZERO);
    runtime.confirm_recording_start(Duration::ZERO, Duration::ZERO).unwrap();
    runtime.command(Command::ServiceSafety { at: Duration::from_millis(1999) }).unwrap();
    assert_eq!(runtime.command(Command::StartController {
        controller: CONTROLLER, at: Duration::from_secs(2),
    }), Err(Error::RecordingUnavailable));
    let QueryResult::Output(output) = runtime.query(Query::Output(ActuatorId::new(
        PLANT, lab_core::HEATER_POWER,
    ))).unwrap() else { panic!() };
    assert!(output.fault_latched);
}

#[test]
fn direct_output_ack_has_separate_requested_authorized_send_and_ack_facts() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let at = Duration::from_millis(100);
    let CommandResult::Output(OutputResult::Lease(lease)) = runtime.command(Command::Output {
        actuator, command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(77), lifetime: Duration::from_secs(1),
        }, at,
    }).unwrap() else { panic!() };
    runtime.command(Command::Output {
        actuator, command: OutputCommand::Propose(OutputProposal {
            lease, value: Value::Float(37.0), unit: Unit::PERCENT,
            ttl: Duration::from_millis(200),
        }), at,
    }).unwrap();
    let CommandResult::Output(OutputResult::Dispatched(dispatch)) = runtime.command(Command::Output {
        actuator, command: OutputCommand::BeginDispatch, at,
    }).unwrap() else { panic!() };
    runtime.command(Command::Output {
        actuator, command: OutputCommand::Complete {
            dispatch_id: dispatch.id(), outcome: DispatchOutcome::Acknowledged,
        }, at,
    }).unwrap();
    let stages: Vec<_> = runtime.take_recording_facts().iter().filter_map(|fact| match fact {
        RecordingFact::Output { stage, .. } => Some(*stage), _ => None,
    }).collect();
    assert_eq!(stages, [OutputStage::Requested, OutputStage::Authorized,
        OutputStage::SendStarted, OutputStage::Acknowledged]);
    let QueryResult::Output(snapshot) = runtime.query(Query::Output(actuator)).unwrap() else { panic!() };
    assert!(snapshot.acknowledged.is_some());
    assert!(snapshot.readback.is_none());
}

#[test]
fn delayed_required_failure_notification_still_revokes_a_newer_manual_lease() {
    let mut runtime = fixture();
    runtime.require_recording(Duration::ZERO);
    runtime.confirm_recording_start(Duration::ZERO, Duration::ZERO).unwrap();
    let actuator = ActuatorId::new(PLANT, lab_core::HEATER_POWER);
    let CommandResult::Output(OutputResult::Lease(_)) = runtime.command(Command::Output {
        actuator, command: OutputCommand::Acquire {
            owner: OutputOwner::Manual(88), lifetime: Duration::from_secs(1),
        }, at: Duration::from_secs(1),
    }).unwrap() else { panic!() };
    runtime.recording_failure(Duration::ZERO); // stale owner notification
    let QueryResult::Output(output) = runtime.query(Query::Output(actuator)).unwrap() else { panic!() };
    assert!(output.lease.is_none());
    assert!(output.fault_latched);
}

#[test]
fn committed_pid_and_ramp_revisions_emit_configuration_facts_before_future_outputs() {
    let mut runtime = fixture();
    runtime.enable_recording_facts();
    let pid = PidConfig {kp:4.0,ki:0.4,kd:0.2,output_min:0.0,output_max:100.0};
    runtime.command(Command::ConfigureControllerPid {
        controller: CONTROLLER, pid, expected_revision:1,
    }).unwrap();
    let ramp = ReferenceId::new(605);
    runtime.command(Command::RegisterReference(ReferenceConfig::Ramp {
        id:ramp,start:20.0,target:30.0,rate:1.0,unit:Unit::CELSIUS,
        at:Duration::ZERO,
    })).unwrap();
    runtime.command(Command::RetuneRampReference {
        reference:ramp,expected_revision:1,target:40.0,rate:2.0,
        at:Duration::from_secs(1),
    }).unwrap();
    let facts = runtime.take_recording_facts();
    assert!(facts.iter().any(|fact| matches!(fact,
        RecordingFact::Controller {controller,config_revision:2,pid:Some(config),..}
            if *controller==CONTROLLER && *config==pid)));
    assert!(facts.iter().any(|fact| matches!(fact,
        RecordingFact::Reference {reference,revision:2,..} if *reference==ramp)));
    assert!(facts.windows(2).all(|pair|pair[0].sequence()<pair[1].sequence()));
}
