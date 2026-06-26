# Built-in Tools

Colossus exposes built-in tools through `ToolSpec` objects with strict JSON Schema
inputs, optional JSON output schemas, permission metadata, timeouts, and output caps.
All model-callable tools go through policy, approval, audit, and the brokered execution
path where applicable.

List the installed catalog:

```bash
uv run colossus tools list
```

## Permission Defaults

| Family | Tools | Approval | Offline | Notes |
| --- | --- | --- | --- | --- |
| Filesystem read | `filesystem.list`, `filesystem.read`, `filesystem.search` | No | Yes | Workspace-scoped text discovery, read, and search. |
| Filesystem write | `filesystem.write`, `filesystem.replace` | Yes | Yes | Exact text mutation inside the workspace. |
| Git inspect | `git.status`, `git.diff`, `git.show` | No | Yes | Brokered structured git argv, bounded output. |
| Shell | `shell.run` | Yes | Yes | Structured argv only; shell wrappers are denied by default. |
| Task state | `task.create`, `task.update`, `task.list` | No | Yes | Session-scoped progress tracking persisted in SQLite state. |
| Key decisions | `decision.create`, `decision.update`, `decision.list`, `decision.archive`, `decision.supersede` | Mutations | Yes | Durable session commitments injected into future context. |
| Memories | `memory.create`, `memory.update`, `memory.list`, `memory.search`, `memory.archive`, `memory.supersede` | No | Yes | Durable global/repo/session context retrieved through SQLite FTS. |
| Plan state | `plan.create`, `plan.show`, `plan.approve_request` | Approval request only | Yes | Draft plans are runtime-local; approval is policy-gated. |
| Verification | `test.run`, `lint.run`, `typecheck.run`, `build.run` | Yes | Yes | Fixed command templates through `uv` and the subprocess broker. |
| Patch | `patch.preview`, `patch.apply`, `patch.reverse` | Apply/reverse only | Yes | Exact text patch preview and mutation. |
| Repo context | `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary` | No | Yes | Local file map, symbol extraction, references, and summaries. |
| Subagents | `agent.delegate`, `agent.result`, `agent.list` | No by default | Yes | Durable queued child-agent jobs with bounded local concurrency. |
| Web/docs | `web.fetch`, `docs.fetch` | Yes | Yes | Approval-gated HTTP(S) fetches. `web.search` is exposed only when a search adapter such as SearXNG is configured. |
| MCP/discovery | `mcp.servers`, `mcp.tools`, `tool.search` | No | Yes | MCP listing returns unconfigured state; `tool.search` searches the local catalog. `mcp.call` is not exposed unless an MCP adapter is installed. |
| Research mode | `colossus research`, `/research` | Network/MCP lanes | Partial | Persists cited reports, sources, claims, and research status events. |
| Trace/eval | `trace.show`, `trace.export`, `eval.run` | Export/eval only | Yes | Trace export writes a bounded snapshot; eval wraps local pytest. |
| Context | `context.show`, `context.compact`, `context.snapshots`, `context.restore` | Restore only | Yes | Durable snapshots reduce model input without deleting raw history. |
| Smoke test | `echo` | No | Yes | Deterministic smoke-test tool. |

## Input And Output Shapes

Every tool input schema uses `additionalProperties: false`. Important shapes:

- File tools accept workspace-relative `path` values and return relative paths.
- Search tools return arrays of `{path, line, text}` style match objects plus
  `truncated` when applicable.
- Command wrappers return `{command, exit_code, stdout, stderr}`.
- Patch preview returns `{path, replacements, diff}`; patch apply/reverse return
  `{path, replacements}`.
- Task, key decision, memory, and plan tools return a single structured `task`,
  `decision`, `memory`, or `plan` object, or arrays for list commands. Memory mutations
  also return a concise `notice` string for user-visible saved-memory feedback.
  Subagent tools return durable `agent` job records with status, parent ids, child run
  id, output, and error fields.
- `web.fetch` and `docs.fetch` return `{url, status_code, content_type, content,
  truncated}` for bounded HTTP(S) responses. `web.search` returns normalized
  `{title, url, snippet, metadata}` results from the configured provider, and remains
  hidden in the default tool catalog.
- Research runs persist a `ResearchRun` plus labeled `ResearchSource` records and
  source-backed `ResearchClaim` records. Reports cite persisted labels such as `[R1]`.
- Context tools return session budget/status, snapshot records, or restore confirmation.

## Security Notes

- The default policy requires approval for high-risk tools, network-capable tools,
  declared mutations, and tools with explicit `approval_required`.
- `--approval-mode full-access` auto-approves approval-required tools without prompting,
  but it does not change tool schemas, filesystem roots, network implementations, or
  deterministic policy denies.
- Subprocess-backed tools use fixed argv templates or structured argv arrays. Colossus
  does not use `shell=True`.
- For `shell.run`, pass the executable and each argument as separate `argv` entries. A
  process-count request should use `["ps", "-A", "-o", "pid="]` and count returned
  lines; `["ps", "-A", "|", "wc", "-l"]` passes `|` literally and is not a pipeline.
- Web fetch tools require explicit approval and depend on network availability. In
  airgapped environments they should not be approved or will fail at the network layer.
  Web search and MCP calls remain adapter extension points.
- Deep Research Mode asks for approval before configured web search or MCP collection.
  If a source lane is disabled or denied, the run continues with available evidence and
  records the limitation in the report.
- Task records are durable per session and are visible with `/tasks` in the REPL or
  `colossus tasks list`. Key decisions are visible with `/decisions` in the REPL or
  `colossus decisions list`. Memories are visible with `/memories` in the REPL or
  `colossus memories list/search`. Subagent jobs are durable queued records visible
  with `/agents` in the REPL or `colossus agents list/show/cancel`.
- Context snapshots are durable SQLite records, but raw session messages remain the
  source of truth. Active key decisions are injected before snapshot summaries; archived
  and superseded decisions remain persisted but do not steer future context.
- Memories are context, not instructions. Relevant active memories are injected after
  active key decisions and before snapshot summaries; archived and superseded memories
  remain persisted for history only.
