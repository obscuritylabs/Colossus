---
status: archived
replacement:
  - /get-started/core-concepts/
  - /reference/
  - /develop/architecture/
---

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

- [Architecture](../../docs/develop/architecture.md)
- [Security architecture](../../docs/develop/security-architecture.md)
- [Tools and action classes](../../docs/reference/tools-actions.md)
- [Configuration fields](../../docs/reference/configuration.md)
- [Skills](../../docs/extend/skills.md)
- [Packs](../../docs/extend/packs.md)
- [Integrations](../../docs/extend/integrations.md)

### Clean Reconstruction Handoff

For a clean reconstruction, give the implementer this file before providing the existing
source or implementation-focused documentation. Ask it to:

1. Treat every MUST in this document as the compatibility contract and use the
   [Rust Acceptance Matrix](rust-acceptance-matrix.md) as its executable evidence index.
2. Choose its own language, frameworks, module layout, and internal algorithms.
3. Produce a requirement-to-test matrix keyed by the feature ids in Section 5.
4. Implement foundational security and storage requirements before higher-level
   behaviors, keeping each acceptance gate runnable.
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

The default installation MUST work without credentials and continue normally offline.
Except for the documented, credential-free, fixed-origin stable release discovery check,
it MUST NOT make network calls until the operator explicitly configures and permits a
network-capable feature.

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
- Scripts found inside skill directories are never executed automatically.
- Context compaction does not replace or rewrite persisted conversation history.
- Memories are contextual facts and preferences, not privileged instructions.
- Provider hidden chain-of-thought is not a user-facing or persisted product surface.
- The initial product does not require a graphical desktop interface or remote cloud
  control plane.
- Child-agent trees are bounded and cannot become self-replicating.
- The baseline does not require a universal dedicated verification-tool family;
  repository verification remains available through structured commands and capability
  packs.

## 4. Primary Users And Journeys

### 4.1 Repository Developer

The user selects a workspace, asks for a change, observes reads and commands, approves
mutations, receives a verified result, and can resume the same session later.

### 4.2 Interactive Operator

The user works in a persistent terminal UI, changes model roles and display preferences, manages
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
| ACCESS-01 | Unified access profiles | P0 | One metadata registry resolves model-visible tools and built-in action decisions without bypassing trust, policy, or sandbox enforcement. |
| TOOL-01 | Brokered tool system | P0 | Schemas, permissions, policy, approvals, execution, and audit run in order. |
| SAFE-01 | Policy and approval modes | P0 | `deny`, `ask`, `risk-auto`, and `full-access` preserve exact safety semantics. |
| STATE-01 | Durable sessions and run history | P0 | Sessions and events survive restarts and can be resumed. |
| UX-01 | One-shot CLI and interactive TUI | P0 | Both surfaces use the same application behavior and event stream; the superseded `repl` alias is removed. |
| UX-02 | Human-first terminal presentation | P1 | Interactive CLI and TUI surfaces restore the Python 0.5 semantic cards, Markdown, tables, guided choices, completion, and transcript behavior without weakening structured automation output. |
| UX-03 | Responsive terminal UI | P1 | A Ratatui TUI owns terminal rendering, restores the durable transcript, pins the composer and footer, keeps input responsive during runs, and bridges approvals, questions, and cancellation identically in embedded and worker modes. |
| CTX-01 | Context composition and compaction | P1 | Raw history is retained while provider input stays within budget. |
| WORK-01 | Tasks, decisions, memories, and plans | P1 | Long-running work state is persisted and inspectable; plans support optimistic refinement, discard, approval, and atomic Direct or Goal consumption. |
| MEM-01 | Canonical memory and disposable indexes | P1 | Memory lifecycle state remains authoritative and usable while lexical or semantic indexes lag or rebuild. |
| FLOW-01 | Durable versioned workflows | P1 | YAML workflows, fixed-cadence schedules, authenticated webhooks, and repository-event subscriptions are validated, hash-pinned, restartable, bounded, and use normal authorization for every dispatch and effectful step. |
| GOAL-01 | Bounded Goal Mode | P1 | Autonomous iterations stop on terminal status or budget exhaustion, and an Active goal can resume only its remaining budget. |
| AGENT-01 | Durable subagents | P1 | Queued child jobs support bounded concurrency and lifecycle controls. |
| RES-01 | Deep Research | P1 | Evidence collection, claims, citations, progress, and fallback are durable. |
| OBS-01 | Telemetry and observability | P1 | Run duration, event, tool, approval, risk, research, and error metrics are queryable. |
| EXT-01 | Skills and resources | P1 | Skills compose prompt context under precedence and resource-access rules. |
| INT-01 | Integrations and credential broker | P2 | Connected services expose normal tools without exposing credentials. |
| PACK-01 | Capability packs | P2 | Executable extensions are declared, verified, trusted, and lifecycle-managed. |
| DIST-01 | Trusted distribution | P2 | Fixed-origin bootstrap installers verify exact public release assets, record direct ownership, and preserve offline bundle plus signed pack/skill verification without weakening offline operation. |
| DIST-02 | Fail-soft update discovery | P2 | The standalone CLI and asynchronous TUI check one fixed stable channel at most daily; strict cached metadata and typed offline/rate-limit outcomes never block normal operation or replace a binary. |
| DIST-03 | Install-aware direct updates | P2 | The standalone CLI embeds the reviewed bootstrap, replaces only the exact executable owned by a validated direct receipt, refuses downgrades and other owners, and preserves the prior binary on failure. |

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
- CLI and terminal UI surfaces collect input and render typed output only. They MUST NOT contain
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

The default maximum is 100 model turns per run. The configured value MUST be at least 1
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
- for a completed Plan Mode run, the canonical created or updated plan and its identity

The terminal MUST display a human-readable elapsed duration after normal runs and Goal
Mode runs.

The public application API, protobuf contract, and SDK completed-result and cancellation
types expose that identity as optional `plan_id`. Execute runs, cancellations before
persistence, and older durable payloads read it as absent.

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
| `PlanWritten` (`plan_written`) | Release the canonical Draft and revision after the one required Plan Mode write succeeds. |
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
- trusted source (`core`, integration, MCP, or signed pack);
- action class and exact effect/Safety Kernel identities;
- static prerequisites and a bounded availability reason;
- risk level: low, medium, or high;
- timeout and maximum output bytes.

The execution order is fixed:

```text
access resolution -> schema validation -> deterministic policy -> optional risk review
-> approval decision -> brokered execution -> bounded result -> event and audit
```

The optional `access` block selects `minimal`, `development`, `allow_all`, or `pinned`;
omission selects `allow_all`. Profiles operate on the metadata above; an exact include
changes visibility only. Run
scopes such as Plan Mode, Goal Mode, interactive UI, and child-agent execution may narrow
the resolved catalog but never broaden it. An unclassified built-in or untrusted
extension fails closed.

### 10.2 Available Offline Tool Catalog

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
- Plans: `plan.create`, `plan.update`, `plan.show`, `plan.approve_request`.
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
- `web.search` is exposed when the profile selects it and `search.roles.agent` resolves;
  `pinned` additionally needs an exact include. SearXNG and SerpAPI share a normalized
  provider-neutral result contract.
- `mcp.call` is exposed only through an explicitly configured, allowlisted gateway.
- Connected native integrations expose namespaced tools such as `github.*`,
  `searxng.*`, and `opensearch.*`.
- Imported API operations use names like `openapi.NAME.OPERATION`.

Connected integration and enabled reverified signed-pack tools enter the same access,
schema, policy, approval, execution, event, and audit pipeline as built-ins. Discovered
or untrusted extensions remain absent.

### 10.4 Filesystem And Subprocess Rules

- Declared/isolation mode confines generic file, repository, patch, and trace paths to
  the selected workspace and denies Colossus control directories. Acknowledged
  `danger_full_access` deliberately permits canonical absolute and traversing host paths,
  including control paths, while retaining permit, audit, no-follow, and atomic-write
  checks.
- Text reads and writes are bounded and reject unsafe file types where applicable.
- Mutating file and patch results include a diff and changed line ranges.
- `shell.run` accepts exactly one of a non-interactive `command` interpreted by a
  trusted configured/derived shell or a structured `argv` with exact executable
  resolution. Startup profiles, interactive stdin, persistent PTYs, and background
  sessions are excluded.
- Global `--workspace` is canonicalized once. `workspace-development` derives a writable
  workspace, trusted shell, read-only command roots, isolated home/temp, and sanitized
  path only for terminal users and agents outside workflow lineage. Colossus control
  state remains protected by the selected native, Windows, or OCI backend.
- Output caps and post-effect release remain enforced in every mode. Isolating backends
  enforce their configured working-directory, environment, process-tree, timeout, and
  resource boundaries. Direct Unix full-access supervision is best effort for process
  trees and resources: a deliberately detached descendant can outlive the recorded
  effect, so strict containment requires native/OCI host isolation.
- Under declared authority, network destination `*` means public HTTP(S) only and
  private, loopback, link-local, and metadata origins require exact entries. Remote
  plaintext HTTP requires acknowledged ambient authority in the active permit. Ambient
  authority needs no duplicate destination entry; URL validation, DNS pinning, TLS for
  HTTPS, response bounds, permits, and audit remain.

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
- `risk-auto`: run model-assisted review only for eligible model or child-agent
  `shell.run`, `web.search`, bodyless `network.http` GET, and configured top-level
  `mcp.call` effects outside workflow lineage. Auto-approve only low-risk calls whose
  recommendation is allow. Medium, high, malformed, unavailable, unsupported, and deny
  recommendations fall back to an explicit user prompt or denial.
- `full-access`: auto-approve approval-required operations without prompting and skip
  model-assisted risk review.

`full-access` MUST NOT expand workspace roots, grant missing network implementations,
change tool schemas, bypass deterministic denies, or make unknown tools executable.

One-shot runs default to deny unless approval prompting is requested. Interactive TUI
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
records MUST be immutable, append-only, and hash-chained. Encryption is enabled by an
explicit platform or environment key provider; the keyless default is plaintext and
emits a security-posture warning. Every
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
bodies, unbounded command output, and hidden reasoning are prohibited. When configured,
sensitive event payloads use authenticated encryption with keys from an explicit
platform or environment key provider. An established journal never silently changes its
protection mode; operators must create fresh storage to change it.

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
export loop. The directory adapter is deterministic and replay-safe; it does not claim
WORM durability. The HTTPS WORM adapter MUST use deterministic content-hashed object
names, create-only conditional PUT, an exact-origin permit, late environment credential
resolution, no response-body release, and unknown-outcome blocking. The remote service,
not Colossus, MUST independently enforce retention lock.

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

Plans are `draft`, `approved`, `executed`, or `discarded`. A plan stores the originating
prompt, Markdown content, ordered steps, mutation flags, session id, timestamps, and an
optimistic revision. Legacy records without a revision read as zero; new records start
at one. Every content update and lifecycle transition increments the revision, and a
stale update, approval, discard, Direct execution, or Goal handoff MUST fail with a
conflict.

Plan Mode has trusted Create and Update targets. Create permits exactly one successful
`plan.create`. Update binds an exact same-session Draft id and revision on the server and
permits exactly one successful `plan.update`; the model supplies only replacement
content and ordered steps, and the original prompt remains unchanged. The target-specific
instructions are appended after skill composition and therefore retain precedence.

A completed planning turn MUST persist exactly one target write. A second write is
blocked before dispatch. If the model produces final output before writing, the runtime
permits one corrective provider turn and then fails closed. A failed, cancelled, or
disconnected turn is not retried automatically and can contain zero or one durable plan;
the typed `PlanWritten` event and run/cancellation result preserve the canonical evidence
when a write completed.

Plan Mode may use the target write plus its exact inspection/state allowlist:
`echo`, `filesystem.list`, `filesystem.read`, `filesystem.search`, `git.status`,
`git.diff`, `git.show`, `repo.map`, `repo.symbol_search`, `repo.references`,
`repo.file_summary`, `patch.preview`, `task.create`, `task.list`, `decision.list`,
`plan.show`, `memory.list`, `memory.search`, `agent.result`, `agent.list`,
`tool.search`, `user.ask`, `context.show`, `context.snapshots`, and
`skill.resource.read`. Normal access and prerequisites may narrow it further. Plan Mode
MUST exclude filesystem writes, patch application, execution, approval, networking,
delegation, discard, and plan execution, and MUST NOT claim implementation is complete.

`plan.update` and operator-only `plan.discard` are Local State actions.
`plan.approve_request` is Administration. Both Direct execution and approved-plan Goal
handoff are `plan.execute` Execution actions. Every transition crosses the normal effect
gateway; no terminal command owns a policy or persistence bypass.

A Draft can be refined, reviewed, approved, or discarded. An Approved plan is immutable
and can be discarded, executed directly once, or atomically consumed into Goal Mode.
Consumption preserves the plan id for lineage and returns tagged canonical evidence for
completion, cancellation, or bounded failure.

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
drain. CLI, TUI, an embedded API, and an optional single-writer worker all invoke that
same application layer.

Persisted schedules bind an operator-selected identifier, validated input snapshot, and
fixed cadence of 60 seconds through 31 days to an exact workflow hash. UTC RFC3339 `Z`
timestamps are authoritative. Enable/disable is explicit and journaled. When multiple
occurrences are overdue, the selected policy MUST either skip the entire backlog or
queue one run for the latest occurrence; a single due occurrence queues once. Next-fire
reconstruction MUST be deterministic and bounded without iterating once per missed
occurrence. A fired schedule transition and its deterministic queued run MUST commit in
one journal batch. Restart and process loss MUST NOT lose or duplicate the occurrence.
Definition, call-graph, and input trust is rechecked before enable and dispatch; actual
invalidation disables the schedule, while storage failure remains a retriable failure.
Trigger-created runs use the ordinary hash-pinned queue, worker lock, policy, approval,
effect, and recovery paths.

Persisted webhooks bind an operator-selected identifier and an `env:` HMAC credential
reference to an exact workflow hash. The raw secret MUST NOT enter configuration,
schemas, model context, telemetry, or journal payloads. Ingress MUST verify HMAC-SHA256
over the exact timestamp, delivery identifier, and raw body; require a UTC RFC3339 `Z`
timestamp within a configured 60-to-3600-second replay window; bound identifiers,
headers, and a nonempty strict-JSON body; reject a known delivery identifier; revalidate
the workflow call graph and input envelope; and submit the complete safe request through
the ordinary policy/effect/audit gateway. The gateway receives a credential reference
and value hash but never the raw secret or submitted signature.

After authorization, ingress MUST hold the writer coordination lock and recheck binding
state, workflow trust, replay state, and deterministic run identity. The accepted
delivery receipt and queued run MUST commit in one journal batch, so concurrent delivery,
restart, or process loss cannot accept the same delivery twice or record acceptance
without its run. The run input envelope contains `body`, `delivery_id`, application
`headers`, and `timestamp` and MUST satisfy the workflow's declared input schema.
Changing or removing the pinned definition disables the webhook with an auditable
bounded reason; storage failure remains retriable. Any bundled HTTP adapter MUST bind
loopback only, bound parsing before allocation, reject chunked/trailing bodies, and leave
public TLS, origin authentication, and rate limiting to a trusted reverse proxy.

Persisted repository-event subscriptions bind an operator-selected identifier, one exact
versioned domain event type, an optional aggregate stream prefix, and a validated input
envelope to an exact workflow hash. `workflow.*` source types and non-domain journal
events MUST be rejected. Creation defaults its global checkpoint to the current journal
head; replay before that point requires an explicit `after_sequence`. Definition,
call-graph, and complete input trust MUST be checked before enable and dispatch, and the
complete event envelope MUST cross `workflow.subscription.dispatch` through the ordinary
policy/effect/audit gateway.

After authorization, dispatch MUST hold the writer coordination lock and recheck binding
state, trust, checkpoint, and immutable source-event identity. The source checkpoint,
delivery receipt, and deterministic queued run MUST commit in one journal batch. Replay
of a delivered source event MUST acknowledge the existing receipt without another policy
request or run. Unmatched domain events advance the checkpoint; definition or schema
invalidation disables the subscription without advancing past the rejected event, while
storage failure remains retriable. A refused or incomplete control dispatch leaves the
source pending and MUST NOT starve unrelated subscriptions or queued workflow runs.
Trigger-created runs retain ordinary workflow policy, approval, permit, recovery, and
unknown-outcome semantics.

When the worker is active it owns the canonical writer lease. Local clients MUST
authenticate the server before disclosing operation content, authenticate every request
and ordered response frame, bind requests to a single connection, reject replay and
stale timestamps, and bound framing before allocation. Unix sockets are owner-only;
Windows named pipes implement the same logical contract. Supported CLI/TUI operations
auto-discover the worker and fall back to the embedded runtime only when no authenticated
worker is active.

Worker routing covers model runs, core TUI turns, sessions, workflows and workflow
schedules, audit and runtime diagnostics, context and telemetry, plus canonical tasks,
decisions, plans, goals, subagents, memories, and memory-index maintenance. Approval mode
belongs to the worker process; a client-side approval override MUST fail rather than
silently change authority.

Worker protocol v6 replaces the single-purpose controlled operation with one
authenticated `RunInteractive` duplex loop. Its typed variants cover Execute or Plan
turns, exact-revision plan approval and discard, Direct or Goal plan execution, and
Active-goal resume. The same bounded connection carries ordered released events,
automatic-approval notices, focus-taking approval and user prompts, and cooperative
cancellation. Cancellation stops the run and releases every pending prompt waiter.
Non-interactive operations remain available for one-shot CLI and scripted use.

Client and server MUST reject every other protocol version before disclosing operation
content. The released error MUST tell the operator to restart the worker and client with
the same Colossus version. A disconnected or stale-worker request is not replayed
automatically; the operator inspects `/plans` and durable run/Goal evidence before any
retry.

Research, skills, packs, bundles, integrations, MCP, process, and network commands use
typed operations too; `@path` JSON and other file-backed inputs are read by the worker
through the normal filesystem permission boundary rather than by the client.
When a worker is active, the TUI routes its implemented session, context, work,
workflow, research, telemetry, skill, pack, bundle, integration, MCP, audit, projection,
and tool commands through those same typed operations. Embedded and worker TUI hosts also
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
- Cancellation or bounded failure leaves the goal Active. `/goal resume GOAL_ID`
  validates same-session ownership and continues at `iterations_completed + 1` through
  the original iteration budget; it does not allocate a new budget.

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
- Successful foreground `agent.delegate` calls wake the bounded scheduler so
  `agent.result` can receive the completed child output during the same parent turn.
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

The web lane requires an explicit `search.roles.research` route and calls the same
`SearchProvider` port used by `web.search` and `search query`. Routes never fall back or
retry automatically. The deprecated v0.8 `research.search` SearXNG form is accepted only
when top-level `search` is absent.

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
- Skills may be activated by explicit option, sticky TUI state, or prompt mention.
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

## 18. CLI And Terminal UI Surface

### 18.1 Global Options

The CLI supports verbosity, provider/profile overrides, model selection, context-window
override, base URL, API-key value or environment reference, provider/global CA bundles,
mTLS client certificate/key, key-password environment reference, proxy URL or environment
reference, HTTP environment-trust toggle, and shell completion.

### 18.2 Top-Level Commands

- `update check`: read-only fixed-channel stable release discovery with structured,
  fail-soft offline output.
- `update [--version vX.Y.Z]`: direct-install-only stable replacement through the
  embedded reviewed bootstrap, with ownership and downgrade refusal.
- `run`: normal turn, plan creation, approved-plan execution, or plan-to-goal handoff.
- `goal`: bounded autonomous goal loop.
- `research`: deep research with persisted cited output.
- `tui`: interactive session and non-TTY line runner; the former `repl` alias is removed.
- `config`: initialize, show, and explain effective strict configuration; there is no
  automatic migration command.
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

### 18.3 Terminal UI Commands

The TUI includes:

- Runtime: `/model`, `/agent`, `/tools`, `/workspace`, `/status`, `/clear`, `/exit`.
- Sessions/context: `/resume`, `/sessions`, `/session`, `/context`, `/compact`.
- Rendering: `/stream`, `/events`, `/reasoning`, `/transcript`, `/multiline`, `/trace`.
- Preferences/themes: `/theme` and `/tui`.
- Work state: `/tasks`, `/decision`, `/decisions`, `/memory`, `/memories`, `/plans`,
  and `/goals`.
- Plan workflow: `/plan`, `/plan on|off|status|new|list`, `/plan use PLAN_ID`,
  `/plan show [PLAN_ID]`, `/plan approve`, `/plan discard`, and
  `/plan execute [direct|goal [ITERATIONS]]`.
- Workflows: `/goal`, `/goal resume GOAL_ID`, `/research`, `/agents`.
- Extensions: `/skill`, `/skills`, `/integrations`, `/packs`.
- Discovery: `/help`.

The TUI persists display preferences for theme, multiline composition, streaming mode,
event detail, transcript density, and reasoning-summary visibility. Preferences affect
rendering only and MUST NOT change provider, policy, tool, or approval behavior.
Execute/Plan mode and selected-plan state are process-local and MUST NOT enter those
preferences. A process starts in Execute mode with no selection. Mode survives a session
switch, while selection clears on switch or restart. `/plan new` enters Plan mode and
clears selection without discarding the old plan; `/plan off` retains selection.
`/plan use` accepts only same-session Draft or Approved plans. Prompts refine only a
selected Draft; an Approved plan cannot be refined.

Submitted input history MUST be persisted in the authoritative journal through the
normal policy/permit/audit boundary. It MUST use the configured protection mode:
authenticated encryption with platform or environment keys, or hash-chained canonical
plaintext in keyless mode. Rust MUST NOT silently create or reuse a separate plaintext
history sidecar. Embedded and authenticated-worker REPLs MUST hydrate the same bounded
newest entries, suppress consecutive duplicates, and keep history persistence failure
from blocking the requested command.

Rust custom themes MUST be configuration-only JSON or TOML with strict bounded schemas.
Loading MUST reject symlinks, oversized/count-excess libraries, invalid colors, unknown
fields, duplicate identities, and built-in name collisions. Selecting a custom theme
MUST persist the fully resolved palette and source hash so later source mutation or
deletion cannot silently change the reconstructed preference. Embedded and worker REPLs
MUST expose identical list, preview, selection, restart, and ANSI-free redirected-output
behavior. The interactive theme library MUST render as an intentional theme table with
the active selection and readable custom-theme search locations, never as nested JSON.
It MUST support a numbered picker, non-mutating full semantic previews, dynamic theme-name
completion, and theme-aware ghost text. Direct selection saves immediately. Scaffolding
MUST emit a strict template without writing from the interface, and validation MUST report
the already loaded bounded library.

### 18.4 TUI Ownership And Interaction

`colossus` and `colossus tui` launch the interactive TUI when stdin and stdout are TTYs.
The former `colossus repl` alias is removed. Non-TTY stdin uses the bounded line runner
and preserves explicit JSON output; interactive JSON is
rejected with guidance. Alternate-screen mode is the default, except that Zellij uses an
inline viewport; `--no-alt-screen` always selects inline mode.

Exactly one event loop owns terminal writes. The layout keeps a scrollable durable
transcript above an optional activity row, dynamically sized composer, and stable
width-aware footer. The composer remains usable while a run is active and queues at most
eight future turns. Approval and `user.ask` prompts use explicit focus-taking overlays
without overwriting the draft. Blank, cancelled, disconnected, timed-out, malformed, or
replayed responses fail closed.

Transcript restoration MUST exclude system messages, correlate canonical tool results
with assistant tool calls, and load no more than 100 messages or 2 MiB per page. Resize
reflows retained presentation documents without erasing transcript history. Autoscroll
occurs only at the live edge; scrolled-up operators retain position and receive a bounded
new-item count until returning with End.

Ctrl-C clears a draft, cancels a modal, or requests cooperative run cancellation in that
order. Cancellation prevents the next provider or tool effect from starting, lets an
already-started effect reach an auditable terminal state, and appends durable cancelled
tool results for remaining calls.

Plan mode/lifecycle commands and ordinary turns share the existing FIFO queue. Returned
plan state MUST be applied before the next queued item starts, and the queue MUST NOT
drain while the execution-choice overlay is open. With no explicit strategy, that
overlay offers Direct, Goal Mode, and Cancel. Line mode uses the same semantics with a
numbered stdin choice. Goal defaults to 5 iterations and accepts 1 through 50.
Cancellation or failure before consumption preserves mode and selection. Once Direct or
Goal consumption commits, the interface switches to Execute and clears selection even
if later execution fails or is cancelled.

Worker protocol v6 authenticates and sequences the unified interactive duplex frames for
events, notices, approvals, `user.ask`, and cancellation. A version mismatch identifies
that the worker and client must be restarted on the same Colossus version; the failed
operation is not replayed automatically.

### 18.5 Rendering

- Compact, verbose, and off event modes are supported.
- Interactive terminal output is human-first. Stable JSON remains available explicitly
  and remains the automatic format for redirected machine output, but an interactive
  command MUST NOT dump raw JSON unless the operator requests it.
- Assistant answers, plans, research reports, and released child-agent output render as
  bounded terminal Markdown. Buffered Markdown renders only after a complete released
  answer; raw streaming remains available as an explicit preference.
- List surfaces use width-aware semantic tables, single-record surfaces use labeled
  detail cards, and empty states explain the result instead of printing `[]` or `null`.
- Semantic presentation preserves the useful Python 0.5 behavior at
  `python-v0.5.0`: file/source previews with line numbers, styled diffs, separated
  process stdout/stderr, Git status, work/context/repository/skill/research/integration
  summaries, approval/risk/error cards, and bounded generic fallback output.
- `/help` is grouped and includes current display state. Slash commands and active skill
  mentions are discoverable through completion; session and user-input choices use
  guided labeled selection while exact IDs and free-form answers remain available.
- Assistant output can stream without corrupting prompt input or semantic event output.
- Tool results use semantic renderers for files, shell, git, work state, context, repo,
  skills, web/search, MCP, traces, integrations, packs, and generic structured output.
- Comfortable transcript mode uses readable blocks; compact mode minimizes vertical
  space.
- Safe reasoning summaries can be toggled independently from tool/activity events.
- Errors clearly identify whether they are recoverable.
- Long-running activity shows current phase/action and elapsed time.
- `user.ask` pauses only the current foreground agent turn, renders a stable input card
  with answer/cancellation guidance, and MUST NOT continue repainting an activity spinner
  over the operator's input. Durable non-blocking waits use workflow `wait_for_input`.
- Interactive prompts show a 1-based cursor line/column and Unicode-aware draft
  character/line counts without per-keystroke application, worker, policy, or journal
  operations.
- Five built-in data-only palettes MUST style interactive prompts, assistant text,
  semantic labels, and activity frames while redirected output remains ANSI-free.
- Loopback-live Responses and OpenAI-compatible terminal acceptance MUST cover streamed
  tool calls, continuation, final output, redirected ANSI safety, and credential
  non-disclosure; compatible execution MUST cover CLI, TUI, and worker surfaces.

## 19. Configuration And Local Storage

Configuration is strict: unknown fields fail validation. Only `schemaVersion` and
`storage` are required at the top level; omitted `access` defaults to `allow_all`, and
ordinary fields within optional blocks default recursively. It includes access profile
and exact overrides, provider defaults, named model profiles and roles,
provider-neutral search profiles and agent/research routes, context budgets, agent turn
limits, subagent concurrency, memory index, global HTTP transport, research
limits/sources/MCP, and skill override policy.

Configuration uses fresh strict YAML and accepts credential references such as
`env:NAME`. Raw secret values MUST
not be written back when configuration is shown.

The global workspace defaults to the current directory and anchors relative
configuration, state, workflow, skill, and pack paths. Workers publish the canonical
workspace and reject mismatched clients. Fresh initialization keeps access and sandbox
implicit, resolving to `allow_all` plus acknowledged `danger_full_access`. Passing an
explicit `--sandbox-profile` selects a complete platform-isolating preset instead;
`--from` preserves source settings except for the deliberately fresh storage identity.

`schemaVersion: 2` remains active, but removed exact tool/action lists are rejected.
Before 1.0, configuration changes are applied by directly updating the strict YAML or
generating a fresh configuration; there is no automatic configuration migration command.
Effective diagnostics MUST be bounded and credential-free.

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

Signed collections MUST inventory every nested pack and data-only skill, require trusted
collection and pack signatures, validate exact pack dependency closure, and refuse
destination replacement. Registry pull and push MUST cross the normal effect gateway,
use the signed collection as the authority, pin an explicitly granted network origin,
resolve only granted credential references inside the adapter, disable implicit redirects
and proxies, bound transport and extraction, and classify an ambiguous remote mutation as
an unknown outcome. Registry configuration is optional and MUST NOT add hidden network use
to local or air-gapped operation.

## 21. Non-Functional Requirements

### 21.1 Security

- No shell-string execution in brokered subprocess paths.
- No raw secret in model-visible schemas, prompts, transcripts, telemetry, or audit.
- No mutation before schema validation, policy, and approval.
- No hidden background network use except documented, bounded, credential-free stable
  release discovery. Acknowledged full access also exposes model-requested generic
  HTTP(S) effects without destination entries; configured providers, search, MCP,
  integrations, packs, credentials, and trust are never invented.
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

## 22. Delivery Status

Rust 0.10.7 is the active stable core release line, and every capability
in the Section 5 release baseline has executable evidence. The detailed
requirement-to-test mapping lives in the
[Rust Acceptance Matrix](rust-acceptance-matrix.md); test names and source paths belong
there rather than being repeated in this product contract. Publication still requires
the explicit platform, security, and artifact release gate.

Latest published Desktop preview proof:

- [v0.10.2-preview.10 release](https://github.com/obscuritylabs/Colossus/releases/tag/v0.10.2-preview.10)
- [Release Process](release-process.md) for the local, pull-request, platform, security, and
  packaging gates required before publication

The frozen Python inventory is fully represented by the Section 5 baseline. Generic
possibilities such as another provider, storage backend, trigger, policy engine, or
terminal adapter are extension points, not unfinished features. A future addition becomes
product scope only after it receives a concrete requirement and acceptance contract;
implementation detail and test evidence remain outside this inventory.
