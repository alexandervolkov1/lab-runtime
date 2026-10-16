//! Schema-v1 scalar, lineage and measurement row encoding/decoding helpers.

use super::*;

pub(super) fn u64_blob(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

// A malformed native fact must roll back the entire mixed batch before any
// checkpoint advances; SQLite may otherwise coerce NaN to NULL silently.
pub(super) fn validate_storage_fact(fact: &RecordingFact) -> Result<(), StorageError> {
    match fact {
        RecordingFact::Measurement {
            sample,
            revision,
            state_revision,
            lineage,
            ..
        } => {
            if sample.value().is_some_and(|value| {
                matches!(value,
                Value::Float(number) if !number.is_finite())
            }) {
                return Err(StorageError("nonfinite measurement fact".into()));
            }
            if state_revision.is_some() && *revision != 1 {
                return Err(StorageError(
                    "managed measurement definition revision is not immutable".into(),
                ));
            }
            if let Some(input) = lineage {
                if state_revision.is_none()
                    || (sample.quality() == SampleQuality::Good
                        && sample.freshness_at() != input.freshness_at)
                    || input.at > sample.at()
                    || input.unit != sample.unit()
                {
                    return Err(StorageError("inconsistent transform input lineage".into()));
                }
                encode_lineage(*input)?;
            }
        }
        RecordingFact::Output {
            value,
            unit,
            authority_epoch,
            resource,
            binding_generation,
            mapping_revision,
            ..
        } => {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(StorageError("nonfinite output fact".into()));
            }
            if unit.is_none() || authority_epoch.is_none() {
                return Err(StorageError(
                    "output fact missing trusted unit or epoch".into(),
                ));
            }
            let binding_fields = usize::from(resource.is_some())
                + usize::from(binding_generation.is_some())
                + usize::from(mapping_revision.is_some());
            if binding_fields != 0 && binding_fields != 3 {
                return Err(StorageError(
                    "output fact has partial binding identity".into(),
                ));
            }
        }
        RecordingFact::Controller { pid, .. } => {
            if pid.is_some_and(|pid| {
                !pid.kp.is_finite()
                    || !pid.ki.is_finite()
                    || !pid.kd.is_finite()
                    || !pid.output_min.is_finite()
                    || !pid.output_max.is_finite()
            }) {
                return Err(StorageError("nonfinite controller fact".into()));
            }
        }
        RecordingFact::Reference {
            value,
            target,
            rate,
            ..
        } => {
            if !value.is_finite()
                || target.is_some_and(|target| !target.is_finite())
                || rate.is_some_and(|rate| !rate.is_finite())
            {
                return Err(StorageError("nonfinite Reference fact".into()));
            }
        }
    }
    Ok(())
}

// Clock facts follow the same records/projection transaction as every other
// durable source fact; actual UTC remains independent of the fixed boot map.
// The full boot/run/interval scope is passed into one atomic projection so an
// actual UTC anchor cannot silently attach to another recording interval.
#[expect(clippy::too_many_arguments, reason = "explicit clock fact scope")]
pub(super) fn insert_clock_projection(
    transaction: &rusqlite::Transaction<'_>,
    boot_id: &[u8; 16],
    sequence: u64,
    anchor_no: u64,
    kind: &str,
    anchor: &TimeAnchor,
    run_no: Option<u64>,
    interval_no: Option<u64>,
    boot_anchor: &TimeAnchor,
) -> Result<(), StorageError> {
    let time = duration_blob(anchor.after())?;
    let wall_estimate = boot_anchor.estimate_us(anchor.after())?;
    let payload = serde_json::json!({
        "kind":kind,
        "monotonic_before_ns":anchor.before().as_nanos().to_string(),
        "monotonic_after_ns":anchor.after().as_nanos().to_string(),
        "uncertainty_ns":anchor.uncertainty_ns().to_string(),
        "actual_wall_us":anchor.wall_us(),
        "unavailable_reason":anchor.unavailable_reason(),
        "wall_basis":"boot_anchor"
    })
    .to_string();
    transaction.execute(
        "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,
         published_at,observed_at,captured_at,wall_estimate_us,wall_basis,origin,payload)
         VALUES(?1,?2,?3,?4,'clock_anchor',1,?5,?5,?5,?6,'boot_anchor',
         'recorder_clock',?7)",
        params![
            boot_id.as_slice(),
            u64_blob(sequence).as_slice(),
            run_no.map(u64_blob).map(|value| value.to_vec()),
            interval_no.map(u64_blob).map(|value| value.to_vec()),
            time.as_slice(),
            wall_estimate,
            payload.as_bytes()
        ],
    )?;
    transaction.execute(
        "INSERT INTO runtime_events(boot_id,record_seq,category,severity,code,data)
         VALUES(?1,?2,'clock','info',?3,?4)",
        params![
            boot_id.as_slice(),
            u64_blob(sequence).as_slice(),
            kind,
            payload.as_bytes()
        ],
    )?;
    transaction.execute(
        "INSERT INTO clock_anchors(boot_id,anchor_no,record_seq,kind,monotonic_before,
         monotonic_after,uncertainty_ns,wall_us,unavailable_reason)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            boot_id.as_slice(),
            u64_blob(anchor_no).as_slice(),
            u64_blob(sequence).as_slice(),
            kind,
            duration_blob(anchor.before())?.as_slice(),
            time.as_slice(),
            u64_blob(anchor.uncertainty_ns()).as_slice(),
            anchor.wall_us(),
            anchor.unavailable_reason()
        ],
    )?;
    Ok(())
}

pub(super) fn parse_boot_id(text: &str) -> Result<[u8; 16], StorageError> {
    if text.len() != 32
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StorageError(
            "boot identity must be 32 lowercase hex digits".into(),
        ));
    }
    let mut result = [0u8; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| StorageError("invalid boot identity".into()))?;
    }
    Ok(result)
}

pub(super) fn duration_blob(value: Duration) -> Result<[u8; 8], StorageError> {
    let nanos = u64::try_from(value.as_nanos())
        .map_err(|_| StorageError("monotonic time exceeds version-one range".into()))?;
    Ok(u64_blob(nanos))
}

pub(super) fn read_u64_blob(value: Vec<u8>) -> Result<u64, rusqlite::Error> {
    let bytes: [u8; 8] = value
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(u64::from_be_bytes(bytes))
}

pub(super) fn read_boot_blob(value: Vec<u8>) -> Result<[u8; 16], rusqlite::Error> {
    value.try_into().map_err(|_| rusqlite::Error::InvalidQuery)
}

pub(super) fn hex_boot(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn hex_hash(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

// The fixed typed lineage envelope is bounded and never reads a current
// managed generation while decoding historical data.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLineage {
    version: u8,
    instrument: String,
    parameter: String,
    value: f64,
    unit: String,
    published_at_ns: String,
    observed_at_ns: String,
    source_generation: String,
    source_revision: String,
    source_state_revision: Option<String>,
}

pub(super) fn encode_lineage(input: CapturedInput) -> Result<Vec<u8>, StorageError> {
    if !input.value.is_finite()
        || input.source_generation == 0
        || input.source_revision == 0
        || input.freshness_at > input.at
    {
        return Err(StorageError("invalid captured input lineage".into()));
    }
    let payload = StoredLineage {
        version: 1,
        instrument: input.signal.instrument().get().to_string(),
        parameter: input.signal.parameter().get().to_string(),
        value: input.value,
        unit: input.unit.id().into(),
        published_at_ns: input.at.as_nanos().to_string(),
        observed_at_ns: input.freshness_at.as_nanos().to_string(),
        source_generation: input.source_generation.to_string(),
        source_revision: input.source_revision.to_string(),
        source_state_revision: input
            .source_state_revision
            .map(|revision| revision.to_string()),
    };
    let encoded = serde_json::to_vec(&payload)
        .map_err(|_| StorageError("cannot encode captured input lineage".into()))?;
    if encoded.len() > 512 {
        return Err(StorageError(
            "captured input lineage exceeds record bound".into(),
        ));
    }
    Ok(encoded)
}

pub(super) fn decode_lineage(bytes: Vec<u8>) -> Result<MeasurementLineage, rusqlite::Error> {
    if bytes.len() > 512 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let stored: StoredLineage =
        serde_json::from_slice(&bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let parse = |value: &str| {
        value
            .parse::<u64>()
            .map_err(|_| rusqlite::Error::InvalidQuery)
    };
    let published = parse(&stored.published_at_ns)?;
    let observed = parse(&stored.observed_at_ns)?;
    let source_generation = parse(&stored.source_generation)?;
    let source_revision = parse(&stored.source_revision)?;
    let source_state_revision = stored
        .source_state_revision
        .as_deref()
        .map(parse)
        .transpose()?;
    if stored.version != 1
        || !stored.value.is_finite()
        || source_generation == 0
        || source_revision == 0
        || observed > published
        || stored.unit.is_empty()
        || stored.unit.len() > 32
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(MeasurementLineage {
        signal: SignalId::new(
            InstrumentId::new(parse(&stored.instrument)?),
            ParameterId::new(parse(&stored.parameter)?),
        ),
        value: stored.value,
        unit: stored.unit,
        published_at: Duration::from_nanos(published),
        observed_at: Duration::from_nanos(observed),
        source_generation,
        source_revision,
        source_state_revision,
    })
}

pub(super) fn decode_measurement_row(
    row: &rusqlite::Row<'_>,
) -> Result<MeasurementRow, rusqlite::Error> {
    let kind: String = row.get(6)?;
    let value = match kind.as_str() {
        "none" => None,
        "float" => Some(Value::Float(row.get(7)?)),
        "integer" => Some(Value::Integer(row.get(8)?)),
        "boolean" => Some(Value::Boolean(row.get::<_, i64>(9)? != 0)),
        "text" => Some(Value::Text(row.get(10)?)),
        "enum" => Some(Value::Enum(row.get(10)?)),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(MeasurementRow {
        record_sequence: read_u64_blob(row.get(0)?)?,
        published_at: Duration::from_nanos(read_u64_blob(row.get(1)?)?),
        observed_at: Duration::from_nanos(read_u64_blob(row.get(2)?)?),
        unit: row.get(3)?,
        quality: row.get(4)?,
        failure: row.get(5)?,
        value,
        generation: read_u64_blob(row.get(11)?)?,
        revision: read_u64_blob(row.get(12)?)?,
        state_revision: row
            .get::<_, Option<Vec<u8>>>(13)?
            .map(read_u64_blob)
            .transpose()?,
        lineage: row
            .get::<_, Option<Vec<u8>>>(14)?
            .map(decode_lineage)
            .transpose()?,
    })
}
