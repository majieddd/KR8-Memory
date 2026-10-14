//! Only native-derived safe profile paths reach SQLite openers.

use crate::{migrations::SCHEMA_VERSION, StoreError};
use kr8_capability::StoreAccess;
use kr8_object_memory::{ProfileIdentity, ProfileKey};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Separate canonical and derived paths under the authenticated app-data home.
#[derive(Clone, Debug)]
pub struct ProfilePaths {
    pub(crate) rig: PathBuf,
    pub(crate) index: PathBuf,
    pub(crate) profile: ProfileKey,
}

impl ProfilePaths {
    /// Derive Windows-safe hashed paths; no request/model filesystem path is accepted.
    pub fn for_app_data(app_data: &Path, identity: &ProfileIdentity) -> Result<Self, StoreError> {
        if !app_data.is_absolute() {
            return Err(StoreError::Invariant("app-data path must be absolute"));
        }
        let profile = identity.key()?;
        Ok(Self {
            rig: app_data
                .join("kr8-rig")
                .join(profile.as_str())
                .join("rig.sqlite"),
            index: app_data
                .join("kr8-recall")
                .join(profile.as_str())
                .join("memory.sqlite"),
            profile,
        })
    }
    /// Canonical record file; expose only inside native code, never a command response.
    pub fn rig_path(&self) -> &Path {
        &self.rig
    }
    /// Rebuildable index file; expose only inside native code.
    pub fn index_path(&self) -> &Path {
        &self.index
    }
    /// Exact bound partition.
    pub fn profile(&self) -> &ProfileKey {
        &self.profile
    }

    /// Delete both exact profile stores after all handles have been closed.
    /// Partial filesystem failures propagate; the native service must retain its retry.
    pub fn delete(self, identity: &ProfileIdentity) -> Result<(), StoreError> {
        if self.profile != identity.key()? {
            return Err(StoreError::ProfileMismatch);
        }
        for (path, kind) in [(&self.rig, "record-log"), (&self.index, "fts-index")] {
            if path.exists() {
                let connection =
                    open_database(path, identity, kind, StoreAccess::ReadOnlyPeer, "")?;
                drop(connection);
            }
        }
        for path in [&self.rig, &self.index] {
            for suffix in ["-wal", "-shm", ""] {
                let mut name = path.as_os_str().to_os_string();
                name.push(suffix);
                match std::fs::remove_file(PathBuf::from(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            if let Some(parent) = path.parent() {
                match std::fs::remove_dir(parent) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn open_database(
    path: &Path,
    identity: &ProfileIdentity,
    kind: &str,
    access: StoreAccess,
    schema: &str,
) -> Result<Connection, StoreError> {
    let writable = access == StoreAccess::ReadWriteHome;
    if writable {
        let parent = path
            .parent()
            .ok_or(StoreError::Invariant("profile parent"))?;
        std::fs::create_dir_all(parent)?;
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(Duration::from_millis(500))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    if !writable {
        connection.pragma_update(None, "query_only", "ON")?;
    }
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 && writable {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if tables != 0 {
            return Err(StoreError::UnsupportedSchema(version));
        }
        let transaction = connection.transaction()?;
        transaction
            .execute_batch("CREATE TABLE metadata (key TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
        transaction.execute_batch(schema)?;
        transaction.execute(
            "INSERT INTO metadata VALUES('identity',?1)",
            [serde_json::to_string(identity)?],
        )?;
        transaction.execute("INSERT INTO metadata VALUES('kind',?1)", [kind])?;
        if kind == "fts-index" {
            transaction.execute(
                "INSERT INTO metadata VALUES('fingerprint',?1)",
                [crate::migrations::FINGERPRINT],
            )?;
        }
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
    } else if version != SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchema(version));
    }
    let stored_identity: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='identity'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let stored_kind: Option<String> = connection
        .query_row("SELECT value FROM metadata WHERE key='kind'", [], |row| {
            row.get(0)
        })
        .optional()?;
    if stored_identity.as_deref() != Some(serde_json::to_string(identity)?.as_str())
        || stored_kind.as_deref() != Some(kind)
    {
        return Err(StoreError::ProfileMismatch);
    }
    if writable {
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
    } else {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        connection.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Select
            | AuthAction::Read { .. }
            | AuthAction::Recursive
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. } => Authorization::Allow,
            AuthAction::Function { function_name }
                if !matches!(function_name, "load_extension" | "writefile") =>
            {
                Authorization::Allow
            }
            AuthAction::Pragma {
                pragma_name:
                    "user_version" | "query_only" | "data_version" | "schema_version" | "page_count"
                    | "page_size" | "foreign_keys",
                pragma_value: None,
            } => Authorization::Allow,
            _ => Authorization::Deny,
        }))?;
    }
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_peer_opener_denies_query_only_downgrade_temp_writes_and_attach(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let identity = ProfileIdentity::new("owner:fixture", "wss://fixture.invalid", "fixture")?;
        let paths = ProfilePaths::for_app_data(directory.path(), &identity)?;
        drop(open_database(
            paths.rig_path(),
            &identity,
            "record-log",
            StoreAccess::ReadWriteHome,
            crate::migrations::LOG_SCHEMA,
        )?);
        let peer = open_database(
            paths.rig_path(),
            &identity,
            "record-log",
            StoreAccess::ReadOnlyPeer,
            "",
        )?;
        let query_only: u32 = peer.pragma_query_value(None, "query_only", |row| row.get(0))?;
        assert_eq!(query_only, 1);
        assert!(peer.pragma_update(None, "query_only", "OFF").is_err());
        assert!(peer
            .execute_batch("CREATE TEMP TABLE forbidden(value TEXT);")
            .is_err());
        assert!(peer
            .execute_batch("ATTACH DATABASE ':memory:' AS forbidden;")
            .is_err());
        assert!(peer
            .execute("INSERT INTO metadata VALUES('forbidden','write')", [])
            .is_err());
        Ok(())
    }
}
