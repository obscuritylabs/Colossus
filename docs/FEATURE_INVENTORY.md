# Feature Inventory And Go Rewrite Map

This document is the rewrite planning checklist for Colossus. It consolidates the
current feature set into one place so a Go prototype or later full rewrite can be scoped
against known behavior instead of rediscovering capabilities from scattered docs and
source files.

Related references:

- [Architecture](ARCHITECTURE.md): service boundaries and dependency direction.
- [Security Model](SECURITY.md): policy, approval, audit, tools, and trust boundaries.
- [User Guide](USER_GUIDE.md): day-to-day command and REPL behavior.
- [Built-in Tools](TOOLS.md): model-callable tool catalog.
- [Configuration](CONFIGURATION.md): provider, model role, HTTP, research, and context
  settings.
- [Skills](SKILLS.md), [Packs](PACKS.md), and [Integrations](INTEGRATIONS.md): extension
  surfaces and executable boundaries.

External UX references:

- [Charmbracelet Crush](https://github.com/charmbracelet/crush): use as a terminal UX
  reference for command palette behavior, session/workspace visibility, permission
  prompts, LSP/MCP status, user-invocable skills, and desktop notifications. Do not
  copy its internals into Colossus; translate useful patterns through Colossus
  application services, typed events, policy, audit, and pack/skill boundaries.

## Rewrite Posture

The recommended first Go effort is a small terminal/frontend prototype, not an immediate
all-Go replacement.

Colossus already has a broad Python application core: model routing, context compaction,
tool policy, approvals, audit, durable state, subagents, research, integrations, skills,
packs, and offline verification. A Go prototype should first solve the problem Go is best
positioned to solve: a stable interactive terminal with clean concurrent input, queued
steering, cancellation, and high-quality rendering.

The cleanest near-term architecture is:

```text
Go TUI / CLI frontend
  <-> JSONL run/control protocol
Python Colossus core
  -> providers, tools, policy, state, audit, skills, research, integrations
```

Only after the protocol is stable should a Go-native core attempt parity with the Python
runtime.

## Non-Negotiable Parity Rules

- Preserve the ports-and-adapters boundary: interfaces render and collect input only;
  application services own orchestration; adapters own providers, subprocesses,
  filesystem, state, audit, and external systems.
- Keep the domain model dependency-light and portable.
- Validate tool arguments before policy, approval, risk review, execution, and audit.
- Treat `full-access` as no-prompt approval, not as a broader filesystem or network
  permission profile.
- Keep deterministic policy denies non-overridable by model risk review.
- Keep skills as prompt/context/resource data, not executable plugins.
- Keep packs as the executable distribution boundary.
- Keep raw secrets out of model requests, tool schemas, transcripts, traces, and audit
  payloads.
- Preserve append-only session history and audit records; context snapshots optimize
  model input but do not replace raw history.
- Render provider reasoning summaries only when exposed as safe typed summaries; do not
  render hidden chain-of-thought.

## Feature Matrix

| Area | Current capability | Rewrite parity requirement | Go prototype priority |
| --- | --- | --- | --- |
| CLI entrypoint | Typer command tree with global provider, model, context-window, API key, TLS, mTLS, proxy, and completion options. | Equivalent global option model and command dispatch, or a compatibility wrapper that forwards to Python. | P1 |
| One-shot runs | `colossus run` executes a prompt with workspace selection, session attach/resume, streaming/events controls, skills, approval mode, max turns, and model role. | Preserve run configuration and output events. | P0 |
| Goal mode | `colossus goal`, REPL `/goal`, and approved plan handoff (`run --execute-plan PLAN --goal`, REPL `/plan goal`, and the plan-review "Approve and goal" choice) create durable goal records, run bounded autonomous iterations through the normal orchestrator, expose `goal.show`/`goal.update` only while a goal is active, preserve `source_plan_id` when present, and stop on complete, blocked, or iteration-budget exhaustion. | Preserve durable goal state, plan-to-goal lineage, bounded continuation semantics, active-goal-only tool exposure, and normal policy/audit/session behavior for each iteration. | P1 |
| REPL | Interactive prompt, slash commands, transcript rendering, streaming toggles, event detail, themes, prompt history, sessions, workspace switching, skills, research, integrations, packs, tasks, decisions, memories, and context commands. | Preserve slash-command semantics and run-control behavior; rendering can improve. | P0 for shell, P1 for all commands |
| Event rendering | Typed events feed compact, verbose, off, and transcript renderers; provider deltas, tool calls/results, approvals, risks, errors, research status/progress, and safe reasoning summaries are normalized. Semantic tool-result rendering covers filesystem, shell, git, work state, context, repo context, skills/resources, web/search, MCP discovery/calls, trace, integrations, and generic structured pack/dynamic outputs. | Define a stable event schema that both Python and Go can consume; keep compact semantic summaries consistent across Python trace/transcript renderers and the Go TUI, with bounded generic structured output before raw previews. | P0 |
| Model routing | Roles include `primary`, `risk_evaluator`, `context_summarizer`, `subagent_default`, `research_planner`, `research_worker`, and `research_synthesizer`. | Keep role/profile separation and legacy single-provider normalization. | P1 |
| Providers | Echo, OpenAI Responses, and local OpenAI-compatible chat completions; provider diagnostics and model catalog inspection. | Support the same role abstraction; Go frontend can initially delegate provider calls to Python. | P0 bridge, P2 native |
| Context compaction | Raw messages and run events remain persisted; snapshots compact model input with deterministic fallback and optional model-assisted summaries. | Preserve raw history, active snapshot selection, and budget estimates. | P1 |
| Sessions | SQLite-backed sessions, latest/resume/list/show, recent messages, updated timestamps, and run association. | Preserve session ids, resume behavior, and message storage compatibility or migration. | P1 |
| Tools | Built-ins plus integration-generated tools, all expressed as `ToolSpec` with JSON Schema, permissions, timeouts, and output caps. | Preserve model-visible schemas, permission metadata, and handler routing. | P0 list/read, P2 execute natively |
| Policy and approvals | `deny`, `ask`, `risk-auto`, and `full-access`; deterministic policy, optional model-assisted `shell.run` risk review, approval events, and audit. | Preserve exact semantics, especially `risk-auto` prompting and `full-access` skipping model risk review. | P0 UI flow, P2 native policy |
| Audit | Hash-chained JSONL audit records with redaction for tool, policy, approval, risk, credential, skill, and bundle activity. | Preserve audit integrity and redaction guarantees. | P1 |
| Filesystem tools | Workspace-scoped list/read/search/write/replace with text safety, denied control dirs, diff visibility, and approval for writes. | Preserve path safety and exact mutation visibility. | P2 native |
| Git tools | Read-only `git.status`, `git.diff`, and `git.show` through bounded structured commands. | Preserve bounded output and structured argv. | P2 native |
| Shell tool | `shell.run` uses structured argv, no `shell=True`, bounded output, timeouts, policy, approval, risk review, and audit. | Preserve no-shell execution and risk/approval ordering. | P2 native |
| Patch tools | `patch.preview`, `patch.apply`, `patch.reverse` with diffs and changed line ranges. | Preserve exact text patch semantics and approval for mutation. | P2 native |
| Repo context tools | `repo.map`, `repo.symbol_search`, `repo.references`, and `repo.file_summary`. | Preserve local-only discovery and bounded summaries. | P2 |
| Durable work state | Tasks, key decisions, memories, and plans are persisted or runtime-scoped as appropriate and injected into future context where applicable. | Preserve status models, visibility commands, and context injection order. | P1 |
| Memory search | SQLite FTS-backed durable memory index with global, repo, and session scopes. | Preserve memory-as-context semantics; no secret storage. | P2 |
| Subagents | Durable queued child jobs using the normal orchestrator, policy, tools, approval mode, and `subagent_default` role; nested delegation removed in child catalogs; CLI/REPL controls cover list, status, show, bounded drain, cancel, and resume/requeue, with parent-visible result previews. | Preserve durable job records, bounded concurrency, queue controls, resume semantics, and parent-child result UX. | P2 |
| Deep research | `colossus research` and `/research` plan bounded repo/web/MCP collection, emit coarse `research.status` and operational `research.progress` events, persist sources/claims/reports, cite sources, and append reports to sessions. | Preserve source-lane limitations, approval for network/MCP lanes, progress-event shape, and cited report persistence. | P2 |
| Web/docs fetch | `web.fetch` and `docs.fetch` are approval-gated bounded HTTP(S) fetchers using global HTTP config. | Preserve fetch bounds, network approval, TLS/proxy settings, and audit. | P2 native |
| Web search | `web.search` is hidden unless a search adapter is configured; research can use DuckDuckGo or SearXNG-backed search. | Preserve disabled-by-default behavior and adapter boundary. | P2 |
| MCP | MCP servers/tools can be listed from configured adapters; `mcp.call` is an adapter extension point and not exposed arbitrarily by default. | Preserve allowlisted, configured, audited access only; redact secrets from discovery output. | P2 |
| Integrations | Local connection registry with credential refs; GitHub, SearXNG, OpenSearch, and OpenAPI imports produce normal tools. | Preserve hidden-until-connected tools and credential-broker boundary. | P2 |
| Skills | Bundled, pack, user, global, workspace, and legacy skills; active skill composition; required-tool validation; active-skill resource tools; authoring tools. | Preserve precedence, override rules, prompt mention semantics, and resource access limits. | P1 composition, P2 authoring |
| Packs | Installable capability packages with manifests, hash-listed files, trust records, integrations, skills, tools, MCP servers, binaries, Docker assets, docs, and tests. | Preserve pack validation, trust, enable/disable/install lifecycle, and executable boundary. | Defer native |
| Offline bundles | Directory bundles with manifest and SHA-256 verification; release bundles should include wheelhouse, locks, SBOM, signatures, skills, and docs. | Preserve verifier and release artifact expectations. | Defer native |
| Configuration | Strict JSON config for provider, model roles, context, agent, subagents, memory, HTTP, research, and skill overrides. | Preserve strict unknown-field rejection and compatibility aliases. | P1 |
| HTTP transport | Global CA bundle, provider CA override, mTLS client cert/key, proxy URL/env, and trust-env controls for Colossus-owned HTTP clients. | Preserve transport config for provider, fetch, search, and integrations. | P2 |
| Credentials | Environment-backed credential refs for providers and integrations; raw secrets should not enter model-visible surfaces. | Preserve ref-only schemas and local resolution. | P1 |
| Provider diagnostics | `provider doctor`, `provider models`, `models list`, and `models doctor` inspect provider readiness and role routing. | Preserve readiness checks and model catalog display. | P1 |
| Themes and preferences | REPL themes are data-only JSON/TOML; saved preferences include theme, multiline mode, streaming, events, transcript style, and reasoning visibility. | Preserve data-only validation and preference storage. | P1 for Go TUI |
| Release and install docs | Source checkout install, development environment, release process, offline/airgap docs, troubleshooting, and workflows. | Keep docs current for both Python and Go tracks. | P0 docs |

## CLI Surface

Global options:

- `--verbose` / `-v`
- `--provider`
- `--model`
- `--context-window-tokens`
- `--base-url`
- `--api-key`
- `--api-key-env`
- `--ca-bundle`
- `--http-ca-bundle`
- `--http-client-cert`
- `--http-client-key`
- `--http-client-key-password-env`
- `--http-proxy`
- `--http-proxy-env`
- `--http-no-trust-env`
- shell completion options

Top-level commands:

- `run`: one agent turn.
- `goal`: bounded autonomous goal loop.
- `research`: deep research with persisted cited output.
- `repl`: interactive mode.
- `config`: `init`, `show`.
- `skills`: `list`, `new`, `validate`, `install`.
- `tools`: `list`.
- `provider`: `doctor`, `models`.
- `models`: `list`, `doctor`.
- `agents`: `list`, `show`, `cancel`.
- `bundle`: `verify`.
- `plans`: `list`, `show`, `approve`.
- `goals`: `list`, `show`.
- `tasks`: `list`.
- `decisions`: `list`, `archive`, `supersede`.
- `memories`: `list`, `search`, `archive`, `supersede`.
- `context`: `show`, `compact`, `snapshots`, `restore`.
- `sessions`: `list`, `show`.
- `integrations`: `list`, `show`, `connect`, `disconnect`, `import-openapi`.
- `packs`: `list`, `show`, `verify`, `validate`, `install`, `enable`, `disable`,
  `uninstall`, `trust list`, `trust add`.

## REPL Surface

Core commands:

- `/help`
- `/status`
- `/agent show|max-turns N`
- `/tools`
- `/workspace [PATH]`
- `/session show|resume|latest|new`
- `/sessions [LIMIT]`
- `/resume [LIMIT]`
- `/context`
- `/compact`
- `/exit`

Display and composer commands:

- `/stream on|raw|off`
- `/events compact|verbose|off`
- `/reasoning on|off`
- `/transcript comfortable|compact`
- `/multiline on|off|toggle`
- `/theme [NAME]`
- `/theme preview [NAME]`
- `/theme save [NAME]`
- `/theme reset`
- `/repl prefs|save|reset`
- `/trace`

State and workflow commands:

- `/tasks [open|all|STATUS]`
- `/decision <text>`
- `/decision archive <id>`
- `/decision supersede <id> <text>`
- `/decisions [all|STATUS]`
- `/memory <text>`
- `/memory search <query>`
- `/memory archive <id>`
- `/memory supersede <id> <text>`
- `/memories [all|STATUS]`
- `/goal [--max-iterations N|list|show ID|OBJECTIVE]`
- `/research on|off|show|sources|QUESTION`
- `/agents`
- `/integrations list|show|connect`
- `/packs list|show|verify|validate|install|enable|disable|trust ...`
- `/skill show|use|drop|clear|new|validate|on|off`

## Model-Callable Tool Surface

Offline-first built-ins:

- Filesystem: `filesystem.list`, `filesystem.read`, `filesystem.search`,
  `filesystem.write`, `filesystem.replace`.
- Git: `git.status`, `git.diff`, `git.show`.
- Shell: `shell.run`.
- Tasks: `task.create`, `task.update`, `task.list`.
- Goal mode: `goal.show`, `goal.update`, exposed to providers only for active goal
  runs. Approved plans can be handed to Goal Mode from the CLI or REPL while keeping
  the plan id on the durable goal record.
- Key decisions: `decision.create`, `decision.update`, `decision.list`,
  `decision.archive`, `decision.supersede`.
- Memories: `memory.create`, `memory.update`, `memory.list`, `memory.search`,
  `memory.archive`, `memory.supersede`.
- Plans: `plan.create`, `plan.show`, `plan.approve_request`.
- Patch: `patch.preview`, `patch.apply`, `patch.reverse`.
- Repo context: `repo.map`, `repo.symbol_search`, `repo.references`,
  `repo.file_summary`.
- Subagents: `agent.delegate`, `agent.result`, `agent.list`.
- Discovery: `mcp.servers`, `mcp.tools`, `tool.search`.
- Trace: `trace.show`, `trace.export`.
- Context: `context.show`, `context.compact`, `context.snapshots`,
  `context.restore`.
- Skill authoring/resources: `skill.scaffold`, `skill.inspect`, `skill.read`,
  `skill.write`, `skill.validate`, `skill.install`, `skill.resource.list`,
  `skill.resource.read`.
- Smoke test: `echo`.

Network or adapter-backed tools:

- HTTP fetch: `web.fetch`, `docs.fetch`.
- Search: `web.search`, exposed only when a search adapter is configured.
- MCP execution: `mcp.call`, not exposed by default unless an adapter is installed and
  policy-approved.
- Native integrations: `github.*`, `searxng.*`, `opensearch.*`.
- Imported APIs: `openapi.NAME.OPERATION`.

## Go Prototype Target

The first prototype should prove that Go can own a better interactive shell while core
provider, policy, tool, and state behavior stays behind service boundaries. The Go track
uses Cobra for command dispatch and Bubble Tea for terminal UI work; Bubble Tea should
remain the only interactive REPL frontend, not a home for model, tool, policy, or state
logic.

UX reference targets from Crush that fit Colossus:

- Promote the slash palette into a full command palette: searchable commands, actions,
  settings, sessions, user-invocable skills, packs, integrations, and active workflow
  shortcuts.
- Make session/workspace state visible: active session, busy sessions, queued prompts,
  attached clients when a future server mode exists, and whether a permission request
  is blocking progress.
- Add a workspace cockpit view without moving orchestration into the TUI: LSP health,
  MCP/integration state, active skills, current model roles, context budget, tasks,
  decisions, memories, and plans.
- Treat permission and approval UX as first-class: active prompt, tool summary,
  risk/approval reason, allow/deny shortcuts, and a durable audit link or trace id.
- Keep skills dual-mode: model-discoverable skills through `SkillComposer`, plus
  user-invocable skills/actions from the palette for deliberate workflow starts.
- Add opt-in terminal/desktop notifications for permission-required and run-complete
  events, driven only by typed events and saved display preferences.
- Preserve Colossus safety differences: no arbitrary config-time command expansion,
  no unreviewed tool hiding/allowlisting shortcuts that bypass policy, and no TUI-owned
  tool execution.

Specific Crush patterns to translate, not copy:

- Use one interactive Bubble Tea app with rectangle-based layout, dialog overlays, and
  focus-aware key routing instead of maintaining separate terminal interaction stacks.
- Keep chat rendering itemized and cache-friendly: assistant messages, tool calls,
  approvals, diffs, search results, web/research sources, and diagnostics should each
  have semantic renderers with compact and verbose variants.
- Make the command palette the primary discovery surface for commands, model-role
  switches, settings, user-invocable skills, packs, integrations, recent sessions, and
  workflow modes like research, plan, goal, and red-team review.
- Surface context providers as workspace status, not hidden magic: LSP, MCP,
  integrations, web search, active skills, ignored files, context budget, and current
  approval mode should be visible in cockpit/status views.
- Treat notifications as display preferences driven by typed events only, such as
  permission-required, run-complete, subagent-complete, and research-complete.
- Keep Crush-inspired config convenience subordinate to Colossus trust boundaries:
  environment expansion is allowed only through explicit secret refs, config files must
  not execute shell snippets at load time, and disabled/allowed tool settings must never
  bypass policy, validation, or audit.

P0 goals:

- Launch a Bubble Tea Go TUI with stable prompt rendering and transcript panes.
- Consume typed JSONL events from a child Python Colossus process or fixture stream.
- Send user prompts, slash commands, approval decisions, cancellation, and queued
  follow-up messages over a control channel.
- Render assistant deltas, final assistant messages, tool calls/results, approval
  prompts, auto-approvals, risk assessments, errors, and run completion without corrupting
  terminal state.
- Support one active run plus queued next user input.
- Preserve Colossus session id, workspace, model role/model, approval mode, event mode,
  and context budget in the status bar.

P1 goals:

- Implement a Python bridge command that exposes a stable run/control JSONL protocol.
- Route existing REPL slash commands through the bridge instead of reimplementing them
  in Go.
- Persist Go TUI preferences only for display behavior.
- Add fixtures and golden transcript tests for terminal rendering.
- Support `run`, `goal`, `repl`, `sessions`, `context`, `tools`, `skills`, `models`,
  and `provider` workflows through the bridge.

P2 goals:

- Decide whether Go should begin replacing core services or remain a frontend.
- If replacing core services, start with provider abstraction, event model, session
  persistence, tool specs, deterministic policy, and audit before any mutating tool.
- Port filesystem/git/shell/patch tools only after path safety, approval, audit, and
  redaction tests are in place.

Deferred for a first prototype:

- Native packs and offline bundle install/verify.
- Native integrations and OpenAPI import.
- Native deep research workers.
- Native subagent runner.
- Native skill authoring tools.
- Native model-assisted context compaction.

## Proposed Bridge Protocol

The protocol should be line-delimited JSON with explicit `type`, `id`, and `session_id`
fields. A transport can be stdio first; sockets can come later.

Frontend-to-core control messages:

- `run.start`: start a normal turn with prompt, workspace, session, model role, approval
  mode, active skills, max turns, stream mode, and event mode.
- `research.start`: start a research run with question, depth, sources, workspace, and
  session.
- `slash.command`: execute a REPL command and return typed display/state events.
- `approval.decide`: approve or deny a pending tool call.
- `user.queue`: queue a prompt to run after the current turn finishes.
- `run.cancel`: request cancellation of the active run.
- `session.switch`: resume, create, or show a session.
- `workspace.switch`: update the active workspace.

Core-to-frontend events:

- `run.started`, `run.completed`, `run.failed`, `run.cancelled`.
- `assistant.delta`, `assistant.message`.
- `reasoning.summary`.
- `tool.call.requested`, `tool.result`, `tool.failed`.
- `approval.requested`, `approval.auto_granted`, `approval.denied`.
- `risk.assessment`, `risk.unavailable`.
- `research.status`, `research.progress`, `research.source`, `research.claim`,
  `research.report`.
- `context.status`, `session.status`, `task.status`, `agent.status`.
- `catalog.tools`, `catalog.skills`, `catalog.integrations`, `catalog.packs`.
- `error`.

`research.progress` is the detailed Deep Research telemetry event. It carries
operational metadata only: `research_id`, `phase`, `action`, `status`
(`started`, `completed`, `skipped`, or `failed`), optional `message`, `query`,
`source_kind`, `current`, `total`, `sources_collected`, `claims_collected`, and
bounded `details`. It must not expose hidden reasoning or raw model internals.

Protocol rules:

- Events must be append-only and replayable for transcript reconstruction.
- Tool arguments in events must follow the same redaction policy as current transcripts.
- Secret values must never appear in protocol payloads.
- Provider raw chunks must stay hidden unless a future explicit debug protocol is added.
- Run ids and pending approval ids must be stable enough for cancellation and approval
  decisions.
- The protocol should allow old Go frontends to ignore unknown event types.

## Full Rewrite Readiness Gates

Do not start an all-Go core rewrite until these gates are satisfied:

- A checked-in bridge protocol spec with fixtures from real Colossus runs.
- Golden tests for transcript rendering, queued input, cancellation, approvals, and tool
  events.
- A migration plan for SQLite state or a deliberate compatibility break.
- A provider parity plan for OpenAI Responses, OpenAI-compatible chat, and echo.
- A tool-spec compatibility test that compares Python and Go schemas.
- A policy/approval/risk parity test suite, especially for `risk-auto` and
  `full-access`.
- An audit format compatibility or migration plan.
- A skills/packs compatibility plan that does not blur the data-vs-executable boundary.
- A release plan for shipping both Python core and Go frontend during transition.

## Major Rewrite Risks

- Feature drift: a Go rewrite that starts from the terminal can accidentally lose
  research, skills, packs, durable state, audit, or integration behavior.
- Security drift: mutating tools are easy to port incorrectly if path safety, policy,
  approval, audit, and redaction are not ported together.
- Terminal ownership: queued input while streaming requires one renderer/input owner.
  Mixed Python prompt rendering plus background updates already showed visible terminal
  corruption.
- Provider normalization: OpenAI Responses, OpenAI-compatible servers, and OpenRouter-like
  endpoints differ in streaming, reasoning summaries, retries, errors, model catalogs,
  and tool-call formats.
- Dependency substitution: Python currently has mature libraries for some document,
  HTTP, SQLite, and CLI workflows; Go replacements must be selected intentionally.
- State compatibility: sessions, context snapshots, tasks, decisions, memories,
  subagents, research records, preferences, integrations, and audit records all carry
  user value across runs.
- Extension compatibility: skills and packs should remain portable; a rewrite should not
  force authors to maintain separate Python and Go package formats.

## Open Decisions

- Should Go remain the permanent terminal/frontend and leave application core in Python?
- Should the bridge protocol be stdio-only, local socket, or support both?
- Should queued steering run as "next prompt after completion" or be able to interrupt
  the current model turn?
- Should cancellation be best-effort provider cancellation, local process cancellation,
  or both?
- Should Go preferences share the current SQLite state database or use a separate display
  config file?
- Should a Go-native core target only local/offline use first, or provider parity first?
- Should packs eventually be language-neutral OCI-style artifacts with executable
  binaries, while skills stay pure text/resources?
