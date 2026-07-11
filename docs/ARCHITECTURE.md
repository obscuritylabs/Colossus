# Architecture

Colossus uses a ports-and-adapters architecture with strict dependency direction.

## Rust Reconstruction Boundary

The new runtime is developed under `rust/` until P0+P1 cutover. Its dependency direction
is stricter than the legacy diagram below:

```text
colossus-cli -> colossus-runtime -> colossus-agent -> colossus-ports
                    |                  |                ^
                    |                  +-> colossus-tools+
                    +-> colossus-policy -----------------+
                    +-> colossus-workflow ---------------+
                    +-> colossus-provider -> colossus-policy
                    +-> colossus-mcp -> colossus-sandbox -> colossus-policy
                    +-> colossus-sandbox  -> colossus-policy

redb and future adapters -> colossus-ports -> colossus-contracts -> colossus-domain
```

`colossus-domain` has no dependencies. Interfaces construct requests, call runtime
services, and render results; they do not own policy, workflow, effect, or storage logic.
Effectful adapters require an opaque permit that only `EffectGateway` can mint. redb is
the canonical event journal, while repositories, projections, memory indexes, and audit
exports are replaceable ports.

`colossus-agent` owns the bounded reusable model/tool loop. It consumes role-routed
`ModelProvider`, `ToolRegistry`, and `ToolExecutor` ports, persists each prepared request
and normalized event, validates tool arguments before execution, preserves assistant
call IDs and tool results across provider turns, performs at most two metadata-only
malformed-argument correction turns, and emits a distinct max-turn terminal event.
`colossus-tools` owns the immutable active catalog and JSON Schema validation. The
runtime adapters translate validated effectful tools into normal gateway requests; pure
computation such as `echo` remains outside the permission boundary by design.

`colossus-session` is the canonical session repository. Session creation and every user,
assistant, and tool-result message append to one optimistic `session:{id}` journal
stream. Agent runs restore those canonical messages before composing the next provider
request and attach the same session id to run/effect provenance. The `sessions-v1`
projection contains only bounded discovery metadata (counts, last run, title, and recent
user preview), never full message bodies; it can be deleted and rebuilt from the journal.

`colossus-context` is the long-session safety boundary. Before every Rust provider turn,
the agent asks the `ContextPreparer` port to estimate the complete request, including
instructions and tool schemas. At the configured threshold it appends an encrypted,
immutable snapshot plus an explicit activation event to the same optimistic session
stream. Provider input becomes a synthetic system snapshot plus the preserved recent
tail, while canonical messages remain untouched. The `context_summarizer` role is used
through the normal provider gateway when available; invalid, failed, or echo summaries
fall back to deterministic extraction. Every turn records `context.prepared.v1` on its
run stream.

`colossus-work` owns typed task, key-decision, and plan lifecycles. Each record has
its own optimistic journal stream; updates append complete validated next-state records,
and decision supersession atomically closes the old stream state while creating its
linked replacement. Mutation adapters are private to the runtime and receive one-use
permits only after `task.*`, `decision.*`, or `plan.*` authorization. The `work-v1` projection is
disposable discovery state, not the write model. Active decisions are loaded from the
canonical repository and injected as
bounded binding system context before snapshots on every provider turn; archived and
superseded decisions remain auditable but are not injected. Strict model-visible task
and decision tools derive the session from the execution context rather than accepting
one from model arguments. The permit-bound executor rechecks that same session against
canonical target records, derives decision source from the model actor, and rejects
cross-session access before repository mutation or release.

Plans are session-scoped canonical records with ordered typed steps and explicit
`draft`, `approved`, `executed`, or `discarded` state. Only drafts can change content;
approval, execution, and discard are append-only transitions, and an approved plan can
be consumed by only one run. Model schemas omit the session id, `plan.approve_request`
uses the ordinary approval-proof re-evaluation path, and `plan.show` rechecks canonical
session ownership before releasing content.

Goal Mode is a bounded loop over the ordinary `colossus-agent` service, not a second
orchestrator or permission domain. Canonical goals record objective, session, optional
plan lineage, a 1..=50 iteration budget, completed iterations, terminal evidence, and
timestamps. Each iteration is a normal session-attached run with goal/plan ids copied
through `ExecutionContext`. `goal.show` and `goal.update` are removed from ordinary
provider requests and exposed only when that context names an active goal. Starting a
goal from an approved plan atomically appends both the plan execution and goal creation,
so one plan cannot race into multiple autonomous loops.
The goal objective is rendered from the canonical approved prompt, Markdown, ordered
steps, and mutation labels rather than accepting a different model-authored handoff.

Subagents are canonical queued work records owned by the same work repository. A job
pins parent session/run/call lineage, an isolated child session, model role, bounded task,
lifecycle timestamps, and bounded released output or redacted error. The runtime drains
queued jobs in batches no larger than `subagents.maxConcurrent` and each child invokes
the normal `colossus-agent` service with the same provider router, policy gateway,
approval provider, tool executor, context preparation, and journal. Child request
definitions remove `agent.delegate`; the executor also rejects delegation whenever
`ExecutionContext.subagent_id` is present. Running jobs found at startup become
`interrupted` and are never silently retried.

`colossus-memory` separates canonical lifecycle state from disposable retrieval. Memory
create/update/archive/supersede events remain authoritative in the encrypted journal.
Updates append a complete validated next-state record while preserving identity, scope,
source, and creation time. Tantivy
stores candidate ids, bounded searchable text, metadata, event-id markers, and a durable
global replay position; failed index work remains in the journal and is retried without
blocking canonical reads. Search always reloads candidate ids from the repository and
reapplies active status, expiry, and global/repository/session scope. Every lifecycle,
read, search, and index operation crosses the gateway. Context composition receives only
post-effect-authorized canonical records and places them after decisions and before
snapshots as non-instructional background. Model tools name only `global`, `repository`,
or `session`; the runtime derives the stable repository identity from the canonical
workspace path and the session identity from the execution context. Targeted access is
checked against the canonical record after authorization, and list limits are applied
after scope filtering.

When configured, `colossus-memory-chroma` replaces only the disposable candidate
projection. It uses Chroma's v2 collection API with caller-generated embeddings and
persists its replay position locally. The collection contains memory ids, bounded text,
bounded metadata, embeddings, and source event ids—not lifecycle authority. Both Chroma
transport and OpenAI-compatible embedding transport are permit-bound, exact-origin,
bounded effects. The offline local embedding profile uses deterministic token/bigram
feature hashing and does not claim model-quality semantic understanding.

Filesystem, subprocess, and HTTP effects now use concrete permit-bound adapters. Exact
subprocess specifications are authenticated to a one-shot helper, which clears the
environment and applies the selected native or OCI isolation profile before spawning.
Native macOS/Linux isolation uses Seatbelt/Landlock, process groups contain descendants,
and native networked subprocesses receive only a loopback allowlist proxy. Networked OCI
jobs use a Colossus proxy sidecar: the workload joins an internal proxy-only network,
the sidecar alone joins a separate egress network, and the authenticated bootstrap pins
the policy-approved origin/address sets. The direct HTTP adapter applies the same
origin and DNS-address constraints and keeps the bounded response in gateway quarantine
until post-effect policy allows release. Windows native filesystem/network isolation
remains fail-closed, and Windows OCI path mapping stays disabled until its live platform
suite passes.

The strict Rust tool catalog implements the complete required offline surface. Repository
mapping/search, context reads and mutations, exact patching, and trace export use private
runtime adapters and the normal policy gateway; `tool.search` and metadata-only
`trace.show` remain pure bounded computation. `user.ask` is an optional interface port
injected only for an interactive embedded REPL, so workers, one-shot commands, and
scripted input cannot unexpectedly read from a terminal. `web.fetch` and `docs.fetch`
share the permit-bound exact-origin HTTP capability and quarantine path.

The journal is authoritative. Application state is reconstructed by replay, and redb
atomically appends events, advances stream/global versions, and queues projection work.
Named projection workers consume that outbox in global order and atomically commit
record mutations with an optimistic position. Work repositories and session discovery
serve those disposable views; canonical session messages are reconstructed directly
from journal streams. Reset/rebuild always replays the canonical journal. A cross-process
writer lease prevents embedded surfaces and the headless worker from opening concurrent
redb writers. The long-running worker owns that lease and serves a versioned local
application protocol over a mode-0600 Unix socket or a Windows named pipe. CLI one-shot
runs, the worker-aware REPL, session operations, and workflow lifecycle operations
auto-discover the worker and otherwise use the same runtime in-process. Durable task,
decision, plan, goal, child-agent, and memory lifecycle commands use that application
protocol as well. Research, declarative skill, signed pack/bundle, integration, MCP,
process, and network terminal operations are also dispatched to the worker when active.
The worker-backed REPL exposes the same implemented slash-command operations as embedded
mode, and both REPL paths accept either an interactive terminal or line-oriented stdin
for automation and acceptance testing.
The transport does not contain provider, policy, workflow, or repository logic.
Independent clients run concurrently, while projection rebuild/drain, memory-index
maintenance, and queued child work share one worker coordination lock so optimistic
positions cannot race.
Workflow definitions are exact-content hash pinned and workflow runs are normal journal
streams. Registration and start validate the complete available call graph, reject direct
or indirect cycles, and enforce a 16-level call-depth ceiling. Manual starts are recorded
as queued and atomically claimed before execution; `workflow run --queued` leaves work for
the worker's bounded drain path. Restart reconstruction restores completed outputs and the
consumed attempt budget. Abandoned attempts become unknown and are never automatically
retried. Known failures may retry once only with an explicit idempotency strategy, and
definition-level compensation effects are dispatched separately through the same policy
gateway. A `workflow` step first authorizes `workflow.start`, then creates a separately
hash-pinned child run carrying parent run, parent step, and call depth. Parents expose the
blocking child ID while waiting, observe the same child on resume, propagate child
terminal failure, and cascade cancellation. The parent intent event contains enough data
to recreate the same child ID after a crash between link and queue, so recovery neither
duplicates nor orphans the call. The composition root opens fresh YAML config and fresh
state; it never silently imports the Python SQLite store.

See [Rust Reconstruction Status](RUST_RECONSTRUCTION.md) for the current implementation
line and remaining milestones.

## Python 0.5 Legacy Architecture

For user-facing documentation, start with [Documentation Home](README.md). This document
is the implementation-boundary reference for contributors.

```text
interfaces -> application -> ports -> domain
adapters   -> application -> ports -> domain
infrastructure may be used by interfaces/adapters/application
domain depends on the Python standard library and Pydantic only
```

The core orchestration loop consumes typed model events, executes approved tools, writes
state and audit records through ports, and emits a typed run result. Providers normalize
OpenAI Responses and local OpenAI-compatible servers into the same event model.
Streaming providers emit `ModelDeltaEvent`, `ToolCallRequestedEvent`, and optional
safe `ReasoningSummaryEvent` values through the same observer path used by the CLI,
and REPL.

The Rust provider boundary consumes Responses API and compatible Chat Completions SSE
incrementally. Each normalized event is quarantined, independently post-authorized when
required, durably appended to the run stream, and only then forwarded to an interface
observer. A terminal stream item carries provider response metadata and must be the final
released item. An interrupted stream preserves already released events and returns an
unknown outcome instead of synthesizing completion or retrying. Provider usage is a typed
event and telemetry aggregates its normalized token counts.

User surfaces must be thin:

- CLI commands compose services and render results.
- REPL parses slash commands and sends turns to application services.

No user surface owns model calls, tool execution, policy decisions, or persistence.

## Model Routing

The application `ModelRouter` resolves named roles such as `primary`,
`risk_evaluator`, `context_summarizer`, and `subagent_default` to concrete provider/model
profiles. Infrastructure builds providers from config; application services consume
role-resolved providers without knowing where they came from. Legacy single-provider
config is normalized into the `primary` role for compatibility.

## Tool Composition

Built-in tool specifications and handlers are composed in adapters, then exposed through
the application `ToolRegistry` and `ToolExecutor` ports. The orchestrator validates tool
arguments before policy and approval handling, then records policy and completion audit
events. CLI and REPL code may list or render tools, but must not implement model, tool,
policy, or state behavior.

Durable `task.*`, `decision.*`, `plan.*`, and `memory.*` model tools use the same composition.
Their public schemas omit session and repository identifiers, the runtime constructs
typed operations with trusted context, and private work/memory executors perform a final
canonical scope check while consuming the permit. Their JSON results remain quarantined
until the configured post-effect decision allows disclosure.

The Rust workspace tools preserve this split: `colossus-tools` owns strict model-visible
schemas, `colossus-runtime` resolves workspace-relative resources and constructs typed
effect requests, and `colossus-sandbox` performs permit-bound filesystem operations.
Recursive search is an in-process adapter rather than an implicit subprocess, respects
ignore files, does not follow links, skips runtime/VCS control directories, and returns
only bounded UTF-8 matches through the post-effect release gate.

Mutation tools send the complete proposed text and mode through the pre-effect decision,
then the permit-bound filesystem adapter evaluates create/overwrite/append or exact
replacement and commits with an atomic rename. The adapter prepares bounded diff evidence
before committing so an oversized result cannot turn a successful write into an
ambiguous adapter failure. Terminal and embedded callers inject an `ApprovalProvider`
when composing the runtime; approval remains an obligation that triggers policy
re-evaluation, never an alternate execution route.

Rust Git and structured-command tools translate to the same `ProcessSpec` and
authenticated helper, but preserve `git.status`, `git.diff`, `git.show`, and `shell.run`
as separate policy/capability identities. The runtime resolves only exact configured
executables and builds literal argv; the policy kernel applies the same executable, cwd,
environment, backend, resource, and network obligations to every process identity. The
sandbox treats a returned nonzero exit as a completed, known process outcome and reserves
failed/unknown effect terminals for execution, timeout, resource, or cleanup failures.

Provider adapters remain strict about malformed tool-call argument payloads. When a
provider raises the standard invalid tool-argument `ProviderError`, the orchestrator may
perform a bounded recovery turn by emitting a recoverable `ErrorEvent`, appending a
metadata-only correction prompt, and auditing the retry. No tool is executed until a
later provider turn emits a valid typed `ToolCallRequestedEvent`.

Shell tool calls can optionally pass through `RiskAssessmentService` after deterministic
policy and before approval. The risk model receives redacted structured metadata with
tools disabled and can only escalate risk or add audit/trace context.

## Integrations

`IntegrationService` owns typed integration manifests, persisted connection records, and
connect/disconnect/import workflows. The first `CredentialBroker` adapter resolves
environment credential refs, but tool schemas and model requests see only normal
operation arguments. Connected integrations are converted to `ToolSpec`s by adapters and
enter the same registry, policy, approval, execution, HTTP configuration, and audit path
as built-in tools.

The initial native connectors are GitHub for coding workflows, SearXNG for local or
private metasearch, and OpenSearch for document-focused search and writes. OpenAPI
imports generate operation tools from JSON OpenAPI documents and execute through the
brokered HTTP adapter. MCP remains an explicit configured integration protocol; it is
not exposed as arbitrary model-callable execution unless configured and policy-approved.
The Rust MCP adapter uses the official `rmcp` protocol models, launches only exact
configured stdio executables through the authenticated sandbox helper, filters every
discovery page through an exact tool allowlist, and validates call arguments against a
freshly discovered schema. Each page and call is a separate normal effect. Environment
credential references are resolved only after permit issuance, server output is bounded
and quarantined, and configured MCP calls can also supply the research collector.

Colossus should not depend on ADK in core. Compatibility comes through importers and
adapters. A future AI proxy should be a separate phase behind the same credential
boundary for model-provider routing, usage, and rate limits; app credentials and model
provider credentials stay separate.

## Context Composition

The application `ContextService` prepares model input messages before each provider turn.
It may replace older raw session messages with a synthetic context snapshot plus recent
tail messages, but raw messages and run events remain persisted in SQLite. Providers do
not own compaction behavior; OpenAI-compatible online and local endpoints receive the
same normalized compacted message list.

## Skill Composition

The application `SkillComposer` resolves agent-allowed skills, parses prompt mentions
such as `@skill:coding`, validates required tools against the active catalog, and
composes skill context into provider instructions. Interfaces may provide sticky skill
names or tab completion, but they do not inject `SKILL.md` content themselves.

## Deep Research

`ResearchService` is an application service that coordinates research phases without
putting orchestration logic in CLI or REPL code. It plans bounded queries, collects
evidence through repo/search/MCP source ports, asks for approval before networked lanes,
persists `ResearchRun`, `ResearchSource`, and `ResearchClaim` records through the state
port, and emits typed `ResearchStatusEvent` values for renderers. It also reads a bounded
prior-session context block for planning and synthesis, then appends the completed cited
report as a normal assistant session message so later chat turns can continue from it.

Search and MCP are adapter concerns. The default config keeps web search disabled and
MCP unconfigured; when enabled, their tool specs become visible through the normal tool
catalog and still pass policy, approval, and audit paths for model-callable use. Search
credentials stay in provider configuration and environment variables rather than in the
provider-neutral `web.search` tool input.
Research planner, worker, and synthesizer model roles default to `primary` unless
configured separately.

## Event Rendering

Terminal activity rendering consumes typed run events. The CLI uses `interfaces.trace`
for `--stream`, `--events compact|verbose|off`, and legacy `--trace` behavior. The REPL
uses `interfaces.transcript` to render a readable interactive transcript with user,
assistant, reasoning-summary, tool, approval, risk, and error blocks. Renderers may show
provider-supplied reasoning summaries, but they must not display raw hidden
chain-of-thought fields.

The REPL composer state and built-in themes also live in the interface layer. They may
render cached model, approval, session, prompt, and context status, but they must
continue to call application services for orchestration and context data rather than
owning those behaviors.

## Telemetry And Observability

`TelemetryService` is an application service that derives operational summaries from
timestamped persisted run events. It reports run duration, event counts, tool
calls/failures, approvals, risk assessments, research/subagent activity, compactions, and
error totals without exposing raw prompts, hidden reasoning, or raw tool outputs by
default. CLI and future TUI surfaces should call the service for run lists, timelines,
and aggregate metrics rather than querying SQLite or parsing transcript text directly.

Saved REPL preferences are modeled as typed domain data, exposed through the state port,
and coordinated by an application service. The interface may load and save those
preferences, but SQLite remains an adapter detail and preference persistence must not
leak into prompt rendering or orchestration code.
