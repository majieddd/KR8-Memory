//! Canonical evidence envelope and exact provenance.

use crate::{bounded, MemoryError, PlaceId, ProfileIdentity, ProfileKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};
use std::fmt;

/// Lowercase SHA-256 with validation during deserialization.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Digest(String);

impl Digest {
    /// Hash exact bytes without locale or map-order dependence.
    pub fn of(bytes: &[u8]) -> Self {
        Self(
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }
    /// Parse only the canonical lowercase hex form.
    pub fn parse(value: &str) -> Result<Self, MemoryError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(MemoryError::Invalid("sha256"));
        }
        Ok(Self(value.to_owned()))
    }
    /// Canonical digest text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Content-addressed canonical record ID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RecordId(String);

impl RecordId {
    /// Parse an exact record ID without accepting arbitrary record aliases.
    pub fn parse(value: &str) -> Result<Self, MemoryError> {
        let digest = value
            .strip_prefix("mem:")
            .ok_or(MemoryError::Invalid("record id"))?;
        Digest::parse(digest)?;
        Ok(Self(value.to_owned()))
    }
    /// Stable identifier copied byte-for-byte into citations.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RecordId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Exact source identity; it never grants filesystem or relay access.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    /// Validate an opaque source identifier, including its source family.
    pub fn parse(value: &str) -> Result<Self, MemoryError> {
        bounded(value, 2048, "source id")?;
        if value.chars().any(char::is_whitespace)
            || ![
                "msg:",
                "vault:",
                "engram:",
                "board:",
                "artifact:",
                "turn:",
                "mem:",
            ]
            .iter()
            .any(|p| value.starts_with(p) && value.len() > p.len())
        {
            return Err(MemoryError::Invalid("source id"));
        }
        Ok(Self(value.to_owned()))
    }
    /// Exact source ID, not a substitute or best-match target.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Availability is distinct from citation identity and evidence authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceAvailability {
    /// Verified source matches its digest.
    Present,
    /// Exact target is absent; never substitute a similarly named target.
    Missing,
    /// Source content changed and must be explicitly reverified.
    Changed,
    /// Source was deleted or retracted.
    Deleted,
}

impl SourceAvailability {
    /// Stable display/transport label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Changed => "changed",
            Self::Deleted => "deleted",
        }
    }
}

/// Verified provenance stamped by a native source resolver.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    /// Exact source identity.
    pub id: SourceId,
    /// Digest of the exact verified source version.
    pub sha256: Digest,
    /// Native-stamped source place, never inferred from prose.
    pub place: PlaceId,
    /// Source timestamp, when known.
    pub created_at_ms: Option<u64>,
    /// Verified availability at creation; later state is projected separately.
    pub availability: SourceAvailability,
}

/// Evidence record kind. Native first-slice commands expose note/pointer/decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordType {
    /// Human free note.
    Note,
    /// Source-backed factual claim.
    Fact,
    /// Source-backed recorded failure.
    Fail,
    /// Source-backed completed work.
    Done,
    /// Open work item.
    Open,
    /// Source-backed decision.
    Decision,
    /// Exact durable target and producing turn.
    Pointer,
    /// Source-backed handoff.
    Handoff,
    /// Source-backed digest.
    Digest,
}

/// Author role is attribution, never standing instruction authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorRole {
    /// Authenticated owner acting through native UI.
    Person,
    /// Harness-captured evidence.
    Bridge,
    /// Agent-authored evidence.
    Agent,
    /// Reviewed evidence attribution.
    Librarian,
}

/// Exact pointer metadata verified by the native source resolver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointerTarget {
    /// Exact target source; absence must be visible.
    pub source: SourceId,
    /// Verified target byte length.
    pub size: u64,
    /// Verified target digest.
    pub sha256: Digest,
    /// Target modification time, when available.
    pub modified_at_ms: Option<u64>,
}

/// Unprivileged content assembled after native source verification.
#[derive(Clone, Debug)]
pub struct RecordDraft {
    /// Opaque object label; not a filesystem locator.
    pub object: String,
    /// Evidence kind.
    pub record_type: RecordType,
    /// At most 400 UTF-8 bytes.
    pub text: String,
    /// Exact source versions.
    pub sources: Vec<SourceRef>,
    /// Canonical records on which this evidence depends.
    pub derived_from: Vec<RecordId>,
    /// Present only for pointer records.
    pub target: Option<PointerTarget>,
}

impl RecordDraft {
    /// A human note has no fabricated provenance requirement.
    pub fn note(object: &str, text: &str) -> Self {
        Self {
            object: object.to_owned(),
            record_type: RecordType::Note,
            text: text.to_owned(),
            sources: vec![],
            derived_from: vec![],
            target: None,
        }
    }
}

/// Native-only creation stamp; request DTOs cannot deserialize it.
#[derive(Clone, Debug)]
pub struct CreationContext {
    profile: ProfileKey,
    place: PlaceId,
    author: String,
    created_at_ms: u64,
    turn: String,
}

impl CreationContext {
    /// Bind one owner action to its authenticated profile, selected place and native time.
    pub fn owner(
        identity: &ProfileIdentity,
        place: PlaceId,
        created_at_ms: u64,
        turn: &str,
    ) -> Result<Self, MemoryError> {
        let profile = identity.key()?;
        place.validate()?;
        if place.profile() != &profile {
            return Err(MemoryError::ScopeMismatch);
        }
        bounded(turn, 512, "turn")?;
        Ok(Self {
            profile,
            place,
            author: identity.owner().to_owned(),
            created_at_ms,
            turn: turn.to_owned(),
        })
    }
}

/// Immutable canonical payload; lifecycle state is a separate projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordPayload {
    /// Canonical record format.
    pub schema: u32,
    /// Profile partition.
    pub profile: ProfileKey,
    /// Native-stamped exact place.
    pub place: PlaceId,
    /// Opaque object identity.
    pub object: String,
    /// Evidence kind.
    pub record_type: RecordType,
    /// Bounded evidence text.
    pub text: String,
    /// Stable sorted exact sources.
    pub sources: Vec<SourceRef>,
    /// Stable sorted source-record IDs.
    pub derived_from: Vec<RecordId>,
    /// Authenticated owner key.
    pub author: String,
    /// Attribution role; new owner records use Person.
    pub author_role: AuthorRole,
    /// Native operation/turn ID.
    pub turn: String,
    /// Native timestamp.
    pub created_at_ms: u64,
    /// Exact pointer metadata, if applicable.
    pub target: Option<PointerTarget>,
}

/// Canonical record and its independently checked content address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordEnvelope {
    id: RecordId,
    hash: Digest,
    payload: RecordPayload,
}

impl RecordEnvelope {
    /// Create an evidence-only envelope from native authority and bounded content.
    pub fn create(context: &CreationContext, mut draft: RecordDraft) -> Result<Self, MemoryError> {
        draft.sources.sort();
        draft.derived_from.sort();
        draft.derived_from.dedup();
        let payload = RecordPayload {
            schema: 1,
            profile: context.profile.clone(),
            place: context.place.clone(),
            object: draft.object,
            record_type: draft.record_type,
            text: draft.text,
            sources: draft.sources,
            derived_from: draft.derived_from,
            author: context.author.clone(),
            author_role: AuthorRole::Person,
            turn: context.turn.clone(),
            created_at_ms: context.created_at_ms,
            target: draft.target,
        };
        validate_payload(&payload)?;
        let hash = Digest::of(&serde_json::to_vec(&payload)?);
        let record = Self {
            id: RecordId(format!("mem:{hash}")),
            hash,
            payload,
        };
        record.validate()?;
        Ok(record)
    }
    /// Canonical identifier.
    pub fn id(&self) -> &RecordId {
        &self.id
    }
    /// Hash over immutable canonical payload bytes.
    pub fn hash(&self) -> &Digest {
        &self.hash
    }
    /// Immutable evidence and provenance.
    pub fn payload(&self) -> &RecordPayload {
        &self.payload
    }
    /// Validate boundaries and the exact content-addressed identity.
    pub fn validate(&self) -> Result<(), MemoryError> {
        validate_payload(&self.payload)?;
        let bytes = serde_json::to_vec(&self.payload)?;
        if bytes.len() > 63_000 {
            return Err(MemoryError::Invalid("record bytes"));
        }
        let actual = Digest::of(&bytes);
        if actual != self.hash || self.id.as_str() != format!("mem:{actual}") {
            return Err(MemoryError::DigestMismatch);
        }
        Ok(())
    }
    /// Canonical fixed-field JSON suitable for durable log persistence.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, MemoryError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
    /// Decode persisted bytes and verify rather than trusting their stored digest.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MemoryError> {
        if bytes.len() > 65_536 {
            return Err(MemoryError::Invalid("record bytes"));
        }
        let record: Self = serde_json::from_slice(bytes)?;
        record.validate()?;
        Ok(record)
    }
}

fn validate_payload(payload: &RecordPayload) -> Result<(), MemoryError> {
    if payload.schema != 1 {
        return Err(MemoryError::Invalid("record schema"));
    }
    payload.place.validate()?;
    if payload.place.profile() != &payload.profile {
        return Err(MemoryError::ScopeMismatch);
    }
    bounded(&payload.object, 2048, "object")?;
    bounded(&payload.author, 256, "author")?;
    bounded(&payload.turn, 512, "turn")?;
    if payload.text.trim().is_empty() || payload.text.len() > 400 || payload.text.contains('\0') {
        return Err(MemoryError::Invalid("text"));
    }
    if payload.sources.len() > 32 || payload.derived_from.len() > 32 {
        return Err(MemoryError::Invalid("provenance count"));
    }
    if payload
        .sources
        .windows(2)
        .any(|pair| pair[0] >= pair[1] || pair[0].id == pair[1].id)
        || payload
            .derived_from
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(MemoryError::Invalid("canonical provenance order"));
    }
    for source in &payload.sources {
        source.place.validate()?;
        if source.place != payload.place {
            return Err(MemoryError::ScopeMismatch);
        }
    }
    if matches!(
        payload.record_type,
        RecordType::Fact
            | RecordType::Decision
            | RecordType::Done
            | RecordType::Pointer
            | RecordType::Handoff
            | RecordType::Digest
    ) && payload.sources.is_empty()
    {
        return Err(MemoryError::Invalid("source required"));
    }
    match (&payload.target, payload.record_type) {
        (Some(target), RecordType::Pointer) => {
            if !payload
                .sources
                .iter()
                .any(|source| source.id == target.source && source.sha256 == target.sha256)
                || !payload
                    .sources
                    .iter()
                    .any(|source| source.id.as_str().starts_with("turn:"))
            {
                return Err(MemoryError::Invalid(
                    "pointer target and turn sources required",
                ));
            }
        }
        (None, RecordType::Pointer) | (Some(_), _) => {
            return Err(MemoryError::Invalid("pointer target"))
        }
        (None, _) => {}
    }
    Ok(())
}
