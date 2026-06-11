# Colossus

Colossus is a secure, layered Python CLI harness for agentic development. It is built
for OpenAI-compatible online providers, local OpenAI-compatible offline endpoints,
bundled skills, brokered tools, local-first state, and auditability.

The default provider is deterministic and credential-free, so new checkouts and
airgapped environments can exercise the harness before any model endpoint is configured.

## Quick Start

```bash
uv sync --extra dev
uv run colossus run "hello"
uv run colossus repl
uv run colossus tui
uv run pytest
```

For a more Codex-like activity stream, the REPL enables streamed assistant output,
comfortable transcript blocks, compact tool/risk/approval events, a themed prompt band,
a power-user status bar, prompt history, and a theme-specific quiet activity spinner when
event blocks are hidden. One-shot runs can use the same event renderer:

```bash
uv run colossus run --stream --events compact "hello"
```

REPL display choices can be previewed and persisted:

```bash
uv run colossus repl --theme high-contrast
# then inside the REPL: /theme preview, /transcript compact, /theme save, /repl prefs
```

Initialize a user config when you are ready to use a non-default provider:

```bash
uv run colossus config init
uv run colossus config show
uv run colossus models list
```

Provider, model, base URL, API key environment variable, and CA bundle can also be
overridden per invocation:

```bash
uv run colossus --provider local-openai-chat --base-url http://localhost:8000/v1 run "hello"
uv run colossus --provider openai-responses --model gpt-4.1-mini --api-key-env OPENAI_API_KEY run "hello"
```

For approval-required tools, one-shot runs can prompt or use model-gated auto approval:

```bash
uv run colossus run --approval-mode ask "Use shell.run with argv [\"echo\", \"ok\"]."
uv run colossus run --approval-mode risk-auto "Use shell.run with argv [\"echo\", \"ok\"]."
```

## Documentation

- [Installation](docs/INSTALLATION.md)
- [Configuration](docs/CONFIGURATION.md)
- [Context compaction](docs/CONTEXT.md)
- [Offline and airgapped operation](docs/OFFLINE_AIRGAP.md)
- [Offline bundle format](docs/BUNDLE_FORMAT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Security model](docs/SECURITY.md)
- [Built-in tools](docs/TOOLS.md)
- [Skills](docs/SKILLS.md)
- [Release process](docs/RELEASE.md)

## Architecture

The package follows dependency-inward layering:

- `domain`: typed values, events, specs, decisions, and errors.
- `ports`: protocols for model providers, tools, state, skills, audit, and approvals.
- `application`: orchestration, skill resolution, tool execution, and service assembly.
- `adapters`: OpenAI-compatible providers, SQLite state, package/filesystem skills,
  subprocess broker, and audit log implementations.
- `interfaces`: Typer CLI, prompt-toolkit REPL, and Textual TUI.
- `infrastructure`: config, package resources, logging, and bundle verification.

Bundled first-party skills live under `src/colossus/bundled_skills/` and are shipped
as package data.

## Built-in Tools

Colossus ships an offline-first local coding tool loop:

- Workspace file list/read/search/write/replace.
- Git status/diff/show and structured `shell.run`.
- Model-callable task, plan, patch, repo context, subagent, trace, eval, and verification
  tools.
- Web/docs and MCP tool schemas that are visible but disabled by default until a
  network-enabled profile or adapter is explicitly configured.
- Automatic context compaction with durable snapshots and per-model context windows.
- Named model roles for primary agent turns, context summarization, subagents, and
  shell-command risk review.

Inspect the current catalog with:

```bash
uv run colossus tools list
```

## Development

Use the same commands locally that CI runs:

```bash
uv run pytest
uv run ruff check .
uv run mypy src/colossus
uv run python -m build
```

Colossus targets Python 3.12 and is packaged with Hatchling.
