//! Per-place lexical provider with atomic cursor catch-up and query-only peers.

use crate::{
    migrations::{FINGERPRINT, INDEX_SCHEMA, SCHEMA_VERSION},
    open::open_database,
    provider_error, ProfilePaths, SqliteRecordLog, StoreError,
};
use kr8_capability::{
    Citation, Cursor, ExportManifest, IndexChange, IndexRecord, IndexUpdate, MemoryFeature,
    MemoryPassage, MemoryStore, ProviderError, Readiness, ScopedQuery, StoreAccess, StoreInfo,
    MEMORY_STORE_V1,
};
use kr8_object_memory::{Digest, LogCursor, ProfileIdentity, ProfileKey, ReadScope};
use rusqlite::Connection;
use std::{collections::BTreeSet, path::Path, sync::Mutex};

/// Derived lexical index, isolated by profile and exact-place corpus.
pub struct SqliteFtsMemoryStore {
    connection: Mutex<Connection>,
    profile: ProfileKey,
    access: StoreAccess,
    canonical_paths: ProfilePaths,
    canonical_identity: ProfileIdentity,
}

impl SqliteFtsMemoryStore {
    /// Native desktop indexer. Probes an actual temporary FTS5 table before use.
    pub fn open_writer(
        paths: &ProfilePaths,
        identity: &ProfileIdentity,
    ) -> Result<Self, StoreError> {
        Self::open(paths, identity, StoreAccess::ReadWriteHome)
    }
    /// ACP/peer reader. Never creates/migrates/probes by writing to the store.
    pub fn open_read_only(
        paths: &ProfilePaths,
        identity: &ProfileIdentity,
    ) -> Result<Self, StoreError> {
        Self::open(paths, identity, StoreAccess::ReadOnlyPeer)
    }
    fn open(
        paths: &ProfilePaths,
        identity: &ProfileIdentity,
        access: StoreAccess,
    ) -> Result<Self, StoreError> {
        if paths.profile != identity.key()? {
            return Err(StoreError::ProfileMismatch);
        }
        let connection = open_database(&paths.index, identity, "fts-index", access, INDEX_SCHEMA)?;
        if access == StoreAccess::ReadWriteHome {
            connection.execute_batch("CREATE VIRTUAL TABLE temp.kr8_fts5_probe USING fts5(text); INSERT INTO temp.kr8_fts5_probe VALUES('probe');")?;
            let matches: i64 = connection.query_row(
                "SELECT count(*) FROM temp.kr8_fts5_probe WHERE kr8_fts5_probe MATCH 'probe'",
                [],
                |row| row.get(0),
            )?;
            if matches != 1 {
                return Err(StoreError::Invariant("FTS5 readiness probe"));
            }
            connection.execute_batch("DROP TABLE temp.kr8_fts5_probe;")?;
        }
        let store = Self {
            connection: Mutex::new(connection),
            profile: identity.key()?,
            access,
            canonical_paths: paths.clone(),
            canonical_identity: identity.clone(),
        };
        store.readiness()?;
        Ok(store)
    }
    /// Catch up from canonical current projections; failures leave the old index cursor.
    pub fn catch_up(&self, log: &SqliteRecordLog) -> Result<Cursor, ProviderError> {
        if self.access != StoreAccess::ReadWriteHome {
            return Err(ProviderError::ReadOnly);
        }
        if self.profile != log.profile {
            return Err(ProviderError::ProfileMismatch);
        }
        let previous = self.cursor()?;
        let delta = log
            .delta(&object_cursor(&previous).map_err(provider_error)?)
            .map_err(provider_error)?;
        let mut changes = Vec::with_capacity(delta.records.len());
        for (id, projected) in delta.records {
            let change =
                if let Some(projected) = projected.filter(|record| record.status.searchable()) {
                    IndexChange::Upsert(canonical_index_record(&projected).map_err(provider_error)?)
                } else {
                    IndexChange::Remove(id.as_str().to_owned())
                };
            changes.push(change);
        }
        self.remember(&IndexUpdate {
            previous: capability_cursor(&delta.previous),
            next: capability_cursor(&delta.next),
            changes,
        })
    }
    /// Production native read seam: derive freshness from the canonical log, then recheck.
    pub fn search_current(
        &self,
        log: &SqliteRecordLog,
        scope: &ReadScope,
        text: &str,
        limit: usize,
        byte_budget: usize,
    ) -> Result<Vec<MemoryPassage>, ProviderError> {
        self.search_canonical(log, scope, text, limit, byte_budget, None)
    }

    fn search_canonical(
        &self,
        log: &SqliteRecordLog,
        scope: &ReadScope,
        text: &str,
        limit: usize,
        byte_budget: usize,
        expected_cursor: Option<&Cursor>,
    ) -> Result<Vec<MemoryPassage>, ProviderError> {
        if scope.profile() != &self.profile || log.profile != self.profile {
            return Err(ProviderError::ProfileMismatch);
        }
        let required = capability_cursor(&log.cursor().map_err(provider_error)?);
        if expected_cursor.is_some_and(|expected| expected != &required) {
            return Err(ProviderError::InvalidCursor);
        }
        let places = scope
            .places()
            .iter()
            .map(|place| {
                place
                    .canonical()
                    .map_err(|error| provider_error(error.into()))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let query = ScopedQuery::new(
            self.profile.as_str().to_owned(),
            places,
            text,
            limit,
            byte_budget,
            required.clone(),
        )?;
        let passages = {
            let mut connection = self
                .connection
                .lock()
                .map_err(|_| ProviderError::Backend("index lock poisoned".to_owned()))?;
            crate::index_query::search(&mut connection, &self.profile, &query, |record| {
                let id = kr8_object_memory::RecordId::parse(&record.id)?;
                let current = log.get_scoped(scope, &id)?.ok_or_else(|| {
                    ProviderError::Corrupt("canonical citation unavailable".to_owned())
                })?;
                let canonical = canonical_index_record(&current)?;
                if !current.status.searchable() || canonical != record {
                    return Err(ProviderError::Corrupt(
                        "derived passage differs from canonical evidence".to_owned(),
                    )
                    .into());
                }
                // The query budgets only canonical evidence after proving every
                // selected candidate's record/citation fields byte-for-byte.
                Ok(canonical)
            })
            .map_err(provider_error)?
        };
        let after = capability_cursor(&log.cursor().map_err(provider_error)?);
        if after != required {
            return Err(ProviderError::StaleCursor {
                indexed: required,
                required: after,
            });
        }
        Ok(passages)
    }
}

impl MemoryStore for SqliteFtsMemoryStore {
    fn describe(&self) -> StoreInfo {
        StoreInfo {
            family: MEMORY_STORE_V1.to_owned(),
            schema: SCHEMA_VERSION,
            provider: "kr8-sqlite-fts5".to_owned(),
            profile: self.profile.as_str().to_owned(),
            access: self.access,
            features: BTreeSet::from([
                MemoryFeature::ProfilePartitioned,
                MemoryFeature::ScopeBeforeRank,
                MemoryFeature::Provenance,
                MemoryFeature::DeterministicOrder,
                MemoryFeature::ReadOnlyPeer,
            ]),
        }
    }
    fn readiness(&self) -> Result<Readiness, ProviderError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProviderError::Backend("index lock poisoned".to_owned()))?;
        let fingerprint: String = connection
            .query_row(
                "SELECT value FROM metadata WHERE key='fingerprint'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| provider_error(error.into()))?;
        if fingerprint != FINGERPRINT {
            return Err(ProviderError::Corrupt("index fingerprint".to_owned()));
        }
        let _: String = connection
            .query_row("SELECT fts5_source_id()", [], |row| row.get(0))
            .map_err(|error| provider_error(error.into()))?;
        // Place corpora are touched only after a scoped query authorizes them.
        // Engine/schema readiness cannot probe forbidden places as a side effect.
        Ok(Readiness::Ready(
            index_cursor(&connection).map_err(provider_error)?,
        ))
    }
    fn remember(&self, update: &IndexUpdate) -> Result<Cursor, ProviderError> {
        if self.access != StoreAccess::ReadWriteHome {
            return Err(ProviderError::ReadOnly);
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ProviderError::Backend("index lock poisoned".to_owned()))?;
        crate::index_write::apply(&mut connection, &self.profile, update).map_err(provider_error)
    }
    fn cursor(&self) -> Result<Cursor, ProviderError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProviderError::Backend("index lock poisoned".to_owned()))?;
        index_cursor(&connection).map_err(provider_error)
    }
    fn search(&self, query: &ScopedQuery) -> Result<Vec<MemoryPassage>, ProviderError> {
        if query.profile() != self.profile.as_str() {
            return Err(ProviderError::ProfileMismatch);
        }
        let places = query
            .allowed_places()
            .iter()
            .map(|place| {
                kr8_object_memory::PlaceId::parse(place)
                    .map_err(|_| ProviderError::InvalidQuery("allowed place"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if places.iter().any(|place| place.profile() != &self.profile) {
            return Err(ProviderError::ProfileMismatch);
        }
        // ScopedQuery is created by a trusted native caller after authorization.
        // Reconstruct exactly that grant; do not add owner-private or siblings.
        let scope = ReadScope::from_authorized_places(self.profile.clone(), places)
            .map_err(|error| provider_error(error.into()))?;
        // The public provider path has the same canonical authority proof. It
        // never creates/migrates the source log or redirects to a writer.
        let log = SqliteRecordLog::open_read_only(&self.canonical_paths, &self.canonical_identity)
            .map_err(provider_error)?;
        self.search_canonical(
            &log,
            &scope,
            query.text(),
            query.limit(),
            query.byte_budget(),
            Some(query.required_cursor()),
        )
    }
    fn export(&self, to: &Path) -> Result<ExportManifest, ProviderError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProviderError::Backend("index lock poisoned".to_owned()))?;
        crate::export::export(&connection, &self.profile, to).map_err(provider_error)
    }
}

pub(crate) fn index_cursor(connection: &Connection) -> Result<Cursor, StoreError> {
    let (sequence, event_id): (i64, Option<String>) = connection.query_row(
        "SELECT sequence,event_id FROM index_state WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cursor = Cursor {
        sequence: crate::unsigned_integer(sequence)?,
        event_id,
    };
    cursor.validate()?;
    Ok(cursor)
}

fn capability_cursor(cursor: &LogCursor) -> Cursor {
    Cursor {
        sequence: cursor.sequence,
        event_id: cursor.event_id.as_ref().map(|id| id.as_str().to_owned()),
    }
}

fn object_cursor(cursor: &Cursor) -> Result<LogCursor, StoreError> {
    cursor.validate()?;
    Ok(LogCursor {
        sequence: cursor.sequence,
        event_id: cursor.event_id.as_deref().map(Digest::parse).transpose()?,
    })
}

fn canonical_index_record(
    projected: &kr8_object_memory::ProjectedRecord,
) -> Result<IndexRecord, StoreError> {
    projected.record.validate()?;
    let payload = projected.record.payload();
    Ok(IndexRecord {
        id: projected.record.id().as_str().to_owned(),
        record_hash: projected.record.hash().as_str().to_owned(),
        profile: payload.profile.as_str().to_owned(),
        place: payload.place.canonical()?,
        author: payload.author.clone(),
        created_at_ms: payload.created_at_ms,
        text: payload.text.clone(),
        sources: payload
            .sources
            .iter()
            .map(|source| Citation {
                source_id: source.id.as_str().to_owned(),
                sha256: source.sha256.as_str().to_owned(),
                availability: projected
                    .source_states
                    .get(&source.id)
                    .copied()
                    .unwrap_or(source.availability)
                    .label()
                    .to_owned(),
            })
            .collect(),
    })
}
