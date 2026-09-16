//! Bounded, side-effect-free deployment parsing and validation.
//!
//! This module owns TOML and filesystem-shaped artifact identities outside Core.
//! Parsing never opens a transport, SQLite database, listener or Lua VM. The
//! returned bundle owns the exact admitted bytes so later activation never has
//! to reread a mutable pathname for provenance.

use crate::definition::{MAX_DEFINITION_BYTES, parse_definition_json};
use lab_core::{AccessMode, ParameterRole, WriteEffect, instrument::KnownOperation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
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
    ManagedLuaSource,
}

/// Fully validated typed deployment used to build a staged Runtime candidate.
///
/// Fields remain private so adapters cannot bypass validation by constructing an
/// instance. Stable read-only accessors are added only where composition needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveDeployment {
    dto: DeploymentDto,
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
    toml_bytes: Arc<[u8]>,
    toml_hash: [u8; 32],
    effective: EffectiveDeployment,
    artifacts: Vec<FrozenArtifact>,
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

    /// Referenced definitions and sources frozen during validation.
    pub fn artifacts(&self) -> &[FrozenArtifact] {
        &self.artifacts
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
        let restart_required = old.runtime.key != new.runtime.key
            || old.server != new.server
            || old.recording != new.recording
            || old_topology != new_topology
            || old.managed_components.len() != new.managed_components.len()
            || old.references.len() != new.references.len()
            || old.controllers.len() != new.controllers.len()
            || old.safe_profiles.len() != new.safe_profiles.len();
        let live_safe = self.toml_hash != active.toml_hash
            || old.runtime.display_name != new.runtime.display_name;
        let ordinary_live = old
            .instruments
            .iter()
            .zip(&new.instruments)
            .any(|(old, new)| old.poll_period_ms() != new.poll_period_ms());
        let rebind = old.resources != new.resources
            || old
                .instruments
                .iter()
                .zip(&new.instruments)
                .any(|(old, new)| {
                    matches!(
                        (old, new),
                        (InstrumentDto::Metakon { .. }, InstrumentDto::Metakon { .. })
                    ) && old != new
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
    let parent = path
        .parent()
        .ok_or_else(|| ConfigurationError::invalid("runtime.toml has no parent"))?;
    let mut reader = FileArtifactReader;
    let bytes = reader.read(path, MAX_RUNTIME_TOML_BYTES)?;
    parse_runtime_toml(&bytes, parent, &mut reader)
}

/// Parse, cross-reference, safety-validate and freeze a candidate.
///
/// Structural validation runs before any referenced artifact read. The function
/// has no Runtime, transport, storage, listener or Lua execution capability.
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
    let dto: DeploymentDto = toml::from_str(text)
        .map_err(|error| ConfigurationError::InvalidToml(bounded_message(&error.to_string())))?;
    validate_structure(&dto)?;

    let mut artifacts = Vec::new();
    let mut total_bytes = bytes.len();
    for instrument in &dto.instruments {
        if let InstrumentDto::Metakon { definition, .. } = instrument {
            freeze_definition(base, definition, reader, &mut artifacts, &mut total_bytes)?;
        }
    }
    for component in &dto.managed_components {
        freeze_source(
            base,
            &component.source,
            reader,
            &mut artifacts,
            &mut total_bytes,
        )?;
    }
    if artifacts.len() > MAX_DEPLOYMENT_ARTIFACTS || total_bytes > MAX_DEPLOYMENT_BYTES {
        return Err(ConfigurationError::TooLarge);
    }
    Ok(FrozenDeployment {
        toml_bytes: Arc::from(bytes),
        toml_hash: Sha256::digest(bytes).into(),
        effective: EffectiveDeployment { dto },
        artifacts,
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
    validate_key(&dto.runtime.key)?;
    validate_display_name(&dto.runtime.display_name)?;
    if dto.server.host != "127.0.0.1" {
        return Err(ConfigurationError::invalid("server host must be 127.0.0.1"));
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
        if resource.kind != ResourceKindDto::WindowsComReadOnly {
            return Err(ConfigurationError::invalid("unsupported resource kind"));
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

    let mut component_ids = BTreeSet::new();
    for component in &dto.managed_components {
        validate_nonzero(component.id, "component id")?;
        validate_nonzero(component.instrument_id, "component instrument id")?;
        validate_key(&component.key)?;
        validate_display_name(&component.display_name)?;
        validate_period(component.period_ms, "component period")?;
        if !component_ids.insert(component.id) || instrument_ids.contains(&component.instrument_id)
        {
            return Err(ConfigurationError::invalid("duplicate component identity"));
        }
        instrument_ids.insert(component.instrument_id);
        if let Some(input) = component.input_instrument_id
            && !instrument_ids.contains(&input)
        {
            return Err(ConfigurationError::invalid("unknown managed input"));
        }
    }

    let reference_ids: BTreeSet<_> = dto.references.iter().map(|item| item.id).collect();
    if reference_ids.len() != dto.references.len() || reference_ids.contains(&0) {
        return Err(ConfigurationError::invalid(
            "duplicate or zero Reference identity",
        ));
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
    }
    let mut controller_ids = BTreeSet::new();
    for controller in &dto.controllers {
        validate_nonzero(controller.id, "controller id")?;
        if !controller_ids.insert(controller.id) {
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
    if definition.parameters.iter().any(|parameter| {
        parameter.access != AccessMode::ReadOnly
            || parameter.role == ParameterRole::Actuator
            || parameter.write_effect == WriteEffect::OutputAffecting
            || parameter.operation == KnownOperation::Output
    }) {
        return Err(ConfigurationError::artifact(
            "physical M8 definition must be read-only",
        ));
    }
    push_artifact(
        ArtifactKind::InstrumentDefinition,
        path,
        bytes,
        artifacts,
        total,
    )
}

fn freeze_source(
    base: &Path,
    declared: &Path,
    reader: &mut impl ArtifactReader,
    artifacts: &mut Vec<FrozenArtifact>,
    total: &mut usize,
) -> Result<(), ConfigurationError> {
    let path = resolve_artifact(base, declared)?;
    let bytes = reader.read(&path, 32 * 1024)?;
    std::str::from_utf8(&bytes)
        .map_err(|_| ConfigurationError::artifact("managed source must be UTF-8"))?;
    push_artifact(
        ArtifactKind::ManagedLuaSource,
        path,
        bytes,
        artifacts,
        total,
    )
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

fn validate_key(key: &str) -> Result<(), ConfigurationError> {
    let mut bytes = key.bytes();
    if key.len() > 64
        || !matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z'))
        || bytes.any(|byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
    {
        return Err(ConfigurationError::invalid("invalid logical key"));
    }
    Ok(())
}

fn validate_display_name(name: &str) -> Result<(), ConfigurationError> {
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct DeploymentDto {
    schema_version: u16,
    runtime: RuntimeDto,
    server: ServerDto,
    recording: RecordingDto,
    #[serde(default)]
    resources: Vec<ResourceDto>,
    #[serde(default)]
    instruments: Vec<InstrumentDto>,
    #[serde(default)]
    managed_components: Vec<ManagedComponentDto>,
    #[serde(default)]
    references: Vec<ReferenceDto>,
    #[serde(default)]
    controllers: Vec<ControllerDto>,
    #[serde(default)]
    safe_profiles: Vec<SafeProfileDto>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RuntimeDto {
    key: String,
    display_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ServerDto {
    host: String,
    port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RecordingDto {
    enabled: bool,
    #[serde(default)]
    path: PathBuf,
    policy: RecordingPolicyDto,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RecordingPolicyDto {
    BestEffort,
    Required,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ResourceDto {
    id: u64,
    key: String,
    kind: ResourceKindDto,
    port: String,
    baud_rate: u32,
    data_bits: u8,
    parity: ParityDto,
    stop_bits: u8,
    flow_control: FlowControlDto,
    read_timeout_ms: u64,
    write_timeout_ms: u64,
    open_timeout_ms: u64,
    recovery_timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResourceKindDto {
    WindowsComReadOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ParityDto {
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FlowControlDto {
    None,
    Software,
    Hardware,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InstrumentDto {
    VirtualMeasurement {
        id: u64,
        key: String,
        display_name: String,
        history_capacity: usize,
        base_temperature: f64,
        #[serde(default = "default_true")]
        measurement_enabled: bool,
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
}

impl InstrumentDto {
    fn id(&self) -> u64 {
        match self {
            Self::VirtualMeasurement { id, .. }
            | Self::ThermalPlant { id, .. }
            | Self::Metakon { id, .. } => *id,
        }
    }

    fn key(&self) -> &str {
        match self {
            Self::VirtualMeasurement { key, .. }
            | Self::ThermalPlant { key, .. }
            | Self::Metakon { key, .. } => key,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::VirtualMeasurement { .. } => "virtual_measurement",
            Self::ThermalPlant { .. } => "thermal_plant",
            Self::Metakon { .. } => "metakon",
        }
    }

    fn poll_period_ms(&self) -> u64 {
        match self {
            Self::VirtualMeasurement { poll_period_ms, .. }
            | Self::ThermalPlant { poll_period_ms, .. }
            | Self::Metakon { poll_period_ms, .. } => *poll_period_ms,
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
                ..
            } => format!(
                "virtual:{id}:{key}:{history_capacity}:{base_temperature}:{measurement_enabled}"
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
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ManagedComponentDto {
    id: u64,
    instrument_id: u64,
    key: String,
    display_name: String,
    source: PathBuf,
    #[serde(default)]
    input_instrument_id: Option<u64>,
    period_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ReferenceDto {
    id: u64,
    kind: ReferenceKindDto,
    value: f64,
    unit_id: String,
    unit_symbol: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReferenceKindDto {
    Fixed,
    Ramp,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ControllerDto {
    id: u64,
    input_instrument_id: u64,
    input_parameter_id: u64,
    output_instrument_id: u64,
    output_parameter_id: u64,
    reference_id: u64,
    period_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SafeProfileDto {
    instrument_id: u64,
    parameter_id: u64,
    min: f64,
    max: f64,
    safe_value: f64,
    max_lease_ms: u64,
    max_proposal_ttl_ms: u64,
    required_evidence: EvidenceDto,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceDto {
    Ack,
    Readback,
}

const fn default_true() -> bool {
    true
}

const fn default_transaction_ms() -> u64 {
    1000
}
