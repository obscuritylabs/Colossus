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
| AUDIT-01 | Tamper-evident event journal | P0 | The authoritative journal reconstructs state and proves every attempted and uncertain effect. |
| AUTHZ-01 | Universal effect authorization | P0 | No external or sensitive effect can execute without the single policy gateway issuing a matching one-use permit. |
| STORE-01 | Replaceable storage ports | P0 | Journals, repositories, projections, signing, indexes, embeddings, and exports have conformance-tested adapters. |
| PROV-01 | Provider abstraction | P0 | Echo, OpenAI Responses, and OpenAI-compatible chat are normalized. |
| TOOL-01 | Brokered tool system | P0 | Schemas, permissions, policy, approvals, execution, and audit run in order. |
| SAFE-01 | Policy and approval modes | P0 | `deny`, `ask`, `risk-auto`, and `full-access` preserve exact safety semantics. |
| STATE-01 | Durable sessions and run history | P0 | Sessions and events survive restarts and can be resumed. |
| UX-01 | One-shot CLI and interactive REPL | P0 | Both surfaces use the same application behavior and event stream. |
| CTX-01 | Context composition and compaction | P1 | Raw history is retained while provider input stays within budget. |
| WORK-01 | Tasks, decisions, memories, and plans | P1 | Long-running work state is persisted and inspectable. |
| MEM-01 | Canonical memory and disposable indexes | P1 | Memory lifecycle state remains authoritative and usable while lexical or semantic indexes lag or rebuild. |
| FLOW-01 | Durable versioned workflows | P1 | YAML workflows are validated, hash-pinned, restartable, bounded, and use normal authorization for every effectful step. |
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

State MUST NOT be hidden behind one monolithic storage interface. The minimum storage
port set is `EventJournal`, `CheckpointSigner`, `ProjectionStore`,
`SessionRepository`, `WorkRepository`, `MemoryRepository`, `WorkflowRepository`,
`ResearchRepository`, `ExtensionRepository`, `MemoryIndex`, `EmbeddingProvider`, and
`AuditExporter`. Adapter implementations MUST pass shared behavioral conformance tests.

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

All providers MUST normalize streaming text, tool calls, final output, token usage, safe
reasoning summaries, diagnostics, and errors into the same domain contracts. Raw SSE
frames remain adapter-private; only normalized items cross the per-item release gate.

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
| `provider.usage` | Record normalized input, output, total, cached-input, and reasoning token counts. |
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

### 11.0 Universal Effect Boundary

Every filesystem access, process spawn, network or provider call, credential use,
memory operation, durable domain mutation, workflow transition, subagent operation, and
executable extension operation MUST enter one effect gateway before an adapter can run.
Effectful adapter constructors remain runtime-private, and effectful methods require an
opaque, authenticated `ExecutionPermit` bound to the canonical request hash, policy
decision, actor, obligations, nonce, and short expiry. Permits are single-use.

The trusted safety kernel rejects invalid schemas, unknown or unsigned capabilities,
path escapes, invalid or expired permits, missing sandbox obligations, unredacted hard
secrets, and unavailable audit durability. Policy cannot override these checks. Journal
append, policy lookup, pure computation, rendering, schema validation, and projection
replay do not recursively require permits.

The versioned `EffectRequest` includes actor/provenance, action/resource, complete
proposed request content, credential references with values removed, declared
capabilities and risk, execution context, correlation/causation/idempotency identifiers,
and pre- or post-effect phase. A strict `PolicyDecision` is allow, deny, or
require-approval and includes decision/revision identifiers, reason, sandbox and
filesystem/network obligations, resource bounds, redactions, post-effect requirements,
audit labels, and retention obligations.

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
is unavailable, the policy input explicitly records `risk.status: unavailable`.

When configured, OPA is the primary policy decision point and Colossus remains the
enforcement point. Local and remote OPA are supported. Remote OPA requires HTTPS, mTLS,
pinned trust, a fixed decision path, readiness checks, and explicit acknowledgement that
logical request content is disclosed. Without OPA, an offline built-in policy provider
implements the same decision contract.

Unknown obligations, invalid or incomplete responses, unhealthy policy bundles,
transport failures, timeouts, and inputs over the default 1 MiB cap fail closed. Full
logical request content is disclosed by default, but credentials, private keys,
authentication headers, hidden reasoning, and key material are always replaced by
references and hashes. Approval is a policy obligation: after a user approves, policy is
re-evaluated with the approval proof before a permit is minted.

Reads, provider/network responses, subprocess output, and memory retrieval support a
two-phase release gate. The request is pre-authorized, output is captured in a bounded
quarantine, content receives a post-effect decision, and only an allow decision releases
it to the requester. Denied content MUST never reach the model, workflow, or user.

### 11.3 Audit

The event journal is the authoritative source for reconstructing product state. Audit
records MUST be immutable, encrypted by default, append-only, and hash-chained. Every
aggregate append uses optimistic concurrency through an expected stream version. Each
envelope carries schema/event versions, a UUIDv7 identifier, global and stream sequence,
classification, actor, correlation and causation context, UTC timestamp, encrypted
payload descriptor, plaintext payload hash, previous record hash, and record hash.

Records cover at least:

- runs and model-turn terminal conditions;
- tool validation, policy, approval, execution, and failure;
- model-assisted risk review;
- provider recovery and recovery exhaustion;
- goal, plan, task, decision, memory, and subagent state transitions;
- context compaction and restore;
- research source-lane approvals and limitations;
- credential-reference use, skill activity, packs, and bundles.

Audit payloads MUST be bounded and redacted. Raw credentials, private keys, full skill
bodies, unbounded command output, and hidden reasoning are prohibited. Sensitive event
payloads use authenticated encryption with keys from an explicit platform or environment
key provider; there is no plaintext downgrade.

The chain is signed at least every 100 events or 60 seconds and at clean shutdown. The
latest secure anchor is stored separately and may also be exported to remote or WORM
storage. Startup verifies the chain, anchor, checkpoint signatures, and projection
positions. Verification failure places the runtime in read-only recovery mode and blocks
new effects. An `effect.started` with no terminal event becomes `outcome_unknown` during
recovery and is never silently retried. Operators can verify, inspect, export, and check
anchor status through bounded redacted audit commands.

Configured audit sinks MUST consume the journal's atomic external-work outbox using an
independent durable position. Export evidence MUST contain sufficient envelope, lineage,
payload-hash, and chain-hash metadata for verification but MUST NOT contain payload
ciphertext, nonce, plaintext, credentials, or hidden reasoning. External delivery crosses
the effect gateway with `audit.export.write` (or an adapter-specific capability), and an
unknown delivery outcome blocks automatic retry. Policy lifecycle events created by an
export MUST remain in the canonical journal but MUST NOT create an unbounded recursive
export loop. The initial directory adapter is deterministic and replay-safe; it does not
claim WORM durability.

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

`MemoryRepository` owns canonical lifecycle events and reconstructs active records.
`MemoryIndex` is a disposable projection exposing `upsert`, `remove`, `search`, `status`,
and `rebuild`. The offline lexical default is Tantivy; Chroma is an optional semantic
adapter that stores candidate ids, caller-supplied embeddings, bounded searchable text,
and bounded metadata, never canonical lifecycle state. Search results are reloaded from
the repository and re-filtered for scope, status, expiry, supersession, and policy.
Index work is queued with event ids as idempotency keys. Each adapter has an independent
durable position and retry record; transient failures back off exponentially while
unknown outcomes require operator-authorized rebuild. Lag, attempt count, next retry,
and bounded redacted errors are visible, and canonical memory remains usable while any
index is unavailable. Chroma, embeddings, and remote index operations cross the effect
gateway.

### 12.6 Plans

Plans are `draft`, `approved`, or `executed`. A plan stores the originating prompt,
markdown content, ordered steps, mutation flags, session id, and timestamps.

Plan Mode may inspect context and create tasks, but it MUST NOT mutate the workspace or
claim implementation is complete. A draft can be reviewed, approved, discarded,
executed once, or handed to Goal Mode. Execution preserves the plan id for lineage.

### 12.7 Durable Workflows

Workflow definitions are loaded from `.colossus/workflows/*.yaml` and the platform user
configuration directory's `workflows/` library. A definition has `apiVersion`, `kind`,
name/version/description metadata, JSON Schema inputs/outputs, maximum declared
capabilities, bounded concurrency and total-step budget, and typed steps: `agent`,
`tool`, `workflow`, `approval`, `condition`, `parallel`, `foreach`, `wait_for_input`, and
`emit`.

Workflow YAML MUST NOT execute inline shell, Rust, JavaScript, Python, or Rego. Conditions
use a small non-executable grammar limited to JSON-pointer lookup, existence, equality,
comparison, and boolean operators. Every definition is content-hashed; any change
invalidates prior trust, and each run pins its exact hash and provenance.

Every effectful step creates a normal effect request containing workflow hash, run id,
step id, and attempt. Parallelism, iteration, recursion depth, and total steps are
bounded; cycles are rejected. Effectful retries require an explicit idempotency strategy.
Compensation is explicit and independently authorized. External exactly-once execution
is never claimed.

Run statuses are `queued`, `running`, `waiting`, `completed`, `failed`, `cancelled`,
and `interrupted`. State is reconstructed from the journal. Recovery marks abandoned
attempts interrupted or unknown rather than rerunning them. The application API exposes
definition validation/registration plus start, get, list, input, resume, cancel, and
drain. CLI, REPL, an embedded API, and an optional single-writer worker all invoke that
same application layer.

When the worker is active it owns the canonical writer lease. Local clients MUST
authenticate the server before disclosing operation content, authenticate every request
and ordered response frame, bind requests to a single connection, reject replay and
stale timestamps, and bound framing before allocation. Unix sockets are owner-only;
Windows named pipes implement the same logical contract. Supported CLI/REPL operations
auto-discover the worker and fall back to the embedded runtime only when no authenticated
worker is active.

Worker routing covers model runs, core REPL turns, sessions, workflows, audit and runtime
diagnostics, context and telemetry, plus canonical tasks, decisions, plans, goals,
subagents, memories, and memory-index maintenance. Approval mode belongs to the worker
process; a client-side approval override MUST fail rather than silently change authority.
Research, skills, packs, bundles, integrations, MCP, process, and network commands use
typed operations too; `@path` JSON and other file-backed inputs are read by the worker
through the normal filesystem permission boundary rather than by the client.
When a worker is active, the REPL routes its implemented session, context, work,
workflow, research, telemetry, skill, pack, bundle, integration, MCP, audit, projection,
and tool commands through those same typed operations. Embedded and worker REPLs also
accept line-oriented stdin without requiring a terminal.

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

Publisher trust MUST bind a publisher identity to an exact Ed25519 public key. A
publisher name or signature key id alone is not trust evidence. Present invalid or
unknown signatures fail closed. Pack and bundle filesystem access, installation,
lifecycle mutation, and trust mutation cross the normal effect gateway and journal.

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
Submitted input history MUST be encrypted in the authoritative journal and persisted
through the normal policy/permit/audit boundary. Rust MUST NOT silently create or reuse a
plaintext history sidecar. Embedded and authenticated-worker REPLs MUST hydrate the same
bounded newest entries, suppress consecutive duplicates, and keep history persistence
failure from blocking the requested command.

Rust custom themes MUST be configuration-only JSON or TOML with strict bounded schemas.
Loading MUST reject symlinks, oversized/count-excess libraries, invalid colors, unknown
fields, duplicate identities, and built-in name collisions. Selecting a custom theme
MUST persist the fully resolved palette and source hash so later source mutation or
deletion cannot silently change the reconstructed preference. Embedded and worker REPLs
MUST expose identical list, preview, selection, restart, and ANSI-free redirected-output
behavior.

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
- Interactive prompts show a 1-based cursor line/column and Unicode-aware draft
  character/line counts without per-keystroke application, worker, policy, or journal
  operations.
- Five built-in data-only palettes MUST style interactive prompts, assistant text,
  semantic labels, and activity frames while redirected output remains ANSI-free.
- Loopback-live Responses and OpenAI-compatible terminal acceptance MUST cover streamed
  tool calls, continuation, final output, redirected ANSI safety, and credential
  non-disclosure; compatible execution MUST cover CLI, REPL, and worker surfaces.

## 19. Configuration And Local Storage

Configuration is strict: unknown fields fail validation. It covers provider defaults,
named model profiles and roles, context budgets, agent turn limits, subagent concurrency,
memory index, global HTTP transport, research limits/sources/search/MCP, and skill
override policy.

Configuration uses fresh strict YAML and accepts credential references such as
`env:NAME`. Raw secret values MUST
not be written back when configuration is shown.

The default canonical adapter is an ACID, crash-safe embedded event store with a stable
file format. A single transaction appends events, advances stream/global sequence
numbers, records projection work, and queues external index/export work. Fresh Rust state
does not silently import legacy state. Schema migration MUST preserve records or fail
safely with a clear recovery path.

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

Checkpoint the passing Python implementation, define AUDIT-01, AUTHZ-01, STORE-01,
FLOW-01, and MEM-01 acceptance tests, then deliver strict Rust contracts and locked
dependencies.

Exit gate: the legacy checkpoint is reproducible and all foundational contracts have
schema and boundary tests.

### Milestone 0A: Audit And Storage Kernel

Deliver the encrypted embedded journal, repositories, projections, signing, chain
verification, key providers, audit export, recovery mode, and adapter conformance suites.

Exit gate: concurrency, crash recovery, tampering, truncation, rotation, signatures,
unknown effects, projection lag, and recovery mode tests pass before any effectful adapter
is added.

### Milestone 0B: Effect Gateway And Policy

Deliver the safety kernel, built-in and OPA policy providers, approvals, permits,
two-phase content release, policy diagnostics, sandbox helper/backends, network allowlist
proxy, and downgrade rules.

Exit gate: no adapter can execute without a valid matching permit, policy failures close,
and denied quarantined content is never released.

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

Deliver versioned YAML workflows, the embedded application API, optional worker, Goal
Mode, active-goal tools, plan handoff, subagent queue, configurable concurrency,
drain/cancel/resume, and parent result previews.

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
- [x] Goal Mode stops correctly on complete, blocked, error, or budget exhaustion.
- [x] Subagents respect configured concurrency, cannot delegate recursively, and can be
  cancelled or resumed after interruption.
- [x] Research records planned queries, lane decisions, source labels, worker progress,
  citations, synthesis choice, and limitations.
- [ ] Compact and verbose renderers cover every event type and every tool family.
- [x] Embedded and worker REPL history has bounded hydration, encrypted permit-bound
  persistence, restart parity, consecutive deduplication, and redacted audit envelopes.
- [x] Built-in theme palettes cover prompt, assistant, semantic event, and activity-frame
  styling without emitting ANSI sequences to redirected output.
- [x] Bounded JSON/TOML custom themes have strict parsing, immutable source-hash-bound
  preference snapshots, legacy data-only schema mapping, embedded/worker parity,
  restart reconstruction, and ANSI-free redirected output.
- [x] Reedline prompt repaint reports Unicode-aware cursor/draft metrics without
  per-keystroke state effects, and loopback-live Responses/compatible streamed tool loops
  pass CLI, REPL, worker, ANSI-safety, continuation, and credential non-disclosure checks.
- [x] Telemetry derives correct duration and counts from persisted event timestamps.
- [ ] Credentials remain references until adapter execution and never appear in model or
  user-visible diagnostic payloads.
- [x] Skills cannot gain executable privilege; packs cannot activate before verification.
- [x] Audit-chain verification detects tampering.
- [x] Journal concurrency, encryption/key rotation, tail truncation, signed checkpoints,
  unknown effects, and read-only recovery behavior pass fault-injection tests.
- [ ] Every effect category is rejected before adapter execution without an unexpired,
  unused permit matching the request, decision, actor, and obligations.
- [ ] OPA allow, deny, approval, full-content disclosure, hard redaction, mTLS, bundle
  revision, invalid response, outage, oversized input, and decision-log checks fail or
  proceed exactly as specified.
- [ ] Two-phase file, network, provider, subprocess, and memory tests prove denied content
  never reaches the requester.
- [ ] In-memory and embedded journals/repositories plus Tantivy and Chroma indexes pass
  the shared conformance contract; canonical memory works during index outage/rebuild.
- [x] Research and extension adapters pass shared factory-reopen conformance for canonical
  citations, integration state, pack lifecycle, publisher trust, bounds, and reconstruction.
- [ ] Workflow schema, trust invalidation, restart, bounded parallelism, cycles, input
  waits, explicit idempotent retries, compensation, cancellation, and unknown outcomes
  pass durable acceptance tests.
- [ ] Sandbox tests cover traversal, symlink, environment, child-process, resource, and
  network escapes on each supported platform.
  Mandatory macOS/Linux arm64/x64 native tests and Windows arm64/x64 fail-closed tests are
  wired; a real Windows filesystem/network isolation backend remains required.
- [x] Production and independent fuzz dependency graphs enforce locked registry sources,
  explicit licenses and versions, banned crates, and warnings-denied RustSec audits.
- [ ] Formatting, warnings-denied lint, workspace tests, fuzzing, dependency/license and
  vulnerability policy, and macOS/Linux/Windows arm64/x64 release smoke tests pass.
  The six-target native runner/build/execute/package matrix is implemented; this remains
  open until one remote run is green for every target.
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
