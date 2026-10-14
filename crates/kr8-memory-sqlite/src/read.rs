use crate::StoreError;
use kr8_object_memory::{
    Digest, LifecycleStatus, LogCursor, PlaceId, ProfileKey, ProjectedRecord, RecordEnvelope,
    RecordId, SourceAvailability, SourceId,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

pub(crate) fn cursor(connection: &Connection) -> Result<LogCursor, StoreError> {
    let head: Option<(i64, String)> = connection
        .query_row(
            "SELECT sequence,event_id FROM events ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match head {
        Some((sequence, id)) => Ok(LogCursor {
            sequence: crate::unsigned_integer(sequence)?,
            event_id: Some(Digest::parse(&id)?),
        }),
        None => Ok(LogCursor::default()),
    }
}

pub(crate) fn verify_cursor(connection: &Connection, cursor: &LogCursor) -> Result<(), StoreError> {
    match (&cursor.event_id, cursor.sequence) {
        (None, 0) => Ok(()),
        (Some(id), sequence) if sequence > 0 => {
            let persisted: Option<String> = connection
                .query_row(
                    "SELECT event_id FROM events WHERE sequence=?1",
                    [crate::sql_integer(sequence)?],
                    |row| row.get(0),
                )
                .optional()?;
            if persisted.as_deref() == Some(id.as_str()) {
                Ok(())
            } else {
                Err(StoreError::InvalidCursor)
            }
        }
        _ => Err(StoreError::InvalidCursor),
    }
}

pub(crate) fn status(value: &str) -> Result<LifecycleStatus, StoreError> {
    match value {
        "active" => Ok(LifecycleStatus::Active),
        "superseded" => Ok(LifecycleStatus::Superseded),
        "retracted" => Ok(LifecycleStatus::Retracted),
        "missing" => Ok(LifecycleStatus::Missing),
        "stale" => Ok(LifecycleStatus::Stale),
        _ => Err(StoreError::Invariant("record status")),
    }
}

pub(crate) fn source_state(value: &str) -> Result<SourceAvailability, StoreError> {
    match value {
        "present" => Ok(SourceAvailability::Present),
        "missing" => Ok(SourceAvailability::Missing),
        "changed" => Ok(SourceAvailability::Changed),
        "deleted" => Ok(SourceAvailability::Deleted),
        _ => Err(StoreError::Invariant("source availability")),
    }
}

pub(crate) fn get(
    connection: &Connection,
    profile: &ProfileKey,
    id: &RecordId,
) -> Result<Option<ProjectedRecord>, StoreError> {
    let row: Option<(Vec<u8>, String, Option<String>, String)> = connection
        .query_row(
            "SELECT payload,status,superseded_by,place FROM records WHERE record_id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((payload, state, superseded_by, place)) = row else {
        return Ok(None);
    };
    let record = RecordEnvelope::from_bytes(&payload)?;
    if record.id() != id
        || &record.payload().profile != profile
        || record.payload().place != PlaceId::parse(&place)?
    {
        return Err(StoreError::ProfileMismatch);
    }
    let mut source_states = BTreeMap::new();
    for source in &record.payload().sources {
        let override_state: Option<String> = connection
            .query_row(
                "SELECT availability FROM source_state WHERE source_id=?1",
                [source.id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(value) = override_state {
            source_states.insert(
                SourceId::parse(source.id.as_str())?,
                source_state(&value)?.max(source.availability),
            );
        }
    }
    Ok(Some(ProjectedRecord {
        record,
        status: status(&state)?,
        superseded_by: superseded_by.map(|id| RecordId::parse(&id)).transpose()?,
        source_states,
    }))
}
