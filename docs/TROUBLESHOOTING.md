# Troubleshooting

Use this guide when Colossus behaves differently than expected. Most failures fall into
one of four buckets: provider/model readiness, tool policy or approval, workspace scope,
or credentials.

## First Checks

```bash
uv run colossus config show
uv run colossus models list
uv run colossus tools list
uv run colossus run "Reply with exactly: ok"
```

If the echo provider is active, the run is a harness smoke test, not evidence that a real
model endpoint is ready.

## Local Model Endpoint Fails

Check the selected provider and base URL:

```bash
uv run colossus --provider local-openai-chat \
  --base-url http://localhost:8000/v1 \
  provider doctor
```

Then try a tiny no-tools prompt:

```bash
uv run colossus --provider local-openai-chat \
  --base-url http://localhost:8000/v1 \
  run "Reply with exactly: ok"
```

If the raw server returns HTTP 200 but Colossus reports no assistant content or tool
calls, the model may be returning hidden reasoning without usable assistant output.
Treat that as a model/endpoint behavior issue, not a successful turn.

## Tool Is Missing

Run:

```bash
uv run colossus tools list
```

Some tools are intentionally hidden by default:

- `web.search` appears only when a search adapter is configured.
- `mcp.call` appears only when MCP execution is explicitly configured.
- `github.*`, `searxng.*`, `opensearch.*`, and `openapi.NAME.*` appear only after the
  integration is connected.

## Tool Requires Approval

Network-capable tools, mutating tools, high-risk tools, and explicit approval-required
tools pause unless the approval mode allows them.

```bash
uv run colossus run --approval-mode ask "Use shell.run with argv [\"echo\", \"ok\"]."
uv run colossus run --approval-mode full-access "Use shell.run with argv [\"echo\", \"ok\"]."
```

`full-access` removes prompts for approval-required tools, but deterministic policy
denies still apply.

## Workspace Looks Wrong

Show or set the workspace:

```bash
uv run colossus run --workspace ../my-project "hello"
uv run colossus tools list --workspace ../my-project
```

Inside the REPL:

```text
/workspace show
/workspace ../my-project
```

Filesystem, shell, repo, research, memories, context, and subagent behavior are scoped to
the active workspace root.

## Integration Auth Fails

Use credential refs, not raw secrets:

```bash
export GITHUB_TOKEN=...
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus integrations show github
```

If the environment variable is missing, Colossus will refuse to connect or create a
pending-auth connection. Secret values should not appear in CLI output, model requests,
transcripts, or audit payloads.

## Web Search Or MCP Does Nothing

Deep Research Mode can request repo, web, and MCP lanes, but unavailable lanes are
recorded as warnings and skipped.

```bash
uv run colossus research "question" --source repo
uv run colossus research "question" --source web
```

For web search, configure DuckDuckGo or SearXNG in [Configuration](CONFIGURATION.md).
For MCP, configure explicit servers and allowlisted tools.

## Context Feels Stale

Inspect context:

```text
/context
/compact
/status
```

Context snapshots reduce provider input but raw messages remain persisted. Restoring an
older snapshot changes future model input and is approval-required.

## Still Stuck

Capture:

- the exact command,
- `uv run colossus config show`,
- `uv run colossus tools list`,
- provider doctor output if a model endpoint is involved,
- the active workspace path,
- whether the failing tool requires approval.

Avoid including raw API keys, bearer tokens, refresh tokens, private keys, or
service-account JSON in reports.
