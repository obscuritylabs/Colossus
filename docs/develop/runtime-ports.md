---
title: Runtime and ports
description: Ownership map for Colossus application services, runtime composition, worker transport, and adapters.
audience: developer
type: concept
---

# Runtime and ports

`colossus-runtime` is the composition root. It parses a fresh strict configuration,
constructs adapters, resolves access, and exposes application operations. It does not
turn infrastructure into domain authority.

## Service ownership

| Concern | Owner | Port or boundary |
| --- | --- | --- |
| Bounded model/tool loop | `colossus-agent` | `ModelProvider`, `ToolRegistry`, `ToolExecutor` |
| Provider role selection | Runtime model router | Named profile and role contracts |
| Sessions | `colossus-session` | Canonical session repository |
| Context compaction | `colossus-context` | `ContextPreparer` |
| Tasks, decisions, plans, goals, agents | `colossus-work` and runtime services | Canonical work repositories and effect executors |
| Memory lifecycle and retrieval | `colossus-memory` | Repository, external-work queue, lexical/semantic index ports |
| Workflow definitions and runs | `colossus-workflow` | Workflow repository and effect dispatcher |
| Research | `colossus-research` | Repository, search, MCP, and model ports |
| Telemetry | `colossus-telemetry` | Persisted run-event query port |
| Tool metadata and validation | `colossus-tools` | Immutable active catalog |
| Capability selection | `colossus-access` | Trusted metadata and prerequisite resolver |
| Effect authorization | `colossus-policy` | Safety Kernel, decisions, approval proofs, permits |
| Presentation documents | `colossus-presentation` | Pure released-contract renderer |

The agent owns provider turns, strict tool argument validation, persisted prepared
requests, normalized events, bounded correction, and terminal max-turn behavior. The
runtime translates validated tool operations into application requests and effect
gateway calls.

## Runtime composition structure

The runtime keeps its public facade stable while separating the startup responsibilities
that a future host needs to reason about independently:

- `config` owns the strict serialized schema and path resolution; `adapter_composition`
  constructs provider, search, and memory adapters from that already-validated schema.
- `storage_composition` opens keys, the canonical journal, projection storage, recovery
  state, and non-secret storage diagnostics as one unit.
- `access_policy` resolves the model-visible catalog and effect policy together. A host
  cannot assemble a tool catalog independently from its action and resource authority.
- `composition` owns ordering, recovery, and final service wiring after those narrower
  units succeed.
- `agent_runs`, `plan_runs`, and `goal_runs` respectively own generic model dispatch,
  Plan lifecycle orchestration, and bounded durable Goal execution.

These are private composition boundaries, not alternate public APIs. CLI, TUI, worker,
gRPC, and embedded callers continue to enter through `Runtime` and the stable SDK
contracts.

## Provider streams

Provider adapters normalize Responses-compatible or Chat-Completions-compatible streams.
Consecutive model-text deltas are coalesced without reordering until they reach 4 KiB,
100 ms elapses, or a different event arrives. Each resulting event is quarantined,
post-authorized where required, durably appended, and only then sent to an observer.
This keeps interactive streaming responsive while making policy and journal volume
independent of a provider's token-fragment granularity. An interrupted stream preserves
already released events, flushes accepted buffered text through the same release gate,
and returns uncertainty rather than synthesized completion.

## Worker transport

The worker owns the local writer lease and serves a versioned authenticated application
protocol over a mode-0600 Unix socket or Windows named pipe. CLI and TUI discover it and
otherwise use the same runtime in-process.

Transport carries typed requests, documents, run events, and one-use interactive prompt
frames. It contains no provider, policy, workflow, repository, or terminal-markup logic.
A wrong, replayed, malformed, stale, or incompatible endpoint fails closed.

## Interface rule

If CLI or TUI code starts deciding policy, invoking adapters directly, persisting
canonical records, or recreating workflow/model logic, move that behavior behind an
application operation. If an adapter starts owning use-case sequencing, introduce or
extend a port and keep the sequence in the application layer.
