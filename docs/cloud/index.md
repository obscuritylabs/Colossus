---
title: Cloud
description: Work in the browser, connect your runtimes, or deploy your own Colossus Control Plane.
audience: user
type: concept
icon: lucide/cloud
---

# Colossus Cloud

Use Colossus in a browser to work with your team's agents, continue conversations,
and review tasks. Your organization runs the Control Plane and chooses whether to
enable hosted chat, connect existing runtimes, or offer both.

<div class="grid cards" markdown>

-   :lucide-messages-square:{ .lg .middle } **Start working**

    ---

    Sign in, choose a project, and start or resume a conversation.

    [Work in Cloud :lucide-arrow-right:](work.md)

-   :lucide-plug:{ .lg .middle } **Connect a runtime**

    ---

    Enroll a Desktop workspace or CLI daemon in your team's Fleet.

    [Connect your agent :lucide-arrow-right:](connect-runtimes.md)

-   :lucide-users:{ .lg .middle } **Manage your team**

    ---

    Set up projects, grant access, and review reported runtime policy.

    [Manage projects and access :lucide-arrow-right:](administration.md)

-   :lucide-server:{ .lg .middle } **Deploy Cloud**

    ---

    Install the Helm chart, choose sign-in and database settings, and operate it.

    [Open deployment guides :lucide-arrow-right:](deployment/index.md)

</div>

## Choose how work runs

| Choose | When you need |
| --- | --- |
| **Chat / Hosted** | Browser conversations with your personal hosted runtime. An operator must enable this service. |
| **Fleet** | An enrolled Desktop or CLI runtime with its own workspace, tools, credentials, and local permissions. |

Hosted chat supports configured providers and approved remote MCP connections.
Use an enrolled runtime for filesystem, shell, native browser/computer, and plugin
execution. Cloud does not provision user development environments.

Project access and runtime permissions both apply. A shared link never grants access,
and administration does not expose another user's personal hosted conversations.
