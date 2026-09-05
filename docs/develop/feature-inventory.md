---
title: Feature ownership inventory
description: Locate the canonical implementation, documentation, and acceptance evidence for supported Colossus capabilities.
audience: developer
type: reference
---

# Feature ownership inventory

This page is a navigation index, not a duplicate product specification. Public behavior
belongs in the linked user, operator, and reference pages; executable truth belongs in
the owning crates and tests.

| Capability | Owning implementation | Canonical documentation | Primary evidence |
| --- | --- | --- | --- |
| Access profiles, approvals, policy, and effect release | `colossus-access`, `colossus-policy`, `colossus-runtime` | [Access and approvals](../admin/access-and-approvals.md), [Security architecture](security-architecture.md) | Policy unit/live tests and runtime deny-before-adapter tests |
| Native, OCI, and Windows isolation | `colossus-sandbox`, platform process crates, sidecar crates | [Sandbox](../admin/sandbox.md) | Platform acceptance suites and CLI sandbox targets |
| Durable journal, projections, recovery, and audit export | journal, projection, audit, telemetry crates | [State and recovery](state-recovery.md), [Audit and recovery](../admin/audit-telemetry-recovery.md) | Shared conformance, tamper, crash, outage, and export tests |
| Providers, model routing, and normalized turns | `colossus-provider`, `colossus-agent`, `colossus-runtime` | [Choose a provider](../use/providers/index.md), [Providers and models](../reference/configuration/providers-models.md) | Provider and agent suites plus CLI provider smoke tests |
| Sessions and context | `colossus-session`, `colossus-context`, `colossus-runtime` | [Sessions and context](../use/sessions-context.md) | Repository, snapshot, compaction, and branch tests |
| Tasks, decisions, plans, goals, and subagents | `colossus-work`, `colossus-agent`, `colossus-runtime` | [Tasks, decisions, and plans](../use/tasks-decisions-plans.md), [Goals and subagents](../use/goals-subagents.md) | Lifecycle, lineage, budget, cancellation, and recovery tests |
| Memory and semantic projection | `colossus-memory`, `colossus-memory-chroma` | [Memories](../use/memories.md), [Context and memory configuration](../reference/configuration/context-memory-research.md) | Canonical lifecycle, Tantivy, Chroma, embedding, and recovery tests |
| Search and source-backed research | `colossus-search`, `colossus-research`, `colossus-runtime` | [Web search](../use/web-search.md), [Deep research](../use/deep-research.md) | Search adapter, evidence/citation, and CLI research tests |
| Workflows and triggers | `colossus-workflow`, `colossus-runtime` | [Workflow authoring](../extend/workflows/authoring.md), [Triggers and recovery](../extend/workflows/triggers-recovery.md) | Parser, control-flow, idempotency, compensation, and restart tests |
| Agent Plugins, integrations, standalone MCP, and release bundles | extension crates and runtime adapters | [Automate and extend](../extend/index.md) | Upstream schema, confinement, OCI, Sigstore, registry, lifecycle, and live-adapter tests |
| Public API, worker, SDKs, and Desktop | API, gRPC, worker, SDK crates and `apps/desktop` | [Public API and SDKs](application-sdk.md), [Colossus Desktop](../get-started/desktop.md) | Protocol, enrollment, parity, sidecar, SDK, and Desktop tests |
| Terminal and presentation | `colossus-presentation`, `colossus-tui`, interface adapters | [Terminal UI](../use/terminal-ui.md), [Extensions and presentation](extensions-presentation.md) | Semantic document, reducer, layout, PTY, and restoration tests |
| Releases and updates | `colossus-update`, CLI installers, release workflows | [Core release operations](releasing.md), [Upgrade and compatibility](../get-started/upgrade-compatibility.md) | Release-contract, clean-install, signature, channel, and platform jobs |

Colossus is alpha software. Configuration, protocols, and supported deployment surfaces
may change before a stable release. Compatibility retained for deployed Rust state or
protocols is documented at its owner; the frozen Python 0.5 runtime remains only on the
historical branches described in [ADR 0001](adr/0001-rust-runtime-cutover.md).
