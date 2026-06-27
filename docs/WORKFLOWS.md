# Workflows

These recipes are small, repeatable paths for common Colossus tasks.

## Smoke Test A Checkout

```bash
uv sync --extra dev
uv run colossus run "hello"
uv run colossus tools list
uv run pytest
```

## Inspect A Repository

```bash
uv run colossus run --workspace ../my-project \
  "Map the repository and identify the main test commands."
```

Inside the REPL:

```text
/workspace ../my-project
/tools
Inspect the repository and summarize the likely entry points.
```

## Make A Small Code Change

```bash
uv run colossus repl --workspace ../my-project --approval-mode ask
```

Suggested REPL flow:

```text
@skill:coding implement the requested fix
/tools
/status
run the focused tests
```

Review changes with normal git commands before committing.

## Continue Prior Work

```bash
uv run colossus repl --resume
```

Inside the REPL:

```text
/resume
/session show
/tasks
/decisions
/context
```

## Run Deep Research

Repository-only:

```bash
uv run colossus research --source repo \
  "What are the highest-risk security boundaries in this checkout?"
```

Interactive:

```text
/research on
How does the tool approval path work?
/research sources
```

## Connect GitHub

```bash
export GITHUB_TOKEN=...
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus tools list
```

Then ask for repository, issue, pull request, check, or release context. GitHub tools are
network-capable, so they still require approval unless approval mode auto-approves them.

## Import An Internal API

```bash
export INTERNAL_API_TOKEN=...
uv run colossus integrations import-openapi internal ./openapi.json \
  --base-url https://internal.example.test \
  --credential-ref env:INTERNAL_API_TOKEN
uv run colossus tools list
```

Ask Colossus to use the generated `openapi.internal.*` tools. Keep write or mutation
operations behind explicit approval.

## Prepare Offline

```bash
uv run colossus tools list
uv run colossus config show
uv run pytest
```

Before isolation, prepare wheels, local model endpoints, and offline bundles. See
[Offline and Airgapped Operation](OFFLINE_AIRGAP.md).
