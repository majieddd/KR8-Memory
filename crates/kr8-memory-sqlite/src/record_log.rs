//! Sole native writer API. Exposed owner reads must use scoped methods.

use crate::{migrations::LOG_SCHEMA, open::open_database, read, ProfilePaths, StoreError};
use kr8_capability::StoreAccess;
use kr8_object_memory::{
    ApplyReceipt, LogCursor, MemoryEvent, ProfileIdentity, ProfileKey, ProjectedRecord, ReadScope,
    RecordId, RecordLog,
};
use rusqlite::{params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Mutex;

/// Canonical profile-bound record log; separate from every retrieval provider.
pub struct SqliteRecordLog {
    pub(crate) connection: Mutex<Connection>,
    pub(crate) profile: ProfileKey,
    pub(crate) owner: String,
    pub(crate) access: StoreAccess,
}

/// Current projections for records changed since an exact index cursor.
pub(crate) struct IndexDelta {
    pub(crate) previous: LogCursor,
    pub(crate) next: LogCursor,
    pub(crate) records: Vec<(RecordId, Option<ProjectedRecord>)>,
}

impl SqliteRecordLog {
    /// Native desktop only. Creates/migrates this exact authenticated profile store.
    pub fn open_writer(
        paths: &ProfilePaths,
        identity: &ProfileIdentity,
    ) -> Result<Self, StoreError> {
        Self::open(paths, identity, StoreAccess::ReadWriteHome)
    }
    /// ACP/peer reader: never creates or migrates; enforces SQLite query_only.
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
        Ok(Self {
            connection: Mutex::new(open_database(
                &paths.rig,
                identity,
                "record-log",
                access,
                LOG_SCHEMA,
            )?),
            profile: identity.key()?,
            owner: identity.owner().to_owned(),
            access,
        })
    }
    /// Apply a single owner action atomically; identical events are harmless replay.
    pub fn append(&self, event: &MemoryEvent) -> Result<ApplyReceipt, StoreError> {
        if self.access != StoreAccess::ReadWriteHome {
            return Err(StoreError::ReadOnly);
        }
        event.validate()?;
        if event.body().profile != self.profile || event.body().actor != self.owner {
            return Err(StoreError::Unauthorized);
        }
        let mut connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE event_id=?1)",
            [event.id().as_str()],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(ApplyReceipt {
                cursor: read::cursor(&transaction)?,
                applied: false,
            });
        }
        crate::apply::apply(&transaction, &self.profile, event)?;
        let cursor = read::cursor(&transaction)?;
        transaction.commit()?;
        Ok(ApplyReceipt {
            cursor,
            applied: true,
        })
    }
    /// Exact durable canonical position; compare with the index before retrieval.
    pub fn cursor(&self) -> Result<LogCursor, StoreError> {
        let connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        read::cursor(&connection)
    }
    /// Internal canonical resolution. Native/UI callers must prefer get_scoped.
    pub fn get(&self, record_id: &RecordId) -> Result<Option<ProjectedRecord>, StoreError> {
        let connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        read::get(&connection, &self.profile, record_id)
    }
    /// Resolve only if the current native membership grant includes the record's place.
    pub fn get_scoped(
        &self,
        scope: &ReadScope,
        record_id: &RecordId,
    ) -> Result<Option<ProjectedRecord>, StoreError> {
        if scope.profile() != &self.profile {
            return Err(StoreError::Unauthorized);
        }
        let connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        let place: Option<String> = connection
            .query_row(
                "SELECT place FROM records WHERE record_id=?1",
                [record_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(place) = place {
            if scope.permits(&kr8_object_memory::PlaceId::parse(&place)?) {
                return read::get(&connection, &self.profile, record_id);
            }
        }
        Ok(None)
    }
    /// List current and historical canonical projections with scope applied before limit.
    pub fn list(
        &self,
        scope: &ReadScope,
        limit: usize,
    ) -> Result<Vec<ProjectedRecord>, StoreError> {
        if scope.profile() != &self.profile {
            return Err(StoreError::Unauthorized);
        }
        if !(1..=256).contains(&limit) {
            return Err(StoreError::TooLarge("list limit"));
        }
        let connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        let mut parameters = scope
            .places()
            .iter()
            .map(|place| place.canonical())
            .collect::<Result<Vec<_>, _>>()?;
        let placeholders = vec!["?"; parameters.len()].join(",");
        parameters.push(limit.to_string());
        let sql = format!("SELECT record_id FROM records WHERE place IN ({placeholders}) ORDER BY record_id LIMIT ?");
        let mut statement = connection.prepare(&sql)?;
        let ids = statement
            .query_map(params_from_iter(parameters), |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let record = read::get(&connection, &self.profile, &RecordId::parse(&id)?)?
                .ok_or(StoreError::Invariant("listed record disappeared"))?;
            records.push(record);
        }
        Ok(records)
    }
    /// Minimal tombstone check, with no readable forgotten payload.
    pub fn is_forgotten(&self, record_id: &RecordId) -> Result<bool, StoreError> {
        Ok(self
            .connection
            .lock()
            .map_err(|_| StoreError::Poisoned)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tombstones WHERE record_id=?1)",
                [record_id.as_str()],
                |row| row.get(0),
            )?)
    }
    pub(crate) fn delta(&self, previous: &LogCursor) -> Result<IndexDelta, StoreError> {
        let mut connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        let transaction = connection.transaction()?;
        read::verify_cursor(&transaction, previous)?;
        let next = read::cursor(&transaction)?;
        let ids = {
            let mut statement = transaction.prepare("SELECT DISTINCT record_id FROM changes WHERE sequence>?1 ORDER BY record_id LIMIT 10001")?;
            let rows = statement
                .query_map([crate::sql_integer(previous.sequence)?], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if ids.len() > 10_000 {
            return Err(StoreError::TooLarge("index catch-up records"));
        }
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let id = RecordId::parse(&id)?;
            records.push((id.clone(), read::get(&transaction, &self.profile, &id)?));
        }
        transaction.commit()?;
        Ok(IndexDelta {
            previous: previous.clone(),
            next,
            records,
        })
    }
}

impl RecordLog for SqliteRecordLog {
    type Error = StoreError;
    fn append(&self, event: &MemoryEvent) -> Result<ApplyReceipt, Self::Error> {
        Self::append(self, event)
    }
    fn cursor(&self) -> Result<LogCursor, Self::Error> {
        Self::cursor(self)
    }
    fn get(&self, record_id: &RecordId) -> Result<Option<ProjectedRecord>, Self::Error> {
        Self::get(self, record_id)
    }
}
