//! Acceptance contracts for one authority and explicitly stepped simulated effects.
use lab_core::{output::*, *};
use std::time::Duration;

const INSTRUMENT: InstrumentId = InstrumentId::new(1);
fn actuator() -> ActuatorId { ActuatorId::new(INSTRUMENT, HEATER_POWER) }
fn time(ms: u64) -> Duration { Duration::from_millis(ms) }
fn action(runtime: &mut Runtime, command: OutputCommand, ms: u64) -> Result<OutputResult, Error> {
    match runtime.command(Command::Output { actuator: actuator(), command, at: time(ms) })? {
        CommandResult::Output(result) => Ok(result),
        other => panic!("unexpected {other:?}"),
    }
}
fn snapshot(runtime: &Runtime) -> OutputSnapshot {
    match runtime.query(Query::Output(actuator())).unwrap() {
        QueryResult::Output(value) => value, other => panic!("unexpected {other:?}"),
    }
}
fn setup(safe: f64) -> Runtime {
    let mut runtime = Runtime::new();
    runtime.command(Command::RegisterVirtual(VirtualInstrumentConfig {
        id: INSTRUMENT, name: "fixture".into(), history_capacity: 4,
        base_temperature: 20.0, measurement_enabled: true,
    })).unwrap();
    action(&mut runtime, OutputCommand::BindProfile(SafeProfile {
        min: 0.0, max: 100.0, safe_value: safe,
        max_lease: time(1000), max_proposal_ttl: time(100),
        required_evidence: EvidenceLevel::Readback,
    }), 0).unwrap();
    runtime
}
fn dispatch(runtime: &mut Runtime, ms: u64) -> Dispatch {
    match action(runtime, OutputCommand::BeginDispatch, ms).unwrap() {
        OutputResult::Dispatched(dispatch) => dispatch, other => panic!("unexpected {other:?}"),
    }
}
fn complete(runtime: &mut Runtime, dispatch: Dispatch, outcome: DispatchOutcome, ms: u64) {
    action(runtime, OutputCommand::Complete { dispatch_id: dispatch.id(), outcome }, ms).unwrap();
}
fn make_safe(runtime: &mut Runtime, ms: u64) {
    action(runtime, OutputCommand::RequestSafe, ms).unwrap();
    let sent = dispatch(runtime, ms);
    assert!(sent.is_safe());
    complete(runtime, sent, DispatchOutcome::ReadbackVerified, ms);
}
fn acquire(runtime: &mut Runtime, owner: OutputOwner, ms: u64) -> OutputLease {
    match action(runtime, OutputCommand::Acquire { owner, lifetime: time(1000) }, ms).unwrap() {
        OutputResult::Lease(lease) => lease, other => panic!("unexpected {other:?}"),
    }
}
fn propose(runtime: &mut Runtime, lease: OutputLease, value: Value, unit: Unit, ms: u64) -> Result<OutputResult, Error> {
    action(runtime, OutputCommand::Propose(OutputProposal {
        lease, value, unit, ttl: time(100),
    }), ms)
}

#[test]
fn unverified_and_disarmed_have_no_ordinary_output_authority() {
    let mut runtime = setup(7.0);
    assert_eq!(snapshot(&runtime).state, OutputState::Unverified);
    assert!(action(&mut runtime, OutputCommand::BeginDispatch, 0).is_err());
    assert!(action(&mut runtime, OutputCommand::Acquire { owner: OutputOwner::Manual(1), lifetime: time(100) }, 0).is_err());
    action(&mut runtime, OutputCommand::RequestSafe, 0).unwrap();
    assert_eq!(snapshot(&runtime).state, OutputState::SafePending);
    let sent = dispatch(&mut runtime, 1);
    assert_eq!(sent.value(), 7.0); // Safe is an explicit fixture choice, not universally zero.
    assert_eq!(snapshot(&runtime).state, OutputState::SafePending);
    complete(&mut runtime, sent, DispatchOutcome::ReadbackVerified, 2);
    assert_eq!(snapshot(&runtime).state, OutputState::Disarmed);
    assert!(snapshot(&runtime).lease.is_none());
    assert!(action(&mut runtime, OutputCommand::BeginDispatch, 2).is_err());
}

#[test]
fn one_owner_and_one_bounded_pending_proposal() {
    let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
    let lease = acquire(&mut runtime, OutputOwner::Automatic(1), 0);
    assert!(action(&mut runtime, OutputCommand::Acquire { owner: OutputOwner::Manual(2), lifetime: time(100) }, 0).is_err());
    propose(&mut runtime, lease, Value::Float(40.0), Unit::Percent, 1).unwrap();
    assert!(propose(&mut runtime, lease, Value::Float(50.0), Unit::Percent, 1).is_err());
    let sent = dispatch(&mut runtime, 2);
    assert_eq!(sent.value(), 40.0);
    assert!(action(&mut runtime, OutputCommand::BeginDispatch, 2).is_err());
}

#[test]
fn both_modes_share_strict_validation_and_generic_setter_is_not_a_bypass() {
    for owner in [OutputOwner::Manual(1), OutputOwner::Automatic(2)] {
        let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
        let lease = acquire(&mut runtime, owner, 0);
        for (value, unit) in [
            (Value::Boolean(true), Unit::Percent), (Value::Float(-1.0), Unit::Percent),
            (Value::Float(101.0), Unit::Percent), (Value::Float(f64::NAN), Unit::Percent),
            (Value::Float(f64::INFINITY), Unit::Percent), (Value::Float(5.0), Unit::Celsius),
        ] {
            assert!(propose(&mut runtime, lease, value, unit, 0).is_err());
            assert!(!snapshot(&runtime).pending);
        }
        assert_eq!(runtime.command(Command::ConfigureParameter {
            instrument: INSTRUMENT, parameter: HEATER_POWER, value: Value::Float(5.0)
        }), Err(Error::OperationNotAllowed(HEATER_POWER)));
        propose(&mut runtime, lease, Value::Float(5.0), Unit::Percent, 0).unwrap();
        let sent = dispatch(&mut runtime, 0);
        complete(&mut runtime, sent, DispatchOutcome::Acknowledged, 0);
        assert_eq!(snapshot(&runtime).acknowledged.unwrap().value, 5.0);
        assert!(snapshot(&runtime).readback.is_none());
    }
}

#[test]
fn expiry_and_revoke_fence_queued_work_at_final_send() {
    for revoke in [false, true] {
        let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
        let lease = acquire(&mut runtime, OutputOwner::Automatic(1), 0);
        propose(&mut runtime, lease, Value::Float(90.0), Unit::Percent, 0).unwrap();
        if revoke { action(&mut runtime, OutputCommand::Release(lease), 1).unwrap(); }
        let now = if revoke { 1 } else { 100 };
        // Either reject the stale candidate or dispatch only the reserved safe operation.
        if let Ok(OutputResult::Dispatched(sent)) = action(&mut runtime, OutputCommand::BeginDispatch, now) {
            assert!(sent.is_safe());
            assert_eq!(sent.value(), 0.0);
        }
        assert_ne!(snapshot(&runtime).sent.map(|value| value.value), Some(90.0));
    }
    let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
    let lease = acquire(&mut runtime, OutputOwner::Manual(1), 0);
    assert!(propose(&mut runtime, lease, Value::Float(10.0), Unit::Percent, 1000).is_err());
    assert!(snapshot(&runtime).lease.is_none());
    assert_eq!(snapshot(&runtime).state, OutputState::SafePending);
}

#[test]
fn old_epoch_and_late_result_cannot_restore_authority_or_confirm_new_safe() {
    let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
    let old = acquire(&mut runtime, OutputOwner::Automatic(1), 0);
    propose(&mut runtime, old, Value::Float(90.0), Unit::Percent, 0).unwrap();
    let ordinary = dispatch(&mut runtime, 1);
    action(&mut runtime, OutputCommand::Release(old), 2).unwrap();
    assert!(action(&mut runtime, OutputCommand::BeginDispatch, 2).is_err());
    complete(&mut runtime, ordinary, DispatchOutcome::ReadbackVerified, 3);
    assert_eq!(snapshot(&runtime).state, OutputState::SafePending);
    assert!(snapshot(&runtime).lease.is_none());
    let safe = dispatch(&mut runtime, 3);
    complete(&mut runtime, safe, DispatchOutcome::ReadbackVerified, 4);
    let new = acquire(&mut runtime, OutputOwner::Manual(2), 4);
    assert_ne!(old.epoch(), new.epoch());
    assert!(propose(&mut runtime, old, Value::Float(80.0), Unit::Percent, 4).is_err());
    assert!(action(&mut runtime, OutputCommand::Complete {
        dispatch_id: ordinary.id(), outcome: DispatchOutcome::ReadbackVerified
    }, 4).is_err());
    assert_eq!(snapshot(&runtime).lease, Some(new));
}

#[test]
fn safe_failure_ambiguity_and_insufficient_evidence_never_become_disarmed() {
    for outcome in [DispatchOutcome::Failed, DispatchOutcome::Ambiguous, DispatchOutcome::Acknowledged] {
        let mut runtime = setup(0.0);
        action(&mut runtime, OutputCommand::RequestSafe, 0).unwrap();
        let sent = dispatch(&mut runtime, 0);
        complete(&mut runtime, sent, outcome, 1);
        assert_eq!(snapshot(&runtime).state, OutputState::FaultLatched);
        assert!(action(&mut runtime, OutputCommand::AcknowledgeFault, 1).is_err());
        make_safe(&mut runtime, 2);
        assert_eq!(snapshot(&runtime).state, OutputState::FaultLatched);
        action(&mut runtime, OutputCommand::AcknowledgeFault, 2).unwrap();
        assert_eq!(snapshot(&runtime).state, OutputState::Disarmed);
        assert!(snapshot(&runtime).lease.is_none());
    }
}

#[test]
fn lease_watchdog_release_and_fault_are_explicit_not_query_side_effects() {
    let mut runtime = setup(0.0); make_safe(&mut runtime, 0);
    acquire(&mut runtime, OutputOwner::Manual(1), 0);
    let before = snapshot(&runtime);
    for _ in 0..20 { assert_eq!(snapshot(&runtime), before); }
    action(&mut runtime, OutputCommand::Tick, 1000).unwrap();
    assert_eq!(snapshot(&runtime).state, OutputState::SafePending);
    assert!(!snapshot(&runtime).safe_confirmed);
    let safe = dispatch(&mut runtime, 1000);
    complete(&mut runtime, safe, DispatchOutcome::ReadbackVerified, 1000);
    acquire(&mut runtime, OutputOwner::Automatic(2), 1000);
    action(&mut runtime, OutputCommand::Trip, 1000).unwrap();
    assert!(snapshot(&runtime).lease.is_none());
    assert!(snapshot(&runtime).fault_latched);
}

#[test]
fn invalid_profiles_time_and_unknown_output_are_rejected() {
    let mut runtime = setup(0.0);
    assert!(action(&mut runtime, OutputCommand::BindProfile(SafeProfile {
        min: 0.0, max: 100.0, safe_value: -1.0, max_lease: time(1000),
        max_proposal_ttl: time(100), required_evidence: EvidenceLevel::Readback,
    }), 0).is_err());
    action(&mut runtime, OutputCommand::Tick, 10).unwrap();
    assert!(action(&mut runtime, OutputCommand::Tick, 9).is_err());
    assert!(runtime.query(Query::Output(ActuatorId::new(INSTRUMENT, BASE_TEMPERATURE))).is_err());
}
