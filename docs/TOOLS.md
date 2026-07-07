# Built-in Tools

Colossus exposes built-in tools through `ToolSpec` objects with strict JSON Schema
inputs, optional JSON output schemas, permission metadata, timeouts, and output caps.
All model-callable tools go through policy, approval, audit, and the brokered execution
path where applicable.

List the installed catalog:

```bash
uv run colossus tools list
```

For task-oriented examples, see [Workflows](WORKFLOWS.md). For integration-generated
tools, see [Integrations](INTEGRATIONS.md).

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
| Goal mode | `goal.show`, `goal.update` | No | Active goal runs only | Durable active-goal progress and terminal status, including approved plan lineage when a plan is handed to Goal Mode. |
| Plan state | `plan.create`, `plan.show`, `plan.approve_request` | Approval request only | Yes | Draft plans are runtime-local; approval is policy-gated; approved plans can execute once or hand off to bounded Goal Mode. |
| Patch | `patch.preview`, `patch.apply`, `patch.reverse` | Apply/reverse only | Yes | Exact text patch preview and mutation. |
| Repo context | `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary` | No | Yes | Local file map, symbol extraction, references, and summaries. |
| Subagents | `agent.delegate`, `agent.result`, `agent.list` | No by default | Yes | Durable queued child-agent jobs with bounded local concurrency. |
| Web/docs | `web.fetch`, `docs.fetch` | Yes | Yes | Approval-gated HTTP(S) fetches. `web.search` is exposed only when a search adapter such as SearXNG is configured. |
| MCP/discovery | `mcp.servers`, `mcp.tools`, `tool.search` | No | Yes | MCP listing returns unconfigured state; `tool.search` searches the local catalog. `mcp.call` is not exposed unless an MCP adapter is installed. |
| Integrations | `github.*`, `searxng.*`, `opensearch.*`, `openapi.NAME.*` | Yes | Partial | Hidden until connected. Calls inject auth through credential refs, then pass policy, approval, audit, and HTTP settings. |
| Research mode | `colossus research`, `/research` | Network/MCP lanes | Partial | Persists cited reports, sources, claims, and research status events. |
| Trace | `trace.show`, `trace.export` | Export only | Yes | Trace export writes a bounded snapshot. |
| Context | `context.show`, `context.compact`, `context.snapshots`, `context.restore` | Restore only | Yes | Durable snapshots reduce model input without deleting raw history. |
| Skill authoring | `skill.scaffold`, `skill.inspect`, `skill.read`, `skill.write`, `skill.validate`, `skill.install`, `skill.resource.list`, `skill.resource.read` | Scaffold/write/install only | Yes | Data-only installed-skill edits, local skill validation and install, and active-skill resource reads. |
| Smoke test | `echo` | No | Yes | Deterministic smoke-test tool. |

## Input And Output Shapes

Every tool input schema uses `additionalProperties: false`. Important shapes:

- File tools accept workspace-relative `path` values and return relative paths. The
  workspace defaults to the process current directory, can be selected with
  `--workspace`/`-C` for CLI runs, and can be switched inside the REPL with
  `/workspace PATH`.
- Search tools return arrays of `{path, line, text}` style match objects plus
  `truncated` when applicable.
- Command wrappers return `{command, exit_code, stdout, stderr}`.
- Mutating file tools return edit visibility. `filesystem.write`, `filesystem.replace`,
  `patch.apply`, and `patch.reverse` include `diff` and `changed_line_ranges`; patch
  preview returns the same diff without writing.
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
- Local repo skills under `.agents/skills` are normal workspace files. Agents can use
  regular filesystem and command tools to build and test them under the usual workspace
  policy. `.colossus` remains a denied control directory for generic workspace tools.
- `skill.scaffold` writes only `manifest.json` and `SKILL.md` under the configured
  installed skill directory plus requested resource directories. It can accept model-generated
  `instructions`, trigger words, required tool names, permission labels, offline
  compatibility, Agent Skills frontmatter, and resource directory names, but it does not
  run helpers or write arbitrary paths.
- `skill.inspect` and `skill.read` inspect existing installed user skills under the
  configured user skill directory. They return bounded metadata/content and hashes for
  safe follow-up edits.
- `skill.write` creates or overwrites bounded UTF-8 text files only at `SKILL.md`,
  `manifest.json`, or under `references/`, `scripts/`, `assets/`, `examples/`, or
  `tests/` inside an existing user skill. Existing-file overwrites require the
  `expected_sha256` returned by `skill.read` or `skill.inspect`, and writes are audited
  without content.
- `skill.validate` validates either an installed user skill by `name` or a local skill
  directory by `path`.
- `skill.install` validates a local skill directory and installs it into
  `~/.agents/skills/NAME`. It is approval-required, refuses overwrite unless requested,
  and audits file paths, sizes, and hashes without content.
- `skill.resource.list` and `skill.resource.read` are read-only. The orchestrator injects
  the active skill names, and the tools can only access bounded text-safe files under
  `references/`, `scripts/`, `assets/`, `examples/`, or `tests/` for active skills.
  Resource reads are audited by skill, path, and size.
- Integration tools never include credential fields in model-visible schemas. A connected
  GitHub tool family exposes repository, issue, pull request, check, and release reads.
  A connected SearXNG tool family exposes local/private search and health checks.
  A connected OpenSearch tool family exposes document search, retrieval, indexing,
  partial updates, deletes, mappings, and health checks; document writes are mutating
  high-risk tools.
  Imported OpenAPI tools are named `openapi.NAME.OPERATION` and map path/query/body
  parameters from the OpenAPI operation.

## Security Notes

- The default policy requires approval for high-risk tools, network-capable tools,
  declared mutations, and tools with explicit `approval_required`.
- `--approval-mode risk-auto` auto-approves only low-risk allowed `shell.run` calls;
  model-risk denies are escalated to approval prompts. `--approval-mode full-access`
  auto-approves approval-required tools without prompting and skips model-assisted
  `shell.run` risk review, but it does not change tool schemas, filesystem roots,
  network implementations, or deterministic policy denies.
- Subprocess-backed tools use fixed argv templates or structured argv arrays. Colossus
  does not use `shell=True`.
- For `shell.run`, pass the executable and each argument as separate `argv` entries. A
  process-count request should use `["ps", "-A", "-o", "pid="]` and count returned
  lines; `["ps", "-A", "|", "wc", "-l"]` passes `|` literally and is not a pipeline.
- Web fetch tools require explicit approval and depend on network availability. In
  airgapped environments they should not be approved or will fail at the network layer.
  Configured global HTTP PKI and proxy settings are used for Colossus-owned web fetch
  and web search clients. Web search and MCP calls remain adapter extension points.
- Integrations store only credential refs such as `env:GITHUB_TOKEN`. Raw API keys,
  bearer tokens, OAuth secrets, refresh tokens, and service-account JSON must not appear
  in tool arguments, model requests, transcripts, or audit payloads. V1 resolution is
  environment-backed; OS keychain or encrypted local storage can be added behind the same
  credential broker port.
- Deep Research Mode asks for approval before configured web search or MCP collection.
  If a source lane is disabled or denied, the run continues with available evidence and
  records the limitation in the report.
- Task records are durable per session and are visible with `/tasks` in the REPL or
  `colossus tasks list`. Key decisions are visible with `/decisions` in the REPL or
  `colossus decisions list`. Memories are visible with `/memories` in the REPL or
  `colossus memories list/search`. Subagent jobs are durable queued records visible
  with `/agents` in the REPL or `colossus agents list/status/show/drain/cancel/resume`.
- Context snapshots are durable SQLite records, but raw session messages remain the
  source of truth. Active key decisions are interpreted durable commitments with intent
  and applicability; they are injected before snapshot summaries as binding guidance.
  Archived and superseded decisions remain persisted but do not steer future context.
- Memories are context, not instructions. Relevant active memories are injected after
  active key decisions and before snapshot summaries; archived and superseded memories
  remain persisted for history only.
