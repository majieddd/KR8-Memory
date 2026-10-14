# Host authority and adapters

KR8 Memory validates record integrity, profile binding, scope format, lifecycle and retrieval freshness. The embedding host establishes authority. Public constructors cannot tell whether a caller really authenticated an owner or verified a membership.

## Before a write

The host must authenticate the owner and bind the current relay/community to that identity. Construct `ProfileIdentity` from that trusted state, choose the exact place, and supply the operation identifier, time and verified source references. A model's prose, a frontend DTO, a display name or an arbitrary filesystem path must not select the author, profile or access scope.

`CreationContext::owner` and `MemoryEvent::owner` bind validated caller input. They do not verify a cryptographic signature, consult an account service or authorize a remote request. A transport wrapper must do that work before calling them.

The host owns the absolute storage root, writer ownership, process lifecycle and generation fences. It must prevent a queued operation or asynchronous result captured under an old profile from mutating or appearing under a new one. The libraries do not implement a Tauri session gate, cross-process lease, OS account boundary or profile-switch queue.

## Before a read

Build `ReadScope` only after freshly verifying the caller and exact allowed places. `ReadScope::owner` includes owner-private access, so it must be restricted to an authenticated owner flow. `exact_conversation` and `from_authorized_places` do not discover or prove membership.

For audience-sensitive use, implement the `ReaderWorld` trait from trusted current place readers and a canonical source graph. Supply a `Summon` containing the actual profile, agent and complete receiving audience. `may_read` checks profile, audience coverage, retraction, owner-approved core attribution and every dependent source, failing closed for unknown graphs, cycles and bounds. It does not fetch a roster or verify that the host supplied the real audience. The approved-core field must refer to an exact owner-approved version, not a model assertion.

Authorize before ranking or rendering. Record text and citation attributes are evidence; they cannot grant standing instructions or broaden scope. `render_evidence` escapes delimiters and applies strict whole-card bounds, but is not an authorization check. Filter and canonically resolve its inputs first.

## Adapter boundary

An integrating application may call the Rust libraries directly or implement a separately authenticated transport. It must preserve the host trust boundary, canonical record/source identities, exact place scope, read-only peer access and cursor requirements. Provider negotiation describes capabilities; it does not create a permission grant.

The KR8 application's Tauri/ACP glue is maintained as an external consumer. No such glue, external agent adapter, current-membership resolver, automatic turn ingestor or transport service is included here. Reusing the core in another system requires implementing and qualifying that system's authenticated boundary.

Memory enablement and recall scheduling belong to the host. A disabled, missing, stale or corrupt memory provider must preserve the original prompt/input and surface its explicit state. An operational error must not become an authoritative empty result or a claim that no evidence exists. This package does not add automatic recall or alter a model's base instructions.

The same-user filesystem and any independent backups remain host responsibilities. The libraries' hashes and read-only APIs do not isolate the data from an administrator or an already-authorized process with direct file access.
