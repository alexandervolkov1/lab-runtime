//! M4 acceptance for readable EMA, Reference and PID mathematics.

use lab_core::{
    SampleQuality, Unit,
    control::{Pid, PidConfig},
    processing::{Ema, EmaConfig, EmaStatus},
    reference::{FixedReference, RampReference},
};
use std::time::Duration;

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1.0e-10, "{actual} != {expected}");
}

#[test]
fn ema_uses_uneven_dt_and_explicit_warmup() {
    let mut ema = Ema::new(EmaConfig {
        time_constant: Duration::from_secs(2),
        warmup_samples: 2,
        unit: Unit::CELSIUS,
    }).unwrap();
    let first = ema.update(Some(10.0), SampleQuality::Good, Unit::CELSIUS, Duration::ZERO).unwrap();
    assert_eq!(first.status, EmaStatus::WarmingUp);
    close(first.value.unwrap(), 10.0);

    let second = ema.update(
        Some(20.0),
        SampleQuality::Good,
        Unit::CELSIUS,
        Duration::from_secs(2),
    ).unwrap();
    assert_eq!(second.status, EmaStatus::Ready);
    close(second.value.unwrap(), 10.0 + (1.0 - (-1.0_f64).exp()) * 10.0);

    let third = ema.update(
        Some(30.0),
        SampleQuality::Good,
        Unit::CELSIUS,
        Duration::from_secs(3),
    ).unwrap();
    assert!(third.value.unwrap().is_finite());
    assert!(third.value.unwrap() > second.value.unwrap());
}

#[test]
fn ema_unavailable_and_invalid_inputs_do_not_mutate_state() {
    let mut ema = Ema::new(EmaConfig {
        time_constant: Duration::from_secs(1),
        warmup_samples: 1,
        unit: Unit::CELSIUS,
    }).unwrap();
    ema.update(Some(10.0), SampleQuality::Good, Unit::CELSIUS, Duration::ZERO).unwrap();
    let before = ema.snapshot();
    let unavailable = ema.update(
        None,
        SampleQuality::Unavailable,
        Unit::CELSIUS,
        Duration::from_secs(1),
    ).unwrap();
    assert_eq!(unavailable.status, EmaStatus::Unavailable);
    assert_eq!(ema.snapshot(), before);
    assert!(ema.update(Some(f64::NAN), SampleQuality::Good, Unit::CELSIUS, Duration::from_secs(1)).is_err());
    assert!(ema.update(Some(1.0), SampleQuality::Good, Unit::PERCENT, Duration::from_secs(1)).is_err());
    assert_eq!(ema.snapshot(), before);

    ema.retune(Duration::from_secs(4)).unwrap();
    assert_eq!(ema.snapshot().value, before.value);
    assert_eq!(ema.snapshot().last_at, before.last_at);
}

#[test]
fn fixed_and_ramp_references_use_only_monotonic_time() {
    let mut fixed = FixedReference::new(42.0, Unit::CELSIUS).unwrap();
    assert_eq!(fixed.value_at(Duration::ZERO).unwrap().value, 42.0);
    assert_eq!(fixed.value_at(Duration::from_secs(100)).unwrap().value, 42.0);

    let mut up = RampReference::new(
        0.0,
        10.0,
        2.0,
        Unit::CELSIUS,
        Duration::ZERO,
    ).unwrap();
    close(up.value_at(Duration::from_millis(2500)).unwrap().value, 5.0);
    close(up.value_at(Duration::from_secs(20)).unwrap().value, 10.0);

    let mut down = RampReference::new(
        10.0,
        -2.0,
        3.0,
        Unit::CELSIUS,
        Duration::ZERO,
    ).unwrap();
    close(down.value_at(Duration::from_secs(1)).unwrap().value, 7.0);
    close(down.value_at(Duration::from_secs(20)).unwrap().value, -2.0);
}

#[test]
fn ramp_retune_is_continuous_and_invalid_time_is_atomic() {
    let mut ramp = RampReference::new(
        0.0,
        10.0,
        2.0,
        Unit::CELSIUS,
        Duration::ZERO,
    ).unwrap();
    close(ramp.value_at(Duration::from_secs(2)).unwrap().value, 4.0);
    ramp.retune(-2.0, 3.0, Duration::from_secs(2)).unwrap();
    close(ramp.snapshot().current, 4.0);
    close(ramp.value_at(Duration::from_secs(3)).unwrap().value, 1.0);
    let before = ramp.snapshot();
    assert!(ramp.retune(5.0, 0.0, Duration::from_secs(4)).is_err());
    assert!(ramp.value_at(Duration::from_secs(2)).is_err());
    assert_eq!(ramp.snapshot(), before);
}

#[test]
fn pid_uses_actual_dt_and_derivative_on_measurement() {
    let mut pid = Pid::new(PidConfig {
        kp: 1.0,
        ki: 1.0,
        kd: 2.0,
        output_min: -100.0,
        output_max: 100.0,
    }).unwrap();
    let first = pid.update(10.0, 20.0, Duration::ZERO).unwrap();
    close(first.p, 10.0);
    close(first.i, 0.0);
    close(first.d, 0.0);

    let second = pid.update(10.0, 20.0, Duration::from_secs(2)).unwrap();
    close(second.i, 20.0);
    close(second.d, 0.0);

    let measurement_change = pid.update(15.0, 20.0, Duration::from_secs(3)).unwrap();
    close(measurement_change.d, -10.0);
    let setpoint_change = pid.update(15.0, 40.0, Duration::from_secs(4)).unwrap();
    close(setpoint_change.d, 0.0);
}

#[test]
fn pid_conditional_anti_windup_and_atomic_validation_cover_both_limits() {
    let config = PidConfig {
        kp: 10.0,
        ki: 5.0,
        kd: 0.0,
        output_min: 0.0,
        output_max: 100.0,
    };
    let mut upper = Pid::new(config).unwrap();
    upper.update(0.0, 20.0, Duration::ZERO).unwrap();
    let saturated = upper.update(0.0, 20.0, Duration::from_secs(1)).unwrap();
    assert_eq!(saturated.output, 100.0);
    assert_eq!(saturated.i, 0.0);

    let mut lower = Pid::new(config).unwrap();
    lower.update(20.0, 0.0, Duration::ZERO).unwrap();
    let saturated = lower.update(20.0, 0.0, Duration::from_secs(1)).unwrap();
    assert_eq!(saturated.output, 0.0);
    assert_eq!(saturated.i, 0.0);

    let before = lower.snapshot();
    assert!(lower.update(f64::NAN, 0.0, Duration::from_secs(2)).is_err());
    assert!(lower.update(20.0, 0.0, Duration::from_secs(1)).is_err());
    assert_eq!(lower.snapshot(), before);
}
