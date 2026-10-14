# Progress

Owner: the KR8 Memory maintainer. Public repository established on 2026-10-14.

| Item | State | Acceptance |
| --- | --- | --- |
| Extract three modular libraries and required synthetic fixture | Independently reviewed and published | Exact public allowlist reviewed; two public-only lint repairs disclosed in ORIGINS |
| Standalone workspace, documentation and example | Local Windows and hosted Linux/Windows checks passed | 33 tests per platform, strict Clippy, formatting and explicit lifecycle example on Rust 1.95.0 |
| Dependency resolution and lock | Generated independently and graph reviewed | Three workspace packages and 85 crates.io dependencies; checks use `--locked` |
| Source/privacy/license review | Accepted for the initial 44-file candidate | Independent source and applicable-notice review; private application sources and runtime data excluded |
| Git repository and public source publication | Published and read back | [Public repository](https://github.com/majieddd/KR8-Memory); initial commit `4da8a0578428be39e74133c8ab6cd83d0cb9ebbb` matched all 44 reviewed source blobs |
| Registry distribution | Future packaging work | Qualify sibling dependency versions and the packaged test fixture |

The accepted source scope is the reusable canonical object model, provider contract and SQLite adapter. The host authority boundary is documented in [HOST-INTEGRATION.md](docs/HOST-INTEGRATION.md). App integration, additional providers, agent adapters, automatic capture/recall and efficacy evaluation have separate work and acceptance requirements.

Local Windows compilation, 33 tests, strict lint, formatting and explicit example execution have passed. [Hosted CI](https://github.com/majieddd/KR8-Memory/actions/runs/38517123193) passed on Linux and Windows for the initial commit above. The documentation-test command passed with zero cases. Both CI dependency locks have the same parsed graph and LF-normalized bytes as the committed lock; the Windows checkout uses CRLF. Two minimal lint repairs are described in ORIGINS. Minimum-Rust-version support, registry packaging and broader integration remain separate gates. CI qualification is revision-specific.
