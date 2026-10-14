//! Authored real-SQLite scoped count/immutability cases; unrun in this revision.
mod common;
use common::*;
use kr8_memory_sqlite::{SqliteRecordLog, StoreError};
use kr8_object_memory::*;

fn pointer(
    fixture: &Fixture,
    log: &SqliteRecordLog,
    place: PlaceId,
    availability: SourceAvailability,
    turn: &str,
) -> Result<RecordEnvelope> {
    let source = SourceId::parse(&format!("msg:{turn}"))?;
    let digest = Digest::of(turn.as_bytes());
    let mut draft = RecordDraft::note("object:fixture", "exact synthetic pointer");
    draft.record_type = RecordType::Pointer;
    draft.sources.push(SourceRef {
        id: source.clone(),
        sha256: digest.clone(),
        place: place.clone(),
        created_at_ms: Some(42),
        availability,
    });
    draft.sources.push(SourceRef {
        id: SourceId::parse(turn)?,
        sha256: Digest::of(b"synthetic producing turn"),
        place: place.clone(),
        created_at_ms: Some(42),
        availability: SourceAvailability::Present,
    });
    draft.target = Some(PointerTarget {
        source,
        size: 1,
        sha256: digest,
        modified_at_ms: Some(42),
    });
    let record = RecordEnvelope::create(
        &CreationContext::owner(&fixture.identity, place, 42, turn)?,
        draft,
    )?;
    log.append(&fixture.event(
        turn,
        EventOperation::Create {
            record: record.clone(),
        },
    )?)?;
    Ok(record)
}

#[test]
fn exact_scope_counts_exceed_list_cap_and_distinguish_history_sources_and_forget() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let place = PlaceId::owner_private(fixture.identity.key()?);
    let scope = ReadScope::owner(fixture.identity.key()?, [])?;
    let mut notes = Vec::new();
    for number in 0..257 {
        notes.push(fixture.save(
            &log,
            place.clone(),
            "canonical human note",
            &format!("turn:{number}"),
        )?);
    }
    fixture.save(
        &log,
        fixture.place("forbidden")?,
        "FORBIDDEN-CANARY",
        "turn:forbidden",
    )?;
    let replacement = fixture.note(place.clone(), "explicit replacement", "turn:replace")?;
    log.append(&fixture.event(
        "turn:replace",
        EventOperation::Supersede {
            original: notes[0].id().clone(),
            replacement,
        },
    )?)?;
    log.append(&fixture.event(
        "turn:retract",
        EventOperation::Retract {
            record_id: notes[1].id().clone(),
        },
    )?)?;
    log.append(&fixture.event(
        "turn:forget",
        EventOperation::Forget {
            record_id: notes[2].id().clone(),
        },
    )?)?;
    let missing = pointer(
        &fixture,
        &log,
        place.clone(),
        SourceAvailability::Missing,
        "turn:missing",
    )?;
    let present = pointer(
        &fixture,
        &log,
        place,
        SourceAvailability::Present,
        "turn:present",
    )?;
    let target = present
        .payload()
        .target
        .as_ref()
        .ok_or("pointer target missing")?;
    log.append(&fixture.event(
        "turn:invalidate",
        EventOperation::InvalidateSource {
            source_id: target.source.clone(),
            availability: SourceAvailability::Deleted,
        },
    )?)?;
    let peer = SqliteRecordLog::open_read_only(&fixture.paths, &fixture.identity)?;
    assert_eq!(
        peer.get_scoped(&scope, missing.id())?
            .ok_or("missing pointer absent")?
            .status,
        LifecycleStatus::Missing
    );
    assert_eq!(
        peer.get_scoped(&scope, present.id())?
            .ok_or("stale pointer absent")?
            .status,
        LifecycleStatus::Stale
    );
    let report = peer.diagnostics(&scope)?;
    assert_eq!(peer.list(&scope, 256)?.len(), 256);
    assert_eq!(report.readable_records, 259);
    assert_eq!(report.searchable_records, 255);
    assert_eq!(report.schema, 1);
    assert_eq!(report.cursor, log.cursor()?);
    assert!(log.is_forgotten(notes[2].id())?);
    assert_eq!(
        peer.diagnostics(&fixture.scope("forbidden")?)?
            .readable_records,
        1
    );
    let other = ProfileIdentity::new(
        fixture.identity.owner(),
        fixture.identity.relay(),
        "other-profile",
    )?;
    assert!(matches!(
        peer.diagnostics(&ReadScope::owner(other.key()?, [])?),
        Err(StoreError::Unauthorized)
    ));
    Ok(())
}

#[test]
fn forbidden_corrupt_payload_is_not_read_but_selected_corruption_has_no_count() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let allowed = fixture.save(
        &log,
        fixture.place("allowed")?,
        "allowed evidence",
        "turn:allowed",
    )?;
    let forbidden = fixture.save(
        &log,
        fixture.place("forbidden")?,
        "forbidden evidence",
        "turn:forbidden",
    )?;
    let raw = rusqlite::Connection::open(fixture.paths.rig_path())?;
    raw.execute(
        "UPDATE records SET payload=?1 WHERE record_id=?2",
        rusqlite::params![b"invalid payload".as_slice(), forbidden.id().as_str()],
    )?;
    let peer = SqliteRecordLog::open_read_only(&fixture.paths, &fixture.identity)?;
    assert_eq!(
        peer.diagnostics(&fixture.scope("allowed")?)?
            .readable_records,
        1
    );
    assert!(peer.diagnostics(&fixture.scope("forbidden")?).is_err());
    let mut forged: serde_json::Value = serde_json::from_slice(&allowed.canonical_bytes()?)?;
    forged["payload"]["text"] = serde_json::json!("changed without canonical rehash");
    raw.execute(
        "UPDATE records SET payload=?1 WHERE record_id=?2",
        rusqlite::params![serde_json::to_vec(&forged)?, allowed.id().as_str()],
    )?;
    assert!(peer.diagnostics(&fixture.scope("allowed")?).is_err());
    Ok(())
}

#[test]
fn source_invalid_active_projection_fails_instead_of_becoming_a_searchable_count() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let record = pointer(
        &fixture,
        &log,
        fixture.place("allowed")?,
        SourceAvailability::Present,
        "turn:source",
    )?;
    assert_eq!(
        log.get_scoped(&fixture.scope("allowed")?, record.id())?
            .ok_or("pointer absent")?
            .status,
        LifecycleStatus::Active
    );
    let target = record
        .payload()
        .target
        .as_ref()
        .ok_or("pointer target missing")?;
    let raw = rusqlite::Connection::open(fixture.paths.rig_path())?;
    // Deliberately corrupt the projection: bypass the lifecycle invalidation.
    raw.execute(
        "INSERT INTO source_state(source_id,availability) VALUES(?1,'deleted')",
        [target.source.as_str()],
    )?;
    let peer = SqliteRecordLog::open_read_only(&fixture.paths, &fixture.identity)?;
    assert!(matches!(
        peer.diagnostics(&fixture.scope("allowed")?),
        Err(StoreError::Invariant(_))
    ));
    Ok(())
}

#[test]
fn read_only_counts_do_not_rewrite_store_and_recheck_an_unsupported_schema() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    fixture.save(
        &log,
        fixture.place("allowed")?,
        "durable evidence",
        "turn:allowed",
    )?;
    drop(log);
    let before = std::fs::read(fixture.paths.rig_path())?;
    let peer = SqliteRecordLog::open_read_only(&fixture.paths, &fixture.identity)?;
    assert_eq!(
        peer.diagnostics(&fixture.scope("allowed")?)?
            .readable_records,
        1
    );
    assert_eq!(std::fs::read(fixture.paths.rig_path())?, before);
    let raw = rusqlite::Connection::open(fixture.paths.rig_path())?;
    raw.pragma_update(None, "user_version", 77)?;
    let unsupported = std::fs::read(fixture.paths.rig_path())?;
    assert!(matches!(
        peer.diagnostics(&fixture.scope("allowed")?),
        Err(StoreError::UnsupportedSchema(77))
    ));
    assert_eq!(std::fs::read(fixture.paths.rig_path())?, unsupported);
    Ok(())
}
