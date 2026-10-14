# API and provider contract

The three crates form separate layers. Canonical writes belong to the record log; retrieval providers receive derived index updates. Replacing a retrieval provider must not replace the source of truth.

## Canonical records

`ProfileIdentity` binds an owner, canonical relay URL and community identifier. Its SHA-256 `ProfileKey` supplies a filesystem-safe partition, and the full identity is also checked against each SQLite database. A hash is an identifier, not an authentication credential.

Create an owner note with `RecordDraft::note`, an authorized `CreationContext::owner` and `RecordEnvelope::create`. The envelope has a checked content address and immutable payload containing profile, place, object, author, time, turn and source metadata. Pointer drafts require exact target and source information. The host supplies trustworthy identity, place, time and provenance.

Wrap an explicit change in `MemoryEvent::owner` and append it to `SqliteRecordLog`. Events are content-addressed, bounded and validated. `ApplyReceipt.applied` distinguishes a committed change from an identical replay, while its cursor names the canonical head.

Use `get_scoped` and `list` for exposed canonical reads. The lower-level `get` method is unscoped and belongs only behind an already-authorized host boundary. `diagnostics(&ReadScope)` validates scoped projections and returns readable/searchable counts and a canonical cursor from one read transaction. More than 10,000 selected records returns `TooLarge`, rather than a truncated count.

## SQLite retrieval

`ProfilePaths::for_app_data` accepts a trusted absolute host data root and derives two files per profile:

```text
kr8-rig/<profile-key>/rig.sqlite       canonical records and lifecycle log
kr8-recall/<profile-key>/memory.sqlite rebuildable retrieval index
```

`SqliteFtsMemoryStore` uses separate exact-place FTS corpora within the profile index. Scope is selected before corpus statistics, ranking, limits and byte budgets. Allowed-place results merge deterministically. Every selected candidate is resolved against canonical evidence before metadata can influence output budgets.

An owner indexer opens writers and explicitly calls `catch_up`. A peer opens `SqliteRecordLog::open_read_only` and `SqliteFtsMemoryStore::open_read_only`; it cannot create, migrate or catch up either store. Prefer `search_current` for the common Rust integration. The `MemoryStore::search` implementation is also bound to the canonical paths and identity, opens its canonical reader read-only and preserves the caller's exact required cursor.

`Readiness::Ready(cursor)` alone does not prove the index matches the canonical head. Search checks exact sequence and event identity, then rechecks canonical freshness. Stale, corrupt, foreign or unsupported stores return errors. See [recovery](LIFECYCLE-AND-RECOVERY.md).

## `memory.store/1`

`kr8-capability` uses only the Rust standard library. Its `MemoryStore` trait describes a provider and exposes readiness, indexed cursor, native index updates, scoped search and a consistent derived snapshot export. It is a Rust contract, without a bundled RPC server or external adapter.

`negotiate` checks family, schema, exact profile, required access and the declared features:

- `ProfilePartitioned`
- `ScopeBeforeRank`
- `Provenance`
- `DeterministicOrder`
- `ReadOnlyPeer`

An adapter must implement those guarantees, including canonical citation validation and read-only behavior. Declaring the feature names is not qualification. `ScopedQuery` receives only a freshly authorized place set; its constructor validates shape and bounds, not membership. Export takes a trusted new destination and does not imply a portable import or provider catalogue.

The included SQLite adapter is the implemented provider. Vector search, embeddings, other storage engines and external agent transports have no implementation in this package yet.
