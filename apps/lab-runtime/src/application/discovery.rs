//! Discovery, instrument-description, and managed-component API semantics.
//!
//! Discovery is composed deterministically from committed Runtime and host catalogs.
//! This module creates public projections; it owns none of the discovered objects.

use super::{
    common::{domain_code, id_field},
    projections::quality_name,
};
use crate::{configuration_api, measurements::signal_id_json, service::ServiceHost};
use lab_core::{
    AccessMode, InstrumentId, ParameterDescriptor, ParameterRole, Query, QueryResult, ValueSpec,
    WriteEffect, managed::ComponentId,
};
use serde_json::{Value, json};

pub(super) fn describe(service: &ServiceHost, args: &Value) -> Result<Value, &'static str> {
    let id = id_field(args, "instrument")?;
    let QueryResult::Descriptor(descriptor) = service
        .owner()
        .query(Query::DescribeInstrument(InstrumentId::new(id)))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let parameters: Vec<_> = descriptor.parameters.iter().map(parameter_json).collect();
    Ok(json!({"id":descriptor.id.get().to_string(),"name":descriptor.name,"parameters":parameters}))
}

pub(super) fn component(service: &ServiceHost, args: &Value) -> Result<Value, &'static str> {
    let id = id_field(args, "component")?;
    let owner = service.owner();
    let QueryResult::Component(snapshot) = owner
        .query(Query::Component(ComponentId::new(id)))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let kind = owner
        .component_catalog()
        .iter()
        .find(|(candidate, _)| candidate.get() == id)
        .map(|(_, kind)| *kind)
        .ok_or("unknown_component")?;
    Ok(
        json!({"component":id.to_string(),"instrument":snapshot.instrument.get().to_string(),"kind":kind,
        "implementation":snapshot.implementation.as_str(),
        "generation":snapshot.generation.to_string(),"revision":snapshot.revision.to_string(),
        "state":match snapshot.state{lab_core::managed::ComponentState::Warming=>"warming",lab_core::managed::ComponentState::Ready=>"ready",lab_core::managed::ComponentState::Failed=>"failed"},
        "good_steps":snapshot.good_steps,"pending":snapshot.pending.is_some(),"diagnostics":snapshot.diagnostics}),
    )
}

pub(super) fn records(service: &ServiceHost) -> Result<Vec<Value>, &'static str> {
    let owner = service.owner();
    let QueryResult::Instruments(instruments) =
        owner.query(Query::Discover).map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let mut records = Vec::new();
    for descriptor in instruments {
        let kind = owner.instrument_kind(descriptor.id);
        records.push(json!({"kind":"instrument","id":descriptor.id.get().to_string(),
            "name":descriptor.name,"implementation_kind":kind,
            "identity_class":if kind=="physical"{"physical"}else{"virtual"},
            "capabilities":{"readable":true,"current":descriptor.parameters.iter().any(|parameter|parameter.signal.is_some()),
                "recent_history":descriptor.parameters.iter().any(|parameter|parameter.signal.is_some()),
                "live_subscription":descriptor.parameters.iter().any(|parameter|parameter.signal.is_some()),
                "emulator_publication":descriptor.parameters.iter().any(|parameter|parameter.signal.is_some_and(|signal|owner.emulator_writable(signal))),
                "properties":descriptor.parameters.iter().any(|parameter|parameter.role==ParameterRole::Configuration)}}));
        for parameter in descriptor.parameters {
            if let Some(signal) = parameter.signal {
                let QueryResult::Latest(latest) = owner
                    .query(Query::GetLatestSignal(signal))
                    .map_err(domain_code)?
                else {
                    return Err("internal_error");
                };
                records.push(json!({"kind":"signal","id":signal_id_json(signal),
                    "instrument":descriptor.id.get().to_string(),"name":parameter.name,
                    "signal_kind":match parameter.role {ParameterRole::Measurement=>"measurement",_=>"diagnostic"},
                    "value_type":value_type_name(&parameter.value_spec),
                    "unit":{"id":parameter.unit.id(),"symbol":parameter.unit.symbol()},
                    "quality":latest.as_ref().map_or("unavailable",|sample|quality_name(sample.quality())),
                    "generation":owner.signal_generation(signal).to_string(),
                    "capabilities":{"readable":true,"current":true,"recent_history":true,
                        "durable_history":owner.recording_database_id().is_some(),"live_subscription":true,
                        "emulator_publication":owner.emulator_writable(signal)},
                    "emulator":if owner.emulator_writable(signal) {json!({"writable":true,
                        "states":["good","unavailable"],"timing":"runtime_receipt",
                        "expected_generation":owner.signal_generation(signal).to_string(),
                        "value_constraints":value_constraints(&parameter.value_spec)})} else {Value::Null}}));
            }
        }
    }
    for (id, component_kind) in owner.component_catalog() {
        let QueryResult::Component(snapshot) =
            owner.query(Query::Component(*id)).map_err(domain_code)?
        else {
            return Err("internal_error");
        };
        records.push(json!({"kind":"component","id":id.get().to_string(),
            "component_kind":component_kind,"instrument":snapshot.instrument.get().to_string(),
            "implementation":snapshot.implementation.as_str(),
            "generation":snapshot.generation.to_string(),"revision":snapshot.revision.to_string()}));
    }
    records.extend(
        owner
            .event_log()
            .projection_records()
            .into_iter()
            .filter(|record| {
                matches!(
                    record["kind"].as_str(),
                    Some("controller" | "reference" | "output")
                )
            })
            .map(|record| {
                json!({"kind":record["kind"],"id":record["target"],
                    "state":record["data"]})
            }),
    );
    records.extend(owner.resource_records().into_iter().filter_map(|record| {
        let id = record["target"]["id"].as_str()?.parse().ok()?;
        let state = configuration_api::resource_json(service, id).unwrap_or_else(|_| {
            json!({"resource":id.to_string(),"state":record["data"]["state"],
            "binding_generation":Value::Null,
            "transport_generation":record["data"]["generation"]})
        });
        Some(json!({"kind":"resource","id":id.to_string(),"state":state}))
    }));
    if let Ok(properties) = configuration_api::property_records(service) {
        records.extend(properties);
    }
    records.sort_by_key(Value::to_string);
    Ok(records)
}

fn parameter_json(parameter: &ParameterDescriptor) -> Value {
    let spec = match &parameter.value_spec {
        ValueSpec::Float { min, max } => json!({"type":"float","min":min,"max":max}),
        ValueSpec::Integer { min, max } => {
            json!({"type":"integer","min":min.to_string(),"max":max.to_string()})
        }
        ValueSpec::Boolean => json!({"type":"boolean"}),
        ValueSpec::Text { max_bytes } => json!({"type":"text","max_bytes":max_bytes}),
        ValueSpec::Enum { choices } => json!({"type":"enum","choices":choices}),
    };
    json!({"id":parameter.id.get().to_string(),"name":parameter.name,"unit":{"id":parameter.unit.id(),"symbol":parameter.unit.symbol()},
        "value_spec":spec,"access":match parameter.access {AccessMode::ReadOnly=>"read_only",AccessMode::ReadWrite=>"read_write",AccessMode::WriteOnly=>"write_only"},
        "role":match parameter.role {ParameterRole::Measurement=>"measurement",ParameterRole::Configuration=>"configuration",ParameterRole::Actuator=>"actuator",ParameterRole::Action=>"action",ParameterRole::Diagnostic=>"diagnostic"},
        "write_effect":match parameter.write_effect {WriteEffect::None=>"none",WriteEffect::ConfigurationOnly=>"configuration_only",WriteEffect::OutputAffecting=>"output_affecting"},
        "signal":parameter.signal.map(|signal|json!({"instrument":signal.instrument().get().to_string(),"parameter":signal.parameter().get().to_string()}))})
}

fn value_type_name(spec: &ValueSpec) -> &'static str {
    match spec {
        ValueSpec::Float { .. } => "float",
        ValueSpec::Integer { .. } => "integer",
        ValueSpec::Boolean => "boolean",
        ValueSpec::Text { .. } => "text",
        ValueSpec::Enum { .. } => "enum",
    }
}

fn value_constraints(spec: &ValueSpec) -> Value {
    match spec {
        ValueSpec::Float { min, max } => json!({"minimum":min,"maximum":max}),
        ValueSpec::Integer { min, max } => json!({"minimum":min,"maximum":max}),
        _ => Value::Null,
    }
}
