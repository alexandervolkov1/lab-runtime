//! Resource and deployment-configuration Application operations.
//!
//! Queries project validated laboratory configuration and resource state. Mutations
//! enter the same staged deployment lifecycle used at startup; this module never
//! exposes transport handles, staging buffers, or a generic filesystem input.

use crate::{
    application::common::{id_field, lifecycle_domain_error},
    configuration::{InstrumentDto, ManagedComponentDto, PropertyValue, ResourceKindDto},
    protocol,
    service::ServiceHost,
    sessions::{Mutation, PropertyMutationValue},
};
use lab_core::{
    Error, Query, QueryResult,
    transport::{ExecutorState, ResourceId},
};
use serde_json::{Value, json};

/// Maximum records in one frozen property projection.
pub const PROPERTY_RECORD_LIMIT: usize = 256;
/// Maximum records emitted in one configuration page.
pub const PROPERTY_PAGE_LIMIT: usize = 64;
/// Maximum encoded bytes in one configuration page.
pub const PROPERTY_PAGE_BYTES: usize = 8 * 1024;

/// Complete current configuration lifecycle projection.
pub(crate) fn status_json(service: &ServiceHost) -> Value {
    let Some(lifecycle) = service.loaded_configuration() else {
        return json!({"configured":false,"available":false,"revision":Value::Null,
            "source":{"kind":"compiled_profile","persistent":false},
            "staged_candidate":Value::Null,"runtime_overrides":0});
    };
    let active = lifecycle.active();
    let staged = lifecycle.staged().map(|candidate| {
        json!({"candidate_id":candidate.id().to_string(),
            "base_revision":candidate.base_revision().to_string(),
            "expires_at_ns":candidate.expires_at().as_nanos().to_string(),
            "effects":candidate.diff().effects().iter().map(|effect|effect.as_str()).collect::<Vec<_>>()})
    });
    json!({"configured":true,"available":true,
        "revision":lifecycle.revision().to_string(),
        "source":{"kind":"deployment_file","identity":hex(&active.toml_hash()),
            "automatically_persisted":false},
        "staged_candidate":staged,
        "runtime_overrides":active.property_overlay_count()})
}

/// Deterministic bounded descriptors for all known deployment properties.
pub(crate) fn property_records(service: &ServiceHost) -> Result<Vec<Value>, &'static str> {
    let lifecycle = service
        .loaded_configuration()
        .ok_or("configuration_disabled")?;
    let revision = lifecycle.revision().to_string();
    let dto = &lifecycle.active().effective().dto;
    let mut records = Vec::new();
    for resource in &dto.resources {
        let owner = json!({"kind":"resource","id":resource.id.to_string()});
        records.push(property(
            &owner,
            "port",
            "text",
            json!(resource.port),
            (None, "read_only", "rebind_required"),
            &revision,
        ));
        records.push(property(
            &owner,
            "baud_rate",
            "integer",
            json!(resource.baud_rate),
            (None, "deployment_only", "rebind_required"),
            &revision,
        ));
    }
    for instrument in &dto.instruments {
        let owner = json!({"kind":"instrument","id":instrument.id().to_string()});
        match instrument {
            InstrumentDto::VirtualMeasurement {
                display_name,
                poll_period_ms,
                base_temperature,
                measurement_enabled,
                external_publication,
                ..
            } => {
                records.push(property(
                    &owner,
                    "display_name",
                    "text",
                    json!(display_name),
                    (None, "read_write", "live_safe"),
                    &revision,
                ));
                records.push(property(
                    &owner,
                    "poll_period_ms",
                    "integer",
                    json!(poll_period_ms),
                    (
                        Some(json!({"minimum":1,"maximum":60000})),
                        "read_write",
                        "ordinary_live",
                    ),
                    &revision,
                ));
                records.push(property(
                    &owner,
                    "base_temperature",
                    "number",
                    json!(base_temperature),
                    (
                        Some(json!({"minimum":-100.0,"maximum":100.0})),
                        "deployment_only",
                        "reinitialize",
                    ),
                    &revision,
                ));
                records.push(property(
                    &owner,
                    "measurement_enabled",
                    "boolean",
                    json!(measurement_enabled),
                    (None, "deployment_only", "reinitialize"),
                    &revision,
                ));
                records.push(property(
                    &owner,
                    "external_publication",
                    "boolean",
                    json!(external_publication),
                    (None, "read_only", "reinitialize"),
                    &revision,
                ));
            }
            InstrumentDto::ThermalPlant {
                display_name,
                poll_period_ms,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                ..
            } => {
                records.push(property(
                    &owner,
                    "display_name",
                    "text",
                    json!(display_name),
                    (None, "read_write", "live_safe"),
                    &revision,
                ));
                records.push(property(
                    &owner,
                    "poll_period_ms",
                    "integer",
                    json!(poll_period_ms),
                    (
                        Some(json!({"minimum":1,"maximum":60000})),
                        "read_write",
                        "ordinary_live",
                    ),
                    &revision,
                ));
                for (id, current, unit) in [
                    (
                        "ambient_temperature",
                        json!(ambient_temperature),
                        Some(json!({"id":"degC","symbol":"°C"})),
                    ),
                    (
                        "initial_temperature",
                        json!(initial_temperature),
                        Some(json!({"id":"degC","symbol":"°C"})),
                    ),
                    ("gain_per_percent", json!(gain_per_percent), None),
                    (
                        "time_constant_ms",
                        json!(time_constant_ms),
                        Some(json!({"id":"ms","symbol":"ms"})),
                    ),
                ] {
                    let mut record = property(
                        &owner,
                        id,
                        "number",
                        current,
                        (None, "deployment_only", "reinitialize"),
                        &revision,
                    );
                    record["unit"] = unit.unwrap_or(Value::Null);
                    records.push(record);
                }
            }
            InstrumentDto::Metakon { poll_period_ms, .. } => {
                records.push(property(
                    &owner,
                    "poll_period_ms",
                    "integer",
                    json!(poll_period_ms),
                    (
                        Some(json!({"minimum":1,"maximum":60000})),
                        "read_write",
                        "ordinary_live",
                    ),
                    &revision,
                ));
            }
        }
    }
    for component in &dto.managed_components {
        component_properties(component, &revision, &mut records);
    }
    if records.len() > PROPERTY_RECORD_LIMIT {
        return Err("configuration_capacity");
    }
    records.sort_by_key(Value::to_string);
    Ok(records)
}

fn component_properties(component: &ManagedComponentDto, revision: &str, records: &mut Vec<Value>) {
    let owner = json!({"kind":"component","id":component.id.to_string()});
    records.push(property(
        &owner,
        "period_ms",
        "integer",
        json!(component.period_ms),
        (
            Some(json!({"minimum":1,"maximum":60000})),
            "deployment_only",
            "reinitialize",
        ),
        revision,
    ));
    for (name, value) in &component.config {
        let (inferred_kind, value) = match value {
            toml::Value::Integer(value) => ("integer", json!(value)),
            toml::Value::Float(value) => ("number", json!(value)),
            toml::Value::Boolean(value) => ("boolean", json!(value)),
            toml::Value::String(value) => ("text", json!(value)),
            toml::Value::Array(value) => ("number_array", json!(value)),
            _ => continue,
        };
        let metadata =
            crate::managed_executor::component_property_metadata(&component.implementation)
                .iter()
                .find(|metadata| metadata.id == name);
        let kind = metadata.map_or(inferred_kind, |metadata| metadata.value_type);
        let constraints =
            metadata.and_then(|metadata| match (metadata.minimum, metadata.maximum) {
                (Some(minimum), Some(maximum)) => {
                    Some(json!({"minimum":minimum,"maximum":maximum}))
                }
                _ => None,
            });
        let mutation_class = metadata.map_or("reinitialize", |metadata| metadata.mutation_class);
        records.push(property(
            &owner,
            name,
            kind,
            value,
            (constraints, "deployment_only", mutation_class),
            revision,
        ));
    }
}

fn property(
    owner: &Value,
    id: &str,
    value_type: &str,
    current: Value,
    policy: (Option<Value>, &str, &str),
    revision: &str,
) -> Value {
    let (constraints, access, mutation_class) = policy;
    let unit = if id == "poll_period_ms" || id == "period_ms" {
        json!({"id":"ms","symbol":"ms"})
    } else {
        Value::Null
    };
    json!({"kind":"property","id":{"owner":owner,"property":id},"owner":owner,
        "property":id,"value_type":value_type,"current":current,"unit":unit,
        "access":access,"mutation_class":mutation_class,"constraints":constraints,
        "revision":revision})
}

/// One authoritative semantic resource state selected by stable resource ID.
pub(crate) fn resource_json(
    service: &ServiceHost,
    resource_id: u64,
) -> Result<Value, &'static str> {
    let lifecycle = service.loaded_configuration().ok_or("unknown_resource")?;
    let resource = lifecycle
        .active()
        .effective()
        .dto
        .resources
        .iter()
        .find(|resource| resource.id == resource_id)
        .ok_or("unknown_resource")?;
    let QueryResult::Transport(snapshot) = service
        .owner()
        .query(Query::Transport(ResourceId::new(resource_id)))
        .map_err(|_| "unknown_resource")?
    else {
        return Err("unknown_resource");
    };
    let state = match snapshot.state {
        ExecutorState::Idle => "idle",
        ExecutorState::InFlight => "in_flight",
        ExecutorState::Recovering => "recovering",
        ExecutorState::Offline => "offline",
    };
    let instruments = lifecycle
        .active()
        .effective()
        .dto
        .instruments
        .iter()
        .filter(|instrument| {
            matches!(instrument, InstrumentDto::Metakon { resource_id: bound, .. } if *bound == resource_id)
        })
        .map(|instrument| instrument.id().to_string())
        .collect::<Vec<_>>();
    let kind = match resource.kind {
        ResourceKindDto::WindowsComReadOnly => "serial_read_only",
        ResourceKindDto::WindowsCom => "serial",
    };
    let binding_generation = service
        .owner()
        .configured_resource_generation(ResourceId::new(resource_id))
        .ok_or("unknown_resource")?;
    Ok(
        json!({"resource":resource_id.to_string(),"name":resource.key,"kind":kind,
        "identity_class":"physical","physical":true,"virtual":false,"state":state,
        "available":matches!(snapshot.state,ExecutorState::Idle|ExecutorState::InFlight),
        "status":if snapshot.state==ExecutorState::Offline {"transport_unavailable"} else {state},
        "binding_generation":binding_generation.to_string(),
        "transport_generation":snapshot.generation.to_string(),"instruments":instruments,
        "configuration_revision":lifecycle.revision().to_string(),
        "capabilities":{"reconnect":true,"configuration":true},
        "deployment":{"port":resource.port},
        "failure":if snapshot.state==ExecutorState::Offline {json!({"code":"transport_unavailable"})} else {Value::Null}}),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decode one configuration/resource mutation after registry shape validation.
pub(crate) fn decode_mutation(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match operation {
        "reload_configuration" => Mutation::ReloadConfiguration,
        "stage_configuration" => Mutation::StageConfiguration,
        "apply_configuration" => Mutation::ApplyConfiguration {
            candidate_id: id_field(args, "candidate_id")?,
            expected_revision: id_field(args, "expected_revision")?,
        },
        "property_configure" => {
            let target = args.get("target").ok_or("invalid_args")?;
            let target_kind = target
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            let property = args
                .get("property")
                .and_then(Value::as_str)
                .ok_or("invalid_args")?;
            if target_kind.len() > protocol::SEMANTIC_NAME_LIMIT
                || property.is_empty()
                || property.len() > protocol::SEMANTIC_NAME_LIMIT
            {
                return Err("invalid_args");
            }
            let value = match args.get("value").ok_or("invalid_args")? {
                Value::Number(value) => {
                    PropertyMutationValue::Integer(value.as_i64().ok_or("invalid_args")?)
                }
                Value::String(value) => PropertyMutationValue::Text(value.clone()),
                _ => return Err("invalid_args"),
            };
            Mutation::ConfigureProperty {
                target_kind: target_kind.to_owned(),
                target_id: id_field(target, "id")?,
                property: property.to_owned(),
                value,
                expected_revision: id_field(args, "expected_revision")?,
            }
        }
        "reconnect_resource" => Mutation::ReconnectResource {
            resource: id_field(args, "resource")?,
            expected_binding_generation: id_field(args, "expected_binding_generation")?,
        },
        _ => return Err("unsupported_operation"),
    })
}

/// Apply an admitted configuration/resource mutation through `ServiceHost`.
pub(crate) fn dispatch_mutation(
    service: &mut ServiceHost,
    mutation: Mutation,
) -> Result<Value, Error> {
    match mutation {
        Mutation::ReloadConfiguration => service
            .reload_configuration()
            .map(|result| json!({"revision":result.revision.to_string()}))
            .map_err(lifecycle_domain_error),
        Mutation::StageConfiguration => service
            .stage_configuration()
            .map(|staged| {
                let effects: Vec<_> = staged
                    .diff()
                    .effects()
                    .iter()
                    .map(|effect| effect.as_str())
                    .collect();
                json!({"candidate_id":staged.id().to_string(),
                    "base_revision":staged.base_revision().to_string(),
                    "expires_at_ns":staged.expires_at().as_nanos().to_string(),
                    "effects":effects})
            })
            .map_err(lifecycle_domain_error),
        Mutation::ApplyConfiguration {
            candidate_id,
            expected_revision,
        } => service
            .apply_staged_configuration(candidate_id, expected_revision)
            .map(|result| json!({"revision":result.revision.to_string()}))
            .map_err(lifecycle_domain_error),
        Mutation::ConfigureProperty {
            target_kind,
            target_id,
            property,
            value,
            expected_revision,
        } => {
            let value = match value {
                PropertyMutationValue::Integer(value) => PropertyValue::Integer(value),
                PropertyMutationValue::Text(value) => PropertyValue::Text(value),
            };
            service
                .configure_property(&target_kind, target_id, &property, value, expected_revision)
                .map(|result| {
                    json!({"target":{"kind":target_kind,"id":target_id.to_string()},
                    "property":property,"revision":result.revision.to_string(),
                    "persisted_to_deployment_source":false})
                })
                .map_err(lifecycle_domain_error)
        }
        Mutation::ReconnectResource {
            resource,
            expected_binding_generation,
        } => service
            .reconnect_resource(resource, expected_binding_generation)
            .map(|result| {
                json!({"resource":result.resource_id.to_string(),
                    "binding_generation":result.binding_generation.to_string()})
            })
            .map_err(lifecycle_domain_error),
        _ => Err(Error::InvalidConfiguration(
            "unexpected configuration mutation",
        )),
    }
}
