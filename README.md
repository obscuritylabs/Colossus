# Colossus

Colossus is being reconstructed in Rust as an event-sourced, policy-enforced workflow
runtime. The active alpha lives in [`rust/`](rust/README.md); the passing Python 0.5
implementation is frozen at `python-v0.5.0` and on `python-legacy` until Rust completes
the P0+P1 cutover.

Colossus is a secure, layered Python CLI harness for agentic development. It is built
for OpenAI-compatible online providers, local OpenAI-compatible offline endpoints,
bundled skills, brokered tools, local-first state, and auditability.

The default provider is deterministic and credential-free, so new checkouts and
airgapped environments can exercise the harness before any model endpoint is configured.

## Quick Start

Rust foundation smoke test (fresh YAML and fresh encrypted state):

```bash
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- config init
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- echo hello
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- run --stream "hello"
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- audit verify
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- state doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- sandbox doctor
```

The current Python 0.5 surface remains available during reconstruction:

```bash
uv sync --extra dev
uv run colossus run "hello"
uv run colossus repl
uv run pytest
```

Choose a workspace for repository-scoped work:

```bash
uv run colossus run --workspace ../my-project "Inspect the failing tests"
uv run colossus repl --workspace ../my-project
```

Resume prior local sessions:

```bash
uv run colossus run --resume "continue where we left off"
uv run colossus repl --resume
```

Run deep research:

```bash
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- \
  research run "Summarize the local tool security posture" --source repo
```

Connect an integration without exposing raw secrets to the model:

```bash
export GITHUB_TOKEN=...
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus integrations connect searxng --base-url http://localhost:8888
docker compose -f docker-compose.opensearch.yml up -d
uv run colossus integrations connect opensearch \
  --base-url http://localhost:9200 \
  --auth-type none
uv run colossus tools list
```

Initialize a user config when you are ready to use a non-default provider:

```bash
uv run colossus config init
uv run colossus config show
uv run colossus models list
```

## Documentation

[Documentation Home](docs/README.md) is the canonical index.

Start here:

- [Getting Started](docs/GETTING_STARTED.md)
- [User Guide](docs/USER_GUIDE.md)
- [Workflows](docs/WORKFLOWS.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)

Capability docs:

- [Product requirements](docs/FEATURE_INVENTORY.md)
- [Built-in Tools](docs/TOOLS.md)
- [Integrations](docs/INTEGRATIONS.md)
- [Skills](docs/SKILLS.md)
- [Packs](docs/PACKS.md)
- [Context compaction](docs/CONTEXT.md)

Reference docs:

- [Configuration](docs/CONFIGURATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Security model](docs/SECURITY.md)
- [Rust reconstruction status](docs/RUST_RECONSTRUCTION.md)
- [Rust foundational acceptance matrix](docs/RUST_ACCEPTANCE_MATRIX.md)
- [Offline and airgapped operation](docs/OFFLINE_AIRGAP.md)
- [Release process](docs/RELEASE.md)

## Capabilities

Colossus ships an offline-first local coding tool loop:

- Workspace file list/read/search/write/replace.
- Git status/diff/show and structured `shell.run`.
- Model-callable task, key decision, memory, plan, patch, repo context, subagent, trace,
  context, and skill-authoring tools. Repository verification runs through structured
  shell commands or explicitly installed pack tools.
- Web/docs fetch tools plus opt-in web search and MCP calls when adapters are explicitly
  configured.
- Connected integration tools for GitHub, SearXNG, OpenSearch, and imported OpenAPI
  specs, exposed only after connection configuration and policy validation.
- Automatic context compaction with durable snapshots, active key-decision injection,
  relevant memory injection, and per-model context windows.
- Session discovery and explicit resume for prior local conversations.
- Named model roles for primary agent turns, context summarization, subagents, and
  shell-command risk review, plus research planner/worker/synthesizer turns.

The package follows dependency-inward layering:

- `domain`: typed values, events, specs, decisions, memories, and errors.
- `ports`: protocols for model providers, tools, state, skills, audit, and approvals.
- `application`: orchestration, skill resolution, tool execution, and service assembly.
- `adapters`: OpenAI-compatible providers, SQLite state, package/filesystem skills,
  subprocess broker, integration runtimes, and audit log implementations.
- `interfaces`: Typer CLI and prompt-toolkit REPL.
- `infrastructure`: config, package resources, logging, and bundle verification.

## Development

Use the same commands locally that CI runs:

```bash
uv run pytest
uv run ruff check .
uv run mypy src/colossus
uv run python -m build
```

Colossus targets Python 3.12 and is packaged with Hatchling.

The Rust alpha uses Rust 1.96 and edition 2024:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
