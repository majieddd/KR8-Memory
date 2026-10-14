use crate::{fts::index_cursor, StoreError};
use kr8_capability::{Citation, IndexChange, IndexRecord, IndexUpdate, ProviderError};
use kr8_object_memory::{Digest, PlaceId, ProfileKey, RecordId, SourceId};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

pub(crate) fn table_name(place: &str) -> String {
    format!("place_{}", Digest::of(place.as_bytes()))
}

pub(crate) fn apply(
    connection: &mut Connection,
    profile: &ProfileKey,
    update: &IndexUpdate,
) -> Result<kr8_capability::Cursor, StoreError> {
    update.previous.validate()?;
    update.next.validate()?;
    if update.next.sequence < update.previous.sequence || update.changes.len() > 10_000 {
        return Err(StoreError::InvalidCursor);
    }
    if update.previous == update.next && !update.changes.is_empty() {
        return Err(StoreError::InvalidCursor);
    }
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let current = index_cursor(&transaction)?;
    if current == update.next && update.previous != update.next {
        return Ok(current);
    }
    if current != update.previous {
        return Err(ProviderError::StaleCursor {
            indexed: current,
            required: update.previous.clone(),
        }
        .into());
    }
    let mut seen = BTreeSet::new();
    let mut previous_id: Option<&str> = None;
    for change in &update.changes {
        let id = match change {
            IndexChange::Upsert(record) => record.id.as_str(),
            IndexChange::Remove(id) => id.as_str(),
        };
        RecordId::parse(id)?;
        if !seen.insert(id) || previous_id.is_some_and(|previous| previous >= id) {
            return Err(StoreError::Invariant("index update order"));
        }
        previous_id = Some(id);
        if let IndexChange::Upsert(record) = change {
            validate(record, profile)?;
        }
        let old_place: Option<String> = transaction
            .query_row(
                "SELECT place FROM indexed_records WHERE record_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(place) = old_place {
            let table = verified_table(&transaction, &place)?
                .ok_or(StoreError::Invariant("indexed record place"))?;
            transaction.execute(&format!("DELETE FROM {table} WHERE record_id=?1"), [id])?;
            transaction.execute("DELETE FROM indexed_records WHERE record_id=?1", [id])?;
        }
        if let IndexChange::Upsert(record) = change {
            let table = match verified_table(&transaction, &record.place)? {
                Some(table) => table,
                None => {
                    let table = table_name(&record.place);
                    transaction.execute_batch(&format!("CREATE VIRTUAL TABLE {table} USING fts5(record_id UNINDEXED,text,tokenize='unicode61 remove_diacritics 2');"))?;
                    transaction.execute(
                        "INSERT INTO places(place,table_name) VALUES(?1,?2)",
                        params![record.place, table],
                    )?;
                    table
                }
            };
            transaction.execute(
                "INSERT INTO indexed_records(record_id,place,payload) VALUES(?1,?2,?3)",
                params![record.id, record.place, encode(record)?],
            )?;
            transaction.execute(
                &format!("INSERT INTO {table}(record_id,text) VALUES(?1,?2)"),
                params![record.id, record.text],
            )?;
        }
    }
    transaction.execute(
        "UPDATE index_state SET sequence=?1,event_id=?2 WHERE singleton=1",
        params![
            crate::sql_integer(update.next.sequence)?,
            update.next.event_id
        ],
    )?;
    transaction.commit()?;
    Ok(update.next.clone())
}

pub(crate) fn verified_table(
    connection: &Connection,
    place: &str,
) -> Result<Option<String>, StoreError> {
    let table: Option<String> = connection
        .query_row(
            "SELECT table_name FROM places WHERE place=?1",
            [place],
            |row| row.get(0),
        )
        .optional()?;
    if table
        .as_ref()
        .is_some_and(|table| table != &table_name(place))
    {
        return Err(StoreError::Invariant("FTS table identity"));
    }
    Ok(table)
}

pub(crate) fn validate(record: &IndexRecord, profile: &ProfileKey) -> Result<(), StoreError> {
    if record.profile != profile.as_str() {
        return Err(StoreError::ProfileMismatch);
    }
    let place = PlaceId::parse(&record.place)?;
    if place.profile() != profile {
        return Err(StoreError::ProfileMismatch);
    }
    RecordId::parse(&record.id)?;
    Digest::parse(&record.record_hash)?;
    if record.id != format!("mem:{}", record.record_hash) {
        return Err(StoreError::Invariant("index record hash"));
    }
    if record.text.trim().is_empty()
        || record.text.len() > 400
        || record.text.contains('\0')
        || record.author.is_empty()
        || record.author.len() > 256
        || record.sources.len() > 32
    {
        return Err(StoreError::Invariant("index record bounds"));
    }
    let mut sources = BTreeSet::new();
    for citation in &record.sources {
        SourceId::parse(&citation.source_id)?;
        Digest::parse(&citation.sha256)?;
        if citation.availability != "present" || !sources.insert(&citation.source_id) {
            return Err(StoreError::Invariant("active index provenance"));
        }
    }
    Ok(())
}

type StoredRecord = (
    String,
    String,
    String,
    String,
    String,
    u64,
    String,
    Vec<(String, String, String)>,
);

pub(crate) fn encode(record: &IndexRecord) -> Result<Vec<u8>, StoreError> {
    let sources: Vec<_> = record
        .sources
        .iter()
        .map(|source| (&source.source_id, &source.sha256, &source.availability))
        .collect();
    Ok(serde_json::to_vec(&(
        &record.id,
        &record.record_hash,
        &record.profile,
        &record.place,
        &record.author,
        record.created_at_ms,
        &record.text,
        sources,
    ))?)
}

pub(crate) fn decode(bytes: &[u8], profile: &ProfileKey) -> Result<IndexRecord, StoreError> {
    if bytes.len() > 65_536 {
        return Err(StoreError::Invariant("index payload bytes"));
    }
    let (id, record_hash, profile_id, place, author, created_at_ms, text, sources): StoredRecord =
        serde_json::from_slice(bytes)?;
    let record = IndexRecord {
        id,
        record_hash,
        profile: profile_id,
        place,
        author,
        created_at_ms,
        text,
        sources: sources
            .into_iter()
            .map(|(source_id, sha256, availability)| Citation {
                source_id,
                sha256,
                availability,
            })
            .collect(),
    };
    validate(&record, profile)?;
    Ok(record)
}
