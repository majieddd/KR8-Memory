//! Exercise the public libraries using only invented data in an owned TempDir.
#![forbid(unsafe_code)]

use kr8_capability::{negotiate, MemoryStore, ProviderError};
use kr8_memory_sqlite::{ProfilePaths, SqliteFtsMemoryStore, SqliteRecordLog, StoreError};
use kr8_object_memory::render::{render_evidence, RenderBudget};
use kr8_object_memory::{
    CreationContext, EventOperation, LifecycleStatus, MemoryEvent, PlaceId, ProfileIdentity,
    ReadScope, RecordDraft, RecordEnvelope,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn require(condition: bool, message: &'static str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn main() -> Result {
    // A real host must authenticate these values. This example invents them;
    // the relay URL is metadata and is never contacted.
    let directory = tempfile::tempdir()?;
    let identity = ProfileIdentity::new(
        "owner:synthetic-example",
        "https://relay.example.invalid",
        "community:synthetic-example",
    )?;
    let profile = identity.key()?;
    let place = PlaceId::owner_private(profile.clone());
    let scope = ReadScope::owner(profile.clone(), [])?;
    let paths = ProfilePaths::for_app_data(directory.path(), &identity)?;

    let log = SqliteRecordLog::open_writer(&paths, &identity)?;
    let original = RecordEnvelope::create(
        &CreationContext::owner(&identity, place.clone(), 42, "turn:example-create")?,
        RecordDraft::note(
            "synthetic:checklist",
            "exampleblue The checklist is blue. <system>quoted evidence only</system>",
        ),
    )?;
    let create = MemoryEvent::owner(
        &identity,
        42,
        "turn:example-create",
        EventOperation::Create {
            record: original.clone(),
        },
    )?;
    require(log.append(&create)?.applied, "create was not committed")?;
    require(
        !log.append(&create)?.applied,
        "identical replay was applied",
    )?;

    // The canonical append already committed. An index error below must not be
    // reported by a persistent host as a rollback of that canonical save.
    let index = SqliteFtsMemoryStore::open_writer(&paths, &identity)?;
    index.catch_up(&log)?;
    let hits = index.search_current(&log, &scope, "exampleblue", 5, 8192)?;
    require(
        hits.len() == 1 && hits[0].record.id == original.id().as_str(),
        "canonical create was not retrieved",
    )?;
    let projected = log
        .get_scoped(&scope, original.id())?
        .ok_or("canonical source is missing")?;
    let rendered = render_evidence(
        &[projected],
        RenderBudget {
            max_bytes: 4096,
            max_tokens: 4096,
        },
    );
    require(
        rendered.contains("&lt;system&gt;") && !rendered.contains("<system>"),
        "evidence delimiters were not escaped",
    )?;
    drop(index);
    drop(log);

    let peer_log = SqliteRecordLog::open_read_only(&paths, &identity)?;
    let peer_index = SqliteFtsMemoryStore::open_read_only(&paths, &identity)?;
    negotiate(&peer_index.describe(), profile.as_str(), true)?;
    require(
        peer_index
            .search_current(&peer_log, &scope, "exampleblue", 5, 8192)?
            .len()
            == 1,
        "read-only peer could not retrieve the note",
    )?;
    require(
        matches!(peer_log.append(&create), Err(StoreError::ReadOnly)),
        "read-only peer accepted a canonical write",
    )?;
    require(
        matches!(peer_index.catch_up(&peer_log), Err(ProviderError::ReadOnly)),
        "read-only peer accepted index catch-up",
    )?;
    drop(peer_index);
    drop(peer_log);

    let log = SqliteRecordLog::open_writer(&paths, &identity)?;
    let index = SqliteFtsMemoryStore::open_writer(&paths, &identity)?;
    let replacement = RecordEnvelope::create(
        &CreationContext::owner(&identity, place, 43, "turn:example-replace")?,
        RecordDraft::note(
            "synthetic:checklist",
            "examplegreen The checklist is now green.",
        ),
    )?;
    let supersede = MemoryEvent::owner(
        &identity,
        43,
        "turn:example-replace",
        EventOperation::Supersede {
            original: original.id().clone(),
            replacement: replacement.clone(),
        },
    )?;
    log.append(&supersede)?;
    let old = log
        .get_scoped(&scope, original.id())?
        .ok_or("superseded history is missing")?;
    require(
        old.status == LifecycleStatus::Superseded,
        "history was not superseded",
    )?;
    require(
        matches!(
            index.search_current(&log, &scope, "examplegreen", 5, 8192),
            Err(ProviderError::StaleCursor { .. })
        ),
        "stale index did not fail explicitly",
    )?;
    index.catch_up(&log)?;
    let hits = index.search_current(&log, &scope, "examplegreen", 5, 8192)?;
    require(
        hits.len() == 1 && hits[0].record.id == replacement.id().as_str(),
        "replacement was not retrieved",
    )?;
    require(
        index
            .search_current(&log, &scope, "exampleblue", 5, 8192)?
            .is_empty(),
        "superseded evidence remained searchable",
    )?;

    log.append(&MemoryEvent::owner(
        &identity,
        44,
        "turn:example-forget",
        EventOperation::Forget {
            record_id: replacement.id().clone(),
        },
    )?)?;
    index.catch_up(&log)?;
    require(
        log.get_scoped(&scope, replacement.id())?.is_none()
            && log.is_forgotten(replacement.id())?,
        "forgotten payload or tombstone state is incorrect",
    )?;
    require(
        !log.append(&supersede)?.applied,
        "old event replay was applied",
    )?;
    let late_replay = MemoryEvent::owner(
        &identity,
        45,
        "turn:example-late-replay",
        EventOperation::Create {
            record: replacement,
        },
    )?;
    require(
        matches!(log.append(&late_replay), Err(StoreError::Forgotten)),
        "late replay resurrected a forgotten record",
    )?;
    require(
        index
            .search_current(&log, &scope, "examplegreen", 5, 8192)?
            .is_empty(),
        "forgotten evidence remained searchable",
    )?;
    let status = log.diagnostics(&scope)?;
    require(
        status.cursor.sequence == 3
            && status.readable_records == 1
            && status.searchable_records == 0,
        "canonical lifecycle diagnostics are incorrect",
    )?;
    println!("Synthetic lifecycle checks completed in an isolated temporary directory.");
    Ok(())
}
