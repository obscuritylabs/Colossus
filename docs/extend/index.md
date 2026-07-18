---
title: Automate and extend
description: Build durable workflows and add trusted skills, integrations, MCP servers, packs, and collections.
audience: developer
type: concept
---

# Automate and extend

Colossus extensions enter through explicit, inspectable boundaries. Choose the smallest
mechanism that fits the capability.

| Need | Mechanism |
| --- | --- |
| Orchestrate bounded durable steps | [Workflow](workflows/first-workflow.md) |
| Add agent instructions and text resources | [Skill](skills.md) |
| Connect a supported service or OpenAPI operation | [Integration](integrations.md) |
| Run a configured external tool server | [MCP](mcp.md) |
| Distribute executable capabilities and related assets | [Pack](packs.md) |
| Distribute a signed set of packs and skills | [Collection](collections-registry.md) |

## One trust model

Extensions do not create a parallel execution path. Their tools are candidates only
after configuration, connection, verification, or trust makes them applicable. Calls
still pass through access selection, policy, approval, one-use permits, sandbox
enforcement, output bounds, quarantine, release, and audit.

Skills are the exception only in the sense that they never execute: they contribute
instructions and bounded resources. Put executable behavior in a verified pack,
configured MCP server, or integration.

Start with [Your first workflow](workflows/first-workflow.md).
