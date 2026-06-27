# Getting Started

This guide gets a new checkout to a working local Colossus session. The default provider
is `echo`, so the first smoke test does not need network access or credentials.

## Install

From the repository root:

```bash
uv sync --extra dev
uv run colossus run "hello"
```

Expected output includes a deterministic echo response:

```text
[echo:default] hello
```

Run the development checks:

```bash
uv run pytest
uv run ruff check .
uv run mypy src/colossus
```

## Start The REPL

```bash
uv run colossus repl
```

Useful first commands:

```text
/help
/status
/tools
/workspace show
/exit
```

The REPL supports sessions, prompt history, streamed assistant output, compact tool and
approval events, workspace switching, Skill Mode, Deep Research Mode, and integration
management.

## Choose A Workspace

Colossus uses the current directory as the workspace by default. Workspace-scoped tools,
repo research, context, memories, shell commands, and subagents stay relative to that
root.

```bash
uv run colossus run --workspace ../my-project "Inspect this repository"
uv run colossus repl --workspace ../my-project
```

Inside the REPL:

```text
/workspace show
/workspace ../other-project
```

## Understand Approvals

One-shot runs default to blocking approval-required tools. Interactive runs default to
asking.

```bash
uv run colossus run --approval-mode ask "Use shell.run with argv [\"echo\", \"ok\"]."
uv run colossus run --approval-mode risk-auto "Use shell.run with argv [\"echo\", \"ok\"]."
uv run colossus run --approval-mode full-access "Use shell.run with argv [\"echo\", \"ok\"]."
```

`full-access` auto-approves approval-required tools. It does not change filesystem
roots, tool schemas, network adapters, or deterministic policy denies.

## Configure A Real Model

Create and inspect config:

```bash
uv run colossus config init
uv run colossus config show
uv run colossus models list
```

Override the provider for a single run:

```bash
uv run colossus --provider local-openai-chat \
  --base-url http://localhost:8000/v1 \
  run "Reply with exactly: ok"
```

For full provider and model role details, see [Configuration](CONFIGURATION.md).

## Next Steps

- Read the [User Guide](USER_GUIDE.md) for day-to-day usage.
- Try the recipes in [Workflows](WORKFLOWS.md).
- Connect GitHub or import an OpenAPI spec with [Integrations](INTEGRATIONS.md).
- Use [Troubleshooting](TROUBLESHOOTING.md) when a model, tool, or credential behaves
  strangely.
