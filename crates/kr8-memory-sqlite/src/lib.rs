//! Profile-bound canonical SQLite log and separate rebuildable per-place FTS5 index.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod apply;
mod diagnostics;
mod export;
pub mod fts;
mod index_query;
mod index_write;
mod migrations;
pub mod open;
mod read;
pub mod record_log;

pub use diagnostics::RecordLogDiagnostics;
pub use fts::SqliteFtsMemoryStore;
pub use open::ProfilePaths;
pub use record_log::SqliteRecordLog;

/// Native storage failure. Failed canonical/index work is never reported as empty success.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite failure; the caller must retain a durable retry path.
    #[error("memory SQLite failure: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Invalid canonical record, event, or scope.
    #[error(transparent)]
    Memory(#[from] kr8_object_memory::MemoryError),
    /// Invalid persisted JSON.
    #[error("memory JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    /// Trusted profile path could not be read or written.
    #[error("memory filesystem failure: {0}")]
    Io(#[from] std::io::Error),
    /// Database metadata disagrees with the authenticated owner/relay/community.
    #[error("memory database profile mismatch")]
    ProfileMismatch,
    /// Unsupported schema is never rewritten by this adapter.
    #[error("unsupported memory schema {0}")]
    UnsupportedSchema(u32),
    /// A query-only opener cannot mutate either database.
    #[error("memory database is read-only")]
    ReadOnly,
    /// The exact requested record does not exist.
    #[error("memory record was not found")]
    NotFound,
    /// Lifecycle cannot return inactive evidence to active state implicitly.
    #[error("memory record is not active")]
    NotActive,
    /// A content-addressed record was explicitly forgotten.
    #[error("forgotten memory cannot be resurrected")]
    Forgotten,
    /// Owner or exact-place authority did not match.
    #[error("memory operation is unauthorized")]
    Unauthorized,
    /// Cursor does not name the expected canonical log.
    #[error("invalid memory log cursor")]
    InvalidCursor,
    /// Bounded work exceeded a declared limit and must be retried explicitly.
    #[error("memory work exceeds bound: {0}")]
    TooLarge(&'static str),
    /// A prior panic poisoned a shared connection; do not continue uncertain state.
    #[error("memory connection lock is poisoned")]
    Poisoned,
    /// A canonical dependency is absent, inactive, or in another exact place.
    #[error("memory source dependency is unavailable or out of scope")]
    InvalidDependency,
    /// Persisted state violates a checked invariant.
    #[error("invalid memory database: {0}")]
    Invariant(&'static str),
    /// Provider indexing/search failed after the canonical append.
    #[error(transparent)]
    Provider(#[from] kr8_capability::ProviderError),
}

pub(crate) fn provider_error(error: StoreError) -> kr8_capability::ProviderError {
    use kr8_capability::ProviderError;
    match error {
        StoreError::ProfileMismatch => ProviderError::ProfileMismatch,
        StoreError::UnsupportedSchema(schema) => ProviderError::UnsupportedSchema(schema),
        StoreError::ReadOnly => ProviderError::ReadOnly,
        StoreError::InvalidCursor => ProviderError::InvalidCursor,
        StoreError::Provider(error) => error,
        StoreError::Invariant(message) => ProviderError::Corrupt(message.to_owned()),
        other => ProviderError::Backend(other.to_string()),
    }
}

pub(crate) fn sql_integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidCursor)
}

pub(crate) fn unsigned_integer(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Invariant("negative SQLite integer"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sqlite_integer_conversion_rejects_overflow_and_negative_state() {
        assert_eq!(super::sql_integer(i64::MAX as u64).ok(), Some(i64::MAX));
        assert!(super::sql_integer(u64::MAX).is_err());
        assert!(super::unsigned_integer(-1).is_err());
        assert_eq!(super::unsigned_integer(0).ok(), Some(0));
    }
}
