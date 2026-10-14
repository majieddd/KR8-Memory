#![allow(dead_code)]
use kr8_memory_sqlite::{ProfilePaths, SqliteFtsMemoryStore, SqliteRecordLog};
use kr8_object_memory::*;
use tempfile::TempDir;

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Fixture {
    pub directory: TempDir,
    pub identity: ProfileIdentity,
    pub paths: ProfilePaths,
}

impl Fixture {
    pub fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let identity = ProfileIdentity::new("owner:fixture", "wss://fixture.invalid", "fixture")?;
        let paths = ProfilePaths::for_app_data(directory.path(), &identity)?;
        Ok(Self {
            directory,
            identity,
            paths,
        })
    }
    pub fn log(&self) -> Result<SqliteRecordLog> {
        Ok(SqliteRecordLog::open_writer(&self.paths, &self.identity)?)
    }
    pub fn index(&self) -> Result<SqliteFtsMemoryStore> {
        Ok(SqliteFtsMemoryStore::open_writer(
            &self.paths,
            &self.identity,
        )?)
    }
    pub fn place(&self, id: &str) -> Result<PlaceId> {
        Ok(PlaceId::conversation(self.identity.key()?, id)?)
    }
    pub fn scope(&self, id: &str) -> Result<ReadScope> {
        Ok(ReadScope::exact_conversation(self.identity.key()?, id)?)
    }
    pub fn note(&self, place: PlaceId, text: &str, turn: &str) -> Result<RecordEnvelope> {
        Ok(RecordEnvelope::create(
            &CreationContext::owner(&self.identity, place, 42, turn)?,
            RecordDraft::note("object:fixture", text),
        )?)
    }
    pub fn event(&self, turn: &str, change: EventOperation) -> Result<MemoryEvent> {
        Ok(MemoryEvent::owner(&self.identity, 42, turn, change)?)
    }
    pub fn save(
        &self,
        log: &SqliteRecordLog,
        place: PlaceId,
        text: &str,
        turn: &str,
    ) -> Result<RecordEnvelope> {
        let record = self.note(place, text, turn)?;
        log.append(&self.event(
            turn,
            EventOperation::Create {
                record: record.clone(),
            },
        )?)?;
        Ok(record)
    }
}
