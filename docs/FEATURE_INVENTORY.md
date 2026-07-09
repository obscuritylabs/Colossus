# Colossus Product Requirements And Reconstruction Specification

This document is the product requirements document (PRD) for Colossus. It is intended
to be complete enough that an implementation team or coding model can build a compatible
system without reading the existing source code.

The specification is implementation-language neutral. It describes observable behavior,
data contracts, security invariants, and acceptance criteria. Framework names, file
layouts, and historical implementation strategy are intentionally excluded.

Requirement keywords have their usual meanings:

- **MUST** is required for a compatible release.
- **SHOULD** is expected unless a documented product decision says otherwise.
- **MAY** is optional.

Related references provide additional detail but are not required to understand this PRD:

- [Architecture](ARCHITECTURE.md)
- [Security Model](SECURITY.md)
- [Built-in Tools](TOOLS.md)
- [Configuration](CONFIGURATION.md)
- [Skills](SKILLS.md)
- [Packs](PACKS.md)
- [Integrations](INTEGRATIONS.md)

### Clean Reconstruction Handoff

For a clean reconstruction, give the implementer this file before providing the existing
source or implementation-focused documentation. Ask it to:

1. Treat every MUST and the System Acceptance Checklist as the compatibility contract.
2. Choose its own language, frameworks, module layout, and internal algorithms.
3. Produce a requirement-to-test matrix keyed by the feature ids in Section 5.
4. Implement milestones in order, keeping each exit gate runnable.
5. Record ambiguities as explicit product decisions and choose the more restrictive
   security interpretation until clarified.

Reference source may be used later for black-box parity checks, but it is not needed to
design the replacement.

## 1. Product Definition

Colossus is a secure, local-first command-line agent harness for software development,
repository analysis, and source-backed research. It connects language models to a
brokered tool environment while preserving explicit permissions, user approvals, durable
state, context continuity, and auditability.

The product must be useful in three environments:

1. Fully offline with a deterministic smoke provider or a local compatible model server.
2. Locally operated with selected network integrations enabled and approved.
3. Connected to an online model provider while keeping tools and durable state local.

The default installation MUST work without credentials and MUST NOT make network calls
until the operator explicitly configures and permits a network-capable feature.

## 2. Product Goals

- Execute multi-turn coding tasks with validated model-callable tools.
- Make every consequential action observable, policy-checked, approval-aware, and audited.
- Preserve sessions, tasks, decisions, memories, plans, goals, research, and child-agent
  work across process restarts.
- Keep long conversations usable through automatic context compaction without deleting
  raw history.
- Support bounded autonomous Goal Mode and bounded parallel subagents without unbounded
  recursion.
- Produce cited research reports from repository, web, and configured MCP evidence.
- Run against online and local OpenAI-compatible providers through one normalized event
  contract.
- Remain useful in offline and airgapped environments.

## 3. Non-Goals

- Colossus is not an unrestricted shell wrapper.
- Approval-free mode is not a filesystem, network, or operating-system sandbox escape.
- Skills are not executable plugins. Executable extensions belong in capability packs.
- Context compaction does not replace or rewrite persisted conversation history.
- Memories are contextual facts and preferences, not privileged instructions.
- Provider hidden chain-of-thought is not a user-facing or persisted product surface.
- The initial product does not require a graphical desktop interface or remote cloud
  control plane.

## 4. Primary Users And Journeys

### 4.1 Repository Developer

The user selects a workspace, asks for a change, observes reads and commands, approves
mutations, receives a verified result, and can resume the same session later.

### 4.2 Interactive Operator

The user works in a persistent REPL, changes model roles and display preferences, manages
tasks and decisions, reviews context usage, and switches sessions without restarting the
application.

### 4.3 Autonomous Goal Operator

The user supplies an objective or an approved plan. Colossus runs bounded iterations,
preserves progress between iterations, and stops only when the goal is complete, blocked,
or its iteration budget is exhausted.

### 4.4 Researcher

The user asks a question, selects repository, web, or MCP sources, observes safe progress
telemetry, and receives a persisted report whose claims cite persisted source labels.

### 4.5 Airgapped Operator

The user installs from a verified bundle, uses local providers and offline tools, and can
inspect hashes, audit records, and package contents without network access.

## 5. Release Feature Baseline

| ID | Capability | Priority | Release acceptance summary |
| --- | --- | --- | --- |
| CORE-01 | Agent orchestration loop | P0 | Multi-turn model and tool execution is bounded, observable, and durable. |
| CORE-02 | Typed event stream | P0 | Provider and harness activity uses strict, replayable events. |
| PROV-01 | Provider abstraction | P0 | Echo, OpenAI Responses, and OpenAI-compatible chat are normalized. |
| TOOL-01 | Brokered tool system | P0 | Schemas, permissions, policy, approvals, execution, and audit run in order. |
| SAFE-01 | Policy and approval modes | P0 | `deny`, `ask`, `risk-auto`, and `full-access` preserve exact safety semantics. |
| STATE-01 | Durable sessions and run history | P0 | Sessions and events survive restarts and can be resumed. |
| UX-01 | One-shot CLI and interactive REPL | P0 | Both surfaces use the same application behavior and event stream. |
| CTX-01 | Context composition and compaction | P1 | Raw history is retained while provider input stays within budget. |
| WORK-01 | Tasks, decisions, memories, and plans | P1 | Long-running work state is persisted and inspectable. |
| GOAL-01 | Bounded Goal Mode | P1 | Autonomous iterations stop on terminal status or budget exhaustion. |
| AGENT-01 | Durable subagents | P1 | Queued child jobs support bounded concurrency and lifecycle controls. |
| RES-01 | Deep Research | P1 | Evidence collection, claims, citations, progress, and fallback are durable. |
| OBS-01 | Telemetry and observability | P1 | Run duration, event, tool, approval, risk, research, and error metrics are queryable. |
| EXT-01 | Skills and resources | P1 | Skills compose prompt context under precedence and resource-access rules. |
| INT-01 | Integrations and credential broker | P2 | Connected services expose normal tools without exposing credentials. |
| PACK-01 | Capability packs | P2 | Executable extensions are declared, verified, trusted, and lifecycle-managed. |
| DIST-01 | Offline bundles | P2 | Distributions can be verified and installed without network access. |

P0 is the minimum useful and secure product. P1 provides full agent workflow parity. P2
provides the complete extension and distribution ecosystem.

## 6. Logical Architecture

The implementation MUST preserve these logical boundaries even if it uses different
module names or deployment units:

```text
user interfaces -> application workflows -> ports/contracts -> domain values
adapters --------> application workflows -> ports/contracts -> domain values
infrastructure may compose implementations but must not own product behavior
```

- The domain contains strict values, statuses, requests, results, events, and errors.
- Application workflows own orchestration, policy ordering, compaction, goals, research,
  subagents, telemetry, and state transitions.
- Ports define provider, tool, state, approval, audit, skill, credential, research, and
  integration contracts.
- Adapters interact with model APIs, subprocesses, filesystems, databases, HTTP, MCP,
  package resources, and audit sinks.
- CLI and REPL surfaces collect input and render typed output only. They MUST NOT contain
  provider calls, tool execution, policy decisions, or persistence logic.

Dependency direction MUST point inward. A boundary test SHOULD fail the build if domain
or application code imports a user-interface implementation.

## 7. Agent Orchestration

### 7.1 Run Lifecycle

Every normal agent run MUST perform the following sequence:

1. Create a stable run identifier and ensure the session exists.
2. Persist the user message.
3. Resolve the requested model role and active model profile.
4. Compose active decisions, relevant memories, context snapshot, recent messages,
   agent instructions, and active skills.
5. Build the active tool catalog and emit `model.request.prepared`.
6. Ask the provider for a typed event stream.
7. Persist and observe safe provider events as they arrive.
8. If the provider requests tools, validate all arguments before policy evaluation.
9. Apply deterministic policy, optional risk review, and approval handling.
10. Execute approved tools, persist results, and continue the provider loop.
11. Stop on final visible output, a non-recoverable error, or the model-turn limit.
12. Persist the final assistant message and return run metadata including elapsed time.

The default maximum is 24 model turns per run. The configured value MUST be at least 1
and MUST NOT exceed 100. Reaching the turn limit MUST be distinguishable from provider
failure in audit and telemetry.

### 7.2 Provider Failure Recovery

- Provider adapters MUST reject malformed or non-object tool-call arguments.
- The orchestrator MUST NOT repair malformed JSON into an executable tool call.
- It MUST emit a recoverable error, append a metadata-only correction message, and ask
  the provider to retry at most two times when turn budget remains.
- Recovery and exhaustion MUST be audited.
- No policy, approval, or tool execution stage may receive the malformed call.
- A provider response containing neither visible assistant content nor a tool call MUST
  fail with a diagnostic that includes normalized response shape information. A future
  bounded retry for this separate condition may be added without changing tool safety.

### 7.3 Run Result

A completed run returns at least:

- `run_id`
- `session_id`, when attached
- final visible assistant output
- number of recorded events
- elapsed seconds

The terminal MUST display a human-readable elapsed duration after normal runs and Goal
Mode runs.

## 8. Model Providers And Routing

### 8.1 Provider Types

The baseline provider catalog contains:

- A deterministic, credential-free echo provider for installation and offline smoke tests.
- An OpenAI Responses API provider.
- A local or hosted OpenAI-compatible Chat Completions provider.

All providers MUST normalize streaming text, tool calls, final output, safe reasoning
summaries, diagnostics, and errors into the same domain contracts.

### 8.2 Model Roles

Configuration maps named roles to named profiles. Required roles are:

- `primary`
- `risk_evaluator`
- `context_summarizer`
- `subagent_default`
- `research_planner`
- `research_worker`
- `research_synthesizer`

Unconfigured specialized roles SHOULD fall back to the primary profile. A legacy single
provider configuration MAY be normalized into the primary role.

### 8.3 Diagnostics

Operators MUST be able to:

- inspect role-to-profile routing;
- test provider endpoint and credential readiness;
- list provider models when the endpoint supports a model catalog;
- report provider capabilities, including tool-call support;
- distinguish endpoint failure, authentication failure, unsupported tools, malformed
  provider output, and model-turn exhaustion.

## 9. Typed Event Contract

Events MUST be strict, append-only, timestamped when persisted, ordered within a run, and
replayable. Consumers MUST ignore unknown future event types. Raw provider chunks and
hidden reasoning MUST NOT be persisted as default events.

| Event type | Required purpose and minimum payload |
| --- | --- |
| `model.delta` | Stream visible text with `text`. |
| `reasoning.summary` | Safe provider summary with `summary` and optional format/id. |
| `model.request.prepared` | Record turn, model, composed request metadata, and size estimate. |
| `context.prepared` | Record token estimates, window, thresholds, snapshot, and compaction flags. |
| `tool.call.requested` | Record `call_id`, tool name, and validated arguments. |
| `approval.requested` | Record `call_id` and reason. |
| `approval.auto_granted` | Record `call_id` and no-prompt approval reason. |
| `risk.assessment` | Record tool, risk level, summary, concerns, recommendation, and model route. |
| `tool.call.completed` | Record call id, name, bounded output, and exit code. |
| `handoff` | Record source agent, destination agent, and optional reason. |
| `subagent.status` | Record job id, lifecycle status, role, task, and safe message. |
| `research.status` | Record coarse research phase, status, message, and source count. |
| `research.progress` | Record bounded operational progress as defined below. |
| `final.output` | Record visible final assistant text. |
| `error` | Record safe error text and whether recovery is possible. |

`research.progress` contains:

- `research_id`
- `phase`
- `action`
- `status`: `started`, `completed`, `skipped`, or `failed`
- optional `message`, `query`, and `source_kind`
- numeric `current`, `total`, `sources_collected`, and `claims_collected`
- bounded structured `details`

Progress events expose operational metadata only. They MUST NOT expose hidden reasoning,
raw prompts, provider internals, credentials, or unbounded source content.

## 10. Tool System

### 10.1 Tool Contract

Every model-callable tool MUST declare:

- unique name and user-meaningful description;
- strict JSON input schema with unknown fields rejected;
- optional output schema;
- filesystem permission: none, read, or write;
- network permission: deny or allow;
- whether approval is required;
- whether the operation mutates state;
- whether a working root is required;
- risk level: low, medium, or high;
- timeout and maximum output bytes.

The execution order is fixed:

```text
schema validation -> deterministic policy -> optional risk review
-> approval decision -> brokered execution -> bounded result -> event and audit
```

### 10.2 Required Offline Tool Catalog

- Filesystem: `filesystem.list`, `filesystem.read`, `filesystem.search`,
  `filesystem.write`, `filesystem.replace`.
- Git inspection: `git.status`, `git.diff`, `git.show`.
- Structured command execution: `shell.run`.
- Interactive question: `user.ask`, exposed only when an interactive prompt handler is
  available.
- Tasks: `task.create`, `task.update`, `task.list`.
- Decisions: `decision.create`, `decision.update`, `decision.list`,
  `decision.archive`, `decision.supersede`.
- Memories: `memory.create`, `memory.update`, `memory.list`, `memory.search`,
  `memory.archive`, `memory.supersede`.
- Plans: `plan.create`, `plan.show`, `plan.approve_request`.
- Active goals: `goal.show`, `goal.update`.
- Patches: `patch.preview`, `patch.apply`, `patch.reverse`.
- Repository context: `repo.map`, `repo.symbol_search`, `repo.references`,
  `repo.file_summary`.
- Subagents: `agent.delegate`, `agent.result`, `agent.list`.
- Discovery: `tool.search`, `mcp.servers`, `mcp.tools`.
- Trace: `trace.show`, `trace.export`.
- Context: `context.show`, `context.compact`, `context.snapshots`,
  `context.restore`.
- Skill authoring/resources: `skill.scaffold`, `skill.inspect`, `skill.read`,
  `skill.write`, `skill.validate`, `skill.install`, `skill.resource.list`,
  `skill.resource.read`.
- Deterministic smoke test: `echo`.

The base catalog intentionally does not invent dedicated `test.run`, `lint.run`,
`typecheck.run`, `build.run`, or `eval.run` tools. Agents use structured `shell.run` or
pack-provided tools for repository-specific verification.

### 10.3 Network And Adapter-Backed Tools

- `web.fetch` and `docs.fetch` provide approval-gated, bounded HTTP(S) retrieval.
- `web.search` is exposed only when a search adapter is configured.
- `mcp.call` is exposed only through an explicitly configured, allowlisted gateway.
- Connected native integrations expose namespaced tools such as `github.*`,
  `searxng.*`, and `opensearch.*`.
- Imported API operations use names like `openapi.NAME.OPERATION`.

Integration-generated tools enter the same schema, policy, approval, execution, event,
and audit pipeline as built-ins.

### 10.4 Filesystem And Subprocess Rules

- Generic file tools are confined to the selected workspace.
- Control directories owned by Colossus are denied to generic workspace mutation tools.
- Text reads and writes are bounded and reject unsafe file types where applicable.
- Mutating file and patch results include a diff and changed line ranges.
- Subprocesses use structured argument arrays and MUST NOT use a shell interpreter by
  default.
- Timeouts, output caps, working directory, and environment allowlists are enforced by a
  broker.

## 11. Policy, Risk, Approval, And Audit

### 11.1 Approval Modes

- `deny`: block every operation that requires approval.
- `ask`: prompt the user before approval-required operations.
- `risk-auto`: run model-assisted review for eligible `shell.run` calls and auto-approve
  only low-risk calls whose recommendation is allow. A model recommendation to deny is
  escalated to an explicit user prompt.
- `full-access`: auto-approve approval-required operations without prompting and skip
  model-assisted shell risk review.

`full-access` MUST NOT expand workspace roots, grant missing network implementations,
change tool schemas, bypass deterministic denies, or make unknown tools executable.

One-shot runs default to deny unless approval prompting is requested. Interactive REPL
sessions default to ask.

### 11.2 Deterministic Policy

Deterministic policy MUST run before model-assisted risk review. A risk model may
escalate risk or add context but MUST NOT override a deterministic deny. If risk review
is unavailable or malformed, the system records that fact and continues with the
deterministic decision.

### 11.3 Audit

Audit records MUST be append-only and hash-chained. Records cover at least:

- runs and model-turn terminal conditions;
- tool validation, policy, approval, execution, and failure;
- model-assisted risk review;
- provider recovery and recovery exhaustion;
- goal, plan, task, decision, memory, and subagent state transitions;
- context compaction and restore;
- research source-lane approvals and limitations;
- credential-reference use, skill activity, packs, and bundles.

Audit payloads MUST be bounded and redacted. Raw credentials, private keys, full skill
bodies, unbounded command output, and hidden reasoning are prohibited.

## 12. Sessions, Context, Decisions, And Memories

### 12.1 Sessions

Sessions are durable local records with stable ids, titles, creation/update timestamps,
message counts, and recent-user previews. Users can create, list, inspect, resume by id,
or resume the most recently updated session.

Messages and run events remain append-only. A run may attach to an existing session or
create one. Session resumption MUST restore conversation continuity without silently
changing workspace or approval boundaries.

### 12.2 Context Compaction

Before each provider turn, Colossus estimates the composed request against the active
model context window. Default behavior:

- automatic compaction enabled;
- fallback context window: 32,768 tokens;
- compact at 70 percent;
- target 45 percent after compaction;
- preserve the most recent 8 messages;
- reserve a bounded portion for tool schemas;
- prefer a model-assisted summary with deterministic fallback.

A snapshot contains a summary, source-message range, pinned facts, open tasks, files
touched, notable tool results, strategy, and timestamp. Activating a snapshot changes
future provider input but never deletes raw messages.

Compaction MUST emit `context.prepared` with original and resulting estimates. User
surfaces SHOULD visibly mark automatic compaction in the flow so operators can tell when
history has been summarized.

### 12.3 Tasks

Tasks are session-scoped and use `pending`, `in_progress`, `completed`, `blocked`, or
`cancelled`. They contain id, title, description, and timestamps.

### 12.4 Key Decisions

Decisions are durable future-facing commitments with source, priority, status, title,
decision, intent, applicability, rationale, source excerpt, and optional goal/plan links.
Active decisions are injected before memories and snapshots. Archived and superseded
records remain available for history but do not steer future turns.

### 12.5 Memories

Memories have global, repository, or session scope and a kind, confidence, source,
status, text, rationale, staleness/expiry metadata, and optional supersession link.
Relevant active memories are retrieved through full-text search and injected after
active decisions. Memories MUST NOT store secrets.

### 12.6 Plans

Plans are `draft`, `approved`, or `executed`. A plan stores the originating prompt,
markdown content, ordered steps, mutation flags, session id, and timestamps.

Plan Mode may inspect context and create tasks, but it MUST NOT mutate the workspace or
claim implementation is complete. A draft can be reviewed, approved, discarded,
executed once, or handed to Goal Mode. Execution preserves the plan id for lineage.

## 13. Goal Mode

Goal Mode is a bounded autonomous continuation loop built on the normal agent
orchestrator. It is not a separate permission domain.

- Starting Goal Mode creates a durable goal with objective, session, status, iteration
  budget, timestamps, and optional source plan id.
- Status is `active`, `complete`, or `blocked`.
- Each iteration runs a normal agent request with the same tools, policy, approval,
  context, skills, audit, and session behavior.
- Only active Goal Mode turns expose `goal.show` and `goal.update`.
- The agent marks a genuinely finished objective complete and records a concise summary.
- The agent marks a goal blocked only when meaningful progress requires user input or an
  external state change, and records a reason.
- If work remains, the goal stays active and the next iteration begins.
- The loop stops on complete, blocked, error, or iteration-budget exhaustion.

The default budget is 5 iterations and the CLI maximum is 50. Each iteration also has
the normal model-turn limit. Budget exhaustion returns control to the user; it does not
pretend the goal completed. Results include per-iteration run ids and elapsed durations,
plus total elapsed time.

## 14. Subagents

Subagents are durable queued child-agent jobs. Each job stores parent run/call ids,
parent and child session ids, task, model role, status, child run id, final output, error,
and lifecycle timestamps.

Required statuses are `queued`, `running`, `completed`, `failed`, `cancelled`, and
`interrupted`.

- The default maximum concurrency is 10 and MUST be configurable to any positive value.
- Child agents use the normal provider, tools, policy, approval mode, risk review,
  context, state, and audit paths.
- Child tool catalogs MUST remove nested delegation to prevent recursive job trees.
- Queued jobs are scheduled only while capacity is available.
- Running jobs left by a stopped process are marked interrupted at startup.
- Failed, cancelled, and interrupted jobs can be requeued; completed jobs cannot.
- Users can list jobs, inspect queue status and result previews, drain with an optional
  timeout, cancel, and resume/requeue.
- Parent runs SHOULD show child status and bounded result previews in the main transcript.

## 15. Deep Research

Research is a separate application workflow with four phases:

1. **Planning:** generate bounded queries or use deterministic fallback queries.
2. **Collecting:** collect repository, web, and configured MCP evidence per query.
3. **Workers:** extract source-backed claims from persisted sources.
4. **Synthesis:** assemble a cited report or use a deterministic fallback report.

Research depth is `quick`, `standard`, or `deep`. Defaults are 20 maximum sources and 4
workers; configuration supports up to 100 sources and 16 workers.

Repository collection is read-only. Web and MCP lanes require configuration and network
approval. Disabled, skipped, denied, or failed lanes are recorded as limitations while
the run continues with available evidence.

Each source stores a stable id, human citation label, kind, title, URI, bounded content,
originating query, metadata, and timestamp. Each claim stores text and one or more source
labels. Reports cite labels such as `[R1]`, are persisted on the research run, and are
appended as a normal assistant session message.

Progress visibility MUST include:

- generated or fallback query list;
- each repo/web/MCP lane per query, including configured, skipped, approved, failed, and
  result count states;
- bounded source labels and titles as sources are saved;
- claim extraction position per source with current/total counts;
- synthesis prompt assembly, model start, accepted report, or deterministic fallback.

Terminal rendering SHOULD group progress under one subtle phase block with indented
activity lines. It SHOULD NOT create a large bordered panel for every source or query.
The bottom activity indicator SHOULD show the most recent action during long collection
and synthesis phases.

## 16. Skills, Packs, And Integrations

### 16.1 Skills

A skill is prompt/context data with a manifest, instructions, trigger words, required
tools, permission labels, offline compatibility, source, and optional resources.

- Skill precedence and user override behavior are deterministic.
- Skills may be activated by explicit option, sticky REPL state, or prompt mention.
- Required tools are validated against the active catalog but never auto-approved.
- Resource access is read-only, active-skill-scoped, path-safe, regular-file-only, and
  bounded.
- Authoring supports scaffold, inspect, read, optimistic-concurrency write, validate,
  and install.
- Skill directories may contain scripts as resources, but Colossus does not execute
  them directly as privileged plugins.

### 16.2 Capability Packs

Packs are the executable extension boundary. A pack manifest may declare integrations,
skills, tools, MCP servers, binaries, container assets, documentation, and tests.

Executable files are hash-listed and permission-declared. Pack workflows include list,
show, verify, validate, install, enable, disable, uninstall, and publisher trust
management. Installation MUST validate containment, hashes, signatures when present,
permissions, and trust before activation.

### 16.3 Integrations

Connections store manifests, local configuration, scopes, status, and credential
references, never raw secrets. Credentials are resolved at execution time by a broker.

Baseline integrations include:

- GitHub repository, issue, pull request, check, and release reads;
- SearXNG search and health;
- OpenSearch cluster information, health, index/mapping discovery, search, document
  retrieval, indexing, update, and delete;
- JSON OpenAPI import that maps path, query, and body parameters into namespaced tools;
- configured MCP server discovery and allowlisted execution.

Tools remain hidden until their integration is connected and valid.

## 17. Telemetry And Observability

Telemetry is derived from timestamped persisted run events. It MUST NOT require parsing
rendered transcript text.

Operators can list recent runs, inspect one run by full id or unique prefix, and aggregate
metrics over recent runs. Summary fields include:

- run/session ids, start/end timestamps, and duration;
- total events and counts by event type;
- visible model-output characters;
- tool calls and tool errors;
- approval requests and automatic approvals;
- risk assessments;
- research and subagent events;
- context compactions;
- recoverable and terminal errors;
- final-output count.

Default telemetry output is metadata-only and MUST NOT reveal raw prompts, hidden
reasoning, credentials, or raw tool output. Detailed event replay remains bounded and
uses the same redaction rules as normal traces.

## 18. CLI And REPL Surface

### 18.1 Global Options

The CLI supports verbosity, provider/profile overrides, model selection, context-window
override, base URL, API-key value or environment reference, provider/global CA bundles,
mTLS client certificate/key, key-password environment reference, proxy URL or environment
reference, HTTP environment-trust toggle, and shell completion.

### 18.2 Top-Level Commands

- `run`: normal turn, plan creation, approved-plan execution, or plan-to-goal handoff.
- `goal`: bounded autonomous goal loop.
- `research`: deep research with persisted cited output.
- `repl`: interactive session.
- `config`: initialize and show strict configuration.
- `skills`: list, create, validate, and install.
- `tools`: list the active catalog.
- `provider` and `models`: diagnostics, catalogs, and routing.
- `agents`: list, status, show, drain, cancel, and resume.
- `bundle`: verify an offline bundle.
- `goals`, `plans`, `tasks`, `decisions`, and `memories`: inspect or manage durable work.
- `context`: show, compact, list snapshots, and restore.
- `sessions`: list and show.
- `telemetry`: list runs, show a run, and aggregate metrics.
- `integrations`: list, show, connect, disconnect, and import OpenAPI.
- `packs`: lifecycle, validation, verification, and trust operations.

### 18.3 REPL Commands

The REPL includes:

- Runtime: `/model`, `/agent`, `/tools`, `/workspace`, `/status`, `/clear`, `/exit`.
- Sessions/context: `/resume`, `/sessions`, `/session`, `/context`, `/compact`.
- Rendering: `/stream`, `/events`, `/reasoning`, `/transcript`, `/multiline`, `/trace`.
- Preferences/themes: `/theme`, `/repl`.
- Work state: `/tasks`, `/decision`, `/decisions`, `/memory`, `/memories`.
- Workflows: `/plan`, `/goal`, `/research`, `/agents`.
- Extensions: `/skill`, `/skills`, `/integrations`, `/packs`.
- Discovery: `/help`.

The REPL persists display preferences for theme, multiline composition, streaming mode,
event detail, transcript density, and reasoning-summary visibility. Preferences affect
rendering only and MUST NOT change provider, policy, tool, or approval behavior.

### 18.4 Rendering

- Compact, verbose, and off event modes are supported.
- Assistant output can stream without corrupting prompt input or semantic event output.
- Tool results use semantic renderers for files, shell, git, work state, context, repo,
  skills, web/search, MCP, traces, integrations, packs, and generic structured output.
- Comfortable transcript mode uses readable blocks; compact mode minimizes vertical
  space.
- Safe reasoning summaries can be toggled independently from tool/activity events.
- Errors clearly identify whether they are recoverable.
- Long-running activity shows current phase/action and elapsed time.

## 19. Configuration And Local Storage

Configuration is strict: unknown fields fail validation. It covers provider defaults,
named model profiles and roles, context budgets, agent turn limits, subagent concurrency,
memory index, global HTTP transport, research limits/sources/search/MCP, and skill
override policy.

Configuration accepts credential references such as `env:NAME`. Raw secret values MUST
not be written back when configuration is shown.

The default local state store is SQLite or an equivalent transactional embedded store.
It persists sessions, messages, run events, snapshots, tasks, decisions, memories,
plans, goals, subagents, research sources/claims/reports, preferences, integrations,
packs, trust records, and telemetry inputs. Schema migration MUST preserve existing user
records or fail safely with a clear recovery path.

Colossus-owned HTTP clients share configurable CA, mTLS, proxy, and environment-trust
settings. Transport configuration does not grant network approval.

## 20. Offline Distribution

An offline release bundle SHOULD contain application artifacts, locked dependencies,
wheelhouse or equivalent dependency cache, SBOM, manifests, checksums, signatures,
bundled skills, and documentation.

Bundle verification MUST reject missing files, hash mismatches, path traversal, symlink
escapes, malformed manifests, and invalid signatures. Verification works without network
access and produces retainable evidence for operators.

## 21. Non-Functional Requirements

### 21.1 Security

- No shell-string execution in brokered subprocess paths.
- No raw secret in model-visible schemas, prompts, transcripts, telemetry, or audit.
- No mutation before schema validation, policy, and approval.
- No hidden network use in the default configuration.
- No hidden reasoning in default persisted or rendered data.
- Path containment is verified after canonicalization and across symlinks.

### 21.2 Reliability

- State transitions are atomic and restart-safe.
- Provider streams can fail without losing prior persisted events.
- Long-running loops are bounded by turn, iteration, worker, source, timeout, and output
  limits.
- Disabled integrations degrade to explicit unavailable/skipped states.
- Deterministic fallbacks exist for context compaction and research synthesis.

### 21.3 Testability

- A deterministic provider can exercise the full harness without credentials.
- Providers, state, audit, approvals, tools, research sources, and user prompts are
  replaceable behind contracts.
- Every behavior change includes focused tests.
- Boundary, security, schema round-trip, renderer, persistence, and restart tests are
  first-class suites.

### 21.4 Performance

- Streaming output is incremental.
- Tool, source, and event outputs are bounded before rendering or persistence.
- Subagent and research concurrency is explicitly capped.
- Full-text memory search and recent-run telemetry remain responsive for normal local
  state volumes.

## 22. Implementation Milestones

### Milestone 0: Contracts And Safety

Deliver strict domain models, typed events, tool schemas, deterministic policy, approval
modes, state/audit ports, redaction, and boundary tests.

Exit gate: invalid tools cannot reach execution, audit chaining verifies, and event
round trips pass.

### Milestone 1: Minimum Agent

Deliver echo and compatible model providers, role routing, one-shot runs, streaming,
workspace reads, git inspection, structured shell execution, mutations with approval,
run persistence, and elapsed time.

Exit gate: a clean checkout can smoke test offline and complete a multi-turn tool run.

### Milestone 2: Durable Interactive Work

Deliver REPL, session resume, semantic rendering, preferences/themes, tasks, decisions,
memories, plans, context budgets, snapshots, and automatic compaction.

Exit gate: a long session can compact, restart, resume, and preserve raw history and
active commitments.

### Milestone 3: Autonomous Workflows

Deliver Goal Mode, active-goal tools, plan handoff, subagent queue, configurable
concurrency, drain/cancel/resume, and parent result previews.

Exit gate: bounded goals and parallel child jobs survive process interruption without
unbounded delegation.

### Milestone 4: Research And Observability

Deliver research planning/collection/workers/synthesis, progress events, source-backed
citations, deterministic fallbacks, telemetry run lists/details/metrics, and elapsed
activity rendering.

Exit gate: a repository-only research run produces a persisted cited report and an
operator can inspect its run telemetry without raw sensitive payloads.

### Milestone 5: Extension And Distribution

Deliver skills/resources, credential-brokered integrations, API import, MCP allowlists,
capability packs, publisher trust, and offline bundle verification.

Exit gate: an extension can be installed, verified, enabled, used through normal policy,
disabled, and audited without exposing its credentials.

## 23. System Acceptance Checklist

A reconstruction is complete only when all applicable checks pass:

- [ ] Offline install and echo smoke test require no credentials or network.
- [ ] One agent run supports streaming text, multiple tool turns, final output, run id,
  session id, event count, and elapsed time.
- [ ] Malformed provider tool arguments are retried within bounds and never executed.
- [ ] Max-turn exhaustion is reported separately from empty or malformed provider output.
- [ ] Filesystem escapes, shell wrappers, unknown tool arguments, and deterministic
  policy denies stop before execution.
- [ ] Every approval mode has tests for allowed, denied, and approval-required tools.
- [ ] Sessions, messages, events, tasks, decisions, memories, plans, goals, subagents,
  and research survive restart.
- [ ] Automatic compaction is visible, preserves raw history, and has deterministic
  fallback.
- [ ] Plan Mode cannot mutate; approved plans can execute once or enter Goal Mode.
- [ ] Goal Mode stops correctly on complete, blocked, error, or budget exhaustion.
- [ ] Subagents respect configured concurrency, cannot delegate recursively, and can be
  cancelled or resumed after interruption.
- [ ] Research records planned queries, lane decisions, source labels, worker progress,
  citations, synthesis choice, and limitations.
- [ ] Compact and verbose renderers cover every event type and every tool family.
- [ ] Telemetry derives correct duration and counts from persisted event timestamps.
- [ ] Credentials remain references until adapter execution and never appear in model or
  user-visible diagnostic payloads.
- [ ] Skills cannot gain executable privilege; packs cannot activate before verification.
- [ ] Audit-chain verification detects tampering.
- [ ] Strict configuration rejects unknown fields and safely redacts displayed secrets.
- [ ] Unit, integration, boundary, security, type, lint, and packaging checks pass.

## 24. Explicitly Deferred Product Decisions

These choices are not required to reconstruct the baseline product and should not block
the milestones above:

- Remote multi-user control plane and authentication.
- Graphical desktop or browser interface.
- Operating-system container sandbox beyond the subprocess broker boundary.
- Unbounded or self-replicating child-agent trees.
- Storage of raw provider chain-of-thought.
- Automatic execution of scripts found inside skill directories.
- A universal dedicated verification tool family; repository verification remains
  available through structured commands and capability packs.
