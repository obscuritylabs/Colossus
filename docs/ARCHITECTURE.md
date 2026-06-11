# Architecture

Colossus uses a ports-and-adapters architecture with strict dependency direction.

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
REPL, and TUI.

User surfaces must be thin:

- CLI commands compose services and render results.
- REPL parses slash commands and sends turns to application services.
- TUI subscribes to application/run state and renders it.

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
events. CLI, REPL, and TUI code may list or render tools, but must not implement model,
tool, policy, or state behavior.

Shell tool calls can optionally pass through `RiskAssessmentService` after deterministic
policy and before approval. The risk model receives redacted structured metadata with
tools disabled and can only escalate risk or add audit/trace context.

## Context Composition

The application `ContextService` prepares model input messages before each provider turn.
It may replace older raw session messages with a synthetic context snapshot plus recent
tail messages, but raw messages and run events remain persisted in SQLite. Providers do
not own compaction behavior; OpenAI-compatible online and local endpoints receive the
same normalized compacted message list.

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
