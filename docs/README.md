# Colossus Documentation

These pages document the active Rust runtime. Fresh installations use strict YAML,
encrypted redb state, and the native `colossus` executable. Python 0.5 documentation is
retained on `python-v0.5.0` and `python-legacy`.

The same Markdown is published as a searchable mdBook site. Build it locally with
`mdbook build` from the repository root, or use `mdbook serve --open` for live preview.
The documentation build uses the Rust-native mdBook tool and does not reintroduce a
Python package or runtime dependency.

## Start Here

- [Getting Started](GETTING_STARTED.md): install, initialize, run the offline smoke, and
  connect a model provider.
- [User Guide](USER_GUIDE.md): daily CLI, TUI, session, work, memory, research, and
  worker operations.
- [Configuration](CONFIGURATION.md): strict YAML, providers, policy, storage, sandbox,
  memory, MCP, skills, and workflows.
- [Troubleshooting](TROUBLESHOOTING.md): provider, key, policy, sandbox, worker, and
  recovery diagnostics.

## Capabilities

- [Built-in Tools](TOOLS.md)
- [Provider-Neutral Web Search](SEARCH.md)
- [Workflows](WORKFLOWS.md)
- [Integrations](INTEGRATIONS.md)
- [Skills](SKILLS.md)
- [Packs](PACKS.md)
- [Context Compaction](CONTEXT.md)
- [Terminal UX](TERMINAL_UX.md)
- [Offline and Airgapped Operation](OFFLINE_AIRGAP.md)
- [Offline Bundle Format](BUNDLE_FORMAT.md)

## Engineering And Operations

- [Architecture](ARCHITECTURE.md)
- [Security Model](SECURITY.md)
- [Rust Reconstruction Status](RUST_RECONSTRUCTION.md)
- [Rust Acceptance Matrix](RUST_ACCEPTANCE_MATRIX.md)
- [Feature Inventory](FEATURE_INVENTORY.md#22-delivery-status)
- [Installation](INSTALLATION.md)
- [Release Process](RELEASE.md)
- [Contributing](CONTRIBUTING.md)

User surfaces are interfaces only. When behavior changes, update the relevant guide and
the authoritative architecture, security, feature, or acceptance contract in the same
change.
