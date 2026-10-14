use crate::{fts::index_cursor, StoreError};
use kr8_capability::ExportManifest;
use kr8_object_memory::ProfileKey;
use rusqlite::{
    backup::{Backup, StepResult},
    Connection,
};
use std::{fs::OpenOptions, path::Path};

pub(crate) fn export(
    connection: &Connection,
    profile: &ProfileKey,
    to: &Path,
) -> Result<ExportManifest, StoreError> {
    match OpenOptions::new().write(true).create_new(true).open(to) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(kr8_capability::ProviderError::ExportExists.into())
        }
        Err(error) => return Err(error.into()),
    }
    // The trusted caller owns this new destination. A failure leaves an explicit
    // incomplete file; never overwrite another destination or claim a valid receipt.
    let mut destination = Connection::open(to)?;
    let backup = Backup::new(connection, &mut destination)?;
    let mut complete = false;
    for _ in 0..1024 {
        match backup.step(64)? {
            StepResult::Done => {
                complete = true;
                break;
            }
            StepResult::More => {}
            StepResult::Busy | StepResult::Locked => {
                return Err(StoreError::Invariant(
                    "export is busy; retry to a new destination",
                ))
            }
            _ => return Err(StoreError::Invariant("unsupported backup result")),
        }
    }
    if !complete {
        return Err(StoreError::TooLarge("export pages"));
    }
    drop(backup);
    let cursor = index_cursor(&destination)?;
    let records: i64 =
        destination.query_row("SELECT count(*) FROM indexed_records", [], |row| row.get(0))?;
    Ok(ExportManifest {
        schema: crate::migrations::SCHEMA_VERSION,
        profile: profile.as_str().to_owned(),
        cursor,
        records: crate::unsigned_integer(records)?,
    })
}
