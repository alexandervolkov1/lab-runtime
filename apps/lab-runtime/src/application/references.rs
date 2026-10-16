//! Reference query, configuration, retune, and projection handling.
//!
//! The Application translates validated request fields into Core commands. Core
//! remains authoritative for Reference state, units, revisions, and validation.

use super::{
    common::{domain_code, float_field, id_field},
    projections::{nanos, reference_json},
};
use crate::{host::Clock, service::ServiceHost, sessions::Mutation, wire::WireRequestId};
use lab_core::{
    Command, CommandResult, Error, Query, QueryResult,
    reference::{ReferenceConfig, ReferenceId, ReferenceSnapshot},
};
use serde_json::{Value, json};

pub(super) fn query(service: &ServiceHost, args: &Value) -> Result<Value, &'static str> {
    let id = id_field(args, "reference")?;
    match service
        .owner()
        .query(Query::Reference(ReferenceId::new(id)))
        .map_err(domain_code)?
    {
        QueryResult::Reference(snapshot) => Ok(reference_json(id, snapshot)),
        _ => Err("internal_error"),
    }
}

pub(super) fn decode(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    match operation {
        "reference_configure" => {
            let reference = id_field(args, "reference")?;
            let expected_revision = id_field(args, "expected_revision")?;
            match args.get("kind").and_then(Value::as_str) {
                Some("fixed") if args.get("target").is_none() && args.get("rate").is_none() => {
                    Ok(Mutation::ConfigureReferenceFixed {
                        reference,
                        expected_revision,
                        value: float_field(args, "value")?,
                    })
                }
                Some("ramp") if args.get("value").is_none() => {
                    Ok(Mutation::ConfigureReferenceRamp {
                        reference,
                        expected_revision,
                        target: float_field(args, "target")?,
                        rate: float_field(args, "rate")?,
                    })
                }
                _ => Err("invalid_args"),
            }
        }
        "reference_retune" => Ok(Mutation::RetuneRamp {
            reference: id_field(args, "reference")?,
            expected_revision: id_field(args, "expected_revision")?,
            target: float_field(args, "target")?,
            rate: float_field(args, "rate")?,
        }),
        _ => Err("unsupported_operation"),
    }
}

pub(super) fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    request_id: &WireRequestId,
) -> Result<Value, Error> {
    let at = service.clock().now();
    let (reference, command) = match mutation {
        Mutation::ConfigureReferenceFixed {
            reference,
            expected_revision,
            value,
        } => {
            let QueryResult::Reference(snapshot) = service
                .owner()
                .query(Query::Reference(ReferenceId::new(reference)))?
            else {
                return Err(Error::InvalidConfiguration("unexpected Reference query"));
            };
            let unit = match snapshot {
                ReferenceSnapshot::Fixed { unit, .. } => unit,
                ReferenceSnapshot::Ramp { state, .. } => state.unit,
            };
            (
                reference,
                Command::ReconfigureReference {
                    reference: ReferenceId::new(reference),
                    config: ReferenceConfig::Fixed {
                        id: ReferenceId::new(reference),
                        value,
                        unit,
                    },
                    expected_revision,
                },
            )
        }
        Mutation::ConfigureReferenceRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => {
            let QueryResult::Reference(snapshot) = service
                .owner()
                .query(Query::Reference(ReferenceId::new(reference)))?
            else {
                return Err(Error::InvalidConfiguration("unexpected Reference query"));
            };
            let (start, unit) = match snapshot {
                ReferenceSnapshot::Fixed { value, unit, .. } => (value, unit),
                ReferenceSnapshot::Ramp { state, .. } => (state.current, state.unit),
            };
            (
                reference,
                Command::ReconfigureReference {
                    reference: ReferenceId::new(reference),
                    config: ReferenceConfig::Ramp {
                        id: ReferenceId::new(reference),
                        start,
                        target,
                        rate,
                        unit,
                        at,
                    },
                    expected_revision,
                },
            )
        }
        Mutation::RetuneRamp {
            reference,
            expected_revision,
            target,
            rate,
        } => (
            reference,
            Command::RetuneRampReference {
                reference: ReferenceId::new(reference),
                expected_revision,
                target,
                rate,
                at,
            },
        ),
        _ => return Err(Error::InvalidConfiguration("unexpected Reference mutation")),
    };
    match service
        .owner_mut()
        .command_with_cause(command, Some((request_id.scope.clone(), request_id.seq)))?
    {
        CommandResult::ReferenceRetuned(retuned) => Ok(json!({
            "reference":reference.to_string(),"revision":retuned.revision.to_string(),
            "value":retuned.state.current,"target":retuned.state.target,"rate":retuned.state.rate,
            "committed_at":nanos(retuned.state.last_at)})),
        CommandResult::ReferenceConfigured(snapshot) => Ok(reference_json(reference, snapshot)),
        _ => Err(Error::InvalidConfiguration("unexpected command result")),
    }
}
