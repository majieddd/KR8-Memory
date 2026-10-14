//! Explicit canonical events, lifecycle projections, and bounded spool decoding.

use crate::{
    bounded, Digest, MemoryError, ProfileIdentity, ProfileKey, RecordEnvelope, RecordId,
    SourceAvailability, SourceId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable lifecycle projection; record payloads never rewrite their own history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleStatus {
    /// Current source-valid evidence.
    Active,
    /// Explicit replacement linked this record.
    Superseded,
    /// Explicitly retracted evidence.
    Retracted,
    /// Pointer's exact target is absent and must be reported as absent.
    Missing,
    /// A source was changed, deleted, or invalidated.
    Stale,
    /// Payload erased; only a tombstone may remain.
    Forgotten,
}

impl LifecycleStatus {
    /// Stable lifecycle label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Retracted => "retracted",
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::Forgotten => "forgotten",
        }
    }
    /// Only source-valid active evidence participates in retrieval.
    pub fn searchable(self) -> bool {
        self == Self::Active
    }
}

/// Canonical record plus its current lifecycle/source projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedRecord {
    /// Immutable verified envelope.
    pub record: RecordEnvelope,
    /// Current lifecycle.
    pub status: LifecycleStatus,
    /// Explicit replacement, if any.
    pub superseded_by: Option<RecordId>,
    /// Current source state overrides, keyed by exact source ID.
    pub source_states: BTreeMap<SourceId, SourceAvailability>,
}

/// Exact ordered canonical-log position, including chain identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogCursor {
    /// Zero is the empty log.
    pub sequence: u64,
    /// None only for the empty log; otherwise the final canonical event ID.
    pub event_id: Option<Digest>,
}

/// One explicit owner lifecycle action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum EventOperation {
    /// Create distinct evidence; contradictory notes do not overwrite one another.
    Create {
        /// New record.
        record: RecordEnvelope,
    },
    /// Atomically create a replacement and link its original.
    Supersede {
        /// Existing active record ID.
        original: RecordId,
        /// New same-place evidence.
        replacement: RecordEnvelope,
    },
    /// Explicitly withdraw current evidence.
    Retract {
        /// Exact record ID.
        record_id: RecordId,
    },
    /// Invalidate the exact source and all dependent record projections.
    InvalidateSource {
        /// Exact source ID.
        source_id: SourceId,
        /// Missing, changed, or deleted; never an implicit restore.
        availability: SourceAvailability,
    },
    /// Purge readable payload and retain minimal anti-resurrection state.
    Forget {
        /// Exact record ID.
        record_id: RecordId,
    },
}

/// Canonical event body, separate from the log's sequence/cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBody {
    /// Canonical event schema.
    pub schema: u32,
    /// Bound profile.
    pub profile: ProfileKey,
    /// Authenticated native owner.
    pub actor: String,
    /// Native event time.
    pub created_at_ms: u64,
    /// Native operation ID for replay/idempotency.
    pub turn: String,
    /// Explicit change.
    pub change: EventOperation,
}

/// Content-addressed immutable event, except readable payload redaction on forget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEvent {
    id: Digest,
    body: EventBody,
}

impl MemoryEvent {
    /// Stamp an authorized native owner action; JavaScript supplies no actor/profile/time.
    pub fn owner(
        identity: &ProfileIdentity,
        created_at_ms: u64,
        turn: &str,
        change: EventOperation,
    ) -> Result<Self, MemoryError> {
        bounded(turn, 512, "event turn")?;
        let body = EventBody {
            schema: 1,
            profile: identity.key()?,
            actor: identity.owner().to_owned(),
            created_at_ms,
            turn: turn.to_owned(),
            change,
        };
        let event = Self {
            id: Digest::of(&serde_json::to_vec(&body)?),
            body,
        };
        event.validate()?;
        Ok(event)
    }
    /// Canonical event identity.
    pub fn id(&self) -> &Digest {
        &self.id
    }
    /// Immutable action and native attribution.
    pub fn body(&self) -> &EventBody {
        &self.body
    }
    /// Record whose readable payload is held by this event, if any.
    pub fn payload_record(&self) -> Option<&RecordEnvelope> {
        match &self.body.change {
            EventOperation::Create { record } => Some(record),
            EventOperation::Supersede { replacement, .. } => Some(replacement),
            _ => None,
        }
    }
    /// Validate canonical digest and all embedded record/profile boundaries.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.body.schema != 1 {
            return Err(MemoryError::Invalid("event schema"));
        }
        bounded(&self.body.actor, 256, "actor")?;
        bounded(&self.body.turn, 512, "event turn")?;
        if let Some(record) = self.payload_record() {
            record.validate()?;
            if record.payload().profile != self.body.profile {
                return Err(MemoryError::ScopeMismatch);
            }
            if record.payload().author != self.body.actor {
                return Err(MemoryError::Unauthorized);
            }
        }
        if matches!(
            self.body.change,
            EventOperation::InvalidateSource {
                availability: SourceAvailability::Present,
                ..
            }
        ) {
            return Err(MemoryError::Invalid(
                "source restoration requires reverification",
            ));
        }
        let bytes = serde_json::to_vec(&self.body)?;
        if bytes.len() > 64_000 {
            return Err(MemoryError::Invalid("event bytes"));
        }
        if Digest::of(&bytes) != self.id {
            return Err(MemoryError::DigestMismatch);
        }
        Ok(())
    }
    /// Exact fixed-field event bytes used by the native log/spool.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, MemoryError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
    /// Verify incoming spool bytes before materializing them.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MemoryError> {
        if bytes.len() > 65_536 {
            return Err(MemoryError::Invalid("event bytes"));
        }
        let event: Self = serde_json::from_slice(bytes)?;
        event.validate()?;
        Ok(event)
    }
}

/// Receipt distinguishes an applied transaction from replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyReceipt {
    /// Current canonical position.
    pub cursor: LogCursor,
    /// False when an identical event was already durably applied.
    pub applied: bool,
}

/// Storage is deliberately separate from search-provider contracts.
pub trait RecordLog {
    /// Persistence-specific error that must propagate to the native owner flow.
    type Error: std::error::Error;
    /// Apply one canonical action atomically, checking replay and tombstones first.
    fn append(&self, event: &MemoryEvent) -> Result<ApplyReceipt, Self::Error>;
    /// Exact current log head.
    fn cursor(&self) -> Result<LogCursor, Self::Error>;
    /// Resolve a record; forgotten payloads must never be returned.
    fn get(&self, record_id: &RecordId) -> Result<Option<ProjectedRecord>, Self::Error>;
}

/// Bounded result of decoding one agent-owned append-only spool segment.
#[derive(Clone, Debug)]
pub struct SpoolBatch {
    /// Complete validated events in their source order.
    pub events: Vec<MemoryEvent>,
    /// A single incomplete final JSON line was ignored.
    pub ignored_torn_tail: bool,
}

/// Decode complete lines and ignore only an EOF-truncated final unterminated line.
pub fn decode_spool(bytes: &[u8]) -> Result<SpoolBatch, MemoryError> {
    if bytes.len() > 1_048_576 {
        return Err(MemoryError::Invalid("spool bytes"));
    }
    let mut result = SpoolBatch {
        events: vec![],
        ignored_torn_tail: false,
    };
    let lines: Vec<_> = bytes.split(|byte| *byte == b'\n').collect();
    for (position, line) in lines.iter().enumerate() {
        let final_line = position + 1 == lines.len();
        if final_line && line.is_empty() {
            break;
        }
        if line.is_empty() || line.len() > 65_536 {
            return Err(MemoryError::Invalid("spool line"));
        }
        match MemoryEvent::from_bytes(line) {
            Ok(event) => result.events.push(event),
            Err(MemoryError::Encoding(error))
                if final_line && !bytes.ends_with(b"\n") && error.is_eof() =>
            {
                result.ignored_torn_tail = true;
            }
            Err(error) => return Err(error),
        }
        if result.events.len() > 512 {
            return Err(MemoryError::Invalid("spool event count"));
        }
    }
    Ok(result)
}
