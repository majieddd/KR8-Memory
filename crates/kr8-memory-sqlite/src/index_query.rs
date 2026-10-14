use crate::{fts::index_cursor, index_write, StoreError};
use kr8_capability::{IndexRecord, MemoryPassage, ProviderError, ScopedQuery};
use kr8_object_memory::{PlaceId, ProfileKey};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

pub(crate) fn search(
    connection: &mut Connection,
    profile: &ProfileKey,
    query: &ScopedQuery,
    mut canonicalize: impl FnMut(IndexRecord) -> Result<IndexRecord, StoreError>,
) -> Result<Vec<MemoryPassage>, StoreError> {
    if query.profile() != profile.as_str() {
        return Err(StoreError::ProfileMismatch);
    }
    for place in query.allowed_places() {
        if PlaceId::parse(place)?.profile() != profile {
            return Err(StoreError::ProfileMismatch);
        }
    }
    let terms: Vec<_> = query.text().split_whitespace().collect();
    if terms.len() > 32 {
        return Err(ProviderError::InvalidQuery("term count").into());
    }
    let literal = terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    let transaction = connection.transaction()?;
    let indexed = index_cursor(&transaction)?;
    if &indexed != query.required_cursor() {
        return Err(ProviderError::StaleCursor {
            indexed,
            required: query.required_cursor().clone(),
        }
        .into());
    }
    let mut ranked = Vec::new();
    let mut seen = BTreeSet::new();
    for place in query.allowed_places() {
        let Some(table) = index_write::verified_table(&transaction, place)? else {
            continue;
        };
        let hits = {
            let sql = format!("SELECT record_id,text,bm25({table}) FROM {table} WHERE {table} MATCH ?1 ORDER BY bm25({table}),record_id LIMIT ?2");
            let mut statement = transaction.prepare(&sql)?;
            let rows = statement
                .query_map(
                    params![
                        literal,
                        i64::try_from(query.limit())
                            .map_err(|_| StoreError::TooLarge("query limit"))?
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, f64>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (position, (id, text, score)) in hits.into_iter().enumerate() {
            if !score.is_finite() || !seen.insert(id.clone()) {
                return Err(StoreError::Invariant("FTS result identity or score"));
            }
            let payload: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT payload FROM indexed_records WHERE record_id=?1 AND place=?2",
                    params![id, place],
                    |row| row.get(0),
                )
                .optional()?;
            let payload = payload.ok_or(StoreError::Invariant("orphan FTS row"))?;
            let record = index_write::decode(&payload, profile)?;
            if record.id != id || &record.place != place || record.text != text {
                return Err(StoreError::Invariant("FTS scope projection"));
            }
            // Every selected candidate must be checked before any metadata can
            // affect merge order, result limits, or budget elimination. Native
            // evidence queries supply the scoped canonical reader here.
            let canonical = canonicalize(record)?;
            ranked.push(MemoryPassage {
                record: canonical,
                ordinal: position + 1,
                local_score: score,
            });
        }
    }
    ranked.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.record.place.cmp(&right.record.place))
            .then_with(|| left.record.id.cmp(&right.record.id))
    });
    let mut used = 0usize;
    let mut result = Vec::new();
    for passage in ranked {
        let size = passage.record.text.len()
            + passage.record.id.len()
            + passage.record.place.len()
            + passage.record.author.len()
            + passage.record.record_hash.len()
            + passage.record.profile.len()
            + passage
                .record
                .sources
                .iter()
                .map(|source| {
                    source.source_id.len() + source.sha256.len() + source.availability.len()
                })
                .sum::<usize>();
        if used.saturating_add(size) <= query.byte_budget() {
            used += size;
            result.push(passage);
            if result.len() == query.limit() {
                break;
            }
        }
    }
    transaction.commit()?;
    Ok(result)
}
