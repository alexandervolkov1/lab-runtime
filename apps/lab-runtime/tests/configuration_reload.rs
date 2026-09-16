//! C5-C8 acceptance for staged configuration lifecycle and no-rearm failure.

use lab_runtime::{
    configuration::{ArtifactReader, ConfigurationError, FrozenDeployment, parse_runtime_toml},
    deployment::{ApplyError, ApplyPort, ApplyResult, DeploymentLifecycle, DiffEffect, StageError},
};
use std::{path::Path, time::Duration};

#[derive(Default)]
struct NoArtifacts;

impl ArtifactReader for NoArtifacts {
    fn read(&mut self, _: &Path, _: usize) -> Result<Vec<u8>, ConfigurationError> {
        panic!("virtual fixture must not read artifacts")
    }
}

fn deployment(name: &str, poll_ms: u64, ambient: f64, port: u16) -> FrozenDeployment {
    let bytes = format!(
        r#"schema_version=1
[runtime]
key="bench"
display_name="{name}"
[server]
host="127.0.0.1"
port={port}
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=1
key="plant"
kind="thermal_plant"
display_name="{name} plant"
history_capacity=32
ambient_temperature={ambient}
initial_temperature=20.0
gain_per_percent=0.8
time_constant_ms=8000
poll_period_ms={poll_ms}
"#
    );
    parse_runtime_toml(bytes.as_bytes(), Path::new("C:/lab"), &mut NoArtifacts).unwrap()
}

#[derive(Default)]
struct ProbePort {
    armed: bool,
    safe_succeeds: bool,
    calls: Vec<&'static str>,
    committed_hash: Option<[u8; 32]>,
}

impl ApplyPort for ProbePort {
    fn enter_safe_barrier(&mut self, _at: Duration) -> Result<bool, ApplyError> {
        self.calls.push("barrier");
        self.armed = false;
        Ok(self.safe_succeeds)
    }

    fn prepare_bindings(&mut self, _: &FrozenDeployment) -> Result<(), ApplyError> {
        self.calls.push("bindings");
        Ok(())
    }

    fn commit_configuration(&mut self, candidate: &FrozenDeployment) -> Result<(), ApplyError> {
        self.calls.push("commit");
        self.committed_hash = Some(candidate.toml_hash());
        Ok(())
    }
}

#[test]
fn c5_live_change_commits_once_with_explicit_diff() {
    let mut lifecycle = DeploymentLifecycle::new(deployment("old", 100, 20.0, 7420));
    let staged = lifecycle
        .stage(deployment("new", 250, 20.0, 7420), Duration::ZERO)
        .unwrap();
    assert!(staged.diff().effects().contains(&DiffEffect::LiveSafe));
    assert!(staged.diff().effects().contains(&DiffEffect::OrdinaryLive));
    assert!(!staged.diff().requires_safe_barrier());
    let mut port = ProbePort {
        armed: true,
        safe_succeeds: true,
        ..ProbePort::default()
    };

    let result = lifecycle
        .apply(
            staged.id(),
            staged.base_revision(),
            Duration::from_secs(1),
            &mut port,
        )
        .unwrap();

    assert_eq!(result, ApplyResult::Applied { revision: 2 });
    assert_eq!(lifecycle.revision(), 2);
    assert_eq!(port.calls, ["commit"]);
    assert!(
        port.armed,
        "live-safe apply must not invent a safety transition"
    );
}

#[test]
fn c6_c7_failed_safe_barrier_keeps_old_config_but_never_rearms() {
    let old = deployment("bench", 100, 20.0, 7420);
    let old_hash = old.toml_hash();
    let mut lifecycle = DeploymentLifecycle::new(old);
    let staged = lifecycle
        .stage(deployment("bench", 100, 25.0, 7420), Duration::ZERO)
        .unwrap();
    assert!(staged.diff().requires_safe_barrier());
    let mut port = ProbePort {
        armed: true,
        safe_succeeds: false,
        ..ProbePort::default()
    };

    let result = lifecycle
        .apply(
            staged.id(),
            staged.base_revision(),
            Duration::from_secs(1),
            &mut port,
        )
        .unwrap();

    assert_eq!(result, ApplyResult::FailedBeforeCommit);
    assert_eq!(lifecycle.revision(), 1);
    assert_eq!(lifecycle.active().toml_hash(), old_hash);
    assert_eq!(port.calls, ["barrier"]);
    assert!(!port.armed, "barrier failure must not roll authority back");
    assert_eq!(port.committed_hash, None);
}

#[test]
fn c8_stale_expired_and_restart_required_candidates_never_touch_runtime() {
    let mut lifecycle = DeploymentLifecycle::new(deployment("old", 100, 20.0, 7420));
    let staged = lifecycle
        .stage(deployment("new", 100, 20.0, 7420), Duration::ZERO)
        .unwrap();
    let mut port = ProbePort::default();
    assert_eq!(
        lifecycle.apply(staged.id(), 9, Duration::from_secs(1), &mut port),
        Err(ApplyError::Conflict)
    );
    assert_eq!(
        lifecycle.apply(
            staged.id(),
            staged.base_revision(),
            Duration::from_secs(31),
            &mut port
        ),
        Err(ApplyError::Expired)
    );
    assert!(port.calls.is_empty());

    let staged = lifecycle
        .stage(deployment("old", 100, 20.0, 8000), Duration::from_secs(32))
        .unwrap();
    assert!(
        staged
            .diff()
            .effects()
            .contains(&DiffEffect::RestartRequired)
    );
    assert_eq!(
        lifecycle.apply(
            staged.id(),
            staged.base_revision(),
            Duration::from_secs(33),
            &mut port
        ),
        Err(ApplyError::RestartRequired)
    );
    assert!(port.calls.is_empty());

    assert_eq!(
        lifecycle.stage(
            deployment("another", 100, 20.0, 7420),
            Duration::from_secs(33)
        ),
        Err(StageError::Busy)
    );
}
