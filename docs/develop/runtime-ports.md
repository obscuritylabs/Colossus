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

## Provider streams

Provider adapters normalize Responses-compatible or Chat-Completions-compatible streams.
Each event is quarantined, post-authorized where required, durably appended, and only
then sent to an observer. An interrupted stream preserves already released events and
returns uncertainty rather than synthesized completion.

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
