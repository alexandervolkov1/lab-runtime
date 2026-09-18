//! Test-only second registration proving the ordinary native extension seam.

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

pub(super) const IMPLEMENTATION: &str = "native.test.scale.v1";
pub(super) const PROPERTIES: &[ComponentPropertyMetadata] = &[ComponentPropertyMetadata {
    id: "scale",
    value_type: "integer",
    minimum: Some(1),
    maximum: Some(4),
    mutation_class: "reinitialize",
}];

const SCALE_FIELD: &str = "scale";
const STEPS_FIELD: &str = "steps";

pub(super) fn compose(
    request: NativeComponentDefinition,
) -> Result<ComponentDefinition, ComponentError> {
    let input = request.input.ok_or(ComponentError::InvalidConfiguration)?;
    configured_scale(&request.config)?;
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
            warmup_samples: 1,
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
    ) || job.definition.manifest.warmup_samples != 1
    {
        return Err(ComponentError::InvalidConfiguration);
    }
    configured_scale(&job.definition.config).map(|_| ())
}

fn configured_scale(config: &PlainData) -> Result<f64, ComponentError> {
    if config.fields.len() != 1 {
        return Err(ComponentError::InvalidConfiguration);
    }
    let Some(PlainValue::Number(scale)) = config.fields.get(SCALE_FIELD) else {
        return Err(ComponentError::InvalidConfiguration);
    };
    if !scale.is_finite() || scale.fract() != 0.0 || !(1.0..=4.0).contains(scale) {
        return Err(ComponentError::InvalidConfiguration);
    }
    Ok(*scale)
}

pub(super) fn run(
    job: &Invocation,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<ComponentResult, ComponentError> {
    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
        return Err(ComponentError::Deadline);
    }
    if job.phase == InvocationPhase::Init {
        if job.input.is_some() {
            return Err(ComponentError::InvalidConfiguration);
        }
        return Ok(ComponentResult {
            status: ComponentStatus::Init,
            value: None,
            unit: job.definition.manifest.unit,
            state: PlainData {
                fields: [(STEPS_FIELD.into(), PlainValue::Number(0.0))]
                    .into_iter()
                    .collect(),
            },
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
    let Some(PlainValue::Number(steps)) = job.state.fields.get(STEPS_FIELD) else {
        return Err(ComponentError::InvalidResult);
    };
    if !steps.is_finite() || *steps < 0.0 || steps.fract() != 0.0 {
        return Err(ComponentError::InvalidResult);
    }
    let value = input.value * configured_scale(&job.definition.config)?;
    let next_steps = *steps + 1.0;
    if !value.is_finite() || !next_steps.is_finite() {
        return Err(ComponentError::InvalidResult);
    }
    Ok(ComponentResult {
        status: ComponentStatus::Ready,
        value: Some(value),
        unit: job.definition.manifest.unit,
        state: PlainData {
            fields: [(STEPS_FIELD.into(), PlainValue::Number(next_steps))]
                .into_iter()
                .collect(),
        },
        diagnostics: Vec::new(),
    })
}
