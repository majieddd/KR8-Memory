mod common;
use common::*;
use kr8_capability::{MemoryStore, ProviderError, ScopedQuery};
use kr8_memory_sqlite::{ProfilePaths, SqliteFtsMemoryStore, SqliteRecordLog};
use kr8_object_memory::*;

fn signature(hits: Vec<kr8_capability::MemoryPassage>) -> Vec<(String, usize, u64)> {
    hits.into_iter()
        .map(|hit| (hit.record.id, hit.ordinal, hit.local_score.to_bits()))
        .collect()
}

#[test]
fn forbidden_exact_match_canaries_cannot_change_allowed_ids_order_or_score_bits() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    fixture.save(&log, fixture.place("allowed")?, "harbor apple", "turn:a")?;
    fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor harbor pear",
        "turn:b",
    )?;
    index.catch_up(&log)?;
    let before =
        signature(index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?);
    for number in 0..100 {
        fixture.save(
            &log,
            fixture.place("forbidden")?,
            "harbor harbor harbor CANARY-FORBIDDEN-991",
            &format!("turn:canary-{number}"),
        )?;
    }
    let other = ProfileIdentity::new(
        fixture.identity.owner(),
        fixture.identity.relay(),
        "other-profile",
    )?;
    let paths = ProfilePaths::for_app_data(fixture.directory.path(), &other)?;
    let other_log = SqliteRecordLog::open_writer(&paths, &other)?;
    let other_place = PlaceId::conversation(other.key()?, "allowed")?;
    let other_record = RecordEnvelope::create(
        &CreationContext::owner(&other, other_place, 1, "turn:other")?,
        RecordDraft::note("object:other", "harbor CANARY-OTHER-PROFILE"),
    )?;
    other_log.append(&MemoryEvent::owner(
        &other,
        1,
        "turn:other",
        EventOperation::Create {
            record: other_record,
        },
    )?)?;
    index.catch_up(&log)?;
    let after =
        signature(index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?);
    assert_eq!(before, after);
    assert_eq!(after.len(), 2);
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 1, 8192)?
            .len(),
        1
    );
    assert!(log
        .get_scoped(&fixture.scope("forbidden")?, &RecordId::parse(&after[0].0)?)?
        .is_none());
    Ok(())
}

#[test]
fn multiple_authorized_places_merge_by_ordinal_and_label_not_raw_scores() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    for (place, turn, text) in [
        ("a", "turn:a1", "harbor"),
        ("a", "turn:a2", "harbor extra extra"),
        ("b", "turn:b1", "harbor harbor harbor"),
        ("b", "turn:b2", "harbor details"),
    ] {
        fixture.save(&log, fixture.place(place)?, text, turn)?;
    }
    index.catch_up(&log)?;
    let scope = ReadScope::owner(
        fixture.identity.key()?,
        [fixture.place("a")?, fixture.place("b")?],
    )?;
    let hits = index.search_current(&log, &scope, "harbor", 5, 8192)?;
    assert_eq!(
        hits.iter().map(|hit| hit.ordinal).collect::<Vec<_>>(),
        vec![1, 1, 2, 2]
    );
    assert!(hits[0].record.place < hits[1].record.place);
    assert_eq!(hits[0].record.place, hits[2].record.place);
    Ok(())
}

#[test]
fn corrupt_fts_rejects_search_and_preserves_committed_log_for_rebuild_retry() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor original",
        "turn:original",
    )?;
    index.catch_up(&log)?;
    drop(index);
    let raw = rusqlite::Connection::open(fixture.paths.index_path())?;
    let table: String = raw.query_row("SELECT table_name FROM places LIMIT 1", [], |row| {
        row.get(0)
    })?;
    raw.execute_batch(&format!("DROP TABLE {table};"))?;
    drop(raw);
    let peer = SqliteFtsMemoryStore::open_read_only(&fixture.paths, &fixture.identity)?;
    assert!(peer
        .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)
        .is_err());
    drop(peer);
    let record = fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor committed despite index failure",
        "turn:later",
    )?;
    assert!(log.get(record.id())?.is_some());
    let paths = fixture.paths.clone();
    drop(log);
    std::fs::remove_file(paths.index_path())?; // Disposable fixture's derived file only.
    let log = fixture.log()?;
    let index = fixture.index()?;
    index.catch_up(&log)?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
            .len(),
        2
    );
    Ok(())
}

#[test]
fn failed_index_cursor_commit_rolls_back_rows_and_can_be_retried() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let raw = rusqlite::Connection::open(fixture.paths.index_path())?;
    raw.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON index_state BEGIN SELECT RAISE(ABORT,'fixture cursor failure'); END;")?;
    let record = fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor retry",
        "turn:retry",
    )?;
    assert!(index.catch_up(&log).is_err());
    assert_eq!(index.cursor()?.sequence, 0);
    let indexed: i64 =
        raw.query_row("SELECT count(*) FROM indexed_records", [], |row| row.get(0))?;
    assert_eq!(indexed, 0);
    assert!(log.get(record.id())?.is_some());
    assert!(matches!(
        index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192),
        Err(ProviderError::StaleCursor { .. })
    ));
    raw.execute_batch("DROP TRIGGER fail_cursor;")?;
    index.catch_up(&log)?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn newer_schema_and_wrong_profile_fail_without_rewriting_files() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    fixture.save(&log, fixture.place("allowed")?, "harbor", "turn:fixture")?;
    drop(log);
    let other = ProfileIdentity::new(fixture.identity.owner(), fixture.identity.relay(), "other")?;
    let other_paths = ProfilePaths::for_app_data(fixture.directory.path(), &other)?;
    std::fs::create_dir_all(other_paths.rig_path().parent().ok_or("missing parent")?)?;
    std::fs::copy(fixture.paths.rig_path(), other_paths.rig_path())?;
    let before = std::fs::read(other_paths.rig_path())?;
    assert!(SqliteRecordLog::open_writer(&other_paths, &other).is_err());
    assert_eq!(std::fs::read(other_paths.rig_path())?, before);
    let raw = rusqlite::Connection::open(fixture.paths.rig_path())?;
    raw.pragma_update(None, "user_version", 2)?;
    drop(raw);
    let before = std::fs::read(fixture.paths.rig_path())?;
    assert!(SqliteRecordLog::open_writer(&fixture.paths, &fixture.identity).is_err());
    assert_eq!(std::fs::read(fixture.paths.rig_path())?, before);
    Ok(())
}

#[test]
fn export_is_consistent_and_profile_delete_removes_both_exact_stores() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor export",
        "turn:export",
    )?;
    index.catch_up(&log)?;
    let destination = fixture.directory.path().join("derived-export.sqlite");
    let manifest = index.export(&destination)?;
    assert_eq!(manifest.records, 1);
    assert_eq!(manifest.cursor, index.cursor()?);
    assert_eq!(index.export(&destination), Err(ProviderError::ExportExists));
    drop(index);
    drop(log);
    fixture.paths.clone().delete(&fixture.identity)?;
    assert!(!fixture.paths.rig_path().exists());
    assert!(!fixture.paths.index_path().exists());
    Ok(())
}

#[test]
fn derived_evidence_cannot_reuse_a_canonical_id_hash_with_altered_fields() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let record = fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor canonical evidence",
        "turn:canonical",
    )?;
    index.catch_up(&log)?;
    let raw = rusqlite::Connection::open(fixture.paths.index_path())?;
    let original: Vec<u8> = raw.query_row(
        "SELECT payload FROM indexed_records WHERE record_id=?1",
        [record.id().as_str()],
        |row| row.get(0),
    )?;
    // Keep the canonical ID and hash fixed; each other returned field must still
    // match its canonical projection. Removing the full comparison exposes the
    // forged author/time/source cases even if selected FTS text is checked.
    for field in [4usize, 5, 6, 7] {
        let mut altered: serde_json::Value = serde_json::from_slice(&original)?;
        altered[field] = match field {
            4 => "forged-author".into(),
            5 => 9999u64.into(),
            6 => "harbor forged evidence".into(),
            7 => serde_json::json!([[
                "msg:forged-source",
                Digest::of(b"forged").as_str(),
                "present"
            ]]),
            _ => unreachable!(),
        };
        raw.execute(
            "UPDATE indexed_records SET payload=?1 WHERE record_id=?2",
            rusqlite::params![serde_json::to_vec(&altered)?, record.id().as_str()],
        )?;
        assert!(
            matches!(
                index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192),
                Err(ProviderError::Corrupt(_))
            ),
            "altered field {field}"
        );
    }
    raw.execute(
        "UPDATE indexed_records SET payload=?1 WHERE record_id=?2",
        rusqlite::params![original, record.id().as_str()],
    )?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn oversized_forged_provenance_cannot_hide_before_canonical_validation() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let record = fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor canonical evidence",
        "turn:budget-canonical",
    )?;
    index.catch_up(&log)?;
    let query = ScopedQuery::new(
        fixture.identity.key()?.as_str().to_owned(),
        std::collections::BTreeSet::from([fixture.place("allowed")?.canonical()?]),
        "harbor",
        5,
        8192,
        index.cursor()?,
    )?;
    assert_eq!(index.search(&query)?.len(), 1);
    let raw = rusqlite::Connection::open(fixture.paths.index_path())?;
    let original: Vec<u8> = raw.query_row(
        "SELECT payload FROM indexed_records WHERE record_id=?1",
        [record.id().as_str()],
        |row| row.get(0),
    )?;
    let mut forged: serde_json::Value = serde_json::from_slice(&original)?;
    let citations: Vec<_> = (0..5)
        .map(|position| {
            serde_json::json!([
                format!("msg:forged-{position}-{}", "x".repeat(1800)),
                Digest::of(b"forged-source").as_str(),
                "present"
            ])
        })
        .collect();
    forged[7] = citations.into();
    let bytes = serde_json::to_vec(&forged)?;
    assert!(
        bytes.len() < 65_536,
        "fixture must pass index payload bound"
    );
    assert!(
        forged[7]
            .as_array()
            .ok_or("sources absent")?
            .iter()
            .map(|source| source[0].as_str().ok_or("source id absent").map(str::len))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .sum::<usize>()
            > 8192
    );
    raw.execute(
        "UPDATE indexed_records SET payload=?1 WHERE record_id=?2",
        rusqlite::params![bytes, record.id().as_str()],
    )?;
    assert!(
        matches!(
            index.search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192),
            Err(ProviderError::Corrupt(_))
        ),
        "untrusted metadata must be validated even if it would exceed the result budget"
    );
    assert!(
        matches!(index.search(&query), Err(ProviderError::Corrupt(_))),
        "the public provider contract must prove the same canonical authority before budget"
    );
    raw.execute(
        "UPDATE indexed_records SET payload=?1 WHERE record_id=?2",
        rusqlite::params![original, record.id().as_str()],
    )?;
    assert_eq!(
        index
            .search_current(&log, &fixture.scope("allowed")?, "harbor", 5, 8192)?
            .len(),
        1
    );
    assert_eq!(index.search(&query)?.len(), 1);
    Ok(())
}

#[test]
fn public_trait_search_binds_profile_exact_scope_cursor_and_read_only_source() -> Result {
    let fixture = Fixture::new()?;
    let log = fixture.log()?;
    let index = fixture.index()?;
    let allowed = fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor allowed",
        "turn:trait-allowed",
    )?;
    fixture.save(
        &log,
        PlaceId::owner_private(fixture.identity.key()?),
        "harbor OWNER-PRIVATE-CANARY",
        "turn:trait-private",
    )?;
    fixture.save(
        &log,
        fixture.place("sibling")?,
        "harbor SIBLING-CANARY",
        "turn:trait-sibling",
    )?;
    index.catch_up(&log)?;
    let profile = fixture.identity.key()?.as_str().to_owned();
    let places = std::collections::BTreeSet::from([fixture.place("allowed")?.canonical()?]);
    let original = ScopedQuery::new(
        profile.clone(),
        places.clone(),
        "harbor",
        5,
        8192,
        index.cursor()?,
    )?;
    let hits = index.search(&original)?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, allowed.id().as_str());
    fixture.save(
        &log,
        fixture.place("allowed")?,
        "harbor newer",
        "turn:trait-newer",
    )?;
    assert!(
        matches!(index.search(&original), Err(ProviderError::InvalidCursor)),
        "a stale original request cannot silently upgrade its canonical snapshot"
    );
    index.catch_up(&log)?;
    assert!(
        matches!(index.search(&original), Err(ProviderError::InvalidCursor)),
        "even a caught-up provider must preserve the requested canonical cursor"
    );
    let wrong_profile = ScopedQuery::new(
        "other-profile".into(),
        places.clone(),
        "harbor",
        5,
        8192,
        index.cursor()?,
    )?;
    assert!(matches!(
        index.search(&wrong_profile),
        Err(ProviderError::ProfileMismatch)
    ));
    let other = ProfileIdentity::new(fixture.identity.owner(), fixture.identity.relay(), "other")?;
    let foreign_place = ScopedQuery::new(
        profile.clone(),
        std::collections::BTreeSet::from([PlaceId::owner_private(other.key()?).canonical()?]),
        "harbor",
        5,
        8192,
        index.cursor()?,
    )?;
    assert!(matches!(
        index.search(&foreign_place),
        Err(ProviderError::ProfileMismatch)
    ));
    let noncanonical = ScopedQuery::new(
        profile.clone(),
        std::collections::BTreeSet::from([format!(" {}", fixture.place("allowed")?.canonical()?)]),
        "harbor",
        5,
        8192,
        index.cursor()?,
    )?;
    assert!(matches!(
        index.search(&noncanonical),
        Err(ProviderError::InvalidQuery(_))
    ));
    let current = ScopedQuery::new(profile, places, "harbor", 5, 8192, index.cursor()?)?;
    assert_eq!(index.search(&current)?.len(), 2);
    drop(log);
    std::fs::remove_file(fixture.paths.rig_path())?; // Disposable fixture source only.
    assert!(index.search(&current).is_err());
    assert!(
        !fixture.paths.rig_path().exists(),
        "public search must never recreate a missing canonical log"
    );
    Ok(())
}
