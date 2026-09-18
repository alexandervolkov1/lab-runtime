//! Stable bounded Application-protocol vocabulary shared by framing and dispatch.
//!
//! This module owns public protocol identity, operation/capability declarations,
//! and the finite error taxonomy. It intentionally contains no Runtime domain
//! objects and does not mirror arbitrary Rust errors onto the wire.

use serde_json::{Value, json};

use crate::{events, server, sessions, wire};

/// Stable identity of the local laboratory Application protocol.
pub const PROTOCOL_ID: &str = "lab-runtime.application";
/// Only protocol envelope version accepted by this pre-release implementation.
pub const PROTOCOL_VERSION: u8 = 1;
/// Semantic Application API revision, independent of package and SQLite versions.
pub const APPLICATION_API_VERSION: &str = "0.1-pre";
/// Maximum bytes in any public human-readable error message.
pub const ERROR_MESSAGE_LIMIT: usize = 256;
/// Maximum serialized bytes in optional structured public error details.
pub const ERROR_DETAILS_LIMIT: usize = 2_048;
/// Maximum operation or capability name length.
pub const SEMANTIC_NAME_LIMIT: usize = 64;
/// Maximum capabilities returned by hello.
pub const CAPABILITY_LIMIT: usize = 32;

/// Query versus mutating operation admission semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// Synchronous read of committed owner state.
    Query,
    /// Deduplicated mutation carrying a stable request identity.
    Mutation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Always,
    Recorder,
    Configuration,
    ResourceReconnect,
    EmulatorPublication,
    VirtualModelLifecycle,
}

/// One authoritative bounded public operation declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationSpec {
    /// Stable public operation name.
    pub name: &'static str,
    /// Whether a request is a query or a deduplicated mutation.
    pub kind: OperationKind,
    /// Strict permitted top-level argument names.
    pub argument_fields: &'static [&'static str],
    availability: Availability,
}

macro_rules! operation {
    ($name:literal, $kind:ident, [$($field:literal),* $(,)?]) => {
        OperationSpec {
            name: $name,
            kind: OperationKind::$kind,
            argument_fields: &[$($field),*],
            availability: Availability::Always,
        }
    };
    ($name:literal, $kind:ident, $availability:ident, [$($field:literal),* $(,)?]) => {
        OperationSpec {
            name: $name,
            kind: OperationKind::$kind,
            argument_fields: &[$($field),*],
            availability: Availability::$availability,
        }
    };
}

/// Complete v1 operation registry. Wire validation and hello both consume it.
pub const OPERATIONS: &[OperationSpec] = &[
    operation!("hello", Query, ["scope"]),
    operation!("discover", Query, []),
    operation!("discovery_page", Query, ["projection", "index"]),
    operation!("describe", Query, ["instrument"]),
    operation!("resource", Query, ResourceReconnect, ["resource"]),
    operation!("configuration_status", Query, Configuration, []),
    operation!("configuration_properties", Query, Configuration, []),
    operation!(
        "configuration_page",
        Query,
        Configuration,
        ["projection", "index"]
    ),
    operation!("latest", Query, ["signal"]),
    operation!("measurements_current", Query, []),
    operation!("measurements_page", Query, ["projection", "index"]),
    operation!("measurement_window", Query, ["signal", "max_records"]),
    operation!("controller", Query, ["controller"]),
    operation!("reference", Query, ["reference"]),
    operation!("component", Query, ["component"]),
    operation!("output", Query, ["actuator"]),
    operation!("operation_status", Query, ["request_id"]),
    operation!("subscribe", Query, ["after", "filter"]),
    operation!("unsubscribe", Query, ["subscription"]),
    operation!(
        "reference_configure",
        Mutation,
        [
            "reference",
            "expected_revision",
            "kind",
            "value",
            "target",
            "rate"
        ]
    ),
    operation!(
        "reference_retune",
        Mutation,
        ["reference", "expected_revision", "target", "rate"]
    ),
    operation!(
        "controller_configure_pid",
        Mutation,
        ["controller", "expected_revision", "pid"]
    ),
    operation!(
        "controller_configure",
        Mutation,
        [
            "controller",
            "expected_revision",
            "pid",
            "ema",
            "max_input_age_ns",
            "max_tick_gap_ns",
            "lease_lifetime_ns",
            "proposal_ttl_ns"
        ]
    ),
    operation!("controller_start", Mutation, ["controller"]),
    operation!("controller_pause", Mutation, ["controller"]),
    operation!("controller_resume", Mutation, ["controller"]),
    operation!("controller_reset_failed", Mutation, ["controller"]),
    operation!("runtime_shutdown", Mutation, []),
    operation!("stage_configuration", Mutation, Configuration, []),
    operation!(
        "apply_configuration",
        Mutation,
        Configuration,
        ["candidate_id", "expected_revision"]
    ),
    operation!("reload_configuration", Mutation, Configuration, []),
    operation!(
        "property_configure",
        Mutation,
        Configuration,
        ["target", "property", "value", "expected_revision"]
    ),
    operation!(
        "emulator_publish",
        Mutation,
        EmulatorPublication,
        ["signal", "state", "value", "expected_generation"]
    ),
    operation!(
        "virtual_models_restart",
        Mutation,
        VirtualModelLifecycle,
        []
    ),
    operation!(
        "reconnect_resource",
        Mutation,
        ResourceReconnect,
        ["resource", "expected_binding_generation"]
    ),
    operation!("recording_status", Query, []),
    operation!("recording_start", Mutation, Recorder, ["label"]),
    operation!("recording_stop", Mutation, Recorder, ["run_id"]),
    operation!("experiment_annotate", Mutation, Recorder, ["name", "data"]),
    operation!(
        "history_read",
        Mutation,
        Recorder,
        [
            "mode",
            "database_id",
            "boot_id",
            "run_id",
            "signal",
            "from_ns",
            "to_ns",
            "max_records",
            "cursor"
        ]
    ),
    operation!("history_page", Query, Recorder, ["page_token"]),
    operation!("history_release", Query, Recorder, ["page_token"]),
];

/// Runtime-composition facts used to filter actually usable public operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolFeatures {
    /// Durable Recorder and history service is configured.
    pub recorder: bool,
    /// Declarative deployment lifecycle is configured.
    pub configuration: bool,
    /// At least one configured physical resource supports explicit reconnect.
    pub resource_reconnect: bool,
    /// At least one explicit virtual signal accepts external publication.
    pub emulator_publication: bool,
    /// At least one Runtime-owned native virtual model supports restart.
    pub virtual_model_lifecycle: bool,
}

/// Find one declared operation without allocating.
pub fn operation_spec(name: &str) -> Option<&'static OperationSpec> {
    OPERATIONS.iter().find(|spec| spec.name == name)
}

/// Whether one declared operation is supported by this service composition.
pub fn operation_supported(spec: &OperationSpec, features: ProtocolFeatures) -> bool {
    match spec.availability {
        Availability::Always => true,
        Availability::Recorder => features.recorder,
        Availability::Configuration => features.configuration,
        Availability::ResourceReconnect => features.resource_reconnect,
        Availability::EmulatorPublication => features.emulator_publication,
        Availability::VirtualModelLifecycle => features.virtual_model_lifecycle,
    }
}

/// Ordered, bounded operation names advertised by hello.
pub fn supported_operations(features: ProtocolFeatures) -> Vec<&'static str> {
    OPERATIONS
        .iter()
        .filter(|spec| operation_supported(spec, features))
        .map(|spec| spec.name)
        .collect()
}

#[derive(Clone, Copy)]
struct CapabilitySpec {
    name: &'static str,
    stability: &'static str,
    operation: &'static str,
}

const CAPABILITIES: &[CapabilitySpec] = &[
    CapabilitySpec {
        name: "operation_lifecycle",
        stability: "stable",
        operation: "operation_status",
    },
    CapabilitySpec {
        name: "structured_discovery",
        stability: "stable",
        operation: "discover",
    },
    CapabilitySpec {
        name: "live_subscriptions",
        stability: "stable",
        operation: "subscribe",
    },
    CapabilitySpec {
        name: "current_measurements",
        stability: "stable",
        operation: "measurements_current",
    },
    CapabilitySpec {
        name: "recent_measurement_history",
        stability: "stable",
        operation: "measurement_window",
    },
    CapabilitySpec {
        name: "instrument_queries",
        stability: "stable",
        operation: "describe",
    },
    CapabilitySpec {
        name: "reference_read_write",
        stability: "stable",
        operation: "reference_configure",
    },
    CapabilitySpec {
        name: "controller_status",
        stability: "stable",
        operation: "controller",
    },
    CapabilitySpec {
        name: "controller_configuration",
        stability: "stable",
        operation: "controller_configure",
    },
    CapabilitySpec {
        name: "controller_lifecycle",
        stability: "stable",
        operation: "controller_start",
    },
    CapabilitySpec {
        name: "managed_components",
        stability: "stable",
        operation: "component",
    },
    CapabilitySpec {
        name: "output_status",
        stability: "stable",
        operation: "output",
    },
    CapabilitySpec {
        name: "runtime_shutdown",
        stability: "stable",
        operation: "runtime_shutdown",
    },
    CapabilitySpec {
        name: "recording_status",
        stability: "stable",
        operation: "recording_status",
    },
    CapabilitySpec {
        name: "recording_control",
        stability: "stable",
        operation: "recording_start",
    },
    CapabilitySpec {
        name: "measurement_history",
        stability: "stable",
        operation: "history_read",
    },
    CapabilitySpec {
        name: "resource_status",
        stability: "stable",
        operation: "resource",
    },
    CapabilitySpec {
        name: "configuration_read",
        stability: "stable",
        operation: "configuration_status",
    },
    CapabilitySpec {
        name: "configuration_properties",
        stability: "stable",
        operation: "configuration_properties",
    },
    CapabilitySpec {
        name: "configuration_write",
        stability: "stable",
        operation: "property_configure",
    },
    CapabilitySpec {
        name: "deployment_configuration",
        stability: "stable",
        operation: "stage_configuration",
    },
    CapabilitySpec {
        name: "resource_reconnect",
        stability: "stable",
        operation: "reconnect_resource",
    },
    CapabilitySpec {
        name: "virtual_instruments",
        stability: "stable",
        operation: "discover",
    },
    CapabilitySpec {
        name: "emulator_publication",
        stability: "stable",
        operation: "emulator_publish",
    },
    CapabilitySpec {
        name: "virtual_model_lifecycle",
        stability: "stable",
        operation: "virtual_models_restart",
    },
];

/// Bounded structured semantic capabilities supported by this composition.
pub fn capabilities(features: ProtocolFeatures) -> Vec<Value> {
    let result: Vec<_> = CAPABILITIES
        .iter()
        .filter(|spec| {
            operation_spec(spec.operation)
                .is_some_and(|operation| operation_supported(operation, features))
        })
        .map(|spec| json!({"name":spec.name,"version":1,"stability":spec.stability}))
        .collect();
    debug_assert!(result.len() <= CAPABILITY_LIMIT);
    result
}

/// Actual fixed protocol and server bounds advertised by hello.
pub fn limits() -> Value {
    let configuration = json!({
        "property_records":crate::configuration_api::PROPERTY_RECORD_LIMIT,
        "page_records":crate::configuration_api::PROPERTY_PAGE_LIMIT,
        "page_bytes":crate::configuration_api::PROPERTY_PAGE_BYTES,
        "runtime_overrides":crate::configuration::MAX_PROPERTY_OVERLAYS,
        "staged_candidates":1,
        "candidate_retention_seconds":crate::deployment::CANDIDATE_LIFETIME.as_secs()
    });
    let emulator = json!({"targets":64,"records_per_request":1,
        "metadata_bytes":0,"pending_per_scope":sessions::MAX_PENDING_SCOPE});
    let mut limits = json!({
        "frame_bytes": wire::FRAME_LIMIT,
        "json_depth": wire::DEPTH_LIMIT,
        "json_values": wire::VALUE_LIMIT,
        "json_string_bytes": wire::STRING_LIMIT,
        "clients": server::MAX_CLIENTS,
        "owner_reactor_mailbox": server::QUEUE,
        "client_pending_requests": server::CLIENT_IN,
        "client_reply_queue": server::CLIENT_OUT,
        "client_event_queue": server::CLIENT_EVENTS,
        "network_sweep_bytes": server::SWEEP_BYTES,
        "client_deadline_ms": server::CLIENT_DEADLINE.as_millis(),
        "scopes": sessions::MAX_SCOPES,
        "pending_operations_per_scope": sessions::MAX_PENDING_SCOPE,
        "pending_operations": sessions::MAX_PENDING,
        "terminal_operations_per_scope": sessions::MAX_TERMINAL_SCOPE,
        "terminal_operations": sessions::MAX_TERMINAL,
        "terminal_retention_seconds": sessions::TERMINAL_TTL.as_secs(),
        "detached_scope_retention_seconds": sessions::IDLE_SCOPE_TTL.as_secs(),
        "terminal_result_bytes": sessions::OUTCOME_SIZE_LIMIT,
        "event_replay_records": events::EVENT_RING_LIMIT,
        "event_bytes": events::EVENT_SIZE_LIMIT,
        "discovery_page_entries": 64,
        "discovery_page_bytes": 8 * 1024,
        "current_page_entries": 64,
        "current_page_bytes": 8 * 1024,
        "recent_history_records": 128,
        "durable_history_records": 128,
        "history_page_bytes": 8 * 1024,
        "history_jobs": 8,
        "subscriptions_per_client": 1,
        "subscription_kinds": 8,
        "subscription_targets": 16,
        "reference_result_records": 1,
        "controller_result_records": 1,
        "pid_configuration_fields": 5,
        "controller_configuration_fields": 6,
        "recorder":{"label_bytes":128,"status_records":1,
            "event_bytes":events::EVENT_SIZE_LIMIT},
        "capabilities": CAPABILITY_LIMIT,
        "semantic_name_bytes": SEMANTIC_NAME_LIMIT,
        "error_message_bytes": ERROR_MESSAGE_LIMIT,
        "error_details_bytes": ERROR_DETAILS_LIMIT
    });
    limits
        .as_object_mut()
        .expect("protocol limits are an object")
        .insert("configuration".into(), configuration);
    limits
        .as_object_mut()
        .expect("protocol limits are an object")
        .insert("emulator".into(), emulator);
    limits
        .as_object_mut()
        .expect("protocol limits are an object")
        .insert(
            "history_cursors".into(),
            json!(crate::application::MAX_HISTORY_CURSORS),
        );
    limits
}

/// One allowlisted bounded public error description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicError {
    code: &'static str,
    category: &'static str,
    message: &'static str,
    retryable: bool,
    resync_required: bool,
}

impl PublicError {
    /// Map an expected internal code to bounded public semantics.
    pub fn from_code(code: &str) -> Self {
        error_spec(code).unwrap_or_else(operation_failed)
    }

    /// Map arbitrary internal text without ever copying it to the wire.
    pub fn from_internal_code(code: &str) -> Self {
        error_spec(code).unwrap_or_else(operation_failed)
    }

    /// Stable public code, which may be more specific than the category.
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Stable broad machine-readable class.
    pub const fn category(self) -> &'static str {
        self.category
    }

    /// Fixed bounded human-readable explanation.
    pub const fn message(self) -> &'static str {
        self.message
    }

    /// Whether retry may make sense without changing the rejected request.
    pub const fn retryable(self) -> bool {
        self.retryable
    }

    /// Whether the client must rebuild state before continuing incrementally.
    pub const fn resync_required(self) -> bool {
        self.resync_required
    }

    /// Insert the common fields into an existing response object.
    pub fn apply_to(self, value: &mut Value) {
        value["code"] = json!(self.code);
        value["category"] = json!(self.category);
        value["message"] = json!(self.message);
        value["retryable"] = json!(self.retryable);
        value["resync_required"] = json!(self.resync_required);
    }

    /// Attach trusted structured details only when their serialized form fits.
    pub fn apply_details(self, value: &mut Value, details: Value) -> bool {
        if serde_json::to_vec(&details).is_ok_and(|encoded| encoded.len() <= ERROR_DETAILS_LIMIT) {
            value["details"] = details;
            true
        } else {
            false
        }
    }
}

const fn public_error(
    code: &'static str,
    category: &'static str,
    message: &'static str,
    retryable: bool,
    resync_required: bool,
) -> PublicError {
    PublicError {
        code,
        category,
        message,
        retryable,
        resync_required,
    }
}

const fn operation_failed() -> PublicError {
    public_error(
        "operation_failed",
        "operation_failed",
        "The operation failed.",
        false,
        false,
    )
}

fn error_spec(code: &str) -> Option<PublicError> {
    let result = match code {
        "invalid_args" | "invalid_id" | "missing_request_id" | "invalid_shape"
        | "unknown_field" | "duplicate_key" | "json_values" | "string_too_large"
        | "invalid_number" | "invalid_cursor" | "duplicate_msg_id" | "already_hello"
        | "hello_required" | "scope_unknown" | "scope_in_use" | "sequence_gap"
        | "request_conflict" | "outcome_unknown" => public_error(
            code_for_invalid(code),
            "invalid_request",
            message_for_invalid(code),
            false,
            false,
        ),
        "unsupported_operation" | "unsupported_history_mode" | "configuration_disabled" => {
            public_error(
                canonical_code(code),
                "unsupported_operation",
                "The requested operation is not supported by this Runtime.",
                false,
                false,
            )
        }
        "invalid_configuration" | "invalid_state" | "output_rejected" | "domain_rejected" => {
            public_error(
                canonical_code(code),
                "invalid_configuration",
                "The requested configuration or state transition is invalid.",
                false,
                false,
            )
        }
        "revision_conflict" | "history_archive_mismatch" | "history_cursor_mismatch" => {
            public_error(
                canonical_code(code),
                "revision_conflict",
                "The request conflicts with current authoritative state.",
                false,
                true,
            )
        }
        "unknown_controller"
        | "unknown_reference"
        | "unknown_actuator"
        | "unknown_resource"
        | "unknown_instrument"
        | "unknown_parameter"
        | "unknown_signal"
        | "history_database_unknown"
        | "history_page_expired"
        | "history_cursor_expired"
        | "snapshot_expired" => public_error(
            canonical_code(code),
            "not_found",
            "The requested object or retained result is not available.",
            false,
            false,
        ),
        "input_unavailable"
        | "stale_input"
        | "client_disconnected"
        | "shutdown_in_progress"
        | "shutdown_before_execution" => public_error(
            canonical_code(code),
            "unavailable",
            "The requested service or input is currently unavailable.",
            true,
            false,
        ),
        "transport_unavailable" => public_error(
            canonical_code(code),
            "transport_unavailable",
            "The required transport is unavailable.",
            true,
            false,
        ),
        "recording_unavailable" | "recorder_disabled" | "recording_failed" => public_error(
            canonical_code(code),
            "recording_unavailable",
            "Required recording service is unavailable.",
            true,
            false,
        ),
        "history_timeout" | "timeout" => public_error(
            canonical_code(code),
            "timeout",
            "The operation reached its bounded deadline.",
            true,
            false,
        ),
        "busy"
        | "history_busy"
        | "subscription_busy"
        | "snapshot_capacity"
        | "configuration_capacity"
        | "history_page_oversize"
        | "history_token_exhausted" => public_error(
            canonical_code(code),
            "capacity_exhausted",
            "A bounded service capacity is exhausted.",
            true,
            false,
        ),
        "instance_changed" | "event_gap" => public_error(
            canonical_code(code),
            "protocol_error",
            "Incremental client state is no longer valid; resynchronization is required.",
            false,
            true,
        ),
        "frame_too_large" | "incomplete_frame" | "invalid_utf8" | "invalid_json" | "json_depth"
        | "version_mismatch" | "encode_failed" | "response_too_large" | "event_error"
        | "internal_error" => public_error(
            canonical_code(code),
            "protocol_error",
            message_for_protocol(code),
            false,
            false,
        ),
        "operation_failed" => operation_failed(),
        _ => return None,
    };
    debug_assert!(result.code.len() <= SEMANTIC_NAME_LIMIT);
    debug_assert!(result.category.len() <= SEMANTIC_NAME_LIMIT);
    debug_assert!(result.message.len() <= ERROR_MESSAGE_LIMIT);
    Some(result)
}

fn code_for_invalid(code: &str) -> &'static str {
    match code {
        "invalid_args" => "invalid_args",
        "invalid_id" => "invalid_id",
        "missing_request_id" => "missing_request_id",
        "invalid_shape" => "invalid_shape",
        "unknown_field" => "unknown_field",
        "duplicate_key" => "duplicate_key",
        "json_values" => "json_values",
        "string_too_large" => "string_too_large",
        "invalid_number" => "invalid_number",
        "invalid_cursor" => "invalid_cursor",
        "duplicate_msg_id" => "duplicate_msg_id",
        "already_hello" => "already_hello",
        "hello_required" => "hello_required",
        "scope_unknown" => "scope_unknown",
        "scope_in_use" => "scope_in_use",
        "sequence_gap" => "sequence_gap",
        "request_conflict" => "request_conflict",
        "outcome_unknown" => "outcome_unknown",
        _ => "invalid_request",
    }
}

fn message_for_invalid(code: &str) -> &'static str {
    match code {
        "hello_required" => "A successful hello is required before this request.",
        "already_hello" => "This connection already completed hello.",
        "duplicate_msg_id" => "The connection already has this message identifier in flight.",
        "missing_request_id" => "A mutating operation requires a request identity.",
        "sequence_gap" => "The operation sequence is not the next admissible sequence.",
        "request_conflict" => "The request identity was previously used for different work.",
        "outcome_unknown" => "The retained operation outcome is no longer known.",
        _ => "The request does not satisfy the bounded protocol schema.",
    }
}

fn message_for_protocol(code: &str) -> &'static str {
    match code {
        "frame_too_large" => "The NDJSON frame exceeds the protocol byte limit.",
        "version_mismatch" => "The requested protocol version is not supported.",
        "response_too_large" => "The operation result exceeds the protocol frame limit.",
        _ => "The request or response violates the protocol contract.",
    }
}

fn canonical_code(code: &str) -> &'static str {
    match code {
        "unsupported_operation" => "unsupported_operation",
        "unsupported_history_mode" => "unsupported_history_mode",
        "invalid_configuration" => "invalid_configuration",
        "invalid_state" => "invalid_state",
        "output_rejected" => "output_rejected",
        "domain_rejected" => "domain_rejected",
        "revision_conflict" => "revision_conflict",
        "history_archive_mismatch" => "history_archive_mismatch",
        "history_cursor_mismatch" => "history_cursor_mismatch",
        "unknown_controller" => "unknown_controller",
        "unknown_reference" => "unknown_reference",
        "unknown_actuator" => "unknown_actuator",
        "unknown_resource" => "unknown_resource",
        "unknown_instrument" => "unknown_instrument",
        "unknown_parameter" => "unknown_parameter",
        "unknown_signal" => "unknown_signal",
        "history_database_unknown" => "history_database_unknown",
        "history_page_expired" => "history_page_expired",
        "history_cursor_expired" => "history_cursor_expired",
        "snapshot_expired" => "snapshot_expired",
        "input_unavailable" => "input_unavailable",
        "stale_input" => "stale_input",
        "client_disconnected" => "client_disconnected",
        "shutdown_in_progress" => "shutdown_in_progress",
        "shutdown_before_execution" => "shutdown_before_execution",
        "transport_unavailable" => "transport_unavailable",
        "recording_unavailable" => "recording_unavailable",
        "recorder_disabled" => "recorder_disabled",
        "recording_failed" => "recording_failed",
        "history_timeout" => "history_timeout",
        "timeout" => "timeout",
        "busy" => "busy",
        "history_busy" => "history_busy",
        "subscription_busy" => "subscription_busy",
        "snapshot_capacity" => "snapshot_capacity",
        "history_page_oversize" => "history_page_oversize",
        "history_token_exhausted" => "history_token_exhausted",
        "instance_changed" => "instance_changed",
        "event_gap" => "event_gap",
        "frame_too_large" => "frame_too_large",
        "incomplete_frame" => "incomplete_frame",
        "invalid_utf8" => "invalid_utf8",
        "invalid_json" => "invalid_json",
        "json_depth" => "json_depth",
        "version_mismatch" => "version_mismatch",
        "encode_failed" => "encode_failed",
        "response_too_large" => "response_too_large",
        "event_error" => "event_error",
        "internal_error" => "internal_error",
        _ => "operation_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_and_errors_fit_their_published_bounds() {
        assert!(OPERATIONS.len() <= 64);
        assert!(OPERATIONS.iter().all(|operation| {
            operation.name.len() <= SEMANTIC_NAME_LIMIT
                && operation
                    .argument_fields
                    .iter()
                    .all(|field| field.len() <= SEMANTIC_NAME_LIMIT)
        }));
        assert!(CAPABILITIES.len() <= CAPABILITY_LIMIT);
        assert!(CAPABILITIES.iter().all(|capability| {
            capability.name.len() <= SEMANTIC_NAME_LIMIT
                && operation_spec(capability.operation).is_some()
        }));
        for code in [
            "invalid_args",
            "unsupported_operation",
            "invalid_configuration",
            "revision_conflict",
            "unknown_signal",
            "input_unavailable",
            "transport_unavailable",
            "recording_unavailable",
            "history_timeout",
            "busy",
            "operation_failed",
            "version_mismatch",
        ] {
            let error = PublicError::from_code(code);
            assert!(error.code().len() <= SEMANTIC_NAME_LIMIT);
            assert!(error.category().len() <= SEMANTIC_NAME_LIMIT);
            assert!(error.message().len() <= ERROR_MESSAGE_LIMIT);
        }
        let error = PublicError::from_code("invalid_args");
        let mut response = json!({});
        assert!(error.apply_details(&mut response, json!({"field":"scope"})));
        assert!(!error.apply_details(
            &mut response,
            json!({"text":"x".repeat(ERROR_DETAILS_LIMIT + 1)})
        ));
        assert_eq!(response["details"], json!({"field":"scope"}));
    }

    #[test]
    fn review_inventory_is_an_exact_projection_of_the_registry() {
        let queries = [
            "hello",
            "discover",
            "discovery_page",
            "describe",
            "resource",
            "configuration_status",
            "configuration_properties",
            "configuration_page",
            "latest",
            "measurements_current",
            "measurements_page",
            "measurement_window",
            "controller",
            "reference",
            "component",
            "output",
            "operation_status",
            "subscribe",
            "unsubscribe",
            "recording_status",
            "history_page",
            "history_release",
        ];
        let mutations = [
            "reference_configure",
            "reference_retune",
            "controller_configure_pid",
            "controller_configure",
            "controller_start",
            "controller_pause",
            "controller_resume",
            "controller_reset_failed",
            "runtime_shutdown",
            "stage_configuration",
            "apply_configuration",
            "reload_configuration",
            "property_configure",
            "emulator_publish",
            "virtual_models_restart",
            "reconnect_resource",
            "recording_start",
            "recording_stop",
            "experiment_annotate",
            "history_read",
        ];
        let projected = |kind| {
            OPERATIONS
                .iter()
                .filter(|operation| operation.kind == kind)
                .map(|operation| operation.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(projected(OperationKind::Query), queries);
        assert_eq!(projected(OperationKind::Mutation), mutations);
        assert_eq!(OPERATIONS.len(), 42);
        let capabilities = [
            "operation_lifecycle",
            "structured_discovery",
            "live_subscriptions",
            "current_measurements",
            "recent_measurement_history",
            "instrument_queries",
            "reference_read_write",
            "controller_status",
            "controller_configuration",
            "controller_lifecycle",
            "managed_components",
            "output_status",
            "runtime_shutdown",
            "recording_status",
            "recording_control",
            "measurement_history",
            "resource_status",
            "configuration_read",
            "configuration_properties",
            "configuration_write",
            "deployment_configuration",
            "resource_reconnect",
            "virtual_instruments",
            "emulator_publication",
            "virtual_model_lifecycle",
        ];
        assert_eq!(
            CAPABILITIES
                .iter()
                .map(|capability| capability.name)
                .collect::<Vec<_>>(),
            capabilities
        );
        assert_eq!(CAPABILITIES.len(), 25);
        assert!(CAPABILITIES.iter().all(|capability| {
            matches!(capability.stability, "stable" | "transitional")
                && operation_spec(capability.operation).is_some()
        }));
        assert!(OPERATIONS.iter().all(|operation| {
            operation
                .argument_fields
                .iter()
                .enumerate()
                .all(|(index, field)| !operation.argument_fields[..index].contains(field))
        }));

        let registry_contract = OPERATIONS
            .iter()
            .map(|operation| {
                format!(
                    "{}|{:?}|{:?}|{}",
                    operation.name,
                    operation.kind,
                    operation.availability,
                    operation.argument_fields.join(",")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            registry_contract,
            concat!(
                "hello|Query|Always|scope\n",
                "discover|Query|Always|\n",
                "discovery_page|Query|Always|projection,index\n",
                "describe|Query|Always|instrument\n",
                "resource|Query|ResourceReconnect|resource\n",
                "configuration_status|Query|Configuration|\n",
                "configuration_properties|Query|Configuration|\n",
                "configuration_page|Query|Configuration|projection,index\n",
                "latest|Query|Always|signal\n",
                "measurements_current|Query|Always|\n",
                "measurements_page|Query|Always|projection,index\n",
                "measurement_window|Query|Always|signal,max_records\n",
                "controller|Query|Always|controller\n",
                "reference|Query|Always|reference\n",
                "component|Query|Always|component\n",
                "output|Query|Always|actuator\n",
                "operation_status|Query|Always|request_id\n",
                "subscribe|Query|Always|after,filter\n",
                "unsubscribe|Query|Always|subscription\n",
                "reference_configure|Mutation|Always|reference,expected_revision,kind,value,target,rate\n",
                "reference_retune|Mutation|Always|reference,expected_revision,target,rate\n",
                "controller_configure_pid|Mutation|Always|controller,expected_revision,pid\n",
                "controller_configure|Mutation|Always|controller,expected_revision,pid,ema,max_input_age_ns,max_tick_gap_ns,lease_lifetime_ns,proposal_ttl_ns\n",
                "controller_start|Mutation|Always|controller\n",
                "controller_pause|Mutation|Always|controller\n",
                "controller_resume|Mutation|Always|controller\n",
                "controller_reset_failed|Mutation|Always|controller\n",
                "runtime_shutdown|Mutation|Always|\n",
                "stage_configuration|Mutation|Configuration|\n",
                "apply_configuration|Mutation|Configuration|candidate_id,expected_revision\n",
                "reload_configuration|Mutation|Configuration|\n",
                "property_configure|Mutation|Configuration|target,property,value,expected_revision\n",
                "emulator_publish|Mutation|EmulatorPublication|signal,state,value,expected_generation\n",
                "virtual_models_restart|Mutation|VirtualModelLifecycle|\n",
                "reconnect_resource|Mutation|ResourceReconnect|resource,expected_binding_generation\n",
                "recording_status|Query|Always|\n",
                "recording_start|Mutation|Recorder|label\n",
                "recording_stop|Mutation|Recorder|run_id\n",
                "experiment_annotate|Mutation|Recorder|name,data\n",
                "history_read|Mutation|Recorder|mode,database_id,boot_id,run_id,signal,from_ns,to_ns,max_records,cursor\n",
                "history_page|Query|Recorder|page_token\n",
                "history_release|Query|Recorder|page_token"
            )
        );
        let capability_contract = CAPABILITIES
            .iter()
            .map(|capability| {
                format!(
                    "{}|{}|{}",
                    capability.name, capability.stability, capability.operation
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            capability_contract,
            concat!(
                "operation_lifecycle|stable|operation_status\n",
                "structured_discovery|stable|discover\n",
                "live_subscriptions|stable|subscribe\n",
                "current_measurements|stable|measurements_current\n",
                "recent_measurement_history|stable|measurement_window\n",
                "instrument_queries|stable|describe\n",
                "reference_read_write|stable|reference_configure\n",
                "controller_status|stable|controller\n",
                "controller_configuration|stable|controller_configure\n",
                "controller_lifecycle|stable|controller_start\n",
                "managed_components|stable|component\n",
                "output_status|stable|output\n",
                "runtime_shutdown|stable|runtime_shutdown\n",
                "recording_status|stable|recording_status\n",
                "recording_control|stable|recording_start\n",
                "measurement_history|stable|history_read\n",
                "resource_status|stable|resource\n",
                "configuration_read|stable|configuration_status\n",
                "configuration_properties|stable|configuration_properties\n",
                "configuration_write|stable|property_configure\n",
                "deployment_configuration|stable|stage_configuration\n",
                "resource_reconnect|stable|reconnect_resource\n",
                "virtual_instruments|stable|discover\n",
                "emulator_publication|stable|emulator_publish\n",
                "virtual_model_lifecycle|stable|virtual_models_restart"
            )
        );
    }
}
