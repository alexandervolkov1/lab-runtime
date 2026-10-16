//! Exact property lifecycle validation, including revision and bounded metadata.

use super::*;

fn record() -> ConfigurationLifecycleRecord {
    ConfigurationLifecycleRecord {
        operation_id: 1,
        operation_kind: "property_configure",
        base_revision: 7,
        committed_revision: 8,
        toml_hash: [1; 32],
        affected: vec!["instrument:1:configuration_revision:8".into()],
        reason: None,
        at: Duration::from_millis(10),
    }
}

#[test]
fn property_lifecycle_requires_exact_checked_revision_increment() {
    assert!(record().valid());
    for (base, committed) in [(7, 7), (7, 9), (0, 1), (u64::MAX, 0)] {
        let mut record = record();
        record.base_revision = base;
        record.committed_revision = committed;
        assert!(!record.valid(), "{base} -> {committed}");
    }
}

#[test]
fn property_lifecycle_does_not_admit_arbitrary_kinds_or_unbounded_metadata() {
    for kind in [
        "property_configure_extra",
        "configure_property",
        "controller_start",
        "",
    ] {
        let mut record = record();
        record.operation_kind = kind;
        assert!(!record.valid());
    }
    let mut invalid = record();
    invalid.operation_id = 0;
    assert!(!invalid.valid());
    invalid = record();
    invalid.affected = vec!["x".into(); 257];
    assert!(!invalid.valid());
    invalid = record();
    invalid.affected = vec!["x".repeat(129)];
    assert!(!invalid.valid());
    invalid = record();
    invalid.affected = vec![String::new()];
    assert!(!invalid.valid());
    invalid = record();
    invalid.reason = Some("x".repeat(513));
    assert!(!invalid.valid());
    invalid = record();
    invalid.affected[0].reserve(64 * 1024);
    assert!(!invalid.valid());
}
