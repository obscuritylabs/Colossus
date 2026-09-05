---
title: Core concepts
description: Understand runs, sessions, tools, authorization, durable state, and extensions before expanding Colossus access.
audience: user
type: concept
---

# Core concepts

## Runs and sessions

A **run** is one bounded agent execution. A **session** is the durable conversation and
work context to which runs append. Starting a new run can create a session, attach to an
exact session, or resume the most recent one.

Messages remain canonical journal events. Protected storage encrypts their payloads;
the keyless default stores plaintext while retaining payload hashes and record-chain
verification. Context compaction creates a derived snapshot for future provider
requests; it does not erase history.

## Models and roles

A provider profile defines a model endpoint and a credential reference. A role routes a
purpose—such as `primary`, `context_summarizer`, `subagent_default`, or a research
worker—to a profile. Unmapped specialized roles can fall back to `primary`.

The model can request tools, but it does not choose provider endpoints, credentials,
search backends, access profiles, or sandbox grants.

## Tools and effects

A tool has a strict input schema and an action classification. Pure tools can run
locally. Effectful tools—filesystem, process, Git, network, integration, and extension
operations—cross the effect gateway.

For an effect to occur:

1. the tool must be visible;
2. policy must allow it or receive a valid approval;
3. the Safety Kernel must issue a matching one-use permit;
4. the execution boundary must supply declared or acknowledged ambient resource
   authority;
5. the adapter must obey bounds; and
6. quarantined output must pass any required post-effect release decision.

## Access, policy, approval, and sandbox

These controls are deliberately independent:

- **Access profile:** selects visible tools and built-in action defaults.
- **Policy:** decides the exact request and obligations.
- **Approval mode:** can satisfy an approval obligation; it cannot reverse a deny.
- **Execution boundary and sandbox:** select declared roots, executables, environment
  names, and network origins or acknowledged ambient authority. Isolating boundaries
  enforce time, memory, output, process-count, and concurrency ceilings; direct Unix
  supervision cannot guarantee process-tree limits or cleanup for deliberately detached
  descendants.

Sparse schema-version-3 configuration starts with `allow_all`, which removes built-in
approval friction, and acknowledged `danger_full_access`, which gives authorized tools
ambient host resources. This is deliberately convenient and unsafe. `development`
adds approval obligations, `minimal` narrows the surface, and `pinned` uses exact tool
and action choices. Choose a native, Windows, OCI, or external execution boundary to
restore resource isolation. Even under full access, configured capability declarations,
policy, one-use permits, audit, quarantine, credentials, transport validation, and
configured bounds remain active. Under direct Unix execution, timeout and output bind
the supervised effect, while a hostile `setsid` or double-fork descendant may escape
process/memory accounting, outlive the effect, and act outside its audit record. Use
native, OCI, Windows Job, or a containing external host boundary when strict process
containment is required. Agent Plugin scripts and MCP servers still use ordinary tools,
explicit MCP enablement, permits, configured limits, and audit; plugin metadata never
grants ambient authority.

## Durable work

Tasks, decisions, plans, goals, subagents, memories, workflows, and research runs are
canonical event-sourced records rather than terminal-only conveniences. Colossus can
reconstruct them after restart and audit their transitions.

An effect that started but lacks a terminal event after a crash becomes
`outcome_unknown`. Colossus does not silently replay it.

## Extensions

- **Agent Plugins** are owner-scoped, immutable packages distributed as whole-plugin OCI
  artifacts. They may contain declarative Agent Skills, resources, and optional MCP servers.
- **Integrations** expose configured external operations as strict tools.
- **MCP servers** are exact configured subprocess identities with tool allowlists.
- **Plugin MCP servers** require an explicit workspace enablement and exact tool allowlist;
  skill-referenced scripts use ordinary process tools and authority.
- **Workflows** orchestrate versioned, bounded steps with durable recovery.

Continue with [Use Colossus](../use/index.md), or see the
[Glossary](../reference/glossary.md) for exact terminology.
