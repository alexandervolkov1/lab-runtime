//! One explicit compile-time registry for trusted native component implementations.

use super::moving_mean;
#[cfg(test)]
use super::reference_component;
use lab_core::{
    InstrumentId, SignalId,
    managed::{
        ComponentDefinition, ComponentError, ComponentId, ComponentResult, Invocation, PlainData,
    },
};
use std::{sync::Arc, sync::atomic::AtomicBool, time::Instant};

/// Stable semantic identity of the reference native transform.
pub const MOVING_MEAN_IMPLEMENTATION: &str = moving_mean::IMPLEMENTATION;

/// Registration-owned configuration metadata projected generically by the API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentPropertyMetadata {
    /// Stable property identity within the implementation configuration.
    pub id: &'static str,
    /// Language-neutral scalar type.
    pub value_type: &'static str,
    /// Inclusive numeric minimum, when applicable.
    pub minimum: Option<i64>,
    /// Inclusive numeric maximum, when applicable.
    pub maximum: Option<i64>,
    /// Runtime lifecycle class selected by trusted registration.
    pub mutation_class: &'static str,
}

/// Host-supplied identity and binding for one statically registered implementation.
pub(crate) struct NativeComponentDefinition {
    pub(crate) id: ComponentId,
    pub(crate) instrument: InstrumentId,
    pub(crate) name: String,
    pub(crate) input: Option<SignalId>,
    pub(crate) config: PlainData,
}

type Compose = fn(NativeComponentDefinition) -> Result<ComponentDefinition, ComponentError>;
type Validate = fn(&Invocation) -> Result<(), ComponentError>;
type Run = fn(&Invocation, Instant, &AtomicBool) -> Result<ComponentResult, ComponentError>;

struct Registration {
    implementation: &'static str,
    properties: &'static [ComponentPropertyMetadata],
    compose: Compose,
    validate: Validate,
    run: Run,
}

const MOVING_MEAN: Registration = Registration {
    implementation: moving_mean::IMPLEMENTATION,
    properties: moving_mean::PROPERTIES,
    compose: moving_mean::compose,
    validate: moving_mean::validate,
    run: moving_mean::run,
};

#[cfg(test)]
const REFERENCE_COMPONENT: Registration = Registration {
    implementation: reference_component::IMPLEMENTATION,
    properties: reference_component::PROPERTIES,
    compose: reference_component::compose,
    validate: reference_component::validate,
    run: reference_component::run,
};

fn registration(implementation: &str) -> Option<&'static Registration> {
    match implementation {
        moving_mean::IMPLEMENTATION => Some(&MOVING_MEAN),
        #[cfg(test)]
        reference_component::IMPLEMENTATION => Some(&REFERENCE_COMPONENT),
        _ => None,
    }
}

/// Property metadata for one registered implementation. The wire layer has no
/// implementation-specific branch, so new registrations need no new operation.
pub fn component_property_metadata(implementation: &str) -> &'static [ComponentPropertyMetadata] {
    registration(implementation).map_or(&[], |entry| entry.properties)
}

/// Validate a deployment binding through the same registration that composes it.
pub(crate) fn validate_component_configuration(
    implementation: &str,
    definition: NativeComponentDefinition,
) -> Result<(), ComponentError> {
    build_component_definition(implementation, definition).map(drop)
}

/// Build the Core-neutral definition selected by one trusted registration entry.
pub(crate) fn build_component_definition(
    implementation: &str,
    definition: NativeComponentDefinition,
) -> Result<ComponentDefinition, ComponentError> {
    let registration = registration(implementation).ok_or(ComponentError::InvalidConfiguration)?;
    (registration.compose)(definition)
}

pub(super) fn validate_invocation(invocation: &Invocation) -> Result<(), ComponentError> {
    let registration = registration(invocation.definition.implementation.id().as_str())
        .ok_or(ComponentError::InvalidConfiguration)?;
    (registration.validate)(invocation)
}

pub(super) fn run_registered(
    invocation: &Invocation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<ComponentResult, ComponentError> {
    let registration = registration(invocation.definition.implementation.id().as_str())
        .ok_or(ComponentError::InvalidConfiguration)?;
    debug_assert_eq!(
        registration.implementation,
        invocation.definition.implementation.id().as_str()
    );
    (registration.validate)(invocation)?;
    (registration.run)(invocation, deadline, &cancelled)
}

#[cfg(test)]
mod tests {
    use super::reference_component;
    use crate::{
        application::Application,
        service::{ServiceHost, ServiceOptions},
        wire::{decode_frame, encode_frame},
    };
    use serde_json::{Value, json};
    use std::{fs, path::PathBuf, thread, time::Duration};

    fn temporary_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "lab-runtime-native-registry-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn ask(service: &mut ServiceHost, application: &mut Application, value: Value) -> Vec<Value> {
        let request = decode_frame(&encode_frame(&value).unwrap()).unwrap();
        application.handle(service, 1, request)
    }

    #[test]
    fn second_registration_reaches_generic_api_surfaces_without_domain_handlers() {
        let path = temporary_path();
        fs::write(
            &path,
            format!(
                r#"schema_version=1
[runtime]
key="native-registry"
display_name="Native registry"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[instruments]]
id=41
key="input"
kind="virtual_measurement"
display_name="Input"
history_capacity=16
base_temperature=22.0
poll_period_ms=1
[[managed_components]]
id=42
instrument_id=42
key="scaled"
display_name="Scaled input"
implementation="{}"
input_instrument_id=41
period_ms=1
config={{scale=2}}
"#,
                reference_component::IMPLEMENTATION
            ),
        )
        .unwrap();
        let argument = path.to_string_lossy().into_owned();
        let mut service = ServiceHost::startup(
            ServiceOptions::parse(&["--serve", "--config", &argument]).unwrap(),
        )
        .unwrap();
        let mut application = Application::new(service.boot_id()).unwrap();
        let hello = ask(
            &mut service,
            &mut application,
            json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
        );
        assert_eq!(hello[0]["type"], "result");

        let cursor = service.owner().event_log().latest_cursor();
        let boot_id = service.boot_id().to_owned();
        let subscribed = ask(
            &mut service,
            &mut application,
            json!({"v":1,"msg_id":"subscribe","op":"subscribe","args":{
                "after":{"boot_id":boot_id,"seq":cursor.to_string()},
                "filter":{"kinds":["signal"],"targets":[]}}}),
        );
        assert_eq!(subscribed[0]["type"], "result");

        let clock = service.clock_copy();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            service.owner_mut().service(&clock).unwrap();
            let current = ask(
                &mut service,
                &mut application,
                json!({"v":1,"msg_id":"current","op":"latest","args":{
                    "signal":{"instrument":"42","parameter":"1"}}}),
            );
            if current[0]["result"]["quality"] == "good" {
                let value = current[0]["result"]["value"].as_f64().unwrap();
                assert!(value.is_finite() && (43.0..=45.0).contains(&value));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "test component did not reach the generic signal path"
            );
            thread::yield_now();
        }

        let discovery = ask(
            &mut service,
            &mut application,
            json!({"v":1,"msg_id":"discover","op":"discover","args":{}}),
        );
        assert!(
            discovery[0]["result"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|record| record["kind"] == "instrument"
                    && record["id"] == "42"
                    && record["implementation_kind"] == "managed")
        );

        let properties = ask(
            &mut service,
            &mut application,
            json!({"v":1,"msg_id":"properties","op":"configuration_properties","args":{}}),
        );
        assert!(
            properties[0]["result"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |record| record["owner"] == json!({"kind":"component","id":"42"})
                        && record["property"] == "scale"
                        && record["current"] == 2
                )
        );

        let window = ask(
            &mut service,
            &mut application,
            json!({"v":1,"msg_id":"window","op":"measurement_window","args":{
                "signal":{"instrument":"42","parameter":"1"},"max_records":16}}),
        );
        assert_eq!(
            window[0]["result"]["source"], "runtime_recent",
            "{window:?}"
        );
        let records = window[0]["result"]["rows"]
            .as_array()
            .unwrap_or_else(|| panic!("unexpected measurement window: {window:?}"));
        assert!(!records.is_empty());
        let mut events = Vec::new();
        for _ in 0..8 {
            events.extend(application.pump_events(&service, 1));
            if events
                .iter()
                .any(|event| event["kind"] == "signal" && event["target"]["instrument"] == "42")
            {
                break;
            }
        }
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == "signal" && event["target"]["instrument"] == "42"),
            "unexpected subscription delivery: {events:?}"
        );

        drop(application);
        drop(service);
        fs::remove_file(path).unwrap();
    }
}
