# Architecture

Colossus uses a ports-and-adapters architecture with strict dependency direction.

## Rust Runtime Boundary

The Rust runtime is the active repository-root implementation. Its dependency direction
is explicit:

```text
colossus-cli -> colossus-runtime -> colossus-agent -> colossus-ports
                    |                  |                ^
                    |                  +-> colossus-tools+
                    +-> colossus-policy -----------------+
                    +-> colossus-workflow ---------------+
                    +-> colossus-provider -> colossus-policy
                    +-> colossus-mcp -> colossus-sandbox -> colossus-policy
                    +-> colossus-audit -> colossus-policy
                    +-> colossus-sandbox  -> colossus-policy
                    +-> colossus-sandbox  -> colossus-windows-process (Windows only)

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
session ownership before releasing content. Plan Mode is a distinct application run
scope that exposes only inspection, task-creation, and plan tools; the agent rejects
provider calls for tools not offered in that scope before the executor can run. Direct
approved-plan execution consumes the plan through `plan.execute` before starting the
fixed-id agent run, so concurrent or repeated execution cannot duplicate effects.

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
Successful model calls to `agent.delegate` notify the owning runtime scheduler and yield
the parent turn. The bounded scheduler claims and completes the child before the parent
continues to `agent.result`, so CLI, TUI, worker, and embedded API runs share the same
foreground delegation behavior. Manual application/CLI queue creation remains durable
and is executed through the explicit drain operation.

`colossus-memory` separates canonical lifecycle state from disposable retrieval. Memory
create/update/archive/supersede events remain authoritative in the encrypted journal.
Updates append a complete validated next-state record while preserving identity, scope,
source, and creation time. The journal atomically enqueues every event into a durable
external-work outbox. Tantivy and optional Chroma consumers hold independent optimistic
checkpoints in redb, persist their adapter position before acknowledging a contiguous batch,
and replay safely by event id after a crash. A failed consumer retains its own work
without blocking canonical reads or another index. Consecutive failures persist a
consumer-local attempt count, stable error category, bounded diagnostic, and exponential
retry deadline from one to 300 seconds. Non-retryable and unknown outcomes remain blocked
until an explicit rebuild. Search merges available candidate ids
from every healthy index, reloads them from the repository, and reapplies active status,
expiry, and global/repository/session scope. Every lifecycle,
read, search, and index operation crosses the gateway. Context composition receives only
post-effect-authorized canonical records and places them after decisions and before
snapshots as non-instructional background. Model tools name only `global`, `repository`,
or `session`; the runtime derives the stable repository identity from the canonical
workspace path and the session identity from the execution context. Targeted access is
checked against the canonical record after authorization, and list limits are applied
after scope filtering.

Adapter compatibility is executable rather than documentary. One shared testkit contract
runs against in-memory and encrypted-redb journal/projection stores, factory-reopened
session/work/memory/workflow repositories on both journals, and Tantivy/Chroma indexes.
Canonical memory fallback remains available when every index is offline or a destructive
rebuild fails, and the recovered index later replays the complete journal.

When configured, `colossus-memory-chroma` adds an optional semantic candidate projection
alongside the offline Tantivy lexical default. It uses Chroma's v2 collection API with
caller-generated embeddings and
persists its replay position locally. The collection contains memory ids, bounded text,
bounded metadata, embeddings, and source event ids—not lifecycle authority. Both Chroma
transport and OpenAI-compatible embedding transport are permit-bound, exact-origin,
bounded effects. The offline local embedding profile uses deterministic token/bigram
feature hashing and does not claim model-quality semantic understanding.

`colossus-audit` consumes the journal's atomic external-work outbox through its own
durable optimistic checkpoint. It converts each canonical envelope into strict
`AuditEvidence` containing lineage, payload algorithm/key identity, plaintext hash, and
chain hashes, but never payload ciphertext, nonce, or plaintext. The initial directory
adapter writes deterministic sequence/event-id JSON files only through
`audit.export.write` and the filesystem effect executor. Delivery failures retain queue
position and bounded retry state; unknown outcomes require an explicit operator reset.
Because authorizing an export appends ordinary effect lifecycle events, those events use
the reserved `system/audit-exporter` actor and are acknowledged without re-export to
prevent recursion. The canonical journal still retains them. Additional exporters reuse
the same evidence, queue, policy, and conformance boundary. Process-level fault tests
terminate the journal immediately before and after redb commit and terminate export after
delivery but before queue acknowledgment; recovery proves atomic rollback or durable
visibility and idempotent evidence replay, respectively. Verified startup repairs a
periodic checkpoint interrupted after its event commit, and checkpoint scheduling uses
distance from the last signed sequence so a multi-event batch cannot cross the interval
unnoticed. The secure anchor is persisted before checkpoint metadata; if termination
lands between them, startup verifies the anchored head and recreates the missing signed
checkpoint.

Repository adapters are verified through shared port-level conformance factories rather
than implementation-specific happy paths. The research suite reopens the adapter and
checks immutable provenance, sequential evidence, citation resolution, terminal state,
filtering, and reconstruction. The extension suite does the same for integration state,
pack lifecycle, publisher trust, aggregate access, deterministic bounds, and restart
reconstruction. A future adapter must pass these suites without weakening its port.

Filesystem, subprocess, and HTTP effects now use concrete permit-bound adapters. Exact
subprocess specifications are authenticated to a one-shot helper. For native macOS/Linux
execution, a trusted outer helper monitors the process tree and passes the same signed job
to a re-authenticated inner helper; only the inner helper clears the environment, applies
Seatbelt/Landlock, and spawns the target. Keeping the monitor outside filesystem
confinement lets Linux account descendants through `/proc` without granting the target
that access. Process groups plus explicit tree termination contain descendants, and
native networked subprocesses receive only a loopback allowlist proxy. Networked OCI
jobs use a Colossus proxy sidecar: the workload joins an internal proxy-only network,
the sidecar alone joins a separate egress network, and the authenticated bootstrap pins
the policy-approved origin/address sets. The direct HTTP adapter applies the same
origin and DNS-address constraints and keeps the bounded response in gateway quarantine
until post-effect policy allows release. Windows `windows_job` execution uses an ephemeral
AppContainer identity for filesystem and default-deny network isolation plus an atomically
attached Job Object for descendant ownership and hard process/memory limits. Networked
jobs receive a per-permit authenticated loopback proxy. Package-SID-scoped dynamic WFP
filters permit only the proxy's exact `127.0.0.1` TCP port and hard-block every other
IPv4/IPv6 connection; the parent proxy then applies the same exact-origin checks used by
the Unix native path. Setup failure blocks process creation. Windows OCI path mapping
stays disabled until its live platform suite passes.

The strict Rust tool catalog implements the complete required offline surface. Repository
mapping/search, context reads and mutations, exact patching, and trace export use private
runtime adapters and the normal policy gateway; `tool.search` and metadata-only
`trace.show` remain pure bounded computation. `user.ask` is an optional trusted interface
port. Embedded TUI runs bridge it through a one-use overlay; worker-backed TUI runs use
authenticated protocol-v4 prompt frames. Headless workers, one-shot commands, and
scripted input cannot fabricate an answer. `web.fetch` and `docs.fetch` share the
permit-bound exact-origin HTTP capability and quarantine path.

The journal is authoritative. Application state is reconstructed by replay, and redb
atomically appends events, advances stream/global versions, and queues projection work.
Named projection workers consume that outbox in global order and atomically commit
record mutations with an optimistic position. Work repositories and session discovery
serve those disposable views; canonical session messages are reconstructed directly
from journal streams. Reset/rebuild always replays the canonical journal. A cross-process
writer lease prevents embedded surfaces and the headless worker from opening concurrent
redb writers. The long-running worker owns that lease and serves a versioned local
application protocol over a mode-0600 Unix socket or a Windows named pipe. CLI one-shot
runs, the worker-backed TUI, session operations, and workflow lifecycle operations
auto-discover the worker and otherwise use the same runtime in-process. A busy Windows
named pipe, including a connected pipe waiting for its authenticated hello, is treated
as a live worker with bounded connection backoff, never as permission to fall through to
a second embedded writer. The Windows listener publishes its replacement pending instance
before dispatching each connection; concurrent terminal clients wait with bounded backoff
instead of opening another writer. Durable task,
decision, plan, goal, child-agent, and memory lifecycle commands use that application
protocol as well. Research, declarative skill, signed pack/bundle, integration, MCP,
process, and network terminal operations are also dispatched to the worker when active.
Offline-bundle build/install are normal pack-adapter effects rather than release-script
bypasses. Build copies a staged target tree, derives a signature from a late-resolved
credential reference, re-verifies it against canonical publisher trust, and publishes
atomically. Install re-verifies and selects the compile-platform target convention before
no-clobber creation in a permitted prefix. Both operations use the same worker routing,
approval proof, one-use permit, disclosure, and effect lifecycle as other writes.
Worker-backed and embedded TUI hosts expose the same typed command and run contracts.
Non-TTY input selects a separate line-oriented compatibility runner for automation and
acceptance testing.
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

Workflow schedules are canonical journal aggregates behind `WorkflowRepository`. A
schedule stores a bounded fixed cadence, enable state, misfire policy, validated input
snapshot, and exact workflow hash. The worker evaluates them under its existing
maintenance coordination lock before draining queued workflow runs. Due-count and the
next UTC occurrence are reconstructed arithmetically from durable state and the supplied
clock; evaluation does not replay an unbounded occurrence loop. For a firing occurrence,
the schedule transition and deterministically identified `queued` run are appended in
one journal batch. Definition/hash/call-graph/input trust is checked again at firing and
explicit enable. Actual invalidation disables the schedule, while repository failures
surface for retry rather than being persisted as a trust decision. Trigger-created runs
then use the ordinary claim, policy, approval, effect, and recovery paths.

Workflow webhooks are canonical hash-pinned bindings behind the same repository. A
binding stores an operator-selected identifier, bounded replay/body limits, enable state,
and a credential reference, never the HMAC secret. The runtime resolves an `env:` secret
only at ingress, verifies HMAC-SHA256 over the exact timestamp, delivery identifier, and
raw JSON body, rejects stale or repeated deliveries, and then submits
`workflow.webhook.ingest` through the ordinary effect gateway. The request discloses the
complete body and application headers plus a credential reference and secret hash to
policy, but not the secret or submitted signature. After authorization, a shared writer
lock rechecks binding trust and replay state; the accepted-delivery receipt and
deterministically identified `queued` run are one journal batch. The optional HTTP adapter
is a bounded loopback-only HTTP/1.1 listener intended to sit behind a trusted reverse
proxy. CLI, TUI, embedded runtime, and authenticated worker transports remain interfaces
to this application behavior rather than owning authentication or persistence rules.

Runtime execution identity is separate from the static YAML step ID. Root steps retain
their declared ID, while nested repeated work receives bounded paths such as
`each[1]/approval` and `parallel.branch[0]/tool`. Journal completion, waiting input,
idempotency, retry, effect context, and child-workflow links use that scoped identity.
Replay therefore cannot apply one `foreach` item's result or approval to another item.

Recovery binds an abandoned attempt to its scoped execution id as well as its static step
id. A process loss after a durable primary effect permits operator resume only when that
exact primary step declared idempotency. An uncertain compensation is phase-labeled and
always remains fail-closed because ordinary run resume would otherwise return to the primary
sequence. A durable step completion found after its matching start is resumed from the next
root step rather than labeled unknown or executed again. Nested sequence replay recognizes
already durable scoped completions without appending duplicate completion events. If both a
parent and its linked child are interrupted, parent resume fails before changing state until
the child is explicitly recovered; after the child completes, the parent consumes the same
durable link without repeating `workflow.start`.

Interactive work refresh is an application-layer aggregate, not terminal-owned state.
`Runtime::work_state` reconstructs a bounded exact-session snapshot of tasks, active
decisions, actionable plans, current goals, and nonterminal subagents. The `work` CLI,
embedded `/work`, and worker-backed `/work` all use this same contract; the authenticated
worker never grants clients direct repository access.

See [Rust Reconstruction Status](RUST_RECONSTRUCTION.md) for the current implementation
line and remaining milestones.

## Application Services And Adapters

For user-facing documentation, start with [Documentation Home](README.md). This document
is the implementation-boundary reference for contributors.

The Rust provider boundary consumes Responses API and compatible Chat Completions SSE
incrementally. Each normalized event is quarantined, independently post-authorized when
required, durably appended to the run stream, and only then forwarded to an interface
observer. A terminal stream item carries provider response metadata and must be the final
released item. An interrupted stream preserves already released events and returns an
unknown outcome instead of synthesizing completion or retrying. Provider usage is a typed
event and telemetry aggregates its normalized token counts.

User surfaces must be thin:

- CLI commands compose services and render results.
- TUI parses interactive commands and sends turns to an `InteractiveHost`.

No user surface owns model calls, tool execution, policy decisions, or persistence.

## Model Routing

The application `ModelRouter` resolves named roles such as `primary`,
`risk_evaluator`, `context_summarizer`, and `subagent_default` to concrete provider/model
profiles. Infrastructure builds providers from config; application services consume
role-resolved providers without knowing where they came from. Unconfigured specialized
roles resolve through the `primary` profile.

## Tool Composition

Built-in tool specifications and handlers are composed in adapters, then exposed through
the application `ToolRegistry` and `ToolExecutor` ports. The orchestrator validates tool
arguments before policy and approval handling, then records policy and completion audit
events. CLI and TUI code may list or render tools, but must not implement model, tool,
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
tail messages, but raw messages and run events remain encrypted in the canonical event
journal. Providers do not own compaction behavior; OpenAI-compatible online and local
endpoints receive the same normalized compacted message list.

## Skill Composition

The application `SkillComposer` resolves agent-allowed skills, parses prompt mentions
such as `@skill:coding`, validates required tools against the active catalog, and
composes skill context into provider instructions. Interfaces may provide sticky skill
names or tab completion, but they do not inject `SKILL.md` content themselves.

## Deep Research

`ResearchService` is an application service that coordinates research phases without
putting orchestration logic in CLI or TUI code. It plans bounded queries, collects
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

Terminal activity rendering consumes strict `RunEventEnvelope` values through the
`RunEventObserver` port. Provider events remain a separate normalized contract; the agent
wraps them with correlated run/session identity and adds durable run phase, tool-start,
tool-completion, recoverability, and elapsed-time events. Provider and tool content is
observed only after its corresponding authoritative journal event is durable and any
post-effect release policy has allowed it. Authenticated worker protocol v4 transports
the same envelopes used by embedded callers.

`colossus-presentation` renders compact, verbose, or off activity modes and semantic
file, shell, Git, work, context, repository, skill, web, MCP, trace, integration, pack,
and generic tool families. Renderers may show provider-supplied safe reasoning summaries,
but they never receive raw provider frames or hidden chain-of-thought fields.

The TUI composer state and theme selection live in the interface layer. Embedded and
authenticated-worker hosts supply cached model, approval, session, context, work-state,
and presentation status. Cache refreshes use bounded application/worker operations only
after relevant mutations or completed runs rather than querying repositories or creating
per-keystroke audit traffic. Read-only commands return their document without forcing a
full footer refresh.

`colossus-tui` is a reducer plus one terminal event loop. It owns Unicode editing,
history navigation, completion, retained transcript layout, scrolling, overlays, queueing,
and terminal restoration. Background runtime tasks can only publish typed `HostEvent`
values through bounded channels. Draft content stays interface-local, and per-keystroke
edits never cross worker IPC, repositories, the effect gateway, or the journal.

`TerminalPalette` is a pure data-only presentation mapping for the five built-in theme
identities and resolved custom-theme snapshots. `ThemeLibrary` performs bounded,
strict JSON/TOML configuration loading; selection persists an immutable palette plus
source hash through `PresentationRepository` rather than retaining a mutable file
reference. Both supply Ratatui styles, semantic terminal strings, assistant styling,
bounded activity frames, theme-aware type-ahead, and visual preview documents. The
theme picker remains interface-only, while scaffold output is a validated template that
never writes a file from the terminal surface. ANSI emission is selected only by the
terminal interface after an `IsTerminal` check; the renderer defaults to unstyled text
so workers, pipes, logs, and embedded callers cannot receive accidental control
sequences.

## Telemetry And Observability

`TelemetryService` is an application service that derives operational summaries from
timestamped persisted run events. It reports run duration, event counts, tool
calls/failures, approvals, risk assessments, research/subagent activity, compactions, and
error totals without exposing raw prompts, hidden reasoning, or raw tool outputs by
default. CLI and TUI surfaces call the service for run lists, timelines, and
aggregate metrics rather than querying redb or parsing transcript text directly.

Saved terminal preferences and bounded submitted-input history are exposed through
`PresentationRepository`. The Rust runtime uses an encrypted event-sourced adapter and
sends every preference or history mutation through the effect gateway before appending
its canonical event. The TUI receives only the newest 1,000 decrypted entries through the
application API; it never owns a plaintext history file. The CLI and authenticated worker
only coordinate the application operation; they do not write presentation files or
canonical storage directly. The legacy stream and event identities remain unchanged for
state compatibility. Pure semantic rendering remains in
`colossus-presentation`, and preference values never enter provider routing, policy,
tool, approval, or prompt-composition decisions. Legacy preference state is not imported.
