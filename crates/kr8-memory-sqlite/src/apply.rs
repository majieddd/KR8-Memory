use crate::{read, StoreError};
use kr8_object_memory::{
    AuthorRole, EventOperation, LifecycleStatus, MemoryEvent, ProfileKey, RecordEnvelope, RecordId,
    RecordType, SourceAvailability,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{BTreeSet, VecDeque};

pub(crate) fn apply(
    transaction: &Transaction<'_>,
    profile: &ProfileKey,
    event: &MemoryEvent,
) -> Result<(), StoreError> {
    let (operation, record_id) = match &event.body().change {
        EventOperation::Create { record } => ("create", Some(record.id().as_str())),
        EventOperation::Supersede { replacement, .. } => {
            ("supersede", Some(replacement.id().as_str()))
        }
        EventOperation::Retract { record_id } => ("retract", Some(record_id.as_str())),
        EventOperation::InvalidateSource { .. } => ("invalidate-source", None),
        EventOperation::Forget { record_id } => ("forget", Some(record_id.as_str())),
    };
    let payload = if operation == "forget" {
        None
    } else {
        Some(event.canonical_bytes()?)
    };
    transaction.execute(
        "INSERT INTO events(event_id,operation,record_id,payload) VALUES(?1,?2,?3,?4)",
        params![event.id().as_str(), operation, record_id, payload],
    )?;
    let sequence = u64::try_from(transaction.last_insert_rowid())
        .map_err(|_| StoreError::Invariant("event sequence"))?;
    match &event.body().change {
        EventOperation::Create { record } => create(transaction, profile, sequence, record)?,
        EventOperation::Supersede {
            original,
            replacement,
        } => {
            let previous =
                read::get(transaction, profile, original)?.ok_or(StoreError::NotFound)?;
            if previous.status != LifecycleStatus::Active {
                return Err(StoreError::NotActive);
            }
            if previous.record.payload().place != replacement.payload().place
                || replacement.payload().derived_from.contains(original)
            {
                return Err(StoreError::InvalidDependency);
            }
            create(transaction, profile, sequence, replacement)?;
            transaction.execute(
                "UPDATE records SET status='superseded',superseded_by=?1 WHERE record_id=?2",
                params![replacement.id().as_str(), original.as_str()],
            )?;
            change(transaction, sequence, original)?;
            invalidate_dependents(transaction, sequence, original)?;
        }
        EventOperation::Retract { record_id } => {
            let record = read::get(transaction, profile, record_id)?.ok_or(StoreError::NotFound)?;
            if !matches!(
                record.status,
                LifecycleStatus::Active | LifecycleStatus::Missing | LifecycleStatus::Stale
            ) {
                return Err(StoreError::NotActive);
            }
            transaction.execute(
                "UPDATE records SET status='retracted' WHERE record_id=?1",
                [record_id.as_str()],
            )?;
            change(transaction, sequence, record_id)?;
            invalidate_dependents(transaction, sequence, record_id)?;
        }
        EventOperation::InvalidateSource {
            source_id,
            availability,
        } => {
            let prior: Option<String> = transaction
                .query_row(
                    "SELECT availability FROM source_state WHERE source_id=?1",
                    [source_id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let next = if let Some(prior) = prior {
                read::source_state(&prior)?.max(*availability)
            } else {
                *availability
            };
            transaction.execute("INSERT INTO source_state(source_id,availability) VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET availability=excluded.availability", params![source_id.as_str(), next.label()])?;
            let records = {
                let mut statement = transaction.prepare("SELECT record_id FROM sources WHERE source_id=?1 ORDER BY record_id LIMIT 10001")?;
                let rows = statement
                    .query_map([source_id.as_str()], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            };
            if records.len() > 10_000 {
                return Err(StoreError::TooLarge("source dependents"));
            }
            for id in records {
                let id = RecordId::parse(&id)?;
                let record = read::get(transaction, profile, &id)?
                    .ok_or(StoreError::Invariant("source record"))?;
                if matches!(
                    record.status,
                    LifecycleStatus::Active | LifecycleStatus::Missing | LifecycleStatus::Stale
                ) {
                    transaction.execute(
                        "UPDATE records SET status=?1 WHERE record_id=?2",
                        params![
                            source_status(transaction, &record.record)?.label(),
                            id.as_str()
                        ],
                    )?;
                }
                change(transaction, sequence, &id)?;
                invalidate_dependents(transaction, sequence, &id)?;
            }
        }
        EventOperation::Forget { record_id } => {
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE record_id=?1) OR EXISTS(SELECT 1 FROM tombstones WHERE record_id=?1)", [record_id.as_str()], |row| row.get(0))?;
            if !exists {
                return Err(StoreError::NotFound);
            }
            let mut erased = descendants(transaction, record_id)?;
            erased.insert(record_id.clone());
            for id in erased {
                transaction.execute(
                    "INSERT OR IGNORE INTO tombstones(record_id,event_id) VALUES(?1,?2)",
                    params![id.as_str(), event.id().as_str()],
                )?;
                transaction.execute(
                    "UPDATE events SET payload=NULL WHERE record_id=?1",
                    [id.as_str()],
                )?;
                transaction.execute("DELETE FROM records WHERE record_id=?1", [id.as_str()])?;
                change(transaction, sequence, &id)?;
            }
        }
    }
    Ok(())
}

fn create(
    connection: &Connection,
    profile: &ProfileKey,
    sequence: u64,
    record: &RecordEnvelope,
) -> Result<(), StoreError> {
    record.validate()?;
    if &record.payload().profile != profile || record.payload().author_role != AuthorRole::Person {
        return Err(StoreError::Unauthorized);
    }
    let forgotten: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM tombstones WHERE record_id=?1)",
        [record.id().as_str()],
        |row| row.get(0),
    )?;
    if forgotten {
        return Err(StoreError::Forgotten);
    }
    for parent in &record.payload().derived_from {
        let parent =
            read::get(connection, profile, parent)?.ok_or(StoreError::InvalidDependency)?;
        if parent.status != LifecycleStatus::Active
            || parent.record.payload().place != record.payload().place
        {
            return Err(StoreError::InvalidDependency);
        }
    }
    for source in &record.payload().sources {
        if source.id.as_str().starts_with("mem:")
            && !record
                .payload()
                .derived_from
                .iter()
                .any(|id| id.as_str() == source.id.as_str())
        {
            return Err(StoreError::InvalidDependency);
        }
    }
    connection.execute(
        "INSERT INTO records(record_id,place,payload,status) VALUES(?1,?2,?3,?4)",
        params![
            record.id().as_str(),
            record.payload().place.canonical()?,
            record.canonical_bytes()?,
            source_status(connection, record)?.label()
        ],
    )?;
    for source in &record.payload().sources {
        connection.execute(
            "INSERT INTO sources(record_id,source_id) VALUES(?1,?2)",
            params![record.id().as_str(), source.id.as_str()],
        )?;
    }
    for parent in &record.payload().derived_from {
        connection.execute(
            "INSERT INTO derivations(record_id,parent_id) VALUES(?1,?2)",
            params![record.id().as_str(), parent.as_str()],
        )?;
    }
    change(connection, sequence, record.id())
}

fn source_status(
    connection: &Connection,
    record: &RecordEnvelope,
) -> Result<LifecycleStatus, StoreError> {
    let mut unavailable = false;
    for source in &record.payload().sources {
        let current: Option<String> = connection
            .query_row(
                "SELECT availability FROM source_state WHERE source_id=?1",
                [source.id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let state = current
            .as_deref()
            .map(read::source_state)
            .transpose()?
            .unwrap_or(source.availability)
            .max(source.availability);
        if matches!(
            state,
            SourceAvailability::Changed | SourceAvailability::Deleted
        ) {
            return Ok(LifecycleStatus::Stale);
        }
        unavailable |= state == SourceAvailability::Missing;
    }
    if unavailable {
        Ok(if record.payload().record_type == RecordType::Pointer {
            LifecycleStatus::Missing
        } else {
            LifecycleStatus::Stale
        })
    } else {
        Ok(LifecycleStatus::Active)
    }
}

fn change(connection: &Connection, sequence: u64, id: &RecordId) -> Result<(), StoreError> {
    connection.execute(
        "INSERT OR IGNORE INTO changes(sequence,record_id) VALUES(?1,?2)",
        params![crate::sql_integer(sequence)?, id.as_str()],
    )?;
    Ok(())
}

fn descendants(connection: &Connection, root: &RecordId) -> Result<BTreeSet<RecordId>, StoreError> {
    let mut found = BTreeSet::new();
    let mut queue = VecDeque::from([root.clone()]);
    while let Some(parent) = queue.pop_front() {
        let children = {
            let mut statement = connection.prepare("SELECT record_id FROM derivations WHERE parent_id=?1 ORDER BY record_id LIMIT 10001")?;
            let rows = statement
                .query_map([parent.as_str()], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for child in children {
            let child = RecordId::parse(&child)?;
            if found.insert(child.clone()) {
                queue.push_back(child);
            }
            if found.len() > 10_000 {
                return Err(StoreError::TooLarge("record dependency graph"));
            }
        }
    }
    Ok(found)
}

fn invalidate_dependents(
    connection: &Connection,
    sequence: u64,
    root: &RecordId,
) -> Result<(), StoreError> {
    for id in descendants(connection, root)? {
        connection.execute("UPDATE records SET status='stale' WHERE record_id=?1 AND status IN ('active','missing')", [id.as_str()])?;
        change(connection, sequence, &id)?;
    }
    Ok(())
}
