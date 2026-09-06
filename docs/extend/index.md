---
title: Automate and extend
description: Build durable workflows and add Agent Plugins, integrations, and standalone MCP servers.
audience: developer
type: concept
---

# Automate and extend

Colossus extensions enter through explicit, inspectable boundaries. Choose the smallest
mechanism that fits the capability.

| Need | Mechanism |
| --- | --- |
| Orchestrate bounded durable steps | [Workflow](workflows/first-workflow.md) |
| Distribute Agent Skills, resources, and MCP servers | [Agent Plugin](plugins.md) |
| Connect a supported service or OpenAPI operation | [Integration](integrations.md) |
| Run a configured external tool server | [MCP](mcp.md) |

## One trust model

Extensions do not create a parallel execution path. Their tools are candidates only
after configuration, connection, verification, or trust makes them applicable. Calls
still pass through access selection, policy, approval, one-use permits, sandbox
enforcement, output bounds, quarantine, release, and audit.

Agent Skills contribute instructions and bounded resources from an immutable plugin.
Referenced scripts and plugin MCP servers still cross the ordinary tool, policy, approval,
permit, sandbox, quarantine, and audit boundaries.

Start with [Your first workflow](workflows/first-workflow.md).
