# Colossus

Colossus is an auditable agent and workflow runtime written in Rust. It combines a
bounded model/tool loop, durable YAML workflows, an encrypted event journal, policy-bound
effects, replaceable memory indexes, and an authenticated local worker.

The Rust alpha lives under [`rust/`](rust/README.md) until the P0+P1 cutover. Python 0.5
is frozen at the `python-v0.5.0` tag and on the `python-legacy` branch; new installations
use fresh Rust YAML and fresh redb state.

## Quick Start

Rust 1.96 and edition 2024 are the source-build contract:

```bash
cargo run --offline --manifest-path rust/Cargo.toml \
  -p colossus-cli --bin colossus-rs -- config init
cargo run --offline --manifest-path rust/Cargo.toml \
  -p colossus-cli --bin colossus-rs -- run "hello"
cargo run --offline --manifest-path rust/Cargo.toml \
  -p colossus-cli --bin colossus-rs -- audit verify
```

The generated `echo` profile needs no provider credential or network. Native release
archives contain `install.sh` or `install.ps1`; see [Installation](docs/INSTALLATION.md).

With an installed binary:

```bash
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml run "Reply with exactly: connected"
colossus --config .colossus/config.yaml repl
```

To operate on another repository, start Colossus from that repository and pass an
absolute configuration path. The working directory selects workspace identity; it does
not expand configured policy or sandbox grants.

## What It Provides

- OpenAI Responses, OpenAI-compatible, and credential-free echo providers with role
  routing, streaming, strict tool schemas, and durable multi-turn sessions.
- Filesystem, Git, process, network, memory, research, integration, MCP, skill, pack,
  workflow, goal, and subagent operations routed through one effect gateway.
- Built-in deny-by-default policy or strict OPA decisions, explicit approval proofs,
  one-use permits, bounded quarantine, and post-effect content release.
- XChaCha20-Poly1305 journal payloads, hash chaining, signed checkpoints, secure anchors,
  recovery mode, redacted audit views, and durable external evidence export.
- Canonical redb repositories, disposable projections, Tantivy lexical memory, optional
  Chroma semantic memory, and an authenticated worker over local IPC.

## Documentation

- [Getting Started](docs/GETTING_STARTED.md)
- [User Guide](docs/USER_GUIDE.md)
- [Configuration](docs/CONFIGURATION.md)
- [Built-in Tools](docs/TOOLS.md)
- [Workflows](docs/WORKFLOWS.md)
- [Integrations](docs/INTEGRATIONS.md)
- [Security Model](docs/SECURITY.md)
- [Rust Reconstruction Status](docs/RUST_RECONSTRUCTION.md)
- [Feature Inventory](docs/FEATURE_INVENTORY.md)

## Development

Run the authoritative Rust gates from `rust/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets
```

The frozen Python implementation is maintained only on its tag and legacy branch; its
commands, configuration, state, and packaging are not part of the active Rust contract.
