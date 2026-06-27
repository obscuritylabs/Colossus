# Architecture

Colossus uses a ports-and-adapters architecture with strict dependency direction.

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

Saved REPL preferences are modeled as typed domain data, exposed through the state port,
and coordinated by an application service. The interface may load and save those
preferences, but SQLite remains an adapter detail and preference persistence must not
leak into prompt rendering or orchestration code.
