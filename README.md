# KR8 Memory

KR8 Memory is a modular Rust core for profile-bound records, explicit memory lifecycle events and scoped local search. It keeps canonical evidence separate from its rebuildable retrieval index.

| Crate | Responsibility |
| --- | --- |
| `kr8-object-memory` | Typed records, source provenance, lifecycle events, profile/place types, reader rules and bounded evidence rendering |
| `kr8-capability` | Dependency-free `memory.store/1` provider contract and compatibility negotiation |
| `kr8-memory-sqlite` | Canonical SQLite record log, separate per-place FTS5 index, read-only peers and scoped diagnostics |

These are embeddable libraries. The host authenticates identities, verifies current reader access, controls storage paths and decides when memory is enabled. Constructors bind validated inputs; they do not authenticate a user or resolve membership. See [host integration](docs/HOST-INTEGRATION.md).

## Try the synthetic lifecycle example

The workspace pins Rust 1.95.0. The crate manifests declare Rust 1.88.0 as their minimum; that minimum has not yet been qualified for this standalone package.

```sh
cargo test --workspace --all-targets --locked
cargo run --locked -p kr8-memory-sqlite --example synthetic_lifecycle
```

The example uses real public record-log and FTS APIs in a fresh temporary directory. It creates and searches a synthetic note, reopens read-only peers, supersedes the note, forgets its replacement and checks replay cannot restore it. Its relay identifier is invented metadata; the example makes no network or model calls and accepts no user data path.

The standalone workspace has passed 33 tests, strict Clippy, formatting and the lifecycle example on Windows with Rust 1.95.0 and the committed dependency lock. The documentation-test command passed with zero documentation test cases. Hosted Linux and Windows CI is configured separately; its results must be checked for the selected revision. These functional checks establish the storage contract, not agent-memory efficacy or deployment security.

## Integrate

- [API and provider contract](docs/API.md)
- [Host authority and adapter boundary](docs/HOST-INTEGRATION.md)
- [Lifecycle, freshness and recovery](docs/LIFECYCLE-AND-RECOVERY.md)
- [Origins and the synthetic reader fixture](docs/ORIGINS.md)
- [Current progress](PROGRESS.md)

SQLite FTS5 is the implemented retrieval provider. The package contains no automatic capture or recall scheduler, external agent adapter, transport service, model backend or embedding provider. KR8's app-specific Tauri and ACP integration remains a consumer outside this repository. Provider interchangeability is an API contract; other adapters still need implementation and qualification.

This initial distribution targets a Git workspace. Registry publication needs separate packaging work for the sibling path dependencies and the workspace-relative synthetic fixture; crates.io portability is not yet claimed. No benchmark or best-memory claim is made.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
