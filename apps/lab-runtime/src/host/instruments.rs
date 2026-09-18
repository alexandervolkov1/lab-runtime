//! Static composition point for configured instrument kinds and protocol adapters.
//!
//! An ordinary physical instrument adds its deployment identity/validation, its
//! protocol-specific adapter and one explicit composition arm here. Once the arm
//! registers ordinary Core signals, generic discovery, current/history,
//! subscriptions, controllers and Recorder facts require no instrument-specific
//! Application or SQLite code. Output-capable adapters must still enter the
//! existing OutputAuthority path; this module exposes no generic raw-write hook.

use super::*;

pub(super) struct ConfiguredInstruments {
    pub(super) measurements: Vec<(InstrumentId, Periodic)>,
    pub(super) metakon_reads: Vec<MetakonReadSchedule>,
    pub(super) virtual_model_generations: BTreeMap<InstrumentId, u64>,
    pub(super) emulator_targets: BTreeSet<SignalId>,
    pub(super) physical_instruments: BTreeSet<InstrumentId>,
    pub(super) configured_probes: Vec<ConfiguredProbe>,
}

pub(super) fn register_configured_instruments(
    runtime: &mut Runtime,
    deployment: &FrozenDeployment,
) -> Result<ConfiguredInstruments, Error> {
    let dto = &deployment.effective().dto;
    let mut measurements = Vec::with_capacity(dto.instruments.len());
    let mut metakon_reads = Vec::new();
    let mut virtual_model_generations = BTreeMap::new();
    let mut emulator_targets = BTreeSet::new();
    let mut physical_instruments = BTreeSet::new();
    let mut configured_probes = Vec::new();

    for instrument in &dto.instruments {
        match instrument {
            InstrumentDto::VirtualMeasurement {
                id,
                display_name,
                history_capacity,
                base_temperature,
                measurement_enabled,
                external_publication,
                poll_period_ms,
                ..
            } => {
                runtime.command(Command::RegisterVirtual(
                    lab_core::VirtualInstrumentConfig {
                        id: InstrumentId::new(*id),
                        name: display_name.clone(),
                        history_capacity: *history_capacity,
                        base_temperature: *base_temperature,
                        measurement_enabled: *measurement_enabled,
                    },
                ))?;
                let signal = SignalId::new(InstrumentId::new(*id), lab_core::TEMPERATURE);
                if *external_publication {
                    emulator_targets.insert(signal);
                } else {
                    measurements.push((
                        InstrumentId::new(*id),
                        Periodic::new(Duration::from_millis(*poll_period_ms)),
                    ));
                }
            }
            InstrumentDto::ThermalPlant {
                id,
                display_name,
                history_capacity,
                ambient_temperature,
                initial_temperature,
                gain_per_percent,
                time_constant_ms,
                poll_period_ms,
                ..
            } => {
                runtime.command(Command::RegisterThermalPlant(ThermalPlantConfig {
                    id: InstrumentId::new(*id),
                    name: display_name.clone(),
                    history_capacity: *history_capacity,
                    ambient_temperature: *ambient_temperature,
                    initial_temperature: *initial_temperature,
                    gain_per_percent: *gain_per_percent,
                    time_constant: Duration::from_millis(*time_constant_ms),
                }))?;
                virtual_model_generations.insert(InstrumentId::new(*id), 1);
                measurements.push((
                    InstrumentId::new(*id),
                    Periodic::new(Duration::from_millis(*poll_period_ms)),
                ));
            }
            InstrumentDto::Metakon {
                id,
                definition,
                resource_id,
                address,
                poll_period_ms,
                queue_timeout_ms,
                transaction_timeout_ms,
                ..
            } => {
                let bytes = deployment
                    .artifact_bytes(definition)
                    .ok_or(Error::InvalidConfiguration("frozen definition missing"))?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| Error::InvalidConfiguration("definition is not UTF-8"))?;
                let definition = parse_definition_json(text)
                    .map_err(|_| Error::InvalidConfiguration("invalid frozen definition"))?;
                if definition.id != InstrumentId::new(*id) {
                    return Err(Error::InvalidConfiguration(
                        "definition and deployment instrument IDs differ",
                    ));
                }
                let temperature = definition
                    .parameters
                    .iter()
                    .find(|parameter| parameter.operation == KnownOperation::Temperature)
                    .ok_or(Error::InvalidConfiguration(
                        "read-only Metakon definition lacks temperature",
                    ))?
                    .id;
                let channel_type = definition
                    .parameters
                    .iter()
                    .find(|parameter| parameter.operation == KnownOperation::ChannelType)
                    .ok_or(Error::InvalidConfiguration(
                        "read-only Metakon definition lacks compatibility probe",
                    ))?
                    .id;
                let output_unit = definition
                    .parameters
                    .iter()
                    .find(|parameter| parameter.operation == KnownOperation::Output)
                    .map(|parameter| parameter.unit);
                let instrument_id = InstrumentId::new(*id);
                runtime.command(Command::RegisterMetakon(MetakonInstrumentConfig {
                    definition,
                    binding: MetakonBinding {
                        resource: ResourceId::new(*resource_id),
                        device: *address,
                        channel: 0,
                        binding_generation: 1,
                        mapping_revision: 1,
                        expected_output_unit: output_unit,
                        output_queue_ttl: output_unit
                            .map(|_| Duration::from_millis(*queue_timeout_ms)),
                        output_timeout: output_unit
                            .map(|_| Duration::from_millis(*transaction_timeout_ms)),
                    },
                    history_capacity: 64,
                }))?;
                physical_instruments.insert(instrument_id);
                metakon_reads.push(MetakonReadSchedule {
                    instrument: instrument_id,
                    parameter: temperature,
                    slot: Periodic::new(Duration::from_millis(*poll_period_ms)),
                    queue_ttl: Duration::from_millis(*queue_timeout_ms),
                    timeout: Duration::from_millis(*transaction_timeout_ms),
                });
                configured_probes.push(ConfiguredProbe {
                    instrument: instrument_id,
                    parameter: channel_type,
                    queue_ttl: Duration::from_millis(*queue_timeout_ms),
                    timeout: Duration::from_millis(*transaction_timeout_ms),
                    queued: false,
                    baseline: None,
                });
            }
        }
    }

    Ok(ConfiguredInstruments {
        measurements,
        metakon_reads,
        virtual_model_generations,
        emulator_targets,
        physical_instruments,
        configured_probes,
    })
}
