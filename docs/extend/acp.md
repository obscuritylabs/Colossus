---
title: ACP in editors
description: Connect an ACP-compatible editor to Colossus and work through its local agent.
audience: user
type: how-to
icon: lucide/code
---

# ACP in editors

Agent Client Protocol (ACP) lets a compatible editor use Colossus as its local
agent. You work in the editor's chat interface; Colossus supplies the model route,
tools, approvals, execution boundary, and durable session record.

## Connect your editor

[Install the CLI](../get-started/install.md) and [connect a model](../get-started/connect-model.md)
first. The `primary` model role is the route used for ACP prompts.

In your editor's ACP agent settings, select the installed `colossus` executable
and pass the workspace and `acp` command. For editors that accept a command and
arguments array, the configuration has this shape:

```json
{
  "command": "colossus",
  "args": ["--workspace", "/absolute/path/to/repository", "acp"]
}
```

Use the executable's absolute path for `command` if the editor cannot find
`colossus` on its PATH. The workspace must be the same repository the editor
opens. If the configuration is elsewhere, add `--config` and its absolute path
before `acp` in the arguments array. Your editor's setting names may differ from
the generic `command` and `args` example.

Start a new agent conversation in the editor. Colossus creates a durable session
and streams released responses and tool status back into the editor. It writes
diagnostics to stderr, leaving the ACP protocol on stdin and stdout.

## Approve actions in the editor

When a tool needs approval, the editor can offer **Allow once** or **Reject**.
Allow once applies to that exact request; it cannot override a Colossus policy
denial or widen the configured sandbox. ACP uses this fail-closed approval flow,
so `--approval-mode` is unavailable with `acp`. Configure tools, MCP servers,
access, and sandbox settings in Colossus, as you would for a terminal run.
See [Access and approvals](../admin/access-and-approvals.md) for the underlying
rules.

## Current ACP support

| Area | Supported behavior |
| --- | --- |
| Protocol | Stable ACP v1 over local stdin/stdout. |
| Prompts | Text and resource links. Resource links are references; Colossus does not fetch them on the editor's authority. |
| Sessions | New durable sessions and cooperative cancellation. An editor cannot reload an earlier ACP session after restart. |
| Workspace | One selected workspace. Additional editor-provided directories and MCP servers are unsupported. |
| Concurrency | One active prompt at a time for the workspace. |

Images, audio, and embedded resources are not accepted in ACP prompts yet. You
can still inspect existing Colossus sessions with the [CLI session commands](../use/sessions.md).
If the editor cannot start Colossus because another Colossus process owns the
workspace, close that process before starting the ACP agent. Colossus Desktop
does not currently act as an ACP client.

## What's next?

<div class="grid cards" markdown>

-   :lucide-wrench:{ .lg .middle } **Tools**

    ---

    See the tool catalog Colossus can make available to the agent.

    [Explore tools :lucide-arrow-right:](../use/tools.md)

-   :lucide-messages-square:{ .lg .middle } **Sessions**

    ---

    Find the durable conversation created by an editor run.

    [Inspect sessions :lucide-arrow-right:](../use/sessions.md)

</div>
