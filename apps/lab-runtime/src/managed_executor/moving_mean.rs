//! Bounded `native.moving_mean.v1` implementation and its registration metadata.

use super::registry::{ComponentPropertyMetadata, NativeComponentDefinition};
use lab_core::{
    Unit,
    managed::{
        ComponentDefinition, ComponentError, ComponentKind, ComponentManifest, ComponentResult,
        ComponentStatus, Invocation, InvocationPhase, PlainData, PlainValue,
    },
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) const IMPLEMENTATION: &str = "native.moving_mean.v1";
pub(super) const PROPERTIES: &[ComponentPropertyMetadata] = &[ComponentPropertyMetadata {
    id: "window",
    value_type: "integer",
    minimum: Some(2),
    maximum: Some(64),
    mutation_class: "reinitialize",
}];

const WINDOW_FIELD: &str = "window";
const VALUES_FIELD: &str = "values";

pub(super) fn compose(
    request: NativeComponentDefinition,
) -> Result<ComponentDefinition, ComponentError> {
    let input = request.input.ok_or(ComponentError::InvalidConfiguration)?;
    let warmup_samples = configured_window(&request.config)?;
    Ok(ComponentDefinition {
        manifest: ComponentManifest {
            schema_version: 1,
            id: request.id,
            instrument: request.instrument,
            name: request.name,
            parameter: lab_core::TEMPERATURE,
            kind: ComponentKind::Transform { input },
            unit: Unit::CELSIUS,
            min: -100.0,
            max: 500.0,
            warmup_samples,
            max_input_age: Duration::from_secs(2),
            history_capacity: 32,
        },
        implementation: lab_core::managed::ComponentImplementation::built_in(IMPLEMENTATION)?,
        config: request.config,
    })
}

pub(super) fn validate(job: &Invocation) -> Result<(), ComponentError> {
    if !matches!(
        job.definition.manifest.kind,
        ComponentKind::Transform { .. }
    ) || configured_window(&job.definition.config)? != job.definition.manifest.warmup_samples
    {
        return Err(ComponentError::InvalidConfiguration);
    }
    Ok(())
}

fn configured_window(config: &PlainData) -> Result<usize, ComponentError> {
    if config.fields.len() != 1 {
        return Err(ComponentError::InvalidConfiguration);
    }
    let Some(PlainValue::Number(window)) = config.fields.get(WINDOW_FIELD) else {
        return Err(ComponentError::InvalidConfiguration);
    };
    if !window.is_finite() || window.fract() != 0.0 || !(2.0..=64.0).contains(window) {
        return Err(ComponentError::InvalidConfiguration);
    }
    Ok(*window as usize)
}

fn check_deadline(deadline: Instant, cancelled: &AtomicBool) -> Result<(), ComponentError> {
    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
        Err(ComponentError::Deadline)
    } else {
        Ok(())
    }
}

pub(super) fn run(
    job: &Invocation,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<ComponentResult, ComponentError> {
    check_deadline(deadline, cancelled)?;
    let window = configured_window(&job.definition.config)?;
    if job.phase == InvocationPhase::Init {
        if job.input.is_some() {
            return Err(ComponentError::InvalidConfiguration);
        }
        let mut state = PlainData::default();
        state
            .fields
            .insert(VALUES_FIELD.into(), PlainValue::Numbers(Vec::new()));
        return Ok(ComponentResult {
            status: ComponentStatus::Init,
            value: None,
            unit: job.definition.manifest.unit,
            state,
            diagnostics: Vec::new(),
        });
    }
    let input = job.input.ok_or(ComponentError::InputUnavailable)?;
    if input.unit != job.definition.manifest.unit || !input.value.is_finite() {
        return Err(ComponentError::InputUnavailable);
    }
    if job.state.fields.len() != 1 {
        return Err(ComponentError::InvalidResult);
    }
    let Some(PlainValue::Numbers(previous)) = job.state.fields.get(VALUES_FIELD) else {
        return Err(ComponentError::InvalidResult);
    };
    if previous.len() > window || previous.iter().any(|value| !value.is_finite()) {
        return Err(ComponentError::InvalidResult);
    }
    let retained = previous.len().min(window.saturating_sub(1));
    let mut values = Vec::with_capacity(window);
    values.extend_from_slice(&previous[previous.len() - retained..]);
    values.push(input.value);
    check_deadline(deadline, cancelled)?;
    let mut state = PlainData::default();
    state
        .fields
        .insert(VALUES_FIELD.into(), PlainValue::Numbers(values.clone()));
    let ready = values.len() == window;
    let value = if ready {
        let mut mean = 0.0;
        for (index, sample) in values.iter().enumerate() {
            mean += (*sample - mean) / (index + 1) as f64;
            if !mean.is_finite() {
                return Err(ComponentError::InvalidResult);
            }
        }
        Some(mean)
    } else {
        None
    };
    Ok(ComponentResult {
        status: if ready {
            ComponentStatus::Ready
        } else {
            ComponentStatus::Warming
        },
        value,
        unit: job.definition.manifest.unit,
        state,
        diagnostics: Vec::new(),
    })
}
