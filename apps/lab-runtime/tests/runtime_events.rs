//! Semantic owner events must capture committed changes after individual units.

use lab_core::reference::ReferenceId;
use lab_core::{Command, Query, QueryResult};
use lab_runtime::host::{Clock, HostCore};
use std::time::Duration;

struct Frozen(Duration);
impl Clock for Frozen {
    fn now(&self) -> Duration {
        self.0
    }
}

#[test]
fn pure_queries_emit_no_events_but_retune_publishes_one_ordered_causal_change() {
    let mut owner = HostCore::virtual_demo().unwrap();
    let zero = owner.event_log().latest_cursor();
    for _ in 0..20 {
        owner.query(Query::Reference(ReferenceId::new(1))).unwrap();
    }
    assert_eq!(owner.event_log().latest_cursor(), zero);
    let result = owner
        .command_with_cause(
            Command::RetuneRampReference {
                reference: ReferenceId::new(1),
                target: 40.0,
                rate: 3.0,
                expected_revision: 1,
                at: Duration::ZERO,
            },
            Some(("scope".into(), 1)),
        )
        .unwrap();
    assert!(matches!(
        result,
        lab_core::CommandResult::ReferenceRetuned(_)
    ));
    let events = owner.event_log().after(zero).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "reference" && e["data"]["revision"] == "2")
    );
    let ref_event = events.iter().find(|e| e["kind"] == "reference").unwrap();
    assert_eq!(ref_event["request_id"]["scope"], "scope");
    assert_eq!(ref_event["request_id"]["seq"], "1");
}

#[test]
fn scheduled_observation_and_reference_progress_publish_at_monotonic_cursor() {
    let mut owner = HostCore::virtual_demo().unwrap();
    let before = owner.event_log().latest_cursor();
    owner.service(&Frozen(Duration::from_millis(100))).unwrap();
    let events = owner.event_log().after(before).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "signal" && e["data"]["quality"] == "good")
    );
    assert!(events.iter().any(|e| e["kind"] == "reference"));
    let seqs: Vec<_> = events
        .iter()
        .map(|e| e["seq"].as_str().unwrap().parse::<u64>().unwrap())
        .collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
    let QueryResult::Latest(Some(sample)) = owner
        .query(Query::GetLatestSignal(lab_core::SignalId::new(
            lab_core::InstrumentId::new(1),
            lab_core::TEMPERATURE,
        )))
        .unwrap()
    else {
        panic!("measurement absent")
    };
    assert_eq!(
        events.iter().find(|e| e["kind"] == "signal").unwrap()["data"]["observed_at"],
        sample.at().as_nanos().to_string()
    );
}
