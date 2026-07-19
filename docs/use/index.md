---
title: Use Colossus
description: Choose the right Colossus surface for interactive runs, durable work, memory, and research.
audience: user
type: concept
---

# Use Colossus

Colossus supports quick one-shot requests and long-running, restart-safe work. Both use
the same model routing, tools, authorization, encrypted journal, and recovery semantics.

## Choose a working style

| Need | Start here |
| --- | --- |
| Run one prompt or produce JSON for automation | [Agent runs](agent-runs.md) |
| Work interactively with approvals and live output | [Terminal UI](terminal-ui.md) |
| Resume a conversation or manage model context | [Sessions and context](sessions-context.md) |
| Capture commitments and approve execution | [Tasks, decisions, and plans](tasks-decisions-plans.md) |
| Iterate on an objective or delegate bounded work | [Goals and subagents](goals-subagents.md) |
| Preserve reusable non-secret context | [Memories](memories.md) |
| Produce a durable report from repository, web, or MCP evidence | [Deep research](deep-research.md) |
| Query an operator-configured provider-neutral search route | [Web search](web-search.md) |

## What stays durable

Session messages, tasks, decisions, plans, goals, child jobs, memories, research runs,
workflow runs, and audit evidence are canonical encrypted records. Terminal layout,
search indexes, and projections can be rebuilt; they are not the source of truth.

When an external effect starts but its terminal outcome is not recorded, Colossus marks
it unknown instead of assuming success or retrying silently.
