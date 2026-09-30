//! Bounded, side-effect-free deployment parsing and validation.
//!
//! This module owns TOML and filesystem-shaped artifact identities outside Core.
//! Parsing never opens a transport, SQLite database or listener. The
//! returned bundle owns the exact admitted bytes so later activation never has
//! to reread a mutable pathname for provenance.
//!
//! [`crate::configuration::FrozenDeployment`] preserves the persistent deployment source/artifacts and a
//! fully validated effective DTO. Generic Application property edits add bounded
//! process-local property overlays to a cloned candidate, then re-enter the same
//! validation/stage/apply lifecycle in [`crate::deployment`]. Runtime samples,
//! controller state and generations are ephemeral experiment state, not persistent
//! deployment configuration.

use crate::{
    definition::{MAX_DEFINITION_BYTES, parse_definition_json},
    managed_executor::{
        NativeComponentDefinition, component_property_metadata, validate_component_configuration,
    },
    simple_device::{
        CompiledSimpleDefinition, CompiledSimpleInstance, MAX_SIMPLE_DEFINITION_BYTES,
        MAX_SIMPLE_DEFINITIONS, MAX_SIMPLE_INSTANCES, SimpleCandidateController,
        SimpleCandidateEvidence, SimpleDeviceCandidate, compile_simple_instance,
        correlations_distinguish, parse_simple_definition,
    },
    websocket,
};
use lab_core::{
    AccessMode, InstrumentId, ParameterRole, SignalId, Unit, Value, ValueSpec, WriteEffect,
    instrument::KnownOperation,
    managed::{ComponentId, ComponentImplementationId, PlainData, PlainValue},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Maximum exact bytes admitted for one `runtime.toml` document.
pub const MAX_RUNTIME_TOML_BYTES: usize = 64 * 1024;
/// Maximum syntactic assignments/tables admitted before deserialization.
pub const MAX_RUNTIME_TOML_VALUES: usize = 4096;
/// Maximum configuration nesting admitted by the inexpensive pre-scan.
pub const MAX_RUNTIME_TOML_DEPTH: usize = 8;
/// Maximum immutable provenance entries in one deployment.
pub const MAX_DEPLOYMENT_ARTIFACTS: usize = 128;
/// Maximum exact artifact bytes retained by one deployment candidate.
pub const MAX_DEPLOYMENT_BYTES: usize = 1024 * 1024;
/// Maximum process-local property overlays retained above one deployment source.
pub(crate) const MAX_PROPERTY_OVERLAYS: usize = 32;

/// A bounded source used by validation to freeze referenced files.
///
/// Implementations may perform local file reads. They must not open devices,
/// databases, listeners or execute source code. The caller supplies the maximum
/// before a read so an implementation can enforce it while streaming.
pub trait ArtifactReader {
    /// Read at most `maximum` bytes or return a bounded diagnostic.
    fn read(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError>;
}

/// Local filesystem artifact reader used by trusted service composition.
pub struct FileArtifactReader;

impl ArtifactReader for FileArtifactReader {
    fn read(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>, ConfigurationError> {
        let metadata = fs::metadata(path).map_err(|error| {
            ConfigurationError::artifact_text(&format!("artifact metadata: {error}"))
        })?;
        let length = usize::try_from(metadata.len()).map_err(|_| ConfigurationError::TooLarge)?;
        if length > maximum {
            return Err(ConfigurationError::artifact("artifact exceeds limit"));
        }
        let bytes = fs::read(path).map_err(|error| {
            ConfigurationError::artifact_text(&format!("artifact read: {error}"))
        })?;
        if bytes.len() > maximum {
            return Err(ConfigurationError::artifact("artifact exceeds limit"));
        }
        Ok(bytes)
    }
}

/// Rejected configuration or referenced artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationError {
    /// The main document or aggregate immutable bundle exceeded a fixed limit.
    TooLarge,
    /// UTF-8 BOM or invalid UTF-8 was rejected before TOML parsing.
    InvalidUtf8,
    /// A cheap syntactic bound failed before deserialization.
    SyntaxBound(&'static str),
    /// TOML syntax, duplicate or unknown fields were rejected.
    InvalidToml(String),
    /// Parsed values violate the deployment schema or cross references.
    InvalidConfiguration(String),
    /// A referenced local immutable artifact could not be frozen or validated.
    InvalidArtifact(String),
}

impl ConfigurationError {
    /// Construct a bounded static artifact diagnostic for custom readers.
    pub fn artifact(reason: &'static str) -> Self {
        Self::InvalidArtifact(reason.into())
    }

    fn artifact_text(reason: &str) -> Self {
        Self::InvalidArtifact(bounded_message(reason))
    }

    fn invalid(reason: impl AsRef<str>) -> Self {
        Self::InvalidConfiguration(bounded_message(reason.as_ref()))
    }
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("deployment exceeds bounded size"),
            Self::InvalidUtf8 => formatter.write_str("runtime.toml must be UTF-8 without BOM"),
            Self::SyntaxBound(reason) => write!(formatter, "runtime.toml {reason}"),
            Self::InvalidToml(reason) => write!(formatter, "invalid runtime.toml: {reason}"),
            Self::InvalidConfiguration(reason) => write!(formatter, "invalid deployment: {reason}"),
            Self::InvalidArtifact(reason) => {
                write!(formatter, "invalid deployment artifact: {reason}")
            }
        }
    }
}

impl std::error::Error for ConfigurationError {}

/// Immutable exact content loaded for one declared source or definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenArtifact {
    kind: ArtifactKind,
    declared_path: PathBuf,
    bytes: Arc<[u8]>,
    sha256: [u8; 32],
}

impl FrozenArtifact {
    /// Exact admitted bytes; callers never reconstruct them from the pathname.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// SHA-256 identity of the exact admitted bytes.
    pub const fn hash(&self) -> [u8; 32] {
        self.sha256
    }

    /// Original resolved path retained only as metadata.
    pub fn declared_path(&self) -> &Path {
        &self.declared_path
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ArtifactKind {
    InstrumentDefinition,
    SimpleDeviceDefinition,
}

/// Fully validated typed deployment used to build a staged Runtime candidate.
///
/// Fields remain private so adapters cannot bypass validation by constructing an
/// instance. Stable read-only accessors are added only where composition needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveDeployment {
    pub(crate) dto: DeploymentDto,
}

impl EffectiveDeployment {
    /// Requested loopback port.
    pub const fn server_port(&self) -> u16 {
        self.dto.server.port
    }

    /// Stable cross-boot deployment key.
    pub fn runtime_key(&self) -> &str {
        &self.dto.runtime.key
    }

    /// Whether durable recording is configured.
    pub const fn recording_enabled(&self) -> bool {
        self.dto.recording.enabled
    }

    /// Number of declared physical resources.
    pub fn resource_count(&self) -> usize {
        self.dto.resources.len()
    }
}

/// Exact loaded source set plus its validated effective meaning.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenDeployment {
    base: PathBuf,
    toml_bytes: Arc<[u8]>,
    toml_hash: [u8; 32],
    effective: EffectiveDeployment,
    artifacts: Vec<FrozenArtifact>,
    simple_definitions: Vec<(PathBuf, CompiledSimpleDefinition)>,
    property_overlays: Vec<String>,
    api_simple_instruments: BTreeSet<u64>,
}

/// One bounded scalar admitted by the generic live property operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PropertyValue {
    /// Signed integer property.
    Integer(i64),
    /// UTF-8 text property.
    Text(String),
}

impl FrozenDeployment {
    /// Exact `runtime.toml` bytes accepted by validation.
    pub fn toml_bytes(&self) -> &[u8] {
        &self.toml_bytes
    }

    /// SHA-256 identity of the exact TOML bytes.
    pub const fn toml_hash(&self) -> [u8; 32] {
        self.toml_hash
    }

    /// Fully validated typed effective deployment.
    pub const fn effective(&self) -> &EffectiveDeployment {
        &self.effective
    }

    /// Referenced deployment artifacts frozen during validation.
    pub fn artifacts(&self) -> &[FrozenArtifact] {
        &self.artifacts
    }

    /// Number of process-local property overlays applied above the deployment file.
    pub(crate) fn property_overlay_count(&self) -> usize {
        self.property_overlays.len()
    }

    /// Produce a fully validated fixed-topology candidate for one supported live property.
    pub(crate) fn with_property_override(
        &self,
        target_kind: &str,
        target_id: u64,
        property: &str,
        value: PropertyValue,
    ) -> Result<Self, ConfigurationError> {
        let mut next = self.clone();
        let recorded_value = match &value {
            PropertyValue::Integer(value) => serde_json::json!(value),
            PropertyValue::Text(value) => serde_json::json!(value),
        };
        match target_kind {
            "instrument" => {
                let instrument = next
                    .effective
                    .dto
                    .instruments
                    .iter_mut()
                    .find(|instrument| instrument.id() == target_id)
                    .ok_or_else(|| ConfigurationError::invalid("unknown property target"))?;
                match (property, value) {
                    ("display_name", PropertyValue::Text(value)) => {
                        validate_display_name(&value)?;
                        match instrument {
                            InstrumentDto::VirtualMeasurement { display_name, .. }
                            | InstrumentDto::ThermalPlant { display_name, .. } => {
                                *display_name = value
                            }
                            InstrumentDto::Metakon { .. } | InstrumentDto::SimpleDevice { .. } => {
                                return Err(ConfigurationError::invalid("property is read-only"));
                            }
                        }
                    }
                    ("poll_period_ms", PropertyValue::Integer(value)) => {
                        let value = u64::try_from(value)
                            .map_err(|_| ConfigurationError::invalid("invalid property value"))?;
                        validate_period(value, "poll period")?;
                        if matches!(instrument, InstrumentDto::SimpleDevice { .. }) && value < 10 {
                            return Err(ConfigurationError::invalid(
                                "simple-device poll period must be 10..=60000 ms",
                            ));
                        }
                        match instrument {
                            InstrumentDto::VirtualMeasurement { poll_period_ms, .. }
                            | InstrumentDto::ThermalPlant { poll_period_ms, .. }
                            | InstrumentDto::Metakon { poll_period_ms, .. }
                            | InstrumentDto::SimpleDevice { poll_period_ms, .. } => {
                                *poll_period_ms = value
                            }
                        }
                    }
                    _ => return Err(ConfigurationError::invalid("unsupported property")),
                }
            }
            _ => return Err(ConfigurationError::invalid("unsupported property target")),
        }
        if next.property_overlays.len() >= MAX_PROPERTY_OVERLAYS {
            return Err(ConfigurationError::TooLarge);
        }
        next.property_overlays.push(
            serde_json::json!({"target":{"kind":target_kind,"id":target_id.to_string()},
                "property":property,"value":recorded_value})
            .to_string(),
        );
        Ok(next)
    }

    /// Build one fully validated process-local deployment overlay from an
    /// already normalized Application candidate. No source file is read or
    /// rewritten; the embedded definition becomes one immutable frozen artifact.
    pub(crate) fn with_simple_device_candidate(
        &self,
        candidate: &SimpleDeviceCandidate,
    ) -> Result<Self, ConfigurationError> {
        let mut next = self.clone();
        let definition_name = format!(
            "__application__/{}-{}-{}.json",
            candidate.definition.definition_id,
            candidate.definition.definition_version,
            hex_sha256(candidate.definition.canonical_sha256)
        );
        let declared = PathBuf::from(definition_name);
        let resolved = resolve_artifact(&next.base, &declared)?;
        let existing_definition = next.simple_definitions.iter().find(|(_, definition)| {
            definition.definition_id == candidate.definition.definition_id
                && definition.definition_version == candidate.definition.definition_version
        });
        let definition_path = if let Some((path, definition)) = existing_definition {
            if definition.canonical_sha256 != candidate.definition.canonical_sha256 {
                return Err(ConfigurationError::invalid(
                    "simple definition ID/version has conflicting normalized content",
                ));
            }
            path.strip_prefix(&next.base)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone())
        } else {
            if next.simple_definitions.len() >= MAX_SIMPLE_DEFINITIONS {
                return Err(ConfigurationError::invalid(
                    "simple-device definition limit reached (16)",
                ));
            }
            if next.artifacts.len() >= MAX_DEPLOYMENT_ARTIFACTS
                || next
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.bytes.len())
                    .sum::<usize>()
                    .saturating_add(candidate.definition.canonical.len())
                    > MAX_DEPLOYMENT_BYTES
            {
                return Err(ConfigurationError::TooLarge);
            }
            next.artifacts.push(FrozenArtifact {
                kind: ArtifactKind::SimpleDeviceDefinition,
                declared_path: resolved.clone(),
                bytes: candidate.definition.canonical.clone(),
                sha256: candidate.definition.canonical_sha256,
            });
            next.simple_definitions
                .push((resolved, candidate.definition.clone()));
            declared
        };

        for instance in &candidate.instances {
            next.api_simple_instruments.insert(instance.instrument_id);
            next.effective
                .dto
                .instruments
                .push(InstrumentDto::SimpleDevice {
                    id: instance.instrument_id,
                    key: instance.key.clone(),
                    display_name: instance.display_name.clone(),
                    definition: definition_path.clone(),
                    resource_id: instance.resource_id,
                    address: instance.address,
                    channel: instance.channel,
                    poll_period_ms: instance.poll_period_ms,
                    queue_timeout_ms: instance.queue_timeout_ms,
                    transaction_timeout_ms: instance.transaction_timeout_ms,
                    history_capacity: instance.history_capacity,
                });
            if let Some(profile) = instance.safe_profile {
                let actuator = instance
                    .compiled
                    .parameters
                    .iter()
                    .find(|parameter| parameter.descriptor.role == ParameterRole::Actuator)
                    .ok_or_else(|| ConfigurationError::invalid("safe profile lacks actuator"))?;
                next.effective.dto.safe_profiles.push(SafeProfileDto {
                    instrument_id: instance.instrument_id,
                    parameter_id: actuator.descriptor.id.get(),
                    min: profile.min,
                    max: profile.max,
                    safe_value: profile.safe_value,
                    max_lease_ms: profile.max_lease_ms,
                    max_proposal_ttl_ms: profile.max_proposal_ttl_ms,
                    required_evidence: match profile.required_evidence {
                        SimpleCandidateEvidence::Ack => EvidenceDto::Ack,
                        SimpleCandidateEvidence::Readback => EvidenceDto::Readback,
                    },
                });
            }
            if let Some(controller) = &instance.controller {
                next.effective
                    .dto
                    .controllers
                    .push(controller_dto(controller));
            }
        }
        next.effective
            .dto
            .instruments
            .sort_by_key(InstrumentDto::id);
        next.effective.dto.controllers.sort_by_key(|item| item.id);
        next.effective
            .dto
            .safe_profiles
            .sort_by_key(|item| (item.instrument_id, item.parameter_id));
        validate_structure(&next.effective.dto)?;
        validate_frozen_simple_definitions(
            &next.effective.dto,
            &next.base,
            &next.simple_definitions,
        )?;
        next.property_overlays.push(
            serde_json::json!({
                "source":"process_local_application_candidate",
                "schema_version":candidate.schema_version,
                "definition_id":candidate.definition.definition_id,
                "definition_version":candidate.definition.definition_version,
                "definition_sha256":hex_sha256(candidate.definition.canonical_sha256),
                "candidate_sha256":hex_sha256(Sha256::digest(&candidate.canonical).into()),
                "instances":candidate.instances.iter().map(|instance|instance.instrument_id.to_string()).collect::<Vec<_>>()
            })
            .to_string(),
        );
        if next.property_overlays.len() > MAX_PROPERTY_OVERLAYS {
            return Err(ConfigurationError::TooLarge);
        }
        Ok(next)
    }

    pub(crate) fn resolved_path(&self, declared: &Path) -> Result<PathBuf, ConfigurationError> {
        resolve_artifact(&self.base, declared)
    }

    pub(crate) fn artifact_bytes(&self, declared: &Path) -> Option<&[u8]> {
        let resolved = self.resolved_path(declared).ok()?;
        self.artifacts
            .iter()
            .find(|artifact| artifact.declared_path == resolved)
            .map(FrozenArtifact::bytes)
    }

    pub(crate) fn simple_definition(&self, declared: &Path) -> Option<&CompiledSimpleDefinition> {
        let resolved = self.resolved_path(declared).ok()?;
        self.simple_definitions
            .iter()
            .find(|(path, _)| *path == resolved)
            .map(|(_, definition)| definition)
    }

    pub(crate) fn provenance_entries(
        &self,
        configuration_revision: u64,
    ) -> Vec<(String, String, Vec<u8>)> {
        let mut entries =
            Vec::with_capacity(self.artifacts.len() + self.property_overlays.len() + 1);
        entries.push((
            "runtime_toml".into(),
            "utf8".into(),
            self.toml_bytes.to_vec(),
        ));
        entries.extend(self.artifacts.iter().map(|artifact| {
            (
                match artifact.kind {
                    ArtifactKind::InstrumentDefinition => "instrument_definition",
                    ArtifactKind::SimpleDeviceDefinition => "simple_device_definition_raw",
                }
                .into(),
                "utf8".into(),
                artifact.bytes.to_vec(),
            )
        }));
        entries.extend(self.simple_definitions.iter().map(|(_, definition)| {
            (
                "simple_device_definition_canonical".into(),
                "json".into(),
                definition.canonical.to_vec(),
            )
        }));
        entries.extend(
            self.effective
                .dto
                .instruments
                .iter()
                .filter_map(|instrument| {
                    let InstrumentDto::SimpleDevice {
                        id,
                        key,
                        definition,
                        resource_id,
                        address,
                        channel,
                        poll_period_ms,
                        queue_timeout_ms,
                        transaction_timeout_ms,
                        history_capacity,
                        ..
                    } = instrument
                    else {
                        return None;
                    };
                    let compiled = self.simple_definition(definition)?;
                    let application_source = self.api_simple_instruments.contains(id);
                    let resolved = self.resolved_path(definition).ok()?;
                    let raw_sha256 = self
                        .artifacts
                        .iter()
                        .find(|artifact| artifact.declared_path == resolved)
                        .map(|artifact| hex_sha256(artifact.sha256))?;
                    let content = serde_json::json!({
                        "grammar_version":1,
                        "source":if application_source {"process_local_application_candidate"} else {"persistent_startup_deployment"},
                        "configuration_revision":configuration_revision.to_string(),
                        "instrument_id":id.to_string(),
                        "instrument_key":key,
                        "definition_source":definition.to_string_lossy(),
                        "definition_raw_sha256":raw_sha256,
                        "definition_id":compiled.definition_id,
                        "definition_version":compiled.definition_version,
                        "definition_sha256":hex_sha256(compiled.canonical_sha256),
                        "resource_id":resource_id.to_string(),
                        "binding_generation":"1",
                        "mapping_revision":"1",
                        "address":address,
                        "channel":channel,
                        "poll_period_ms":poll_period_ms,
                        "queue_timeout_ms":queue_timeout_ms,
                        "transaction_timeout_ms":transaction_timeout_ms,
                        "history_capacity":history_capacity,
                    })
                    .to_string()
                    .into_bytes();
                    Some(("simple_device_instance".into(), "json".into(), content))
                }),
        );
        entries.extend(self.property_overlays.iter().cloned().map(|content| {
            (
                "runtime_configuration_overlay".into(),
                "json".into(),
                content.into_bytes(),
            )
        }));
        entries
    }

    pub(crate) fn changes_from(&self, active: &Self) -> DeploymentChanges {
        let old = &active.effective.dto;
        let new = &self.effective.dto;
        let old_topology: Vec<_> = old
            .instruments
            .iter()
            .map(|item| (item.id(), item.key(), item.kind_name()))
            .collect();
        let new_topology: Vec<_> = new
            .instruments
            .iter()
            .map(|item| (item.id(), item.key(), item.kind_name()))
            .collect();
        let old_resource_topology: Vec<_> = old
            .resources
            .iter()
            .map(|item| (item.id, item.key.as_str(), item.kind))
            .collect();
        let new_resource_topology: Vec<_> = new
            .resources
            .iter()
            .map(|item| (item.id, item.key.as_str(), item.kind))
            .collect();
        let restart_required = old.runtime.key != new.runtime.key
            || old.server != new.server
            || old.recording != new.recording
            || old_topology != new_topology
            || old_resource_topology != new_resource_topology
            || old.managed_components.len() != new.managed_components.len()
            || old.references.len() != new.references.len()
            || old.controllers.len() != new.controllers.len()
            || old.safe_profiles.len() != new.safe_profiles.len()
            || self.simple_definitions != active.simple_definitions
            || old
                .instruments
                .iter()
                .zip(&new.instruments)
                .any(|(old, new)| match (old, new) {
                    (
                        InstrumentDto::SimpleDevice {
                            definition: old_definition,
                            resource_id: old_resource,
                            address: old_address,
                            channel: old_channel,
                            queue_timeout_ms: old_queue,
                            transaction_timeout_ms: old_timeout,
                            history_capacity: old_history,
                            ..
                        },
                        InstrumentDto::SimpleDevice {
                            definition: new_definition,
                            resource_id: new_resource,
                            address: new_address,
                            channel: new_channel,
                            queue_timeout_ms: new_queue,
                            transaction_timeout_ms: new_timeout,
                            history_capacity: new_history,
                            ..
                        },
                    ) => {
                        old_definition != new_definition
                            || old_resource != new_resource
                            || old_address != new_address
                            || old_channel != new_channel
                            || old_queue != new_queue
                            || old_timeout != new_timeout
                            || old_history != new_history
                    }
                    _ => false,
                });
        let live_safe = self.toml_hash != active.toml_hash
            || old.runtime.display_name != new.runtime.display_name
            || old
                .instruments
                .iter()
                .zip(&new.instruments)
                .any(|(old, new)| match (old, new) {
                    (
                        InstrumentDto::VirtualMeasurement {
                            display_name: old, ..
                        },
                        InstrumentDto::VirtualMeasurement {
                            display_name: new, ..
                        },
                    )
                    | (
                        InstrumentDto::ThermalPlant {
                            display_name: old, ..
                        },
                        InstrumentDto::ThermalPlant {
                            display_name: new, ..
                        },
                    ) => old != new,
                    (
                        InstrumentDto::SimpleDevice {
                            display_name: old, ..
                        },
                        InstrumentDto::SimpleDevice {
                            display_name: new, ..
                        },
                    ) => old != new,
                    _ => false,
                });
        let ordinary_live = old
            .instruments
            .iter()
            .zip(&new.instruments)
            .any(|(old, new)| old.poll_period_ms() != new.poll_period_ms());
        let rebind = old.resources != new.resources
            || self
                .artifacts
                .iter()
                .filter(|artifact| {
                    matches!(
                        artifact.kind,
                        ArtifactKind::InstrumentDefinition | ArtifactKind::SimpleDeviceDefinition
                    )
                })
                .map(|artifact| (artifact.declared_path.as_path(), artifact.sha256))
                .ne(active
                    .artifacts
                    .iter()
                    .filter(|artifact| {
                        matches!(
                            artifact.kind,
                            ArtifactKind::InstrumentDefinition
                                | ArtifactKind::SimpleDeviceDefinition
                        )
                    })
                    .map(|artifact| (artifact.declared_path.as_path(), artifact.sha256)))
            || old
                .instruments
                .iter()
                .zip(&new.instruments)
                .any(|(old, new)| {
                    matches!(
                        (old, new),
                        (InstrumentDto::Metakon { .. }, InstrumentDto::Metakon { .. })
                            | (
                                InstrumentDto::SimpleDevice { .. },
                                InstrumentDto::SimpleDevice { .. }
                            )
                    ) && old.without_live_fields() != new.without_live_fields()
                });
        let reinitialize = old
            .instruments
            .iter()
            .zip(&new.instruments)
            .any(|(old, new)| {
                matches!(
                    (old, new),
                    (
                        InstrumentDto::ThermalPlant { .. },
                        InstrumentDto::ThermalPlant { .. }
                    ) | (
                        InstrumentDto::VirtualMeasurement { .. },
                        InstrumentDto::VirtualMeasurement { .. }
                    )
                ) && old.without_live_fields() != new.without_live_fields()
            })
            || old.managed_components != new.managed_components;
        let controller_rewarm = reinitialize
            || rebind
            || old.references != new.references
            || old.controllers != new.controllers;
        let safe_barrier = controller_rewarm || old.safe_profiles != new.safe_profiles;
        DeploymentChanges {
            restart_required,
            live_safe,
            ordinary_live,
            reinitialize,
            rebind,
            controller_rewarm,
            safe_barrier,
        }
    }
}

fn controller_dto(controller: &SimpleCandidateController) -> ControllerDto {
    ControllerDto {
        id: controller.id,
        key: controller.key.clone(),
        input_instrument_id: controller.input_instrument_id,
        input_parameter_id: controller.input_parameter_id,
        output_instrument_id: controller.output_instrument_id,
        output_parameter_id: controller.output_parameter_id,
        reference_id: controller.reference_id,
        period_ms: controller.period_ms,
        ema_time_constant_ms: controller.ema_time_constant_ms,
        ema_warmup_samples: controller.ema_warmup_samples,
        kp: controller.kp,
        ki: controller.ki,
        kd: controller.kd,
        output_min: controller.output_min,
        output_max: controller.output_max,
        max_input_age_ms: controller.max_input_age_ms,
        max_tick_gap_ms: controller.max_tick_gap_ms,
        lease_lifetime_ms: controller.lease_lifetime_ms,
        proposal_ttl_ms: controller.proposal_ttl_ms,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeploymentChanges {
    pub(crate) restart_required: bool,
    pub(crate) live_safe: bool,
    pub(crate) ordinary_live: bool,
    pub(crate) reinitialize: bool,
    pub(crate) rebind: bool,
    pub(crate) controller_rewarm: bool,
    pub(crate) safe_barrier: bool,
}

/// Read and validate one main file with its parent as the immutable path base.
pub fn load_runtime_toml(path: &Path) -> Result<FrozenDeployment, ConfigurationError> {
    // Freeze the deployment location once, before parsing. Relative paths inside
    // runtime.toml are then owned by that file rather than by a later process
    // working directory. This is lexical absolutization, not canonicalization:
    // the existing parent-traversal rejection remains authoritative.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                ConfigurationError::artifact_text(&format!(
                    "configuration working directory: {error}"
                ))
            })?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| ConfigurationError::invalid("runtime.toml has no parent"))?;
    let mut reader = FileArtifactReader;
    let bytes = reader.read(&absolute, MAX_RUNTIME_TOML_BYTES)?;
    parse_runtime_toml(&bytes, parent, &mut reader)
}

/// Parse, cross-reference, safety-validate and freeze a candidate.
///
/// Structural validation runs before any referenced artifact read. The function
/// has no Runtime, transport, storage or listener capability.
pub fn parse_runtime_toml(
    bytes: &[u8],
    base: &Path,
    reader: &mut impl ArtifactReader,
) -> Result<FrozenDeployment, ConfigurationError> {
    if bytes.len() > MAX_RUNTIME_TOML_BYTES {
        return Err(ConfigurationError::TooLarge);
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(ConfigurationError::InvalidUtf8);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ConfigurationError::InvalidUtf8)?;
    prescan(text)?;
    let mut dto: DeploymentDto = toml::from_str(text)
        .map_err(|error| ConfigurationError::InvalidToml(bounded_message(&error.to_string())))?;
    // Array-of-table order is presentation, never dependency or scheduling
    // identity. Canonical explicit IDs make equivalent documents deterministic.
    dto.resources.sort_by_key(|item| item.id);
    dto.instruments.sort_by_key(InstrumentDto::id);
    dto.managed_components.sort_by_key(|item| item.id);
    dto.references.sort_by_key(|item| item.id);
    dto.controllers.sort_by_key(|item| item.id);
    dto.safe_profiles
        .sort_by_key(|item| (item.instrument_id, item.parameter_id));
    validate_structure(&dto)?;

    let mut artifacts = Vec::new();
    let mut total_bytes = bytes.len();
    let mut simple_definitions = Vec::new();
    for instrument in &dto.instruments {
        match instrument {
            InstrumentDto::Metakon { definition, .. } => {
                freeze_definition(base, definition, reader, &mut artifacts, &mut total_bytes)?;
            }
            InstrumentDto::SimpleDevice { definition, .. } => {
                freeze_simple_definition(
                    base,
                    definition,
                    reader,
                    &mut artifacts,
                    &mut simple_definitions,
                    &mut total_bytes,
                )?;
            }
            InstrumentDto::VirtualMeasurement { .. } | InstrumentDto::ThermalPlant { .. } => {}
        }
    }
    validate_frozen_metakon_definitions(&dto, base, &artifacts)?;
    validate_frozen_simple_definitions(&dto, base, &simple_definitions)?;
    if artifacts.len() > MAX_DEPLOYMENT_ARTIFACTS || total_bytes > MAX_DEPLOYMENT_BYTES {
        return Err(ConfigurationError::TooLarge);
    }
    Ok(FrozenDeployment {
        base: base.to_path_buf(),
        toml_bytes: Arc::from(bytes),
        toml_hash: Sha256::digest(bytes).into(),
        effective: EffectiveDeployment { dto },
        artifacts,
        simple_definitions,
        property_overlays: Vec::new(),
        api_simple_instruments: BTreeSet::new(),
    })
}

fn prescan(text: &str) -> Result<(), ConfigurationError> {
    let mut depth = 0usize;
    let mut values = 0usize;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if in_basic {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_basic = false;
            }
            continue;
        }
        if in_literal {
            if byte == b'\'' {
                in_literal = false;
            }
            continue;
        }
        match byte {
            b'"' => in_basic = true,
            b'\'' => in_literal = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_RUNTIME_TOML_DEPTH {
                    return Err(ConfigurationError::SyntaxBound("nesting exceeds 8"));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            b'=' => {
                values += 1;
                if values > MAX_RUNTIME_TOML_VALUES {
                    return Err(ConfigurationError::SyntaxBound("value count exceeds 4096"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_structure(dto: &DeploymentDto) -> Result<(), ConfigurationError> {
    if dto.schema_version != 1 {
        return Err(ConfigurationError::invalid("unsupported schema_version"));
    }
    if dto
        .instruments
        .iter()
        .filter(|instrument| matches!(instrument, InstrumentDto::SimpleDevice { .. }))
        .count()
        > MAX_SIMPLE_INSTANCES
    {
        return Err(ConfigurationError::invalid(
            "simple-device instance limit reached (32)",
        ));
    }
    validate_key(&dto.runtime.key)?;
    validate_display_name(&dto.runtime.display_name)?;
    if dto.server.host != "127.0.0.1" {
        return Err(ConfigurationError::invalid("server host must be 127.0.0.1"));
    }
    if dto.server.websocket.enabled {
        websocket::validate_origins(&dto.server.websocket.allowed_origins)
            .map_err(ConfigurationError::invalid)?;
    } else if dto.server.websocket.port != 0 || !dto.server.websocket.allowed_origins.is_empty() {
        return Err(ConfigurationError::invalid(
            "disabled WebSocket endpoint cannot retain configuration",
        ));
    }
    if dto.recording.enabled && dto.recording.path.as_os_str().is_empty() {
        return Err(ConfigurationError::invalid(
            "enabled recording requires path",
        ));
    }
    if !dto.recording.enabled && dto.recording.policy == RecordingPolicyDto::Required {
        return Err(ConfigurationError::invalid(
            "Required recording cannot be disabled",
        ));
    }
    if dto.resources.len() > 8
        || dto.instruments.len() > 64
        || dto.managed_components.len() > 8
        || dto.references.len() > 8
        || dto.controllers.len() > 8
        || dto.safe_profiles.len() > 8
    {
        return Err(ConfigurationError::invalid(
            "deployment object limit exceeded",
        ));
    }

    let mut resource_ids = BTreeSet::new();
    let mut resource_keys = BTreeSet::new();
    let mut ports = BTreeSet::new();
    for resource in &dto.resources {
        validate_nonzero(resource.id, "resource id")?;
        validate_key(&resource.key)?;
        if !resource_ids.insert(resource.id) || !resource_keys.insert(resource.key.clone()) {
            return Err(ConfigurationError::invalid("duplicate resource identity"));
        }
        let normalized = normalize_port(&resource.port)?;
        if !ports.insert(normalized) {
            return Err(ConfigurationError::invalid("duplicate COM binding"));
        }
        validate_serial(resource)?;
    }

    let mut instrument_ids = BTreeSet::new();
    let mut instrument_keys = BTreeSet::new();
    for instrument in &dto.instruments {
        validate_nonzero(instrument.id(), "instrument id")?;
        validate_key(instrument.key())?;
        if !instrument_ids.insert(instrument.id()) || !instrument_keys.insert(instrument.key()) {
            return Err(ConfigurationError::invalid("duplicate instrument identity"));
        }
        match instrument {
            InstrumentDto::Metakon {
                resource_id,
                address,
                poll_period_ms,
                queue_timeout_ms,
                transaction_timeout_ms,
                ..
            } => {
                if !resource_ids.contains(resource_id) {
                    return Err(ConfigurationError::invalid("unknown resource reference"));
                }
                if !(1..=247).contains(address) {
                    return Err(ConfigurationError::invalid("Metakon address out of range"));
                }
                validate_period(*poll_period_ms, "poll period")?;
                validate_transaction_time(*queue_timeout_ms, "queue timeout")?;
                validate_transaction_time(*transaction_timeout_ms, "transaction timeout")?;
            }
            InstrumentDto::SimpleDevice {
                display_name,
                resource_id,
                address: _,
                channel: _,
                poll_period_ms,
                queue_timeout_ms,
                transaction_timeout_ms,
                history_capacity,
                ..
            } => {
                validate_display_name(display_name)?;
                let resource = dto
                    .resources
                    .iter()
                    .find(|resource| resource.id == *resource_id)
                    .ok_or_else(|| ConfigurationError::invalid("unknown resource reference"))?;
                if !matches!(
                    resource.kind,
                    ResourceKindDto::WindowsComReadOnly | ResourceKindDto::WindowsCom
                ) {
                    return Err(ConfigurationError::invalid(
                        "simple device requires an eligible COM resource",
                    ));
                }
                if !(10..=60_000).contains(poll_period_ms) {
                    return Err(ConfigurationError::invalid(
                        "simple-device poll period must be 10..=60000 ms",
                    ));
                }
                if !(1..=2_000).contains(queue_timeout_ms)
                    || !(1..=2_000).contains(transaction_timeout_ms)
                {
                    return Err(ConfigurationError::invalid(
                        "simple-device transaction timing must be 1..=2000 ms",
                    ));
                }
                validate_history(*history_capacity)?;
            }
            InstrumentDto::VirtualMeasurement {
                display_name,
                history_capacity,
                base_temperature,
                poll_period_ms,
                ..
            } => {
                validate_display_name(display_name)?;
                validate_history(*history_capacity)?;
                if !base_temperature.is_finite() || !(-100.0..=100.0).contains(base_temperature) {
                    return Err(ConfigurationError::invalid(
                        "virtual temperature out of range",
                    ));
                }
                validate_period(*poll_period_ms, "poll period")?;
            }
            InstrumentDto::ThermalPlant {
                display_name,
                history_capacity,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                poll_period_ms,
                ..
            } => {
                validate_display_name(display_name)?;
                validate_history(*history_capacity)?;
                if !ambient_temperature.is_finite()
                    || !initial_temperature.is_finite()
                    || !gain_per_percent.is_finite()
                    || *gain_per_percent <= 0.0
                    || *time_constant_ms == 0
                {
                    return Err(ConfigurationError::invalid("invalid thermal plant values"));
                }
                validate_period(*poll_period_ms, "poll period")?;
            }
        }
    }
    for resource in &dto.resources {
        let has_simple = dto.instruments.iter().any(|instrument| {
            matches!(
                instrument,
                InstrumentDto::SimpleDevice { resource_id, .. } if *resource_id == resource.id
            )
        });
        let has_native = dto.instruments.iter().any(|instrument| {
            matches!(
                instrument,
                InstrumentDto::Metakon { resource_id, .. } if *resource_id == resource.id
            )
        });
        if has_simple && has_native {
            return Err(ConfigurationError::invalid(
                "simple and native protocol adapters may not share one resource",
            ));
        }
    }

    let mut component_ids = BTreeSet::new();
    let mut managed_inputs = BTreeMap::new();
    for component in &dto.managed_components {
        validate_nonzero(component.id, "component id")?;
        validate_nonzero(component.instrument_id, "component instrument id")?;
        validate_key(&component.key)?;
        validate_display_name(&component.display_name)?;
        validate_period(component.period_ms, "component period")?;
        ComponentImplementationId::new(component.implementation.clone())
            .map_err(|_| ConfigurationError::invalid("invalid component implementation"))?;
        let properties = component_property_metadata(&component.implementation);
        if properties.is_empty() {
            return Err(ConfigurationError::invalid(
                "unknown managed implementation",
            ));
        }
        component.validate_property_shape(properties)?;
        let config = component.plain_config()?;
        validate_component_configuration(
            &component.implementation,
            NativeComponentDefinition {
                id: ComponentId::new(component.id),
                instrument: InstrumentId::new(component.instrument_id),
                name: component.display_name.clone(),
                input: component
                    .input_instrument_id
                    .map(|input| SignalId::new(InstrumentId::new(input), lab_core::TEMPERATURE)),
                config,
            },
        )
        .map_err(|_| ConfigurationError::invalid("invalid managed implementation configuration"))?;
        if !component_ids.insert(component.id)
            || instrument_ids.contains(&component.instrument_id)
            || managed_inputs
                .insert(component.instrument_id, component.input_instrument_id)
                .is_some()
        {
            return Err(ConfigurationError::invalid("duplicate component identity"));
        }
    }
    let all_instruments: BTreeSet<_> = instrument_ids
        .iter()
        .copied()
        .chain(managed_inputs.keys().copied())
        .collect();
    for component in &dto.managed_components {
        if let Some(input) = component.input_instrument_id
            && !all_instruments.contains(&input)
        {
            return Err(ConfigurationError::invalid("unknown managed input"));
        }
    }
    for start in managed_inputs.keys().copied() {
        let mut visited = BTreeSet::new();
        let mut current = Some(start);
        while let Some(instrument) = current {
            if !visited.insert(instrument) {
                return Err(ConfigurationError::invalid("managed dependency cycle"));
            }
            current = managed_inputs.get(&instrument).copied().flatten();
        }
    }
    instrument_ids.extend(managed_inputs.keys().copied());

    let reference_ids: BTreeSet<_> = dto.references.iter().map(|item| item.id).collect();
    let reference_keys: BTreeSet<_> = dto
        .references
        .iter()
        .map(|item| item.key.as_str())
        .collect();
    if reference_ids.len() != dto.references.len() || reference_ids.contains(&0) {
        return Err(ConfigurationError::invalid(
            "duplicate or zero Reference identity",
        ));
    }
    if reference_keys.len() != dto.references.len() {
        return Err(ConfigurationError::invalid("duplicate Reference key"));
    }
    for reference in &dto.references {
        validate_key(&reference.key)?;
        Unit::new(&reference.unit_id, &reference.unit_symbol)
            .map_err(|_| ConfigurationError::invalid("invalid Reference unit"))?;
        if !reference.value.is_finite() {
            return Err(ConfigurationError::invalid("invalid Reference value"));
        }
        match reference.kind {
            ReferenceKindDto::Fixed if reference.target.is_some() || reference.rate.is_some() => {
                return Err(ConfigurationError::invalid(
                    "Fixed Reference cannot declare ramp fields",
                ));
            }
            ReferenceKindDto::Ramp => {
                let (Some(target), Some(rate)) = (reference.target, reference.rate) else {
                    return Err(ConfigurationError::invalid(
                        "Ramp Reference requires target and rate",
                    ));
                };
                if !target.is_finite() || !rate.is_finite() || rate <= 0.0 {
                    return Err(ConfigurationError::invalid("invalid Ramp Reference"));
                }
            }
            ReferenceKindDto::Fixed => {}
        }
    }
    let safe_outputs: BTreeSet<_> = dto
        .safe_profiles
        .iter()
        .map(|safe| (safe.instrument_id, safe.parameter_id))
        .collect();
    if safe_outputs.len() != dto.safe_profiles.len() {
        return Err(ConfigurationError::invalid("duplicate safe profile"));
    }
    for safe in &dto.safe_profiles {
        validate_nonzero(safe.instrument_id, "safe instrument id")?;
        validate_nonzero(safe.parameter_id, "safe parameter id")?;
        if !instrument_ids.contains(&safe.instrument_id) {
            return Err(ConfigurationError::invalid(
                "unknown safe-profile instrument",
            ));
        }
        if !safe.min.is_finite()
            || !safe.max.is_finite()
            || !safe.safe_value.is_finite()
            || safe.min >= safe.max
            || !(safe.min..=safe.max).contains(&safe.safe_value)
            || safe.max_lease_ms == 0
            || safe.max_proposal_ttl_ms == 0
        {
            return Err(ConfigurationError::invalid("invalid safe profile"));
        }
        let eligible_native_output = dto.instruments.iter().any(|instrument| {
            matches!(
                instrument,
                InstrumentDto::VirtualMeasurement { id, .. }
                    | InstrumentDto::ThermalPlant { id, .. }
                    if *id == safe.instrument_id
                        && safe.parameter_id == lab_core::HEATER_POWER.get()
            )
        }) || dto.instruments.iter().any(|instrument| {
            matches!(instrument, InstrumentDto::Metakon { id, .. } | InstrumentDto::SimpleDevice { id, .. } if *id == safe.instrument_id)
        });
        let simple_output = dto.instruments.iter().any(|instrument| {
            matches!(instrument, InstrumentDto::SimpleDevice { id, .. } if *id == safe.instrument_id)
        });
        if !eligible_native_output || (!simple_output && (safe.min < 0.0 || safe.max > 100.0)) {
            return Err(ConfigurationError::invalid(
                "safe profile targets an ineligible output",
            ));
        }
    }
    let mut controller_ids = BTreeSet::new();
    let mut controller_keys = BTreeSet::new();
    for controller in &dto.controllers {
        validate_nonzero(controller.id, "controller id")?;
        validate_key(&controller.key)?;
        if !controller_ids.insert(controller.id) || !controller_keys.insert(&controller.key) {
            return Err(ConfigurationError::invalid("duplicate controller identity"));
        }
        if !instrument_ids.contains(&controller.input_instrument_id) {
            return Err(ConfigurationError::invalid("unknown controller input"));
        }
        if !reference_ids.contains(&controller.reference_id) {
            return Err(ConfigurationError::invalid("unknown controller Reference"));
        }
        if !safe_outputs.contains(&(
            controller.output_instrument_id,
            controller.output_parameter_id,
        )) {
            return Err(ConfigurationError::invalid(
                "controller output lacks safe profile",
            ));
        }
        validate_period(controller.period_ms, "controller period")?;
        let input_unit = if dto.instruments.iter().any(|instrument| {
            matches!(
                instrument,
                InstrumentDto::VirtualMeasurement { id, .. }
                    | InstrumentDto::ThermalPlant { id, .. }
                    if *id == controller.input_instrument_id
                        && controller.input_parameter_id == lab_core::TEMPERATURE.get()
            )
        }) || dto.managed_components.iter().any(|component| {
            component.instrument_id == controller.input_instrument_id
                && controller.input_parameter_id == lab_core::TEMPERATURE.get()
        }) {
            Some(Unit::CELSIUS.id())
        } else if dto.instruments.iter().any(|instrument| {
            matches!(instrument, InstrumentDto::Metakon { id, .. } | InstrumentDto::SimpleDevice { id, .. } if *id == controller.input_instrument_id)
        }) {
            // Frozen definition validation below resolves physical parameter units.
            None
        } else {
            return Err(ConfigurationError::invalid(
                "controller input is not a signal",
            ));
        };
        let reference = dto
            .references
            .iter()
            .find(|reference| reference.id == controller.reference_id)
            .expect("Reference identity checked above");
        if input_unit.is_some_and(|unit| unit != reference.unit_id) {
            return Err(ConfigurationError::invalid(
                "controller input and Reference unit mismatch",
            ));
        }
        for (duration, label) in [
            (controller.ema_time_constant_ms, "EMA time constant"),
            (controller.max_input_age_ms, "maximum input age"),
            (controller.max_tick_gap_ms, "maximum tick gap"),
            (controller.lease_lifetime_ms, "lease lifetime"),
            (controller.proposal_ttl_ms, "proposal TTL"),
        ] {
            validate_transaction_time(duration, label)?;
        }
        if controller.ema_warmup_samples == 0 || controller.ema_warmup_samples > 1024 {
            return Err(ConfigurationError::invalid("EMA warmup out of range"));
        }
        if ![
            controller.kp,
            controller.ki,
            controller.kd,
            controller.output_min,
            controller.output_max,
        ]
        .into_iter()
        .all(f64::is_finite)
            || controller.output_min >= controller.output_max
        {
            return Err(ConfigurationError::invalid("invalid PID configuration"));
        }
        let safe = dto
            .safe_profiles
            .iter()
            .find(|safe| {
                safe.instrument_id == controller.output_instrument_id
                    && safe.parameter_id == controller.output_parameter_id
            })
            .expect("safe output checked above");
        if controller.output_min < safe.min
            || controller.output_max > safe.max
            || controller.lease_lifetime_ms > safe.max_lease_ms
            || controller.proposal_ttl_ms > safe.max_proposal_ttl_ms
            || controller.proposal_ttl_ms > controller.lease_lifetime_ms
        {
            return Err(ConfigurationError::invalid(
                "controller exceeds safe-profile limits",
            ));
        }
    }
    Ok(())
}

fn freeze_definition(
    base: &Path,
    declared: &Path,
    reader: &mut impl ArtifactReader,
    artifacts: &mut Vec<FrozenArtifact>,
    total: &mut usize,
) -> Result<(), ConfigurationError> {
    let path = resolve_artifact(base, declared)?;
    let bytes = reader.read(&path, MAX_DEFINITION_BYTES)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ConfigurationError::artifact("definition must be UTF-8"))?;
    let definition = parse_definition_json(text)
        .map_err(|error| ConfigurationError::artifact_text(&error.to_string()))?;
    definition
        .validate()
        .map_err(|error| ConfigurationError::artifact_text(&error.to_string()))?;
    push_artifact(
        ArtifactKind::InstrumentDefinition,
        path,
        bytes,
        artifacts,
        total,
    )
}

fn freeze_simple_definition(
    base: &Path,
    declared: &Path,
    reader: &mut impl ArtifactReader,
    artifacts: &mut Vec<FrozenArtifact>,
    definitions: &mut Vec<(PathBuf, CompiledSimpleDefinition)>,
    total: &mut usize,
) -> Result<(), ConfigurationError> {
    let path = resolve_artifact(base, declared)?;
    if definitions.iter().any(|(existing, _)| *existing == path) {
        return Ok(());
    }
    if definitions.len() >= MAX_SIMPLE_DEFINITIONS {
        return Err(ConfigurationError::invalid(
            "simple-device definition limit reached (16)",
        ));
    }
    let bytes = reader.read(&path, MAX_SIMPLE_DEFINITION_BYTES)?;
    let definition = parse_simple_definition(&bytes)
        .map_err(|error| ConfigurationError::artifact_text(&error.to_string()))?;
    push_artifact(
        ArtifactKind::SimpleDeviceDefinition,
        path.clone(),
        bytes,
        artifacts,
        total,
    )?;
    definitions.push((path, definition));
    Ok(())
}

fn validate_frozen_simple_definitions(
    dto: &DeploymentDto,
    base: &Path,
    definitions: &[(PathBuf, CompiledSimpleDefinition)],
) -> Result<(), ConfigurationError> {
    let mut identities = BTreeMap::new();
    let mut instances: Vec<(u64, u64, CompiledSimpleInstance)> = Vec::new();
    for instrument in &dto.instruments {
        let InstrumentDto::SimpleDevice {
            id,
            definition,
            resource_id,
            address,
            channel,
            ..
        } = instrument
        else {
            continue;
        };
        let path = resolve_artifact(base, definition)?;
        let compiled = definitions
            .iter()
            .find(|(candidate, _)| *candidate == path)
            .map(|(_, definition)| definition)
            .ok_or_else(|| ConfigurationError::artifact("frozen simple definition missing"))?;
        let identity = (compiled.definition_id.as_str(), compiled.definition_version);
        if let Some(prior) = identities.insert(identity, compiled.canonical_sha256)
            && prior != compiled.canonical_sha256
        {
            return Err(ConfigurationError::artifact(
                "simple definition ID/version has conflicting normalized content",
            ));
        }
        let instance =
            compile_simple_instance(compiled, InstrumentId::new(*id), *address, *channel)
                .map_err(|error| ConfigurationError::artifact_text(&error.to_string()))?;
        let actuator = instance
            .parameters
            .iter()
            .find(|parameter| parameter.descriptor.role == ParameterRole::Actuator);
        if dto
            .safe_profiles
            .iter()
            .any(|safe| safe.instrument_id == *id)
            && actuator.is_none()
        {
            return Err(ConfigurationError::invalid(
                "simple-device safe profile does not target an actuator",
            ));
        }
        if let Some(actuator) = actuator {
            let resource = dto
                .resources
                .iter()
                .find(|resource| resource.id == *resource_id)
                .expect("resource cross-reference validated before artifacts");
            if resource.kind != ResourceKindDto::WindowsCom {
                return Err(ConfigurationError::invalid(
                    "writable simple device requires windows_com resource",
                ));
            }
            let safe = dto
                .safe_profiles
                .iter()
                .find(|safe| {
                    safe.instrument_id == *id && safe.parameter_id == actuator.descriptor.id.get()
                })
                .ok_or_else(|| {
                    ConfigurationError::invalid("writable simple device lacks safe profile")
                })?;
            for value in [safe.min, safe.max, safe.safe_value] {
                actuator
                    .descriptor
                    .value_spec
                    .validate(&Value::Float(value))
                    .map_err(|_| {
                        ConfigurationError::invalid(
                            "simple-device safe profile exceeds actuator range",
                        )
                    })?;
            }
            let write = actuator
                .write
                .as_ref()
                .expect("compiled actuator has write plan");
            if write.encode(safe.safe_value).is_none() {
                return Err(ConfigurationError::invalid(
                    "simple-device safe value is not exactly encodable",
                ));
            }
            if safe.required_evidence == EvidenceDto::Readback && write.readback.is_none() {
                return Err(ConfigurationError::invalid(
                    "simple-device readback evidence lacks readback plan",
                ));
            }
        }
        for controller in &dto.controllers {
            if controller.input_instrument_id == *id {
                let input = instance
                    .parameters
                    .iter()
                    .find(|parameter| {
                        parameter.descriptor.id.get() == controller.input_parameter_id
                    })
                    .ok_or_else(|| {
                        ConfigurationError::invalid("unknown simple-device controller input")
                    })?;
                let reference = dto
                    .references
                    .iter()
                    .find(|reference| reference.id == controller.reference_id)
                    .expect("Reference cross-reference validated before artifacts");
                if input.descriptor.role != ParameterRole::Measurement
                    || input.descriptor.signal.is_none()
                    || !matches!(input.descriptor.value_spec, ValueSpec::Float { .. })
                    || input.descriptor.unit.id() != reference.unit_id
                {
                    return Err(ConfigurationError::invalid(
                        "simple-device controller input metadata mismatch",
                    ));
                }
            }
            if controller.output_instrument_id == *id
                && instance.parameters.iter().all(|parameter| {
                    parameter.descriptor.id.get() != controller.output_parameter_id
                        || parameter.descriptor.role != ParameterRole::Actuator
                })
            {
                return Err(ConfigurationError::invalid(
                    "controller output is not the simple-device actuator",
                ));
            }
        }
        instances.push((*resource_id, *id, instance));
    }
    for left in 0..instances.len() {
        for right in left + 1..instances.len() {
            if instances[left].0 != instances[right].0 {
                continue;
            }
            for left_plan in &instances[left].2.correlations {
                for right_plan in &instances[right].2.correlations {
                    if !correlations_distinguish(left_plan, right_plan) {
                        return Err(ConfigurationError::invalid(
                            "shared simple-device responses lack instance correlation",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_frozen_metakon_definitions(
    dto: &DeploymentDto,
    base: &Path,
    artifacts: &[FrozenArtifact],
) -> Result<(), ConfigurationError> {
    for instrument in &dto.instruments {
        let InstrumentDto::Metakon {
            id,
            definition,
            resource_id,
            ..
        } = instrument
        else {
            continue;
        };
        let path = resolve_artifact(base, definition)?;
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact.declared_path == path)
            .ok_or_else(|| ConfigurationError::artifact("frozen definition missing"))?;
        let text = std::str::from_utf8(&artifact.bytes)
            .map_err(|_| ConfigurationError::artifact("definition must be UTF-8"))?;
        let definition = parse_definition_json(text)
            .map_err(|error| ConfigurationError::artifact_text(&error.to_string()))?;
        let output = definition
            .parameters
            .iter()
            .find(|parameter| parameter.operation == KnownOperation::Output);
        if let Some(output) = output {
            let resource = dto
                .resources
                .iter()
                .find(|resource| resource.id == *resource_id)
                .expect("resource cross-reference validated before artifacts");
            if resource.kind != ResourceKindDto::WindowsCom {
                return Err(ConfigurationError::invalid(
                    "writable Metakon output requires windows_com resource",
                ));
            }
            if output.access != AccessMode::ReadWrite
                || output.role != ParameterRole::Actuator
                || output.write_effect != WriteEffect::OutputAffecting
                || output.unit != Unit::PERCENT
                || output.scale != 1.0
                || output.value_spec
                    != (ValueSpec::Float {
                        min: -100.0,
                        max: 100.0,
                    })
            {
                return Err(ConfigurationError::artifact(
                    "Metakon register 6 must be read/write signed percent -100..=100 at scale 1",
                ));
            }
            let safe = dto
                .safe_profiles
                .iter()
                .find(|safe| safe.instrument_id == *id && safe.parameter_id == output.id.get())
                .ok_or_else(|| {
                    ConfigurationError::invalid("physical Metakon output lacks safe profile")
                })?;
            if safe.safe_value != 0.0
                || safe.min < -100.0
                || safe.max > 100.0
                || safe.required_evidence != EvidenceDto::Readback
            {
                return Err(ConfigurationError::invalid(
                    "physical Metakon output requires safe zero and readback evidence",
                ));
            }
        }
        for controller in &dto.controllers {
            if controller.input_instrument_id == *id {
                let input = definition
                    .parameters
                    .iter()
                    .find(|parameter| parameter.id.get() == controller.input_parameter_id)
                    .ok_or_else(|| {
                        ConfigurationError::invalid("unknown physical controller input")
                    })?;
                let reference = dto
                    .references
                    .iter()
                    .find(|reference| reference.id == controller.reference_id)
                    .expect("Reference cross-reference validated before artifacts");
                if input.role != ParameterRole::Measurement || input.unit.id() != reference.unit_id
                {
                    return Err(ConfigurationError::invalid(
                        "physical controller input metadata mismatch",
                    ));
                }
            }
            if controller.output_instrument_id == *id
                && output.is_none_or(|output| output.id.get() != controller.output_parameter_id)
            {
                return Err(ConfigurationError::invalid(
                    "controller physical output is not Metakon register 6",
                ));
            }
        }
    }
    Ok(())
}

fn push_artifact(
    kind: ArtifactKind,
    path: PathBuf,
    bytes: Vec<u8>,
    artifacts: &mut Vec<FrozenArtifact>,
    total: &mut usize,
) -> Result<(), ConfigurationError> {
    *total = total
        .checked_add(bytes.len() + 128)
        .ok_or(ConfigurationError::TooLarge)?;
    if *total > MAX_DEPLOYMENT_BYTES || artifacts.len() == MAX_DEPLOYMENT_ARTIFACTS {
        return Err(ConfigurationError::TooLarge);
    }
    let sha256 = Sha256::digest(&bytes).into();
    artifacts.push(FrozenArtifact {
        kind,
        declared_path: path,
        bytes: Arc::from(bytes),
        sha256,
    });
    Ok(())
}

fn resolve_artifact(base: &Path, declared: &Path) -> Result<PathBuf, ConfigurationError> {
    if declared.as_os_str().is_empty() {
        return Err(ConfigurationError::invalid("empty artifact path"));
    }
    let path = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        base.join(declared)
    };
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(ConfigurationError::invalid(
            "artifact path may not contain parent traversal",
        ));
    }
    Ok(path)
}

fn normalize_port(port: &str) -> Result<String, ConfigurationError> {
    let port = port.trim();
    let digits = port
        .strip_prefix("COM")
        .or_else(|| port.strip_prefix("com"))
        .ok_or_else(|| ConfigurationError::invalid("Windows port must be COM<number>"))?;
    let number = digits
        .parse::<u16>()
        .map_err(|_| ConfigurationError::invalid("invalid COM port"))?;
    if number == 0 {
        return Err(ConfigurationError::invalid("invalid COM port"));
    }
    Ok(format!("COM{number}"))
}

fn validate_serial(resource: &ResourceDto) -> Result<(), ConfigurationError> {
    if !(1..=4_000_000).contains(&resource.baud_rate)
        || !matches!(resource.data_bits, 5..=8)
        || !matches!(resource.stop_bits, 1 | 2)
        || !(1..=250).contains(&resource.read_timeout_ms)
        || !(1..=250).contains(&resource.write_timeout_ms)
        || !(1..=2000).contains(&resource.open_timeout_ms)
        || !(1..=2000).contains(&resource.recovery_timeout_ms)
    {
        return Err(ConfigurationError::invalid(
            "invalid bounded serial settings",
        ));
    }
    Ok(())
}

pub(crate) fn validate_key(key: &str) -> Result<(), ConfigurationError> {
    let mut bytes = key.bytes();
    if key.len() > 64
        || !matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z'))
        || bytes.any(|byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
    {
        return Err(ConfigurationError::invalid("invalid logical key"));
    }
    Ok(())
}

pub(crate) fn validate_display_name(name: &str) -> Result<(), ConfigurationError> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(ConfigurationError::invalid("invalid display name"));
    }
    Ok(())
}

fn validate_nonzero(value: u64, label: &'static str) -> Result<(), ConfigurationError> {
    if value == 0 {
        return Err(ConfigurationError::invalid(format!(
            "{label} must be nonzero"
        )));
    }
    Ok(())
}

fn validate_period(value: u64, label: &'static str) -> Result<(), ConfigurationError> {
    if !(1..=60_000).contains(&value) {
        return Err(ConfigurationError::invalid(format!(
            "{label} must be 1..=60000 ms"
        )));
    }
    Ok(())
}

fn validate_transaction_time(value: u64, label: &'static str) -> Result<(), ConfigurationError> {
    if !(1..=60_000).contains(&value) {
        return Err(ConfigurationError::invalid(format!(
            "{label} must be 1..=60000 ms"
        )));
    }
    Ok(())
}

fn validate_history(value: usize) -> Result<(), ConfigurationError> {
    if !(1..=1024).contains(&value) {
        return Err(ConfigurationError::invalid("history capacity out of range"));
    }
    Ok(())
}

fn bounded_message(message: &str) -> String {
    message.chars().take(256).collect()
}

fn hex_sha256(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentDto {
    pub(crate) schema_version: u16,
    pub(crate) runtime: RuntimeDto,
    pub(crate) server: ServerDto,
    pub(crate) recording: RecordingDto,
    #[serde(default)]
    pub(crate) resources: Vec<ResourceDto>,
    #[serde(default)]
    pub(crate) instruments: Vec<InstrumentDto>,
    #[serde(default)]
    pub(crate) managed_components: Vec<ManagedComponentDto>,
    #[serde(default)]
    pub(crate) references: Vec<ReferenceDto>,
    #[serde(default)]
    pub(crate) controllers: Vec<ControllerDto>,
    #[serde(default)]
    pub(crate) safe_profiles: Vec<SafeProfileDto>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeDto {
    pub(crate) key: String,
    pub(crate) display_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerDto {
    pub(crate) host: String,
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) websocket: WebSocketDto,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebSocketDto {
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) allowed_origins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordingDto {
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) path: PathBuf,
    pub(crate) policy: RecordingPolicyDto,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordingPolicyDto {
    BestEffort,
    Required,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceDto {
    pub(crate) id: u64,
    pub(crate) key: String,
    pub(crate) kind: ResourceKindDto,
    pub(crate) port: String,
    pub(crate) baud_rate: u32,
    pub(crate) data_bits: u8,
    pub(crate) parity: ParityDto,
    pub(crate) stop_bits: u8,
    pub(crate) flow_control: FlowControlDto,
    pub(crate) read_timeout_ms: u64,
    pub(crate) write_timeout_ms: u64,
    pub(crate) open_timeout_ms: u64,
    pub(crate) recovery_timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResourceKindDto {
    WindowsComReadOnly,
    WindowsCom,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParityDto {
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlowControlDto {
    None,
    Software,
    Hardware,
}

/// Language-neutral scalar shape of one configured instrument property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstrumentPropertyType {
    Text,
    Integer,
    Number,
    Boolean,
}

impl InstrumentPropertyType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }
}

/// Current property value retained by the validated deployment configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InstrumentPropertyValue<'a> {
    Text(&'a str),
    Unsigned(u64),
    Number(f64),
    Boolean(bool),
}

/// Optional inclusive numeric constraints for one instrument property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InstrumentPropertyConstraints {
    Integer { minimum: i64, maximum: i64 },
    Number { minimum: f64, maximum: f64 },
}

/// Language-neutral engineering unit projected for a configured property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InstrumentPropertyUnit {
    pub(crate) id: &'static str,
    pub(crate) symbol: &'static str,
}

/// Whether the accepted generic configuration operation may mutate a property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstrumentPropertyAccess {
    ReadOnly,
    ReadWrite,
    DeploymentOnly,
}

impl InstrumentPropertyAccess {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
            Self::DeploymentOnly => "deployment_only",
        }
    }
}

/// Runtime lifecycle consequence of changing a configured property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstrumentPropertyMutation {
    LiveSafe,
    OrdinaryLive,
    Reinitialize,
}

impl InstrumentPropertyMutation {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LiveSafe => "live_safe",
            Self::OrdinaryLive => "ordinary_live",
            Self::Reinitialize => "reinitialize",
        }
    }
}

/// Neutral configured property description owned by an instrument configuration.
///
/// This type deliberately contains no JSON or Application DTO. The generic
/// Application projection adds owner identity and configuration revision later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InstrumentPropertyMetadata<'a> {
    pub(crate) id: &'static str,
    pub(crate) value_type: InstrumentPropertyType,
    pub(crate) current: InstrumentPropertyValue<'a>,
    pub(crate) unit: Option<InstrumentPropertyUnit>,
    pub(crate) access: InstrumentPropertyAccess,
    pub(crate) mutation: InstrumentPropertyMutation,
    pub(crate) constraints: Option<InstrumentPropertyConstraints>,
}

/// Static configuration-layer seam consumed by the generic property projection.
///
/// Adding an instrument kind extends this layer and Host composition; it does not
/// add a concrete-instrument branch to the Application API.
pub(crate) trait InstrumentPropertySource {
    fn instrument_id(&self) -> u64;
    fn property_metadata(&self) -> Vec<InstrumentPropertyMetadata<'_>>;
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum InstrumentDto {
    VirtualMeasurement {
        id: u64,
        key: String,
        display_name: String,
        history_capacity: usize,
        base_temperature: f64,
        #[serde(default = "default_true")]
        measurement_enabled: bool,
        #[serde(default)]
        external_publication: bool,
        poll_period_ms: u64,
    },
    ThermalPlant {
        id: u64,
        key: String,
        display_name: String,
        history_capacity: usize,
        ambient_temperature: f64,
        initial_temperature: f64,
        gain_per_percent: f64,
        time_constant_ms: u64,
        poll_period_ms: u64,
    },
    Metakon {
        id: u64,
        key: String,
        definition: PathBuf,
        resource_id: u64,
        address: u8,
        poll_period_ms: u64,
        #[serde(default = "default_transaction_ms")]
        queue_timeout_ms: u64,
        #[serde(default = "default_transaction_ms")]
        transaction_timeout_ms: u64,
    },
    SimpleDevice {
        id: u64,
        key: String,
        display_name: String,
        definition: PathBuf,
        resource_id: u64,
        address: u16,
        channel: u16,
        poll_period_ms: u64,
        queue_timeout_ms: u64,
        transaction_timeout_ms: u64,
        history_capacity: usize,
    },
}

impl InstrumentDto {
    pub(crate) fn id(&self) -> u64 {
        match self {
            Self::VirtualMeasurement { id, .. }
            | Self::ThermalPlant { id, .. }
            | Self::Metakon { id, .. }
            | Self::SimpleDevice { id, .. } => *id,
        }
    }

    fn key(&self) -> &str {
        match self {
            Self::VirtualMeasurement { key, .. }
            | Self::ThermalPlant { key, .. }
            | Self::Metakon { key, .. }
            | Self::SimpleDevice { key, .. } => key,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::VirtualMeasurement { .. } => "virtual_measurement",
            Self::ThermalPlant { .. } => "thermal_plant",
            Self::Metakon { .. } => "metakon",
            Self::SimpleDevice { .. } => "simple_device",
        }
    }

    fn poll_period_ms(&self) -> u64 {
        match self {
            Self::VirtualMeasurement { poll_period_ms, .. }
            | Self::ThermalPlant { poll_period_ms, .. }
            | Self::Metakon { poll_period_ms, .. }
            | Self::SimpleDevice { poll_period_ms, .. } => *poll_period_ms,
        }
    }

    /// Deployment resource used by a physical adapter, if this kind has one.
    pub(crate) fn bound_resource_id(&self) -> Option<u64> {
        match self {
            Self::Metakon { resource_id, .. } | Self::SimpleDevice { resource_id, .. } => {
                Some(*resource_id)
            }
            Self::VirtualMeasurement { .. } | Self::ThermalPlant { .. } => None,
        }
    }

    // Exclude display and cadence fields that have explicitly live semantics.
    fn without_live_fields(&self) -> String {
        match self {
            Self::VirtualMeasurement {
                id,
                key,
                history_capacity,
                base_temperature,
                measurement_enabled,
                external_publication,
                ..
            } => format!(
                "virtual:{id}:{key}:{history_capacity}:{base_temperature}:{measurement_enabled}:{external_publication}"
            ),
            Self::ThermalPlant {
                id,
                key,
                history_capacity,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                ..
            } => format!(
                "plant:{id}:{key}:{history_capacity}:{ambient_temperature}:{initial_temperature}:{gain_per_percent}:{time_constant_ms}"
            ),
            Self::Metakon {
                id,
                key,
                definition,
                resource_id,
                address,
                queue_timeout_ms,
                transaction_timeout_ms,
                ..
            } => format!(
                "metakon:{id}:{key}:{}:{resource_id}:{address}:{queue_timeout_ms}:{transaction_timeout_ms}",
                definition.display()
            ),
            Self::SimpleDevice {
                id,
                key,
                definition,
                resource_id,
                address,
                channel,
                queue_timeout_ms,
                transaction_timeout_ms,
                history_capacity,
                ..
            } => format!(
                "simple:{id}:{key}:{}:{resource_id}:{address}:{channel}:{queue_timeout_ms}:{transaction_timeout_ms}:{history_capacity}",
                definition.display()
            ),
        }
    }
}

impl InstrumentPropertySource for InstrumentDto {
    fn instrument_id(&self) -> u64 {
        self.id()
    }

    fn property_metadata(&self) -> Vec<InstrumentPropertyMetadata<'_>> {
        const MILLISECONDS: InstrumentPropertyUnit = InstrumentPropertyUnit {
            id: "ms",
            symbol: "ms",
        };
        const CELSIUS: InstrumentPropertyUnit = InstrumentPropertyUnit {
            id: "degC",
            symbol: "°C",
        };
        const POLL_BOUNDS: InstrumentPropertyConstraints = InstrumentPropertyConstraints::Integer {
            minimum: 1,
            maximum: 60_000,
        };
        const SIMPLE_POLL_BOUNDS: InstrumentPropertyConstraints =
            InstrumentPropertyConstraints::Integer {
                minimum: 10,
                maximum: 60_000,
            };

        match self {
            Self::VirtualMeasurement {
                display_name,
                poll_period_ms,
                base_temperature,
                measurement_enabled,
                external_publication,
                ..
            } => vec![
                InstrumentPropertyMetadata {
                    id: "display_name",
                    value_type: InstrumentPropertyType::Text,
                    current: InstrumentPropertyValue::Text(display_name),
                    unit: None,
                    access: InstrumentPropertyAccess::ReadWrite,
                    mutation: InstrumentPropertyMutation::LiveSafe,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "poll_period_ms",
                    value_type: InstrumentPropertyType::Integer,
                    current: InstrumentPropertyValue::Unsigned(*poll_period_ms),
                    unit: Some(MILLISECONDS),
                    access: InstrumentPropertyAccess::ReadWrite,
                    mutation: InstrumentPropertyMutation::OrdinaryLive,
                    constraints: Some(POLL_BOUNDS),
                },
                InstrumentPropertyMetadata {
                    id: "base_temperature",
                    value_type: InstrumentPropertyType::Number,
                    current: InstrumentPropertyValue::Number(*base_temperature),
                    unit: None,
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: Some(InstrumentPropertyConstraints::Number {
                        minimum: -100.0,
                        maximum: 100.0,
                    }),
                },
                InstrumentPropertyMetadata {
                    id: "measurement_enabled",
                    value_type: InstrumentPropertyType::Boolean,
                    current: InstrumentPropertyValue::Boolean(*measurement_enabled),
                    unit: None,
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "external_publication",
                    value_type: InstrumentPropertyType::Boolean,
                    current: InstrumentPropertyValue::Boolean(*external_publication),
                    unit: None,
                    access: InstrumentPropertyAccess::ReadOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
            ],
            Self::ThermalPlant {
                display_name,
                poll_period_ms,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                ..
            } => vec![
                InstrumentPropertyMetadata {
                    id: "display_name",
                    value_type: InstrumentPropertyType::Text,
                    current: InstrumentPropertyValue::Text(display_name),
                    unit: None,
                    access: InstrumentPropertyAccess::ReadWrite,
                    mutation: InstrumentPropertyMutation::LiveSafe,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "poll_period_ms",
                    value_type: InstrumentPropertyType::Integer,
                    current: InstrumentPropertyValue::Unsigned(*poll_period_ms),
                    unit: Some(MILLISECONDS),
                    access: InstrumentPropertyAccess::ReadWrite,
                    mutation: InstrumentPropertyMutation::OrdinaryLive,
                    constraints: Some(POLL_BOUNDS),
                },
                InstrumentPropertyMetadata {
                    id: "ambient_temperature",
                    value_type: InstrumentPropertyType::Number,
                    current: InstrumentPropertyValue::Number(*ambient_temperature),
                    unit: Some(CELSIUS),
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "initial_temperature",
                    value_type: InstrumentPropertyType::Number,
                    current: InstrumentPropertyValue::Number(*initial_temperature),
                    unit: Some(CELSIUS),
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "gain_per_percent",
                    value_type: InstrumentPropertyType::Number,
                    current: InstrumentPropertyValue::Number(*gain_per_percent),
                    unit: None,
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
                InstrumentPropertyMetadata {
                    id: "time_constant_ms",
                    value_type: InstrumentPropertyType::Number,
                    current: InstrumentPropertyValue::Unsigned(*time_constant_ms),
                    unit: Some(MILLISECONDS),
                    access: InstrumentPropertyAccess::DeploymentOnly,
                    mutation: InstrumentPropertyMutation::Reinitialize,
                    constraints: None,
                },
            ],
            Self::Metakon { poll_period_ms, .. } => vec![InstrumentPropertyMetadata {
                id: "poll_period_ms",
                value_type: InstrumentPropertyType::Integer,
                current: InstrumentPropertyValue::Unsigned(*poll_period_ms),
                unit: Some(MILLISECONDS),
                access: InstrumentPropertyAccess::ReadWrite,
                mutation: InstrumentPropertyMutation::OrdinaryLive,
                constraints: Some(POLL_BOUNDS),
            }],
            Self::SimpleDevice { poll_period_ms, .. } => vec![InstrumentPropertyMetadata {
                id: "poll_period_ms",
                value_type: InstrumentPropertyType::Integer,
                current: InstrumentPropertyValue::Unsigned(*poll_period_ms),
                unit: Some(MILLISECONDS),
                access: InstrumentPropertyAccess::ReadWrite,
                mutation: InstrumentPropertyMutation::OrdinaryLive,
                constraints: Some(SIMPLE_POLL_BOUNDS),
            }],
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedComponentDto {
    pub(crate) id: u64,
    pub(crate) instrument_id: u64,
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) implementation: String,
    #[serde(default)]
    pub(crate) config: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub(crate) input_instrument_id: Option<u64>,
    pub(crate) period_ms: u64,
}

impl ManagedComponentDto {
    fn validate_property_shape(
        &self,
        properties: &[crate::managed_executor::ComponentPropertyMetadata],
    ) -> Result<(), ConfigurationError> {
        if self.config.len() != properties.len() {
            return Err(ConfigurationError::invalid(
                "managed component properties do not match registration",
            ));
        }
        for property in properties {
            let value = self.config.get(property.id).ok_or_else(|| {
                ConfigurationError::invalid("managed component property is missing")
            })?;
            match (property.value_type, value) {
                ("integer" | "number", toml::Value::Integer(value))
                    if property.minimum.is_none_or(|minimum| *value >= minimum)
                        && property.maximum.is_none_or(|maximum| *value <= maximum) => {}
                ("number", toml::Value::Float(value))
                    if value.is_finite()
                        && property
                            .minimum
                            .is_none_or(|minimum| *value >= minimum as f64)
                        && property
                            .maximum
                            .is_none_or(|maximum| *value <= maximum as f64) => {}
                ("boolean", toml::Value::Boolean(_)) | ("text", toml::Value::String(_)) => {}
                _ => {
                    return Err(ConfigurationError::invalid(
                        "managed component property has invalid type or bounds",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn plain_config(&self) -> Result<PlainData, ConfigurationError> {
        let mut fields = BTreeMap::new();
        for (key, value) in &self.config {
            let value = match value {
                toml::Value::Integer(value) => PlainValue::Number(*value as f64),
                toml::Value::Float(value) => PlainValue::Number(*value),
                toml::Value::Boolean(value) => PlainValue::Boolean(*value),
                toml::Value::String(value) => PlainValue::Text(value.clone()),
                toml::Value::Array(values) => PlainValue::Numbers(
                    values
                        .iter()
                        .map(|value| match value {
                            toml::Value::Integer(value) => Ok(*value as f64),
                            toml::Value::Float(value) => Ok(*value),
                            _ => Err(ConfigurationError::invalid(
                                "managed config arrays must contain numbers",
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                _ => {
                    return Err(ConfigurationError::invalid(
                        "managed config values must be bounded PlainData leaves",
                    ));
                }
            };
            fields.insert(key.clone(), value);
        }
        let data = PlainData { fields };
        data.validate()
            .map_err(|_| ConfigurationError::invalid("invalid managed PlainData config"))?;
        Ok(data)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceDto {
    pub(crate) id: u64,
    pub(crate) key: String,
    pub(crate) kind: ReferenceKindDto,
    pub(crate) value: f64,
    #[serde(default)]
    pub(crate) target: Option<f64>,
    #[serde(default)]
    pub(crate) rate: Option<f64>,
    pub(crate) unit_id: String,
    pub(crate) unit_symbol: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReferenceKindDto {
    Fixed,
    Ramp,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerDto {
    pub(crate) id: u64,
    pub(crate) key: String,
    pub(crate) input_instrument_id: u64,
    pub(crate) input_parameter_id: u64,
    pub(crate) output_instrument_id: u64,
    pub(crate) output_parameter_id: u64,
    pub(crate) reference_id: u64,
    pub(crate) period_ms: u64,
    pub(crate) ema_time_constant_ms: u64,
    pub(crate) ema_warmup_samples: usize,
    pub(crate) kp: f64,
    pub(crate) ki: f64,
    pub(crate) kd: f64,
    pub(crate) output_min: f64,
    pub(crate) output_max: f64,
    pub(crate) max_input_age_ms: u64,
    pub(crate) max_tick_gap_ms: u64,
    pub(crate) lease_lifetime_ms: u64,
    pub(crate) proposal_ttl_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeProfileDto {
    pub(crate) instrument_id: u64,
    pub(crate) parameter_id: u64,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) safe_value: f64,
    pub(crate) max_lease_ms: u64,
    pub(crate) max_proposal_ttl_ms: u64,
    pub(crate) required_evidence: EvidenceDto,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceDto {
    Ack,
    Readback,
}

const fn default_true() -> bool {
    true
}

const fn default_transaction_ms() -> u64 {
    1000
}
