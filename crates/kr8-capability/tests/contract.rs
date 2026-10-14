use kr8_capability::*;
use std::collections::BTreeSet;

fn info() -> StoreInfo {
    StoreInfo {
        family: MEMORY_STORE_V1.into(),
        schema: MEMORY_STORE_SCHEMA,
        provider: "reference".into(),
        profile: "profile".into(),
        access: StoreAccess::ReadOnlyPeer,
        features: BTreeSet::from([
            MemoryFeature::ProfilePartitioned,
            MemoryFeature::ScopeBeforeRank,
            MemoryFeature::Provenance,
            MemoryFeature::DeterministicOrder,
            MemoryFeature::ReadOnlyPeer,
        ]),
    }
}

#[test]
fn negotiation_rejects_every_missing_required_feature() {
    assert!(negotiate(&info(), "profile", true).is_ok());
    for feature in info().features {
        let mut broken = info();
        broken.features.remove(&feature);
        assert_eq!(
            negotiate(&broken, "profile", true),
            Err(ProviderError::MissingFeature(feature))
        );
    }
}

#[test]
fn family_schema_profile_and_peer_access_are_separate_hard_gates() {
    let mut broken = info();
    broken.family = "memory.store/2".into();
    assert!(matches!(
        negotiate(&broken, "profile", true),
        Err(ProviderError::UnsupportedFamily(_))
    ));
    broken = info();
    broken.schema = 2;
    assert_eq!(
        negotiate(&broken, "profile", true),
        Err(ProviderError::UnsupportedSchema(2))
    );
    assert_eq!(
        negotiate(&info(), "other", true),
        Err(ProviderError::ProfileMismatch)
    );
    broken = info();
    broken.access = StoreAccess::ReadWriteHome;
    assert_eq!(
        negotiate(&broken, "profile", true),
        Err(ProviderError::ReadOnly)
    );
}

#[test]
fn queries_have_no_unscoped_or_unbounded_form() {
    let allowed = BTreeSet::from(["native-place".into()]);
    assert!(ScopedQuery::new(
        "profile".into(),
        allowed.clone(),
        "harbor",
        5,
        2048,
        Cursor::default()
    )
    .is_ok());
    for (text, limit, bytes) in [
        ("", 5, 2048),
        ("harbor", 0, 2048),
        ("harbor", 33, 2048),
        ("harbor", 5, 0),
        ("harbor", 5, 16_385),
    ] {
        assert!(ScopedQuery::new(
            "profile".into(),
            allowed.clone(),
            text,
            limit,
            bytes,
            Cursor::default()
        )
        .is_err());
    }
    assert!(ScopedQuery::new(
        "profile".into(),
        BTreeSet::new(),
        "harbor",
        5,
        2048,
        Cursor::default()
    )
    .is_err());
    assert!(Cursor {
        sequence: 1,
        event_id: None
    }
    .validate()
    .is_err());
    assert!(Cursor {
        sequence: 0,
        event_id: Some("a".repeat(64))
    }
    .validate()
    .is_err());
}
