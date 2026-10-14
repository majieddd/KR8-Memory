pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const FINGERPRINT: &str = "sqlite-fts5/unicode61/remove-diacritics-2/per-place-1";

pub(crate) const LOG_SCHEMA: &str = "
CREATE TABLE events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 event_id TEXT NOT NULL UNIQUE,
 operation TEXT NOT NULL,
 record_id TEXT,
 payload BLOB
);
CREATE TABLE records (
 record_id TEXT PRIMARY KEY,
 place TEXT NOT NULL,
 payload BLOB NOT NULL,
 status TEXT NOT NULL,
 superseded_by TEXT
);
CREATE INDEX records_place ON records(place,record_id);
CREATE TABLE sources (
 record_id TEXT NOT NULL REFERENCES records(record_id) ON DELETE CASCADE,
 source_id TEXT NOT NULL,
 PRIMARY KEY(record_id,source_id)
);
CREATE INDEX sources_id ON sources(source_id,record_id);
CREATE TABLE derivations (
 record_id TEXT NOT NULL REFERENCES records(record_id) ON DELETE CASCADE,
 parent_id TEXT NOT NULL,
 PRIMARY KEY(record_id,parent_id)
);
CREATE INDEX derivations_parent ON derivations(parent_id,record_id);
CREATE TABLE source_state (source_id TEXT PRIMARY KEY, availability TEXT NOT NULL);
CREATE TABLE tombstones (record_id TEXT PRIMARY KEY,event_id TEXT NOT NULL);
CREATE TABLE changes (
 sequence INTEGER NOT NULL REFERENCES events(sequence),
 record_id TEXT NOT NULL,
 PRIMARY KEY(sequence,record_id)
);
";

pub(crate) const INDEX_SCHEMA: &str = "
CREATE TABLE index_state (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 sequence INTEGER NOT NULL,
 event_id TEXT
);
INSERT INTO index_state VALUES(1,0,NULL);
CREATE TABLE places (place TEXT PRIMARY KEY, table_name TEXT NOT NULL UNIQUE);
CREATE TABLE indexed_records (
 record_id TEXT PRIMARY KEY,
 place TEXT NOT NULL REFERENCES places(place),
 payload BLOB NOT NULL
);
CREATE INDEX indexed_records_place ON indexed_records(place,record_id);
";
