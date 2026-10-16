//! Virtual/emulator publication and native-model lifecycle operations.
//!
//! Only deployment-declared virtual targets reach these handlers. Runtime validates
//! identity/generation and commits the sample; no result can fabricate physical
//! observation, ACK, readback, or output evidence.

use super::{
    common::{float_field, id_field, lifecycle_domain_error},
    projections::nanos,
};
use crate::{
    host::Clock,
    measurements::signal_id_json,
    service::ServiceHost,
    sessions::{EmulatorPublication, Mutation},
    wire::WireRequestId,
};
use lab_core::{Error, InstrumentId, ParameterId, SignalId};
use serde_json::{Value, json};

pub(super) fn decode(operation: &str, args: &Value) -> Result<Mutation, &'static str> {
    Ok(match operation {
        "emulator_publish" => {
            let signal = args.get("signal").ok_or("invalid_args")?;
            let publication = match args.get("state").and_then(Value::as_str) {
                Some("good") => EmulatorPublication::Good(float_field(args, "value")?),
                Some("unavailable") if args.get("value").is_none() => {
                    EmulatorPublication::Unavailable
                }
                _ => return Err("invalid_args"),
            };
            Mutation::PublishEmulatorMeasurement {
                instrument: id_field(signal, "instrument")?,
                parameter: id_field(signal, "parameter")?,
                expected_generation: id_field(args, "expected_generation")?,
                publication,
            }
        }
        "virtual_models_restart" => Mutation::RestartVirtualModels,
        _ => return Err("unsupported_operation"),
    })
}

pub(super) fn dispatch(
    service: &mut ServiceHost,
    mutation: Mutation,
    request_id: &WireRequestId,
) -> Result<Value, Error> {
    match mutation {
        Mutation::PublishEmulatorMeasurement {
            instrument,
            parameter,
            expected_generation,
            publication,
        } => {
            let at = service.clock().now();
            let signal = SignalId::new(InstrumentId::new(instrument), ParameterId::new(parameter));
            let value = match publication {
                EmulatorPublication::Good(value) => Some(lab_core::Value::Float(value)),
                EmulatorPublication::Unavailable => None,
            };
            let sample = service.owner_mut().publish_emulator_measurement(
                signal,
                value,
                expected_generation,
                at,
                Some((request_id.scope.clone(), request_id.seq)),
            )?;
            let generation = service.owner().signal_generation(signal);
            Ok(json!({"signal":signal_id_json(signal),
                "generation":generation.to_string(),
                "state":if sample.quality()==lab_core::SampleQuality::Good{"good"}else{"unavailable"},
                "committed_at_ns":nanos(sample.at())}))
        }
        Mutation::RestartVirtualModels => service
            .restart_virtual_models()
            .map(|result| {
                json!({"models":result.models.to_string(),
                    "generation":result.generation.to_string()})
            })
            .map_err(lifecycle_domain_error),
        _ => Err(Error::InvalidConfiguration("unexpected virtual mutation")),
    }
}
