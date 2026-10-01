---
title: Extend Colossus
description: Build workflows, add tools and integrations, and customize the terminal.
audience: developer
type: concept
icon: lucide/blocks
---

# Extend Colossus

Choose what you want to build. Each guide starts with a concrete workflow, connection,
or file you can make your own.

## Automate work

<div class="grid cards" markdown>

-   :lucide-play:{ .lg .middle } **First workflow**

    ---

    Create, validate, register, and run a workflow with no external effects.

    [Build a first workflow :lucide-arrow-right:](workflows/first-workflow.md)

-   :lucide-workflow:{ .lg .middle } **Workflow authoring**

    ---

    Define inputs, steps, capabilities, and execution bounds.

    [Author a workflow :lucide-arrow-right:](workflows/authoring.md)

-   :lucide-refresh-cw:{ .lg .middle } **Triggers and recovery**

    ---

    Start workflows from schedules or events and handle interrupted runs.

    [Configure triggers :lucide-arrow-right:](workflows/triggers-recovery.md)

</div>

## Add tools and editors

<div class="grid cards" markdown>

-   :lucide-wrench:{ .lg .middle } **Tools**

    ---

    See the active tool catalog and what each field means.

    [Explore tools :lucide-arrow-right:](../use/tools.md)

-   :lucide-puzzle:{ .lg .middle } **Agent Plugins**

    ---

    Package and distribute Agent Skills, resources, and MCP servers.

    [Explore Agent Plugins :lucide-arrow-right:](plugins.md)

-   :lucide-plug:{ .lg .middle } **Integrations**

    ---

    Connect a supported service or import OpenAPI operations.

    [Connect an integration :lucide-arrow-right:](integrations.md)

-   :lucide-network:{ .lg .middle } **MCP**

    ---

    Configure an external tool server for Colossus to use.

    [Configure MCP :lucide-arrow-right:](mcp.md)

-   :lucide-code:{ .lg .middle } **ACP editor setup**

    ---

    Use Colossus as a local agent in a compatible editor.

    [Connect an editor :lucide-arrow-right:](acp.md)

</div>

## Customize the terminal

<div class="grid cards" markdown>

-   :lucide-paintbrush:{ .lg .middle } **Author a theme**

    ---

    Create a TOML theme, preview it, and select it in the terminal UI.

    [Create your own theme :lucide-arrow-right:](author-theme.md)

</div>

Configured extensions still use Colossus's
[access, approval, sandbox, and audit boundaries](../get-started/core-concepts.md).
