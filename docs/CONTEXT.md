# Context Compaction

Colossus keeps raw session messages and run events append-only, then builds a compacted
working context only when sending a request to a model. Compaction is an optimization
layer, not the source of truth.

## Defaults

- Auto compaction is enabled.
- Unknown models use a `32768` token context window.
- Compaction starts at 70% of the configured model window.
- Compacted context targets 45% of the configured model window.
- The newest 8 messages are kept uncompressed by default.
- Model-assisted summaries are best-effort; deterministic compaction always works
  offline.

Configure model-specific windows:

```json
{
  "provider": {
    "kind": "local_openai_chat",
    "model": "local-model",
    "base_url": "http://localhost:12434/v1",
    "api_key_env": null,
    "ca_bundle": null,
    "model_context_windows": {
      "local-model": 65536
    }
  },
  "context": {
    "auto_compaction": true,
    "default_context_window_tokens": 32768,
    "compact_at_percent": 0.7,
    "target_percent": 0.45,
    "recent_tail_messages": 8,
    "model_assisted": true
  },
  "allow_user_skill_overrides": false
}
```

## Commands

```bash
uv run colossus context show --session SESSION_ID
uv run colossus context compact --session SESSION_ID
uv run colossus context snapshots --session SESSION_ID
uv run colossus context restore SNAPSHOT_ID
```

The REPL supports:

- `/compact`
- `/context`
- `/context snapshots`
- `/context restore SNAPSHOT_ID`

The TUI includes a context panel in the side column.

`context show`, `/context`, `/status`, and the REPL toolbar report the effective prompt
estimate after the active snapshot is applied. When a snapshot is active, they also show
the raw append-only history estimate separately. Raw history can remain above the
threshold while the effective prompt sent to the model is below it.

For one-off provider/model overrides, set the model window on the command line:

```bash
uv run colossus --provider local-openai-chat \
  --model "nex-agi/nex-n2-pro:free" \
  --context-window-tokens 131072 \
  repl
```

## Model-Callable Tools

The built-in context tools are:

- `context.show`
- `context.compact`
- `context.snapshots`
- `context.restore`

`context.restore` requires approval because it changes which snapshot is active for
future model requests. It does not delete or rewrite raw messages.

## Snapshot Contents

Snapshots store:

- source message range,
- summary,
- pinned user facts,
- open tasks,
- files or artifacts mentioned by tool outputs,
- compact tool-result summaries,
- strategy: `deterministic` or `hybrid-model`.

If model-assisted compaction fails or is unavailable, Colossus keeps the deterministic
snapshot and continues.
