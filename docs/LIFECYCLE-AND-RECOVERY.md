# Lifecycle, freshness and recovery

Canonical record payloads are immutable. Explicit events change their projections; conflicting notes stay separate unless an owner supplies a supersede relationship.

| State | Meaning | Searchable |
| --- | --- | --- |
| `Active` | Current, source-valid evidence | Yes |
| `Superseded` | An explicit replacement is linked | No |
| `Retracted` | Evidence was explicitly withdrawn | No |
| `Missing` | An exact pointer target is absent | No |
| `Stale` | A source changed, was deleted or was invalidated | No |
| `Forgotten` | Readable payload was purged and anti-replay state remains | No |

`EventOperation` supports create, supersede, retract, source invalidation and forget. Supersede creates its replacement and links the original atomically in the same profile/place. Source invalidation propagates to dependent projections. A source cannot be restored merely by asserting `Present`; fresh verification and explicit authorized work are required.

Forget removes readable payload from the canonical store and causes derived entries to be removed on catch-up. Minimal tombstones prevent the same content-addressed record from being recreated by replay. Identical old event replay is a no-op; a new event attempting to recreate a forgotten record is rejected. Existing independent backups and storage media are outside that logical lifecycle guarantee.

## Canonical first, derived second

Append canonical changes before attempting index catch-up. A successful canonical transaction remains committed if opening or updating the index later fails. An application should retain the committed record identity and an honest retry/index-pending state, rather than imply the canonical save rolled back.

Canonical and derived heads carry both a sequence and an event identity. `catch_up` derives changes from canonical current projections and advances the index cursor atomically with its batch. Failure leaves the previous derived cursor. Search requires canonical-head equality and rechecks after query work, so a stale index cannot silently return an empty or outdated result.

## Read-only status and recovery

Read-only openers never create, migrate or redirect writes. Scoped diagnostics count validated readable and searchable projections with their cursor in one transaction; status inspection does not rebuild an index. An index readiness cursor still needs comparison with the canonical head.

A missing derived store may be created and rebuilt by an explicitly authorized host writer using `open_writer` and `catch_up`. Corrupt or newer-schema files fail explicitly. The host must preserve and diagnose those failures before any separately authorized replacement; this package has no automatic destructive recovery or unknown-schema migration.

The canonical file is the source of truth. Rebuilding retrieval must not replace it with an index export. Provider export is a consistent derived snapshot to a new trusted destination, not a full canonical backup/import protocol.

## Profile deletion

`ProfilePaths::delete` removes the exact canonical and derived profile stores and their SQLite sidecars after verifying identity. The host must authorize deletion, stop relevant work and close every handle first. Filesystem failures propagate and may leave partial progress; the host must retain its retry state. Removing a profile from a UI or disabling recall is not itself authorization to purge its stores.
