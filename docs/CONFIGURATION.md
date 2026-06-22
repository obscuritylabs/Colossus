# Configuration

Colossus loads `config.json` from the platform user config directory. If the file does
not exist, Colossus uses the built-in defaults.

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
  "subagents": {
    "max_concurrent": 4
  },
  "memory": {
    "index": {
      "kind": "sqlite_fts"
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
and reuses it for `risk_evaluator`, `context_summarizer`, and `subagent_default`.

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
      "subagent_default": "main"
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
such as `Thinking...`, `Using filesystem.read...`, or `Responding...`. A truly persistent
bottom status region belongs in the Textual `colossus tui` surface rather than the REPL.

Composer behavior:

- Single-line mode is the default; `Enter` submits.
- Multiline mode makes `Enter` insert a newline and `Esc+Enter` submit.
- Prompt history is stored as `repl_history.txt` under the Colossus data directory.

Runtime controls:

- `/stream on|off`
- `/events compact|verbose|off`
- `/reasoning on|off`
- `/tasks [open|all|STATUS]`
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
such as prompt size, session/context counters, completion markers, and larger tool
details. Use `/transcript compact` for a tighter terminal stream, or `/transcript
comfortable` for the default Pi-like spacing.

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

Use `ca_bundle` or `--ca-bundle` when an HTTPS provider endpoint is signed by an
enterprise or private CA:

```bash
uv run colossus --ca-bundle ./certs/company-ca.pem --provider local-openai-chat run "hello"
```

## Skill overrides

Bundled skills are always available. User-installed skills can override bundled skills
only when `allow_user_skill_overrides` is set to `true`. Keep this disabled in shared,
regulated, or airgapped deployments unless the override source is reviewed and pinned.

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
