# Origins and fixture context

KR8 Memory extracts the three modular memory crates from the kr8 project. The initial extraction preserved their Rust source, tests and package manifests. Standalone strict lint qualification then made two small changes: the SQLite read-only PRAGMA whitelist uses an equivalent match pattern, and an evidence-rendering test borrows a one-element slice instead of cloning it. The whitelist, default denial and test assertions are unchanged. The new root workspace, documentation and synthetic lifecycle example make that core usable as its own Git workspace.

Each package declares Apache-2.0. The originating Apache license text and applicable copyright/origin attribution are retained in [LICENSE](../LICENSE) and [NOTICE](../NOTICE). The parent project's unrelated application assets, personas and dependency notices are not part of this module. Dependencies remain separately resolved and retain their own license obligations.

## Required synthetic fixture

`kr8-object-memory/tests/reader_oracle.rs` uses `include_str!` to read `docs/kr8/research/memory-eval/dev-v1.json`. That exact 43-query development fixture is retained at the same relative path so the test keeps its existing independently authored labels and canaries.

The fixture states that its names, keys, hosts and tokens are invented. It is a deterministic reader-rule fixture, not a model quality, retrieval efficacy, performance or security-deployment result. No held-out evaluation set or real profile/relay data is included.

Its `spec` and explanatory text retain historical names such as `MEMORY-EVAL-SUITE.md` and `LAYERED-OBJECT-MEMORY.md`. Those are context labels from the originating application specification, not source includes or standalone runtime dependencies. The broader application design documents are not distributed here. This package's public [host contract](HOST-INTEGRATION.md) explains the implemented caller responsibilities.

The test's historical comment about a separate Node oracle likewise does not create a Node dependency. The runnable tests in this workspace use the Rust production reader rule and the retained synthetic JSON fixture.

The crate manifest versions and minimum Rust versions are inherited declarations. This workspace has its own reviewed dependency lock and Windows Rust 1.95.0 functional qualification, recorded in the README. Minimum-version support, other platforms and registry packaging need their own qualification. Application-context test observations do not establish those results for this repository.
