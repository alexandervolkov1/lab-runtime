//! Controller and output-status Application operations.
//!
//! This module projects controller/output state and translates accepted lifecycle
//! and configuration mutations into Runtime commands. It does not own controller
//! state, leases, OutputAuthority, or physical output execution.

use super::{
    common::{domain_code, float_field, id_field},
    projections::{controller_json, controller_projection_json, output_json},
};
use crate::{host::Clock, service::ServiceHost, sessions::Mutation, wire::WireRequestId};
use lab_core::{
    Command, CommandResult, Error, InstrumentId, ParameterId, Query, QueryResult,
    control::{ControllerError, ControllerId, NativeControllerConfig, PidConfig},
    output::ActuatorId,
    processing::EmaConfig,
};
use serde_json::Value;
use std::time::Duration;

pub(super) fn query_controller(service: &ServiceHost, args: &Value) -> Result<Value, &'static str> {
    let id = id_field(args, "controller")?;
    let owner = service.owner();
    let QueryResult::Controller(snapshot) = owner
        .query(Query::Controller(ControllerId::new(id)))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    let QueryResult::ControllerConfig(config) = owner
        .query(Query::ControllerConfig(ControllerId::new(id)))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    Ok(controller_projection_json(snapshot, config))
}

pub(super) fn query_output(service: &ServiceHost, args: &Value) -> Result<Value, &'static str> {
    let actuator = args.get("actuator").ok_or("invalid_args")?;
    let instrument = id_field(actuator, "instrument")?;
    let parameter = id_field(actuator, "parameter")?;
    let QueryResult::Output(snapshot) = service
        .owner()
        .query(Query::Output(ActuatorId::new(
            InstrumentId::new(instrument),
            ParameterId::new(parameter),
        )))
        .map_err(domain_code)?
    else {
        return Err("internal_error");
    };
    Ok(output_json(snapshot))
}

pub(super) fn decode(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match operation {
        "controller_configure_pid" => {
            let pid = args.get("pid").ok_or("invalid_args")?;
            Mutation::ConfigurePid {
                controller: id_field(args, "controller")?,
                expected_revision: id_field(args, "expected_revision")?,
                kp: float_field(pid, "kp")?,
                ki: float_field(pid, "ki")?,
                kd: float_field(pid, "kd")?,
                output_min: float_field(pid, "output_min")?,
                output_max: float_field(pid, "output_max")?,
            }
        }
        "controller_configure" => {
            let pid = args.get("pid").ok_or("invalid_args")?;
            let ema = args.get("ema").ok_or("invalid_args")?;
            Mutation::ConfigureController {
                controller: id_field(args, "controller")?,
                expected_revision: id_field(args, "expected_revision")?,
                kp: float_field(pid, "kp")?,
                ki: float_field(pid, "ki")?,
                kd: float_field(pid, "kd")?,
                output_min: float_field(pid, "output_min")?,
                output_max: float_field(pid, "output_max")?,
                ema_time_constant_ns: id_field(ema, "time_constant_ns")?,
                ema_warmup_samples: id_field(ema, "warmup_samples")?,
                max_input_age_ns: id_field(args, "max_input_age_ns")?,
                max_tick_gap_ns: id_field(args, "max_tick_gap_ns")?,
                lease_lifetime_ns: id_field(args, "lease_lifetime_ns")?,
                proposal_ttl_ns: id_field(args, "proposal_ttl_ns")?,
            }
        }
        "controller_start" => Mutation::Start {
            controller: id_field(args, "controller")?,
        },
        "controller_pause" => Mutation::Pause {
            controller: id_field(args, "controller")?,
        },
        "controller_resume" => Mutation::Resume {
            controller: id_field(args, "controller")?,
        },
        "controller_reset_failed" => Mutation::ResetFailed {
            controller: id_field(args, "controller")?,
        },
        _ => return Err("unsupported_operation"),
    })
}

pub(super) fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    request_id: &WireRequestId,
) -> Result<Value, Error> {
    let at = service.clock().now();
    let command = match mutation {
        Mutation::ConfigurePid {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
        } => Command::ConfigureControllerPid {
            controller: ControllerId::new(controller),
            expected_revision,
            pid: PidConfig {
                kp,
                ki,
                kd,
                output_min,
                output_max,
            },
        },
        Mutation::ConfigureController {
            controller,
            expected_revision,
            kp,
            ki,
            kd,
            output_min,
            output_max,
            ema_time_constant_ns,
            ema_warmup_samples,
            max_input_age_ns,
            max_tick_gap_ns,
            lease_lifetime_ns,
            proposal_ttl_ns,
        } => {
            let id = ControllerId::new(controller);
            let QueryResult::ControllerConfig(current) =
                service.owner().query(Query::ControllerConfig(id))?
            else {
                return Err(Error::InvalidConfiguration("unexpected controller query"));
            };
            let warmup_samples = usize::try_from(ema_warmup_samples)
                .map_err(|_| Error::Controller(ControllerError::InvalidConfiguration))?;
            Command::ReconfigureController {
                controller: id,
                config: NativeControllerConfig {
                    id,
                    input: current.input,
                    output: current.output,
                    reference: current.reference,
                    ema: EmaConfig {
                        time_constant: Duration::from_nanos(ema_time_constant_ns),
                        warmup_samples,
                        unit: current.ema.unit,
                    },
                    pid: PidConfig {
                        kp,
                        ki,
                        kd,
                        output_min,
                        output_max,
                    },
                    max_input_age: Duration::from_nanos(max_input_age_ns),
                    max_tick_gap: Duration::from_nanos(max_tick_gap_ns),
                    lease_lifetime: Duration::from_nanos(lease_lifetime_ns),
                    proposal_ttl: Duration::from_nanos(proposal_ttl_ns),
                },
                expected_revision,
            }
        }
        Mutation::Start { controller } => Command::StartController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Pause { controller } => Command::PauseController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::Resume { controller } => Command::ResumeController {
            controller: ControllerId::new(controller),
            at,
        },
        Mutation::ResetFailed { controller } => Command::ResetFailedController {
            controller: ControllerId::new(controller),
            at,
        },
        _ => {
            return Err(Error::InvalidConfiguration(
                "unexpected controller mutation",
            ));
        }
    };
    match service
        .owner_mut()
        .command_with_cause(command, Some((request_id.scope.clone(), request_id.seq)))?
    {
        CommandResult::ControllerUpdated(snapshot) => Ok(controller_json(snapshot)),
        _ => Err(Error::InvalidConfiguration("unexpected command result")),
    }
}
