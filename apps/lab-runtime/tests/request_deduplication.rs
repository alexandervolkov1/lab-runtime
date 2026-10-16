//! Bounded process-local request IDs must never become a silent replay oracle.

use lab_runtime::sessions::{Admission, Mutation, OperationState, SessionError, SessionStore};
use std::time::Duration;

const BOOT: &str = "0123456789abcdef0123456789abcdef";
fn retune(target: f64, revision: u64) -> Mutation {
    Mutation::RetuneRamp {
        reference: 1,
        expected_revision: revision,
        target,
        rate: 2.0,
    }
}

#[test]
fn reconnecting_identical_request_returns_retained_terminal_outcome_without_execution() {
    let mut store = SessionStore::new(BOOT).unwrap();
    let scope = store.open(None, 10, Duration::ZERO).unwrap().scope;
    assert_eq!(
        store.admit(&scope, 1, retune(40.0, 1), Duration::ZERO),
        Admission::Accepted
    );
    store
        .complete(
            &scope,
            1,
            OperationState::Completed("revision=2".into()),
            Duration::ZERO,
        )
        .unwrap();
    store.detach(10, Duration::from_secs(1));
    let returned = store
        .open(Some(&scope), 20, Duration::from_secs(2))
        .unwrap();
    assert_eq!(returned.scope, scope);
    assert_eq!(returned.next_seq, 2);
    assert_eq!(
        store.admit(&scope, 1, retune(40.0, 1), Duration::from_secs(2)),
        Admission::Known(OperationState::Completed("revision=2".into()))
    );
    assert_eq!(
        store.admit(&scope, 1, retune(41.0, 1), Duration::from_secs(2)),
        Admission::Conflict
    );
    assert_eq!(
        store.admit(&scope, 1, retune(40.0, 2), Duration::from_secs(2)),
        Admission::Conflict
    );
}

#[test]
fn evicted_and_ttl_expired_ids_return_unknown_and_can_never_mutate_again() {
    let mut store = SessionStore::new(BOOT).unwrap();
    let scope = store.open(None, 1, Duration::ZERO).unwrap().scope;
    for seq in 1..=34 {
        let now = Duration::from_secs(seq);
        assert_eq!(
            store.admit(&scope, seq, retune(f64::from(seq as u32), seq), now),
            Admission::Accepted
        );
        store
            .complete(
                &scope,
                seq,
                OperationState::Completed(format!("result-{seq}")),
                now,
            )
            .unwrap();
    }
    assert_eq!(
        store.admit(&scope, 1, retune(1.0, 1), Duration::from_secs(35)),
        Admission::Unknown
    );
    assert_eq!(
        store.admit(&scope, 1, retune(99.0, 1), Duration::from_secs(35)),
        Admission::Unknown
    );
    assert_eq!(store.next_seq(&scope).unwrap(), 35);
    let later = Duration::from_secs(34) + Duration::from_secs(600);
    store.expire(later);
    assert_eq!(
        store.admit(&scope, 34, retune(34.0, 34), later),
        Admission::Unknown
    );
    assert_eq!(
        store.admit(&scope, 35, retune(35.0, 35), later),
        Admission::Accepted
    );
}

#[test]
fn pending_work_never_evicted_and_sequence_gaps_capacity_or_live_attachment_fail() {
    let mut store = SessionStore::new(BOOT).unwrap();
    let scope = store.open(None, 1, Duration::ZERO).unwrap().scope;
    assert_eq!(
        store.open(Some(&scope), 2, Duration::ZERO).unwrap_err(),
        SessionError::ScopeInUse
    );
    assert_eq!(
        store.admit(&scope, 2, retune(30.0, 1), Duration::ZERO),
        Admission::Gap
    );
    for seq in 1..=8 {
        assert_eq!(
            store.admit(&scope, seq, retune(30.0 + seq as f64, 1), Duration::ZERO),
            Admission::Accepted
        );
    }
    assert_eq!(
        store.admit(&scope, 9, retune(40.0, 1), Duration::ZERO),
        Admission::Busy
    );
    assert_eq!(store.next_seq(&scope).unwrap(), 9);
    store.detach(1, Duration::from_secs(1));
    assert_eq!(
        store.admit(&scope, 1, retune(31.0, 1), Duration::from_secs(2)),
        Admission::Known(OperationState::Accepted)
    );
    store
        .complete(
            &scope,
            1,
            OperationState::Failed("invalid_config".into()),
            Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(
        store.admit(&scope, 9, retune(40.0, 1), Duration::from_secs(3)),
        Admission::Accepted
    );
}

#[test]
fn scope_expiry_and_restart_identity_do_not_recreate_unknown_mutations() {
    let mut store = SessionStore::new(BOOT).unwrap();
    let scope = store.open(None, 1, Duration::ZERO).unwrap().scope;
    store.detach(1, Duration::ZERO);
    store.expire(Duration::from_secs(1800));
    assert_eq!(
        store
            .open(Some(&scope), 2, Duration::from_secs(1800))
            .unwrap_err(),
        SessionError::ScopeUnknown
    );
    assert_eq!(
        store.admit(&scope, 1, retune(40.0, 1), Duration::from_secs(1800)),
        Admission::ScopeUnknown
    );
    let mut restarted = SessionStore::new("fedcba9876543210fedcba9876543210").unwrap();
    assert_eq!(
        restarted.open(Some(&scope), 4, Duration::ZERO).unwrap_err(),
        SessionError::InstanceChanged
    );
    assert_eq!(
        restarted.admit(&scope, 1, retune(40.0, 1), Duration::ZERO),
        Admission::ScopeUnknown
    );
}

#[test]
fn one_accepted_operation_can_commit_exactly_one_terminal_outcome() {
    let mut store = SessionStore::new(BOOT).unwrap();
    let scope = store.open(None, 1, Duration::ZERO).unwrap().scope;
    assert_eq!(
        store.admit(&scope, 1, retune(40.0, 1), Duration::ZERO),
        Admission::Accepted
    );
    store
        .complete(
            &scope,
            1,
            OperationState::Completed("revision=2".into()),
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(
        store.complete(
            &scope,
            1,
            OperationState::Failed("operation_failed".into()),
            Duration::from_secs(2),
        ),
        Err(SessionError::InvalidOutcome)
    );
    assert_eq!(
        store.status(&scope, 1),
        Admission::Known(OperationState::Completed("revision=2".into()))
    );
}
