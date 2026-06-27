# User Guide

This guide describes how to use Colossus as a local coding and research agent.

## One-Shot Runs

Use `run` for a single prompt:

```bash
uv run colossus run "Summarize this repository"
uv run colossus run --workspace ../my-project "Find likely failing tests"
uv run colossus run --stream --events compact "Inspect the tool surface"
```

Use `--approval-mode ask` when you want approval prompts for risky tools, and
`--approval-mode full-access` when you intentionally want no prompts for
approval-required tools.

## REPL

Start the REPL:

```bash
uv run colossus repl
```

Core commands:

```text
/help
/status
/tools
/workspace [PATH]
/session show|resume|latest|new
/sessions [LIMIT]
/resume [LIMIT]
/context
/compact
/exit
```

Display commands:

```text
/stream on|raw|off
/events compact|verbose|off
/reasoning on|off
/transcript comfortable|compact
/theme [NAME]
/repl prefs|save|reset
```

## Sessions

Sessions are local SQLite records. Start fresh by default, resume explicitly, or pick
from recent sessions:

```bash
uv run colossus run --resume "continue where we left off"
uv run colossus repl --resume
uv run colossus sessions list
uv run colossus sessions show SESSION_ID
```

Inside the REPL:

```text
/resume
/sessions
/session resume SESSION_ID
/session new
```

## Tools

Inspect the active model-callable catalog:

```bash
uv run colossus tools list
```

Inside the REPL:

```text
/tools
```

The catalog is composed from built-ins plus configured integration tools. Network-capable
and mutation tools still pass through policy, approval, and audit. See
[Built-in Tools](TOOLS.md) for full details.

## Context, Decisions, And Memories

Colossus persists session history and can compact older context into snapshots.

```text
/context
/compact
```

Task, decision, and memory commands help keep long work coherent:

```text
/tasks
/decision This API boundary must stay in application, not interfaces
/decisions
/memory Remember that this repo prefers env credential refs
/memories
```

Memories are context, not instructions. Do not store secrets in memories.

## Skills

Skill Mode is enabled for normal agent turns. Activate skills with prompt mentions,
one-shot options, or REPL sticky state:

```bash
uv run colossus run --skill coding "Implement the approved plan"
```

```text
@skill:coding implement this
/skill show
/skill use coding
/skill drop coding
```

See [Skills](SKILLS.md) for authoring and safety guidance.

## Deep Research

Deep Research Mode collects bounded repository evidence and optional configured web/MCP
sources into a persisted cited report.

```bash
uv run colossus research "How does context compaction work?"
uv run colossus research --workspace ../my-project "Find risky code paths" --source repo
```

Inside the REPL:

```text
/research on
/research show
/research sources
/research How should integrations be secured?
```

Web search and MCP collection only run when configured and approved. Disabled lanes are
recorded as limitations rather than bypassed.

## Integrations

Integrations are hidden until configured and connected:

```bash
uv run colossus integrations list
uv run colossus integrations show github
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus integrations connect searxng --base-url http://localhost:8888
uv run colossus integrations connect opensearch \
  --base-url http://localhost:9200 \
  --auth-type none
uv run colossus tools list
```

Inside the REPL:

```text
/integrations list
/integrations show github
/integrations connect github --credential-ref env:GITHUB_TOKEN
/integrations connect searxng --base-url http://localhost:8888
/integrations connect opensearch --base-url http://localhost:9200 --auth-type none
```

See [Integrations](INTEGRATIONS.md) for GitHub, SearXNG, OpenSearch, OpenAPI import, MCP
positioning, and credential rules.
