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

Messages remain canonical encrypted events. Context compaction creates a derived
snapshot for future provider requests; it does not erase history.

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
4. the sandbox must grant the exact resource;
5. the adapter must obey bounds; and
6. quarantined output must pass any required post-effect release decision.

## Access, policy, approval, and sandbox

These controls are deliberately independent:

- **Access profile:** selects visible tools and built-in action defaults.
- **Policy:** decides the exact request and obligations.
- **Approval mode:** can satisfy an approval obligation; it cannot reverse a deny.
- **Sandbox:** constrains concrete roots, executables, environment names, network
  origins, time, memory, output, process count, and concurrency.

`development` is the ordinary starting profile. `minimal` narrows the surface,
`allow_all` removes built-in approval friction without bypassing enforcement, and
`pinned` uses exact tool and action choices.

## Durable work

Tasks, decisions, plans, goals, subagents, memories, workflows, and research runs are
canonical event-sourced records rather than terminal-only conveniences. Colossus can
reconstruct them after restart and audit their transitions.

An effect that started but lacks a terminal event after a crash becomes
`outcome_unknown`. Colossus does not silently replay it.

## Extensions

- **Skills** are declarative instructions and bounded resources; they do not execute.
- **Integrations** expose configured external operations as strict tools.
- **MCP servers** are exact configured subprocess identities with tool allowlists.
- **Packs** are verified capability packages for executable extensions and related
  assets.
- **Collections** distribute a signed set of packs and skills.
- **Workflows** orchestrate versioned, bounded steps with durable recovery.

Continue with [Use Colossus](../use/index.md), or see the
[Glossary](../reference/glossary.md) for exact terminology.
