mod common;
use common::*;
use kr8_capability::{Cursor, IndexUpdate, MemoryStore, ProviderError};
use kr8_memory_sqlite::{SqliteFtsMemoryStore, SqliteRecordLog, StoreError};
use kr8_object_memory::*;

#[test]
fn canonical_create_replay_close_reopen_and_query_only_peer_use_real_files() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let record = fixture.note(
        fixture.place("allowed")?,
        "harbor durable evidence",
        "turn:durable",
    )?;
    let event = fixture.event(
        "turn:durable",
        EventOperation::Create {
            record: record.clone(),
        },
    )?;
    assert!(log.append(&event)?.applied);
    assert!(!log.append(&event)?.applied);
    assert_eq!(log.cursor()?.sequence, 1);
    assert!(matches!(
        index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192),
        Err(ProviderError::StaleCursor { .. })
    ));
    index.catch_up(&log)?;
    let first = index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].record.id, record.id().as_str());
    assert_eq!(first[0].record.record_hash, record.hash().as_str());
    drop(index);
    drop(log);
    let before_log = std::fs::read(fixture.paths.rig_path())?;
    let before_index = std::fs::read(fixture.paths.index_path())?;
    let log = SqliteRecordLog::open_read_only(&fixture.paths, &fixture.identity)?;
    let index = SqliteFtsMemoryStore::open_read_only(&fixture.paths, &fixture.identity)?;
    assert_eq!(
        index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?,
        first
    );
    assert!(matches!(log.append(&event), Err(StoreError::ReadOnly)));
    assert_eq!(
        index.remember(&IndexUpdate {
            previous: Cursor::default(),
            next: Cursor::default(),
            changes: vec![]
        }),
        Err(ProviderError::ReadOnly)
    );
    drop(index);
    drop(log);
    assert_eq!(std::fs::read(fixture.paths.rig_path())?, before_log);
    assert_eq!(std::fs::read(fixture.paths.index_path())?, before_index);
    Ok(())
}

#[test]
fn explicit_supersede_keeps_unlinked_conflict_and_forget_never_resurrects() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let original = fixture.save(
        &log,
        fixture.place("allowed")?,
        "review Friday",
        "turn:original",
    )?;
    let conflict = fixture.save(
        &log,
        fixture.place("allowed")?,
        "review Saturday",
        "turn:conflict",
    )?;
    index.catch_up(&log)?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "review", 5, 8192)?
            .len(),
        2
    );
    let replacement = fixture.note(
        fixture.place("allowed")?,
        "review Sunday forget-marker-772",
        "turn:replacement",
    )?;
    let replacement_event = fixture.event(
        "turn:replacement",
        EventOperation::Supersede {
            original: original.id().clone(),
            replacement: replacement.clone(),
        },
    )?;
    log.append(&replacement_event)?;
    assert!(index
        .search_current(&log, &fixture.scope("allowed")?, "review", 5, 8192)
        .is_err());
    index.catch_up(&log)?;
    let hits = index.search_current(&log, &fixture.scope("allowed")?, "review", 5, 8192)?;
    assert_eq!(hits.len(), 2);
    assert!(hits
        .iter()
        .any(|hit| hit.record.id == conflict.id().as_str()));
    assert!(hits
        .iter()
        .any(|hit| hit.record.id == replacement.id().as_str()));
    let old = log
        .get_scoped(&fixture.scope("allowed")?, original.id())?
        .ok_or("original missing")?;
    assert_eq!(old.status, LifecycleStatus::Superseded);
    assert_eq!(old.superseded_by.as_ref(), Some(replacement.id()));
    log.append(&fixture.event(
        "turn:forget",
        EventOperation::Forget {
            record_id: replacement.id().clone(),
        },
    )?)?;
    index.catch_up(&log)?;
    assert!(log.get(replacement.id())?.is_none());
    assert!(log.is_forgotten(replacement.id())?);
    assert!(!log.append(&replacement_event)?.applied);
    let replay = fixture.event(
        "turn:new-replay-envelope",
        EventOperation::Create {
            record: replacement.clone(),
        },
    )?;
    assert!(matches!(log.append(&replay), Err(StoreError::Forgotten)));
    drop(index);
    drop(log);
    let raw = rusqlite::Connection::open(fixture.paths.rig_path())?;
    let readable_payloads: i64 = raw.query_row(
        "SELECT count(*) FROM events WHERE record_id=?1 AND payload IS NOT NULL",
        [replacement.id().as_str()],
        |row| row.get(0),
    )?;
    assert_eq!(readable_payloads, 0);
    drop(raw);
    let log = fixture.log()?;
    let index = fixture.index()?;
    index.catch_up(&log)?;
    assert!(index
        .search_current(
            &log,
            &fixture.scope("allowed")?,
            "forget-marker-772",
            5,
            8192
        )?
        .is_empty());
    assert!(log.is_forgotten(replacement.id())?);
    Ok(())
}

#[test]
fn deleted_source_stales_dependents_and_cannot_be_restored_by_a_new_claim() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let place = fixture.place("allowed")?;
    let context = CreationContext::owner(&fixture.identity, place.clone(), 42, "turn:decision")?;
    let source_id = SourceId::parse("msg:deleted-source")?;
    let mut draft = RecordDraft::note("object:fixture", "harbor sourced decision");
    draft.record_type = RecordType::Decision;
    draft.sources = vec![SourceRef {
        id: source_id.clone(),
        sha256: Digest::of(b"verified source"),
        place: place.clone(),
        created_at_ms: Some(1),
        availability: SourceAvailability::Present,
    }];
    let parent = RecordEnvelope::create(&context, draft.clone())?;
    log.append(&fixture.event(
        "turn:decision",
        EventOperation::Create {
            record: parent.clone(),
        },
    )?)?;
    let mut child_draft = RecordDraft::note("object:fixture", "harbor derivative");
    child_draft.derived_from.push(parent.id().clone());
    let child = RecordEnvelope::create(
        &CreationContext::owner(&fixture.identity, place.clone(), 42, "turn:child")?,
        child_draft,
    )?;
    log.append(&fixture.event(
        "turn:child",
        EventOperation::Create {
            record: child.clone(),
        },
    )?)?;
    index.catch_up(&log)?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
            .len(),
        2
    );
    log.append(&fixture.event(
        "turn:source-deletion",
        EventOperation::InvalidateSource {
            source_id: source_id.clone(),
            availability: SourceAvailability::Deleted,
        },
    )?)?;
    assert!(index
        .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)
        .is_err());
    index.catch_up(&log)?;
    assert!(index
        .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
        .is_empty());
    assert_eq!(
        log.get(parent.id())?.ok_or("parent missing")?.status,
        LifecycleStatus::Stale
    );
    assert_eq!(
        log.get(child.id())?.ok_or("child missing")?.status,
        LifecycleStatus::Stale
    );
    let later = RecordEnvelope::create(
        &CreationContext::owner(&fixture.identity, place, 43, "turn:later")?,
        draft,
    )?;
    log.append(&fixture.event(
        "turn:later",
        EventOperation::Create {
            record: later.clone(),
        },
    )?)?;
    assert_eq!(
        log.get(later.id())?.ok_or("later missing")?.status,
        LifecycleStatus::Stale
    );
    index.catch_up(&log)?;
    assert!(index
        .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
        .is_empty());
    Ok(())
}

#[test]
fn forgetting_a_source_record_also_erases_its_derived_payload_and_replay_state() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let root = fixture.save(&log, fixture.place("allowed")?, "private root", "turn:root")?;
    let mut draft = RecordDraft::note("object:fixture", "private derivative");
    draft.derived_from.push(root.id().clone());
    let child = RecordEnvelope::create(
        &CreationContext::owner(
            &fixture.identity,
            fixture.place("allowed")?,
            42,
            "turn:child",
        )?,
        draft,
    )?;
    let event = fixture.event(
        "turn:child",
        EventOperation::Create {
            record: child.clone(),
        },
    )?;
    log.append(&event)?;
    log.append(&fixture.event(
        "turn:forget-root",
        EventOperation::Forget {
            record_id: root.id().clone(),
        },
    )?)?;
    assert!(log.is_forgotten(root.id())?);
    assert!(log.is_forgotten(child.id())?);
    assert!(log.get(child.id())?.is_none());
    assert!(!log.append(&event)?.applied);
    Ok(())
}
