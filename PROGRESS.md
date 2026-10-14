# Progress

Owner: the KR8 Memory maintainer. Initial source package prepared on 2026-10-13.

| Item | State | Acceptance |
| --- | --- | --- |
| Extract three modular libraries and required synthetic fixture | Implemented, awaiting independent review | Preserve original source/test/manifest bytes and the fixture include closure |
| Standalone workspace, documentation and example | Windows checks passed | 33 tests, strict Clippy, formatting and explicit lifecycle example on Rust 1.95.0 |
| Dependency resolution and lock | Generated independently and graph reviewed | Three workspace packages and 85 crates.io dependencies; checks use `--locked` |
| Source/privacy/license review | Pending | Review the exact prospective public file allowlist and applicable notices |
| Git repository and public release | Pending | Maintainer creates, verifies and publishes the reviewed repository |
| Registry distribution | Future packaging work | Qualify sibling dependency versions and the packaged test fixture |

The accepted source scope is the reusable canonical object model, provider contract and SQLite adapter. The host authority boundary is documented in [HOST-INTEGRATION.md](docs/HOST-INTEGRATION.md). App integration, additional providers, agent adapters, automatic capture/recall and efficacy evaluation have separate work and acceptance requirements.

Windows standalone compilation, 33 tests, strict lint, formatting and explicit example execution have passed. The documentation-test command passed with zero cases. Two minimal lint repairs are described in ORIGINS. Hosted Linux/Windows checks, minimum-Rust-version support, registry packaging and broader integration remain separate gates. Publication status is established by the actual repository, not this preparation table.
