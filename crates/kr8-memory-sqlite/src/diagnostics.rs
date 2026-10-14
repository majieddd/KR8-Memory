//! Payload-free owner diagnostics from one scope-bound canonical read snapshot.

use crate::{migrations::SCHEMA_VERSION, read, SqliteRecordLog, StoreError};
use kr8_object_memory::{
    LifecycleStatus, LogCursor, ProfileKey, ReadScope, RecordId, SourceAvailability,
};
use rusqlite::{params_from_iter, Transaction};

const MAX_DIAGNOSTIC_RECORDS: u64 = 10_000;

/// Exact metadata/counts for one native-authorized scope and canonical snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordLogDiagnostics {
    /// Actual supported SQLite user_version, checked inside this read transaction.
    pub schema: u32,
    /// Canonical head of the same snapshot used to validate/count every visible row.
    pub cursor: LogCursor,
    /// Readable canonical projections, including explicit history and missing/stale sources.
    /// Forgotten payloads are absent; this is never a tombstone or capped-list count.
    pub readable_records: u64,
    /// Active projections whose exact current source versions are all present.
    /// This count does not attest the health of any derived FTS corpus.
    pub searchable_records: u64,
}

impl SqliteRecordLog {
    /// Read validated counts without creating/migrating a store or returning payloads.
    /// Scope is applied before reading record bytes. Counts are exact or an error;
    /// work above 10,000 visible projections is unavailable rather than approximate.
    pub fn diagnostics(&self, scope: &ReadScope) -> Result<RecordLogDiagnostics, StoreError> {
        if scope.profile() != &self.profile {
            return Err(StoreError::Unauthorized);
        }
        let mut connection = self.connection.lock().map_err(|_| StoreError::Poisoned)?;
        let transaction = connection.transaction()?;
        let diagnostics = read_snapshot(&transaction, &self.profile, scope)?;
        transaction.commit()?;
        Ok(diagnostics)
    }
}

fn read_snapshot(
    transaction: &Transaction<'_>,
    profile: &ProfileKey,
    scope: &ReadScope,
) -> Result<RecordLogDiagnostics, StoreError> {
    let schema: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema != SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchema(schema));
    }
    let cursor = read::cursor(transaction)?;
    read::verify_cursor(transaction, &cursor)?;
    let parameters = scope
        .places()
        .iter()
        .map(|place| place.canonical())
        .collect::<Result<Vec<_>, _>>()?;
    let placeholders = vec!["?"; parameters.len()].join(",");
    let sql = format!("SELECT record_id FROM records WHERE place IN ({placeholders}) ORDER BY record_id LIMIT 10001");
    let mut statement = transaction.prepare(&sql)?;
    let ids = statement.query_map(params_from_iter(parameters), |row| row.get::<_, String>(0))?;
    let mut readable_records = 0;
    let mut searchable_records = 0;
    for id in ids {
        if readable_records >= MAX_DIAGNOSTIC_RECORDS {
            return Err(StoreError::TooLarge("diagnostic records"));
        }
        let id = RecordId::parse(&id?)?;
        // Reuse the canonical decoder/profile/place/source-state checks. A raw
        // COUNT(*) would silently count corrupt or unreadable projections.
        let projected = read::get(transaction, profile, &id)?
            .ok_or(StoreError::Invariant("diagnostic record disappeared"))?;
        if !scope.permits(&projected.record.payload().place) {
            return Err(StoreError::Unauthorized);
        }
        readable_records += 1;
        if projected.status == LifecycleStatus::Active {
            let source_valid = projected.record.payload().sources.iter().all(|source| {
                projected
                    .source_states
                    .get(&source.id)
                    .copied()
                    .unwrap_or(source.availability)
                    == SourceAvailability::Present
            });
            if !source_valid {
                return Err(StoreError::Invariant(
                    "active diagnostic record has unavailable source",
                ));
            }
            searchable_records += 1;
        }
    }
    Ok(RecordLogDiagnostics {
        schema,
        cursor,
        readable_records,
        searchable_records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProfilePaths;
    use kr8_object_memory::{
        CreationContext, EventOperation, MemoryEvent, PlaceId, ProfileIdentity, RecordDraft,
        RecordEnvelope,
    };
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn counts_and_cursor_share_the_real_read_transaction_during_a_concurrent_append(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let identity = ProfileIdentity::new("owner:fixture", "wss://fixture.invalid", "fixture")?;
        let paths = ProfilePaths::for_app_data(directory.path(), &identity)?;
        let writer = Arc::new(SqliteRecordLog::open_writer(&paths, &identity)?);
        let scope = ReadScope::owner(identity.key()?, [])?;
        let peer = SqliteRecordLog::open_read_only(&paths, &identity)?;
        let record = RecordEnvelope::create(
            &CreationContext::owner(
                &identity,
                PlaceId::owner_private(identity.key()?),
                42,
                "turn:later",
            )?,
            RecordDraft::note("object:fixture", "later canonical note"),
        )?;
        let event = MemoryEvent::owner(
            &identity,
            42,
            "turn:later",
            EventOperation::Create { record },
        )?;
        let fired = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None::<String>));
        let timing_fired = fired.clone();
        let timing_failure = failure.clone();
        // Test-only timing on the real peer connection: append after its
        // canonical cursor SELECT, just before its scoped records SELECT.
        // Read-only flags and query_only remain set. Production code has no hook.
        peer.connection
            .lock()
            .map_err(|_| StoreError::Poisoned)?
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "records",
                        ..
                    }
                ) && !timing_fired.swap(true, Ordering::SeqCst)
                {
                    if let Err(error) = writer.append(&event) {
                        *timing_failure.lock().unwrap() = Some(error.to_string());
                        return Authorization::Deny;
                    }
                }
                match context.action {
                    AuthAction::Select
                    | AuthAction::Read { .. }
                    | AuthAction::Transaction { .. }
                    | AuthAction::Pragma {
                        pragma_value: None, ..
                    } => Authorization::Allow,
                    _ => Authorization::Deny,
                }
            }))?;
        let observed = peer.diagnostics(&scope)?;
        assert!(
            fired.load(Ordering::SeqCst),
            "the concurrent writer did not run"
        );
        assert!(failure.lock().unwrap().is_none());
        assert_eq!(observed.cursor, LogCursor::default());
        assert_eq!(observed.readable_records, 0);
        let current = peer.diagnostics(&scope)?;
        assert_eq!(current.readable_records, 1);
        assert_ne!(current.cursor, observed.cursor);
        Ok(())
    }
}
