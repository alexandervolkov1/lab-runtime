//! Immutable activation and content-addressed provenance transactions.
//!
//! Historical source kinds are accepted here only as bounded schema-v1 archive
//! compatibility; active native composition supplies the semantic model above SQL.

use super::*;

impl SqliteStore {
    /// Commit one bounded immutable activation manifest and deduplicated bytes.
    /// SQL is performed only on the storage worker or in an offline test.
    pub fn commit_provenance(
        &mut self,
        entries: &[ProvenanceEntry],
    ) -> Result<[u8; 32], StorageError> {
        self.commit_activation(entries, &[])
    }

    /// Commit one frozen source set and object baseline in the same transaction.
    pub fn commit_activation(
        &mut self,
        entries: &[ProvenanceEntry],
        objects: &[ProvenanceObject],
    ) -> Result<[u8; 32], StorageError> {
        self.commit_activation_inner(entries, objects, None)
    }

    pub(crate) fn commit_activation_lifecycle(
        &mut self,
        entries: &[ProvenanceEntry],
        objects: &[ProvenanceObject],
        lifecycle: &ConfigurationLifecycleRecord,
        activation_generation: u64,
        assigned: u64,
    ) -> Result<[u8; 32], StorageError> {
        self.commit_activation_inner(
            entries,
            objects,
            Some((lifecycle, activation_generation, assigned)),
        )
    }

    fn commit_activation_inner(
        &mut self,
        entries: &[ProvenanceEntry],
        objects: &[ProvenanceObject],
        lifecycle: Option<(&ConfigurationLifecycleRecord, u64, u64)>,
    ) -> Result<[u8; 32], StorageError> {
        self.require_main_reserve()?;
        self.require_wal_budget()?;
        if entries.is_empty() || entries.len() > 128 {
            return Err(StorageError("provenance count must be 1..=128".into()));
        }
        let mut charge = 0usize;
        let mut indexed = Vec::with_capacity(entries.len());
        let mut entry_hashes = Vec::with_capacity(entries.len());
        // Historical executable-source kinds are archive vocabulary, not active
        // component configuration. Preserve their exact spelling and bounds so old
        // content-addressed evidence stays readable.
        let mut managed_source_hashes = BTreeSet::new();
        for entry in entries {
            if entry.kind.is_empty()
                || entry.kind.len() > 64
                || entry.encoding.is_empty()
                || entry.encoding.len() > 64
                || entry.content.is_empty()
                || entry.content.len() > 64 * 1024
                || (matches!(
                    entry.kind.as_str(),
                    "managed_component_source" | "managed_lua_source"
                ) && entry.content.len() > 32 * 1024)
            {
                return Err(StorageError("invalid bounded provenance entry".into()));
            }
            charge = charge
                .checked_add(
                    entry.kind.capacity()
                        + entry.encoding.capacity()
                        + entry.content.capacity()
                        + 128,
                )
                .ok_or_else(|| StorageError("provenance credit exhausted".into()))?;
            if charge > 1024 * 1024 {
                return Err(StorageError("provenance credit exhausted".into()));
            }
            let content_hash: [u8; 32] = Sha256::digest(&entry.content).into();
            if entry.kind == "managed_component_source" {
                managed_source_hashes.insert(content_hash);
            }
            entry_hashes.push(content_hash);
            indexed.push((entry, content_hash));
        }
        if objects.len() > 256 {
            return Err(StorageError("object baseline count exhausted".into()));
        }
        let mut object_charge = 0usize;
        for object in objects {
            if !matches!(object.id.len(), 8 | 16)
                || object.kind.len() > 64
                || object.logical_key.len() > 128
                || object.label.len() > 128
                || object.descriptor.len() > 4096
                || object.descriptor.is_empty()
                || object.unit_key.as_ref().is_some_and(|unit| unit.len() > 32)
                || object
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.len() > 128)
                || object.definition_entry_index >= entry_hashes.len()
                || (object.kind != "managed_component" && object.source_content_sha256.is_some())
                || object
                    .source_content_sha256
                    .is_some_and(|source_hash| !managed_source_hashes.contains(&source_hash))
            {
                return Err(StorageError("invalid bounded object baseline".into()));
            }
            serde_json::from_str::<serde_json::Value>(&object.descriptor)
                .map_err(|_| StorageError("invalid object descriptor JSON".into()))?;
            object_charge = object_charge
                .checked_add(
                    object.id.capacity()
                        + object.logical_key.capacity()
                        + object.label.capacity()
                        + object.descriptor.capacity()
                        + object.unit_key.as_ref().map_or(0, String::capacity)
                        + object.binding.as_ref().map_or(0, String::capacity)
                        + 256,
                )
                .ok_or_else(|| StorageError("object baseline credit exhausted".into()))?;
            if object_charge > 256 * 1024 {
                return Err(StorageError("object baseline credit exhausted".into()));
            }
        }
        indexed.sort_by(|left, right| {
            (&left.0.kind, &left.0.encoding, left.1).cmp(&(
                &right.0.kind,
                &right.0.encoding,
                right.1,
            ))
        });
        for pair in indexed.windows(2) {
            if pair[0].0.kind == pair[1].0.kind
                && pair[0].0.encoding == pair[1].0.encoding
                && pair[0].1 == pair[1].1
                && pair[0].0.content != pair[1].0.content
            {
                return Err(StorageError("provenance hash/content collision".into()));
            }
        }
        indexed.dedup_by(|left, right| {
            left.0.kind == right.0.kind
                && left.0.encoding == right.0.encoding
                && left.1 == right.1
                && left.0.content == right.0.content
        });
        // The root is the hash of actual stored canonical JSON, so inspection
        // can reconstruct the typed entry references without executable code.
        let refs: Vec<serde_json::Value> = indexed
            .iter()
            .map(|(entry, hash)| {
                serde_json::json!({"kind":entry.kind,"encoding":entry.encoding,
                "sha256":hex_hash(hash)})
            })
            .collect();
        let manifest = serde_json::json!({"encoding_version":1,
            "hash_algorithm":"sha256","entries":refs})
        .to_string()
        .into_bytes();
        if manifest.len() > 64 * 1024
            || charge
                .checked_add(manifest.len() + 128)
                .is_none_or(|total| total > 1024 * 1024)
        {
            return Err(StorageError("provenance manifest credit exhausted".into()));
        }
        let root: [u8; 32] = Sha256::digest(&manifest).into();
        let lifecycle_commit = if let Some((record, generation, assigned)) = lifecycle {
            if !record.valid()
                || generation == 0
                || self.next_record_sequence.checked_add(1) != Some(assigned)
            {
                return Err(StorageError(
                    "invalid configuration lifecycle record".into(),
                ));
            }
            let commit = self
                .commit_no
                .checked_add(1)
                .ok_or_else(|| StorageError("lifecycle commit identity exhausted".into()))?;
            let payload = serde_json::json!({
                "encoding_version":1,
                "operation_id":record.operation_id.to_string(),
                "operation_kind":record.operation_kind,
                "phase":"applied",
                "base_revision":record.base_revision.to_string(),
                "committed_revision":record.committed_revision.to_string(),
                "activation_generation":generation.to_string(),
                "activation_root":hex_hash(&root),
                "runtime_toml_sha256":hex_hash(&record.toml_hash),
                "affected":record.affected,
                "reason":record.reason,
                "committed_at_ns":record.at.as_nanos().to_string(),
            })
            .to_string();
            if payload.len() > 64 * 1024 {
                return Err(StorageError(
                    "configuration lifecycle payload exhausted".into(),
                ));
            }
            Some((record, assigned, commit, payload))
        } else {
            None
        };
        let activation_no = self.next_activation_no;
        let next = activation_no
            .checked_add(1)
            .ok_or_else(|| StorageError("activation identity exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        for (entry, hash) in indexed {
            let existing: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT content FROM provenance_content WHERE content_hash=?1 AND kind=?2 AND encoding=?3",
                    params![hash.as_slice(), entry.kind, entry.encoding],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(content) = existing {
                if content != entry.content {
                    return Err(StorageError("provenance hash/content collision".into()));
                }
            } else {
                transaction.execute(
                    "INSERT INTO provenance_content(content_hash,encoding,kind,content)
                     VALUES(?1,?2,?3,?4)",
                    params![hash.as_slice(), entry.encoding, entry.kind, entry.content],
                )?;
            }
        }
        let existing_root: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT content FROM provenance_content WHERE content_hash=?1 AND kind='activation_manifest' AND encoding='json_v1'",
                params![root.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(content) = existing_root {
            if content != manifest {
                return Err(StorageError(
                    "provenance root hash/content collision".into(),
                ));
            }
        } else {
            transaction.execute(
                "INSERT INTO provenance_content(content_hash,encoding,kind,content)
                 VALUES(?1,'json_v1','activation_manifest',?2)",
                params![root.as_slice(), manifest],
            )?;
        }
        transaction.execute(
            "INSERT INTO configurations(boot_id,activation_no,manifest_root_hash,
             manifest_content_hash,encoding) VALUES(?1,?2,?3,?3,'manifest_v1')",
            params![
                self.boot_id.as_slice(),
                u64_blob(activation_no).as_slice(),
                root.as_slice()
            ],
        )?;
        for object in objects {
            let definition = entry_hashes[object.definition_entry_index];
            let source_hash = object.source_content_sha256.map(|hash| hash.to_vec());
            let safety_hash = (object.kind == "actuator").then_some(definition.to_vec());
            transaction.execute(
                "INSERT INTO object_snapshots(boot_id,activation_no,object_kind,object_id,
                 logical_key,label,generation,descriptor,unit_key,instance_binding,
                 definition_hash,source_hash,safety_hash)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    self.boot_id.as_slice(),
                    u64_blob(activation_no).as_slice(),
                    object.kind,
                    object.id.as_slice(),
                    object.logical_key,
                    object.label,
                    object.generation.map(u64_blob).map(|id| id.to_vec()),
                    object.descriptor,
                    object.unit_key,
                    object.binding,
                    definition.as_slice(),
                    source_hash,
                    safety_hash
                ],
            )?;
        }
        if let Some((record, sequence, commit, payload)) = &lifecycle_commit {
            let run = self.run_no.map(u64_blob);
            let interval = self.interval_no.map(u64_blob);
            let wall_estimate = self.boot_anchor.estimate_us(record.at)?;
            transaction.execute(
                "INSERT INTO records(boot_id,record_seq,run_no,interval_no,kind,version,\
                 published_at,captured_at,wall_estimate_us,wall_basis,origin,payload)\
                 VALUES(?1,?2,?3,?4,'configuration_lifecycle',1,?5,?5,?6,\
                 'boot_anchor','runtime',?7)",
                params![
                    self.boot_id.as_slice(),
                    u64_blob(*sequence).as_slice(),
                    run.as_ref().map(|value| value.as_slice()),
                    interval.as_ref().map(|value| value.as_slice()),
                    duration_blob(record.at)?.as_slice(),
                    wall_estimate,
                    payload.as_bytes(),
                ],
            )?;
            transaction.execute(
                "UPDATE durable_checkpoints SET commit_no=?2,persisted_through_seq=?3 WHERE boot_id=?1",
                params![
                    self.boot_id.as_slice(),
                    u64_blob(*commit).as_slice(),
                    u64_blob(*sequence).as_slice(),
                ],
            )?;
        }
        transaction.commit()?;
        self.next_activation_no = next;
        self.current_activation_no = Some(activation_no);
        if let Some((_, sequence, commit, _)) = lifecycle_commit {
            self.next_record_sequence = sequence;
            self.commit_no = commit;
        }
        Ok(root)
    }
}
