//! M6 domain operations for an independently scheduled and retuned Reference.

use lab_core::{
    Command, CommandResult, Query, QueryResult, Runtime, Unit,
    reference::{ReferenceConfig, ReferenceId, ReferenceSnapshot},
};
use std::time::Duration;

const ID: ReferenceId = ReferenceId::new(7);

fn ramp_state(runtime: &Runtime) -> (lab_core::reference::RampSnapshot, u64) {
    let QueryResult::Reference(ReferenceSnapshot::Ramp {
        state, revision, ..
    }) = runtime.query(Query::Reference(ID)).unwrap()
    else {
        panic!("expected Ramp")
    };
    (state, revision)
}

#[test]
fn live_retune_commits_at_the_old_ramp_value_without_a_jump() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Ramp {
            id: ID,
            start: 20.0,
            target: 60.0,
            rate: 2.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        }))
        .unwrap();
    assert_eq!(ramp_state(&runtime).1, 1);
    let CommandResult::ReferenceEvaluated(first) = runtime
        .command(Command::EvaluateReference {
            reference: ID,
            at: Duration::from_secs(3),
        })
        .unwrap()
    else {
        panic!("expected evaluated Reference")
    };
    assert_eq!(first.value, 26.0);
    assert_eq!(ramp_state(&runtime).1, 1);
    let CommandResult::ReferenceRetuned(committed) = runtime
        .command(Command::RetuneRampReference {
            reference: ID,
            target: 10.0,
            rate: 4.0,
            expected_revision: 1,
            at: Duration::from_secs(5),
        })
        .unwrap()
    else {
        panic!("expected retuned Reference")
    };
    assert_eq!(committed.state.current, 30.0);
    assert_eq!(committed.state.target, 10.0);
    assert_eq!(committed.state.unit, Unit::CELSIUS);
    assert_eq!(committed.revision, 2);
    let (state, revision) = ramp_state(&runtime);
    assert_eq!(state, committed.state);
    assert_eq!(revision, 2);
    let CommandResult::ReferenceEvaluated(next) = runtime
        .command(Command::EvaluateReference {
            reference: ID,
            at: Duration::from_secs(7),
        })
        .unwrap()
    else {
        panic!("expected evaluated Reference")
    };
    assert_eq!(next.value, 22.0);
    assert_eq!(ramp_state(&runtime).1, 2);
}

#[test]
fn invalid_or_conflicting_retune_leaves_reference_state_and_revision_unchanged() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Ramp {
            id: ID,
            start: 0.0,
            target: 50.0,
            rate: 1.0,
            unit: Unit::CELSIUS,
            at: Duration::ZERO,
        }))
        .unwrap();
    runtime
        .command(Command::EvaluateReference {
            reference: ID,
            at: Duration::from_secs(1),
        })
        .unwrap();
    let before = ramp_state(&runtime);
    for (target, rate, expected_revision, at) in [
        (f64::NAN, 1.0, 1, Duration::from_secs(2)),
        (10.0, 0.0, 1, Duration::from_secs(2)),
        (10.0, 2.0, 2, Duration::from_secs(2)),
        (10.0, 2.0, 1, Duration::ZERO),
    ] {
        assert!(
            runtime
                .command(Command::RetuneRampReference {
                    reference: ID,
                    target,
                    rate,
                    expected_revision,
                    at,
                })
                .is_err()
        );
        assert_eq!(ramp_state(&runtime), before);
    }
    let QueryResult::Reference(copy) = runtime.query(Query::Reference(ID)).unwrap() else {
        panic!()
    };
    assert_eq!(
        runtime.query(Query::Reference(ID)).unwrap(),
        QueryResult::Reference(copy)
    );
}

#[test]
fn fixed_reference_can_be_evaluated_but_not_retuned() {
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterReference(ReferenceConfig::Fixed {
            id: ID,
            value: 40.0,
            unit: Unit::CELSIUS,
        }))
        .unwrap();
    runtime
        .command(Command::EvaluateReference {
            reference: ID,
            at: Duration::from_secs(5),
        })
        .unwrap();
    assert!(
        runtime
            .command(Command::RetuneRampReference {
                reference: ID,
                target: 45.0,
                rate: 1.0,
                expected_revision: 1,
                at: Duration::from_secs(5),
            })
            .is_err()
    );
    let QueryResult::Reference(ReferenceSnapshot::Fixed {
        value, revision, ..
    }) = runtime.query(Query::Reference(ID)).unwrap()
    else {
        panic!()
    };
    assert_eq!(value, 40.0);
    assert_eq!(revision, 1);
}
