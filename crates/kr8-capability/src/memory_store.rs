//! Transport-neutral memory.store/1 DTOs and negotiation.

use std::{collections::BTreeSet, fmt, path::Path};

/// Exact family/version negotiated before opening a provider.
pub const MEMORY_STORE_V1: &str = "memory.store/1";
/// Initial canonical provider schema, unrelated to vector/embedder fingerprints.
pub const MEMORY_STORE_SCHEMA: u32 = 1;

/// Store features whose absence prevents safe use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryFeature {
    /// One database partition per authenticated profile.
    ProfilePartitioned,
    /// Forbidden records cannot affect corpus statistics, rank, limit, or budget.
    ScopeBeforeRank,
    /// Exact record/source identities, digests and attribution accompany passages.
    Provenance,
    /// Stable ties and deterministic ordinal merge across allowed places.
    DeterministicOrder,
    /// The same adapter has a genuinely query-only/read-only opener.
    ReadOnlyPeer,
}

/// Connection access is explicit and never silently redirected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreAccess {
    /// Sole native desktop index writer.
    ReadWriteHome,
    /// Query-only exact-profile peer/ACP reader.
    ReadOnlyPeer,
}

/// Provider metadata checked before storage or retrieval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreInfo {
    /// Exact family/version string.
    pub family: String,
    /// Provider-owned schema.
    pub schema: u32,
    /// Stable provider identity.
    pub provider: String,
    /// Stable profile partition.
    pub profile: String,
    /// Connection access.
    pub access: StoreAccess,
    /// Enforced features.
    pub features: BTreeSet<MemoryFeature>,
}

/// Exact log position; sequence alone cannot bind a different log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Zero denotes empty state.
    pub sequence: u64,
    /// Canonical event ID, absent only for empty state.
    pub event_id: Option<String>,
}

impl Cursor {
    /// Check the empty/nonempty cursor invariant.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if (self.sequence == 0) != self.event_id.is_none() {
            return Err(ProviderError::InvalidCursor);
        }
        if let Some(id) = &self.event_id {
            if id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(ProviderError::InvalidCursor);
            }
        }
        Ok(())
    }
}

/// Exact source citation attached to each passage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citation {
    /// Exact source identity, never a substituted target.
    pub source_id: String,
    /// SHA-256 of the source version.
    pub sha256: String,
    /// Explicit source state (present/missing/changed/deleted).
    pub availability: String,
}

/// Validated canonical active projection converted at the adapter boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRecord {
    /// Canonical record identity.
    pub id: String,
    /// Canonical record hash.
    pub record_hash: String,
    /// Exact profile partition.
    pub profile: String,
    /// Exact native-authorized place label.
    pub place: String,
    /// Native author identity.
    pub author: String,
    /// Native record timestamp.
    pub created_at_ms: u64,
    /// Bounded evidence text.
    pub text: String,
    /// Exact source versions.
    pub sources: Vec<Citation>,
}

/// One current projection update. Removals contain no readable payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexChange {
    /// Index a verified active/source-valid canonical record.
    Upsert(IndexRecord),
    /// Remove a lifecycle-invalidated or forgotten record.
    Remove(String),
}

/// Atomic cursor-fenced index batch prepared by the native canonical-log reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexUpdate {
    /// Required previous index head.
    pub previous: Cursor,
    /// Canonical snapshot head after applying this batch.
    pub next: Cursor,
    /// Stable record-ID order; duplicates are rejected.
    pub changes: Vec<IndexChange>,
}

/// Bounded native-derived retrieval request. Deliberately not deserializable.
#[derive(Clone, Debug)]
pub struct ScopedQuery {
    profile: String,
    allowed_places: BTreeSet<String>,
    text: String,
    limit: usize,
    byte_budget: usize,
    required_cursor: Cursor,
}

impl ScopedQuery {
    /// Construct after native profile/membership authorization and log-head capture.
    pub fn new(
        profile: String,
        allowed_places: BTreeSet<String>,
        text: &str,
        limit: usize,
        byte_budget: usize,
        required_cursor: Cursor,
    ) -> Result<Self, ProviderError> {
        if profile.is_empty() || profile.len() > 256 {
            return Err(ProviderError::InvalidQuery("profile"));
        }
        if allowed_places.is_empty()
            || allowed_places.len() > 32
            || allowed_places
                .iter()
                .any(|p| p.is_empty() || p.len() > 4096)
        {
            return Err(ProviderError::InvalidQuery("allowed places"));
        }
        if text.trim().is_empty() || text.len() > 512 || text.chars().any(char::is_control) {
            return Err(ProviderError::InvalidQuery("text"));
        }
        if !(1..=32).contains(&limit) || !(1..=16_384).contains(&byte_budget) {
            return Err(ProviderError::InvalidQuery("bounds"));
        }
        required_cursor.validate()?;
        Ok(Self {
            profile,
            allowed_places,
            text: text.to_owned(),
            limit,
            byte_budget,
            required_cursor,
        })
    }
    /// Bound profile.
    pub fn profile(&self) -> &str {
        &self.profile
    }
    /// Already-authorized exact places, before rank/limit/budget.
    pub fn allowed_places(&self) -> &BTreeSet<String> {
        &self.allowed_places
    }
    /// Bounded literal search text.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Maximum passages, applied after authorized per-place queries.
    pub fn limit(&self) -> usize {
        self.limit
    }
    /// Maximum returned evidence text bytes.
    pub fn byte_budget(&self) -> usize {
        self.byte_budget
    }
    /// Required canonical snapshot head.
    pub fn required_cursor(&self) -> &Cursor {
        &self.required_cursor
    }
}

/// Citation-bearing passage, with local corpus rank and explicit place label.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryPassage {
    /// Exact canonical evidence projection.
    pub record: IndexRecord,
    /// One-based rank within this exact place only.
    pub ordinal: usize,
    /// BM25 score within this place, never fused across corpora.
    pub local_score: f64,
}

/// Readiness reports the observed derived cursor, not invented freshness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The provider can query this snapshot; caller must still compare canonical head.
    Ready(Cursor),
    /// The canonical log is newer and search must fail closed until retry succeeds.
    CatchingUp {
        /// Indexed head.
        indexed: Cursor,
        /// Required canonical head.
        required: Cursor,
    },
}

/// Derived export receipt. No import/catalogue UI is implied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportManifest {
    /// Exact provider schema.
    pub schema: u32,
    /// Profile partition.
    pub profile: String,
    /// Exported index head.
    pub cursor: Cursor,
    /// Number of active evidence rows.
    pub records: u64,
}

/// A real replaceable storage seam; canonical writes remain outside this contract.
pub trait MemoryStore: Send + Sync {
    /// Describe exact family, schema, partition, access and enforced features.
    fn describe(&self) -> StoreInfo;
    /// Probe the actual provider without creating/migrating a peer store.
    fn readiness(&self) -> Result<Readiness, ProviderError>;
    /// Apply one native-indexer batch and its cursor in one transaction.
    fn remember(&self, update: &IndexUpdate) -> Result<Cursor, ProviderError>;
    /// Observe the exact derived head.
    fn cursor(&self) -> Result<Cursor, ProviderError>;
    /// Query only authorized corpora; a cursor mismatch returns an error.
    fn search(&self, query: &ScopedQuery) -> Result<Vec<MemoryPassage>, ProviderError>;
    /// Export a consistent derived SQLite snapshot to a new trusted destination.
    fn export(&self, to: &Path) -> Result<ExportManifest, ProviderError>;
}

/// Reject incompatible providers before a consumer uses them.
pub fn negotiate(
    info: &StoreInfo,
    expected_profile: &str,
    require_read_only: bool,
) -> Result<(), ProviderError> {
    if info.family != MEMORY_STORE_V1 {
        return Err(ProviderError::UnsupportedFamily(info.family.clone()));
    }
    if info.schema != MEMORY_STORE_SCHEMA {
        return Err(ProviderError::UnsupportedSchema(info.schema));
    }
    if info.profile != expected_profile {
        return Err(ProviderError::ProfileMismatch);
    }
    for feature in [
        MemoryFeature::ProfilePartitioned,
        MemoryFeature::ScopeBeforeRank,
        MemoryFeature::Provenance,
        MemoryFeature::DeterministicOrder,
        MemoryFeature::ReadOnlyPeer,
    ] {
        if !info.features.contains(&feature) {
            return Err(ProviderError::MissingFeature(feature));
        }
    }
    if require_read_only && info.access != StoreAccess::ReadOnlyPeer {
        return Err(ProviderError::ReadOnly);
    }
    Ok(())
}

/// Stable errors shared across adapters without workspace or third-party dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// An incompatible family/version cannot be silently substituted.
    UnsupportedFamily(String),
    /// Unknown/newer schema cannot be silently migrated by a reader.
    UnsupportedSchema(u32),
    /// A required feature was not declared.
    MissingFeature(MemoryFeature),
    /// Profile identity does not match the bound database.
    ProfileMismatch,
    /// A peer cannot write or redirect writes to a home store.
    ReadOnly,
    /// The derived head does not match the canonical snapshot.
    StaleCursor {
        /// Actual derived head.
        indexed: Cursor,
        /// Required canonical head.
        required: Cursor,
    },
    /// Malformed or foreign log cursor.
    InvalidCursor,
    /// A bounded query or index field failed validation.
    InvalidQuery(&'static str),
    /// Corrupt persisted index; callers preserve their original prompt.
    Corrupt(String),
    /// A durable backend failure must propagate/retry.
    Backend(String),
    /// Export cannot overwrite an existing destination.
    ExportExists,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for ProviderError {}
