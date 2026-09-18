//! Semantic activation provenance values, independent of SQLite row layout.
//!
//! Active native build identity and bounded historical source compatibility use
//! these same values before the storage adapter maps them to schema version one.

/// Exact already-loaded trusted source/definition bytes; a path is never reread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceEntry {
    /// Stable semantic class, such as `managed_component_source`.
    pub kind: String,
    /// Exact byte encoding, such as `utf8` or `json_v1`.
    pub encoding: String,
    /// Owned bytes captured from the active candidate before later file changes.
    pub content: Vec<u8>,
}

/// One frozen object baseline attached to an immutable activation.
#[derive(Clone, Debug)]
pub struct ProvenanceObject {
    /// Stable object family (`instrument`, `controller`, `reference`, `actuator`).
    pub kind: &'static str,
    /// Stable ID: eight bytes for one ID, sixteen for a composite signal/actuator.
    pub id: Vec<u8>,
    /// Stable logical key, separate from a display label.
    pub logical_key: String,
    /// Human-readable label captured at activation.
    pub label: String,
    /// Bounded canonical JSON of exact committed descriptor/configuration fields.
    pub descriptor: String,
    /// Unit identity, if this object has one.
    pub unit_key: Option<String>,
    /// Component generation or native immutable generation one.
    pub generation: Option<u64>,
    /// Immutable instance/output binding description if relevant.
    pub binding: Option<String>,
    /// Index of the exact bounded definition entry covering this object.
    pub definition_entry_index: usize,
    /// Exact text artifact hash for a text-backed managed component.
    pub source_content_sha256: Option<[u8; 32]>,
}
