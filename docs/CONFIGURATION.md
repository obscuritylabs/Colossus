# Configuration

Colossus loads `config.json` from the platform user config directory. If the file does
not exist, Colossus uses the built-in defaults.

For task-oriented setup, start with [Getting Started](GETTING_STARTED.md). For daily
command usage, see the [User Guide](USER_GUIDE.md).

Create a default config:

```bash
uv run colossus config init
```

Show the resolved config:

```bash
uv run colossus config show
```

Overwrite an existing generated config:

```bash
uv run colossus config init --force
```

## Config schema

The current config is strict: unknown fields are rejected.

```json
{
  "provider": {
    "kind": "echo",
    "model": "default",
    "base_url": null,
    "api_key_env": null,
    "ca_bundle": null,
    "model_context_windows": {}
  },
  "models": {
    "profiles": {},
    "roles": {}
  },
  "context": {
    "auto_compaction": true,
    "default_context_window_tokens": 32768,
    "compact_at_percent": 0.7,
    "target_percent": 0.45,
    "recent_tail_messages": 8,
    "model_assisted": true
  },
  "agent": {
    "max_turns": 24
  },
  "subagents": {
    "max_concurrent": 4
  },
  "memory": {
    "index": {
      "kind": "sqlite_fts"
    }
  },
  "http": {
    "ca_bundle": null,
    "client_cert": null,
    "client_key": null,
    "client_key_password_env": null,
    "proxy_url": null,
    "proxy_url_env": null,
    "trust_env": true
  },
  "research": {
    "default_depth": "standard",
    "max_sources": 20,
    "max_workers": 4,
    "sources": ["repo", "web", "mcp"],
    "search": {
      "kind": "disabled",
      "endpoint": "https://duckduckgo.com/html/",
      "api_key_env": null,
      "auth_header": "Authorization",
      "auth_scheme": "bearer",
      "user_agent": "colossus-agent/0.1"
    },
    "mcp": {
      "servers": {}
    }
  },
  "allow_user_skill_overrides": false
}
```

## Providers

`provider.kind` supports these values:

- `echo`: deterministic local provider. No credentials or network are required.
- `openai_responses`: OpenAI Responses API provider. Requires an API key.
- `local_openai_chat`: OpenAI-compatible local chat completions endpoint.

Provider command-line aliases use hyphenated names:

```bash
uv run colossus --provider echo run "hello"
uv run colossus --provider openai-responses --model gpt-4.1-mini run "hello"
uv run colossus --provider local-openai-chat --base-url http://localhost:8000/v1 run "hello"
```

## Model roles

Use `models.profiles` and `models.roles` when different jobs should use different
models. If this block is empty, Colossus maps the legacy `provider` config to `primary`
and reuses it for `risk_evaluator`, `context_summarizer`, `subagent_default`,
`research_planner`, `research_worker`, and `research_synthesizer`.

```json
{
  "models": {
    "profiles": {
      "main": {
        "provider": "local_openai_chat",
        "model": "coding-model",
        "base_url": "http://localhost:12434/v1",
        "api_key_env": null,
        "ca_bundle": null,
        "context_window_tokens": 65536
      },
      "risk": {
        "provider": "local_openai_chat",
        "model": "risk-model",
        "base_url": "http://localhost:12434/v1"
      }
    },
    "roles": {
      "primary": "main",
      "risk_evaluator": "risk",
      "context_summarizer": "main",
      "subagent_default": "main",
      "research_planner": "main",
      "research_worker": "main",
      "research_synthesizer": "main"
    }
  }
}
```

Inspect resolved roles:

```bash
uv run colossus models list
uv run colossus models doctor --role risk_evaluator
uv run colossus run --model-role risk_evaluator "hello"
```

Global provider/model/base-url/API-key/CA CLI overrides apply to the `primary` role for
that invocation.

## Agent Runtime

`agent.max_turns` controls the maximum number of model/tool turns in a normal agent run.
The default is `24`. It can be overridden for a one-shot run with `--max-turns`, for a
REPL session with `repl --max-turns`, or inside the REPL with `/agent max-turns N`.

## Workspace Selection

Colossus uses the current directory as the workspace root by default. Workspace-bound
tools, shell commands, repo-scoped memories, repository research, context composition,
and subagents stay relative to that root.

Use `--workspace` or `-C` to choose a different root for one-shot runs, research, tool
inspection, or the REPL:

```bash
uv run colossus run --workspace ../my-project "Inspect this repository"
uv run colossus research --workspace ../my-project "Summarize local evidence" --source repo
uv run colossus repl --workspace ../my-project
```

Inside the REPL, `/workspace` shows the current root and `/workspace PATH` switches the
active workspace for later tool calls, context checks, memories, and research runs.
Relative REPL workspace paths are resolved from the current workspace root.

## Integrations

Integrations are persisted local connection records, not config-file secrets. The first
credential adapter resolves refs of the form `env:VARIABLE_NAME`; Colossus stores the ref
and injects the secret only inside the tool handler.

For the integration user guide, see [Integrations](INTEGRATIONS.md).

```bash
export GITHUB_TOKEN=...
uv run colossus integrations list
uv run colossus integrations show github
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus tools list
uv run colossus integrations disconnect github
```

Inside the REPL, use `/integrations list`, `/integrations show github`, and
`/integrations connect github --credential-ref env:GITHUB_TOKEN`. A successful connect
refreshes the live tool catalog.

Imported OpenAPI tools use the same brokered runtime:

```bash
export DEMO_API_TOKEN=...
uv run colossus integrations import-openapi demo ./openapi.json \
  --base-url https://api.example.test \
  --credential-ref env:DEMO_API_TOKEN \
  --auth-type bearer
```

Supported v1 auth labels are `none`, `api-key`, `bearer`,
`oauth2-authorization-code`, and `service-account`. The current local broker resolves
environment refs only; future keychain or encrypted-store adapters should keep the same
credential-ref contract.

OpenSearch is configured through the integration connection record rather than config
files. Use cluster permissions and least-privilege credentials for index access:

```bash
docker compose -f docker-compose.opensearch.yml up -d
uv run colossus integrations connect opensearch \
  --base-url http://localhost:9200 \
  --auth-type none

export OPENSEARCH_TOKEN=...
uv run colossus integrations connect opensearch \
  --base-url https://search.example.test \
  --auth-type bearer \
  --credential-ref env:OPENSEARCH_TOKEN

export OPENSEARCH_USER=...
export OPENSEARCH_PASSWORD=...
uv run colossus integrations connect opensearch \
  --base-url https://search.example.test \
  --auth-type basic \
  --username-ref env:OPENSEARCH_USER \
  --password-ref env:OPENSEARCH_PASSWORD
```

For Amazon OpenSearch Service, put SigV4 signing in a proxy for v1 and connect Colossus
to that proxy with one of the supported auth modes.

The local compose file disables the OpenSearch security plugin and binds to localhost
only. Use it for development and opt-in live integration testing, not production.

## Deep Research

Deep Research Mode is available with:

```bash
uv run colossus research "question"
uv run colossus research "question" --source repo --depth quick
```

The default source preference is `repo`, `web`, and `mcp`, but web search and MCP
collection only run when configured and approved. With default config, research degrades
to local repository evidence and records unavailable source lanes as warnings.
When attached to a session, research uses bounded prior session context and appends the
completed cited report back to that session for later chat turns.

Enable DuckDuckGo-backed web search:

```json
{
  "research": {
    "search": {
      "kind": "duckduckgo"
    }
  }
}
```

Enable self-hosted SearXNG-backed web search. The SearXNG instance must have `json`
enabled under `search.formats`; public instances often disable JSON output or rate-limit
automation.

Start the local development instance:

```bash
docker compose -f docker-compose.searxng.yml up -d
curl 'http://localhost:8888/search?q=colossus&format=json'
```

```json
{
  "research": {
    "search": {
      "kind": "searxng",
      "endpoint": "http://localhost:8888/search"
    }
  }
}
```

For a protected SearXNG instance, keep the secret in the environment and reference only
the variable name from config:

```json
{
  "research": {
    "search": {
      "kind": "searxng",
      "endpoint": "https://search.example.test",
      "api_key_env": "SEARXNG_API_KEY",
      "auth_header": "Authorization",
      "auth_scheme": "bearer"
    }
  }
}
```

For model-callable SearXNG tools, connect the native integration instead of editing the
research search config:

```bash
uv run colossus integrations connect searxng --base-url http://localhost:8888
uv run colossus tools list
```

The integration stores endpoint config and optional credential refs in local connection
state. It does not place raw SearXNG keys in tool schemas or model requests.

Configure MCP research tools with explicit stdio server commands and allowlisted tools.
The gateway uses the official MCP Python SDK when installed:

```json
{
  "research": {
    "mcp": {
      "servers": {
        "docs": {
          "command": "mcp-docs-server",
          "args": [],
          "allowed_tools": ["search_docs"],
          "research_tools": [
            {
              "tool": "search_docs",
              "arguments": {"query": "{query}"},
              "title": "Docs search"
            }
          ]
        }
      }
    }
  }
}
```

## Subagents

`subagents.max_concurrent` controls how many queued child-agent jobs may run at the same
time in a Colossus process. The default is `4`. Subagents use the `subagent_default`
model role unless a delegated job names another role.

Inspect durable jobs with:

```bash
uv run colossus agents list
uv run colossus agents show agent-123
uv run colossus agents cancel agent-123
```

## Run and REPL display

One-shot runs show compact activity events by default. Add `--stream` to print assistant
text as model deltas arrive, and use `--events` to choose how much event detail to show:

```bash
uv run colossus run --stream --events compact "hello"
uv run colossus run --events verbose "Use filesystem.read on pyproject.toml."
uv run colossus run --events off "hello"
```

`--trace` remains a compatibility alias for compact event output. Provider-supplied
reasoning summaries are shown by default; use `--no-reasoning` to hide them.

The REPL starts with streaming, compact events, comfortable transcript blocks, prompt
history, a styled prompt band, and a dense status bar enabled. The prompt band highlights
the `colossus` title, mode badge, active model, and caret so the input location is
visually distinct from the transcript. The transcript renders user prompts, assistant
text, safe reasoning summaries, tool calls/results, approvals, risks, and errors with
theme-aware spacing and colors. The status bar shows composer mode, active model
role/model, theme, approval mode, stream/events/reasoning settings, session id, cursor
position, draft chars/lines, cached context budget, message count, latest snapshot, and
the current `tasks=open/total` summary for the session, and last run status.

The prompt bottom bar is owned by the active input composer, so it naturally disappears
after submit. While a run is active, Colossus keeps compact orientation data visible in a
bounded transient activity indicator using the theme-specific spinner and current phase,
such as `Thinking...`, `Using filesystem.read...`, or `Responding...`.

Composer behavior:

- Single-line mode is the default; `Enter` submits.
- Multiline mode makes `Enter` insert a newline and `Esc+Enter` submit.
- Prompt history is stored as `repl_history.txt` under the Colossus data directory.

Runtime controls:

- `/stream on|raw|off`
- `/events compact|verbose|off`
- `/reasoning on|off`
- `/workspace [PATH]`
- `/resume [LIMIT]`
- `/sessions [LIMIT]`
- `/session show [ID]`
- `/session resume <id>`, `/session latest`, or `/session new`
- `/tasks [open|all|STATUS]`
- `/research [on|off|show|sources|QUESTION]`
- `/decisions [all|STATUS]`
- `/decision <text>` or `/decision archive <id>` or `/decision supersede <id> <text>`
- `/memories [all|STATUS]`
- `/memory <text>` or `/memory search <query>` or `/memory archive <id>` or `/memory supersede <id> <text>`
- `/transcript comfortable|compact`
- `/multiline on|off|toggle`
- `/theme [NAME]`
- `/theme preview [NAME]`
- `/theme save [NAME]`
- `/theme reset`
- `/repl prefs`
- `/repl save`
- `/repl reset`
- `/status`
- `/help`
- `/trace` toggles compact events on and off for compatibility.

Compact events keep the REPL transcript chat-friendly: assistant text, safe reasoning
summaries, tool calls, approval prompts, risk assessments, and collapsed tool result
previews remain visible, while local submit metrics and the final `done` marker are
hidden. `/events off` hides those event blocks but keeps a single-line activity spinner
visible while the run is active, such as `Thinking...`, `Using filesystem.read...`, or
`Reviewing risk for shell.run...`. Use `/events verbose` when debugging run metadata
such as prompt size, session/context counters, the composed model request, completion
markers, and larger tool details. Use `/transcript compact` for a tighter terminal
stream, or `/transcript comfortable` for the default Pi-like spacing.

REPL preferences are stored in the local SQLite state database under the Colossus data
directory. Saved preferences currently include theme, multiline mode, model output
streaming, event detail, transcript style, and reasoning-summary visibility. `--theme`
overrides the saved theme for that REPL launch only; use `/theme save` or `/repl save` to
persist a choice.

You can choose a one-launch startup theme with:

```bash
uv run colossus repl --theme high-contrast
```

Built-in themes are `default`, `mono`, `high-contrast`, `carrot`, and `hacker`. Add user
themes as JSON or TOML files under the Colossus config directory:

```text
<config-dir>/colossus/themes/ocean.json
```

The file is data-only and may override a subset of prompt, toolbar, and trace styles:

```json
{
  "name": "ocean",
  "title": "colossus",
  "caret": ">",
  "continuation": "|",
  "styles": {
    "prompt.caret": "#00ffff bold",
    "bottom-toolbar.key": "bg:#102a2a #00ffff bold"
  },
  "trace": {
    "thinking": "bold cyan",
    "tool_call": "bold #00afff",
    "risk_assessment": "bold yellow"
  },
  "transcript": {
    "user": "#d7ffd7 on #12331d",
    "assistant": "#d7ffd7",
    "tool": "bold #00d7ff",
    "activity_spinner": "line"
  }
}
```

Unsupported style keys, non-string values, invalid Rich spinner names, and non-simple
theme names are rejected at startup. Use `/theme preview ocean` inside the REPL to
inspect prompt, toolbar, event, transcript colors, and the theme's activity spinner
before saving.

## Approval modes

One-shot runs default to `deny` for approval-required tools. Use `ask` to prompt, or
`risk-auto` to let the `risk_evaluator` model auto-approve only low-risk tool calls that
explicitly recommend `allow`.

```bash
uv run colossus run --approval-mode ask "Use shell.run with argv [\"echo\", \"ok\"]."
uv run colossus run --approval-mode risk-auto "Use shell.run with argv [\"echo\", \"ok\"]."
```

`risk-auto` still asks the user for medium/high/unclear calls, denies deterministic policy
violations, and falls back to normal approval if risk review is unavailable.

## Credentials

For OpenAI Responses, set `provider.api_key_env` to the environment variable that holds
the API key. If unset, Colossus falls back to `OPENAI_API_KEY`.

```json
{
  "provider": {
    "kind": "openai_responses",
    "model": "gpt-4.1-mini",
    "base_url": "https://api.openai.com/v1",
    "api_key_env": "OPENAI_API_KEY",
    "ca_bundle": null
  },
  "allow_user_skill_overrides": false
}
```

Per-command overrides are available for local testing:

```bash
uv run colossus --provider openai-responses --api-key-env OPENAI_API_KEY run "hello"
```

`--api-key` is supported for one-off use, but an environment variable is preferred for
shared shells and logs.

## TLS trust

Use global `http.ca_bundle` or `--http-ca-bundle` when Colossus-owned HTTPS clients
need an enterprise or private CA. This applies to model providers, provider diagnostics,
`web.fetch`, `docs.fetch`, and configured web search providers.

The older provider-level `provider.ca_bundle` and `--ca-bundle` settings are still
supported for model providers and override `http.ca_bundle` for provider calls.

```bash
uv run colossus --ca-bundle ./certs/company-ca.pem --provider local-openai-chat run "hello"
```

For mTLS or PKI-protected HTTP sites, configure a client certificate and optional key:

```json
{
  "http": {
    "ca_bundle": "./certs/company-ca.pem",
    "client_cert": "./certs/client.pem",
    "client_key": "./certs/client.key",
    "client_key_password_env": "COLOSSUS_HTTP_CLIENT_KEY_PASSWORD"
  }
}
```

## HTTP proxy

Use `http.proxy_url` or `--http-proxy` to send Colossus-owned HTTP clients through a
proxy. Prefer `http.proxy_url_env` or `--http-proxy-env` when the proxy URL contains
credentials:

```json
{
  "http": {
    "proxy_url_env": "COLOSSUS_HTTP_PROXY"
  }
}
```

Set `http.trust_env` to `false` or pass `--http-no-trust-env` to ignore standard proxy
and certificate environment variables for Colossus-owned `httpx` clients.

## Skill overrides

Bundled skills are always available. Legacy user, user-global, and workspace skills can
override earlier skills only when `allow_user_skill_overrides` is set to `true`. Keep
this disabled in shared, regulated, or airgapped deployments unless the override source
is reviewed and pinned.

## Tool profiles

The current built-in tool profile is offline-first. Local filesystem, git, patch, repo
context, task, key decision, plan, verification, trace, and eval tools are available
through policy and approval controls. Network-capable tools require explicit approval.
`web.fetch` and `docs.fetch` can fetch bounded HTTP(S) responses when approved and
network access exists; `web.search` and `mcp.call` remain adapter extension points.

## Context compaction

Key decisions are durable commitments, not memories. Active key decisions are injected
into prepared model context before compacted snapshots, while archived and superseded
decisions remain historical state only.

Memories are durable context, not instructions. Active memories can be global,
repo-scoped, or session-scoped, are stored in SQLite, and are retrieved with the
configured memory index. V1 supports `memory.index.kind = "sqlite_fts"`; the config
shape is reserved for future index adapters such as vector stores.

Context budgets are calculated as a percentage of the selected model window. Add exact
model windows under `models.profiles.*.context_window_tokens` or the legacy
`provider.model_context_windows`; unknown models use `context.default_context_window_tokens`.
For ad-hoc model overrides, pass `--context-window-tokens` with `--model` so the REPL and
context service do not fall back to the default window.
If a provider's model catalog advertises a window, Colossus uses that as a best-effort
default only when no explicit window is configured. This is useful for providers such as
OpenRouter that include `context_length` in `/models`; OpenAI's official `/models`
response only exposes basic model identity metadata, so explicit config is still needed
there.

```json
{
  "provider": {
    "model": "local-model",
    "model_context_windows": {
      "local-model": 65536
    }
  },
  "context": {
    "compact_at_percent": 0.7,
    "target_percent": 0.45
  }
}
```

See [Context Compaction](CONTEXT.md) for commands and snapshot behavior.
