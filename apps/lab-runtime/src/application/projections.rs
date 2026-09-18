//! Shared public projections used by Application queries and semantic events.
//!
//! These functions translate committed Core snapshots into deliberate bounded JSON
//! views. They do not own controller, Reference, output, or time state.

use lab_core::control::{ControllerSnapshot, ControllerState};
use lab_core::output::{DispatchOutcome, OutputOwner, OutputSnapshot, OutputState};
use lab_core::reference::ReferenceSnapshot;
use serde_json::{Value, json};

pub(crate) fn controller_json(snapshot: ControllerSnapshot) -> Value {
    json!({"controller":snapshot.id.get().to_string(),"kind":"pid","state":controller_state_name(snapshot.state),
        "status":if snapshot.state==ControllerState::Failed{"failed"}else{"valid"},
        "failure":if snapshot.state==ControllerState::Failed{Some("latched")}else{None},
        "active":matches!(snapshot.state,ControllerState::Warming|ControllerState::Running),
        "paused":snapshot.state==ControllerState::Paused,
        "revision":snapshot.config_revision.to_string(),"last_tick":snapshot.last_tick.map(nanos),
        "latest_output":snapshot.latest_output.map(|value|value.output)})
}

pub(crate) fn controller_projection_json(
    snapshot: ControllerSnapshot,
    config: lab_core::control::NativeControllerConfig,
) -> Value {
    let mut view = controller_json(snapshot);
    view["bindings"] = json!({
        "input":{"instrument":config.input.instrument().get().to_string(),
            "parameter":config.input.parameter().get().to_string()},
        "reference":{"id":config.reference.get().to_string()},
        "output":{"instrument":config.output.instrument().get().to_string(),
            "parameter":config.output.parameter().get().to_string()}
    });
    view["config"] = json!({
        "pid":{"kp":config.pid.kp,"ki":config.pid.ki,"kd":config.pid.kd,
            "output_min":config.pid.output_min,"output_max":config.pid.output_max},
        "max_input_age_ns":nanos(config.max_input_age),"max_tick_gap_ns":nanos(config.max_tick_gap),
        "lease_lifetime_ns":nanos(config.lease_lifetime),"proposal_ttl_ns":nanos(config.proposal_ttl),
        "ema":{"time_constant_ns":nanos(config.ema.time_constant),
            "warmup_samples":config.ema.warmup_samples.to_string(),
            "unit":{"id":config.ema.unit.id(),"symbol":config.ema.unit.symbol()}}
    });
    view
}

pub(crate) fn output_json(snapshot: OutputSnapshot) -> Value {
    let owner = snapshot.lease.map(|lease| match lease.owner() {
        OutputOwner::Manual(id) => json!({"kind":"manual","id":id.to_string()}),
        OutputOwner::Automatic(id) => json!({"kind":"automatic","id":id.to_string()}),
    });
    let observation = |observation: Option<lab_core::output::OutputObservation>| {
        observation.map(|value| json!({"value":value.value,"at":nanos(value.at)}))
    };
    json!({"state":match snapshot.state {OutputState::Unverified=>"unverified",OutputState::SafePending=>"safe_pending",
            OutputState::Disarmed=>"disarmed",OutputState::ArmedManual=>"armed_manual",OutputState::ArmedAuto=>"armed_auto",OutputState::FaultLatched=>"fault_latched"},
        "owner":owner,"instance":snapshot.lease.map(|lease|lease.instance().to_string()),"epoch":snapshot.epoch.to_string(),"lease_expires_at":snapshot.lease.map(|lease|nanos(lease.expires())),
        "fault_latched":snapshot.fault_latched,"safe_confirmed":snapshot.safe_confirmed,"pending":snapshot.pending,
        "in_flight":snapshot.in_flight.is_some(),"requested":snapshot.requested,"sent":observation(snapshot.sent),
        "acknowledged":observation(snapshot.acknowledged),"readback":observation(snapshot.readback),
        "reported_readback":observation(snapshot.reported_readback),
        "readback_failure":snapshot.readback_failure.map(|failure|match failure {
            lab_core::output::OutputReadbackFailure::Mismatch=>"mismatch",
            lab_core::output::OutputReadbackFailure::Unavailable=>"unavailable"}),
        "outcome":snapshot.outcome.map(|outcome|match outcome {DispatchOutcome::Acknowledged=>"acknowledged",DispatchOutcome::ReadbackVerified=>"readback_verified",
            DispatchOutcome::Failed=>"failed",DispatchOutcome::Ambiguous=>"ambiguous"})})
}

pub(crate) fn reference_json(id: u64, snapshot: ReferenceSnapshot) -> Value {
    match snapshot {
        ReferenceSnapshot::Fixed {
            value,
            unit,
            last_at,
            revision,
            ..
        } => json!({"reference":id.to_string(),"kind":"fixed","value":value,
            "revision":revision.to_string(),"status":"valid","configurable":true,
            "last_at":last_at.map(nanos),"last_evaluated_at_ns":last_at.map(nanos),
            "unit":{"id":unit.id().to_string(),"symbol":unit.symbol()}}),
        ReferenceSnapshot::Ramp {
            state, revision, ..
        } => json!({"reference":id.to_string(),"kind":"ramp","value":state.current,
            "target":state.target,"rate":state.rate,"revision":revision.to_string(),
            "status":"valid","configurable":true,"last_at":nanos(state.last_at),
            "last_evaluated_at_ns":nanos(state.last_at),
            "unit":{"id":state.unit.id().to_string(),"symbol":state.unit.symbol()}}),
    }
}

pub(crate) const fn controller_state_name(state: ControllerState) -> &'static str {
    match state {
        ControllerState::Created => "created",
        ControllerState::Ready => "ready",
        ControllerState::Warming => "warming",
        ControllerState::Running => "running",
        ControllerState::Paused => "paused",
        ControllerState::Failed => "failed",
    }
}

pub(crate) const fn quality_name(quality: lab_core::SampleQuality) -> &'static str {
    match quality {
        lab_core::SampleQuality::Good => "good",
        lab_core::SampleQuality::Unavailable => "unavailable",
    }
}

pub(crate) fn nanos(at: std::time::Duration) -> String {
    at.as_nanos().to_string()
}
