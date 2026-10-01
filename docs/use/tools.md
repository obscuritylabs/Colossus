---
title: Tools
description: See which tools Colossus can offer and what their output means.
audience: user
type: how-to
icon: lucide/wrench
---

# Tools

Tools let Colossus inspect files, search a repository, run commands, keep durable
work, and use configured services. The active list depends on your workspace,
configuration, and available integrations.

## See the active tools

Enter `/tools` in the terminal UI:

```text
/tools
```

The result lists model-visible tools. This shortened example wraps the fields onto
separate lines for readability:

```text
◆ Tools
  • echo
    Description: Return the supplied text without performing an external effect.
    Max output bytes: 32768
  • filesystem.read
    Description: Read one policy-permitted UTF-8 text file.
    Effect action: filesystem.read · Capability: filesystem.read
    Max output bytes: 1048576
  • filesystem.write
    Description: Create, overwrite, or append bounded UTF-8 workspace text.
    Effect action: filesystem.write · Capability: filesystem.write
    Max output bytes: 1048576
```

The **name** is what the model calls. **Description** explains its task. **Effect
action** names the operation evaluated by policy; a pure tool such as `echo` has no
effect action. **Capability** identifies the resource authority an effect requires.
**Max output bytes** is a result-size ceiling.

A listed tool is available to the model, but a call may still need approval or be
denied for a specific file, command, or destination. See
[Access and approvals](../admin/access-and-approvals.md) for those decisions.

## Find the right family

| To do this | Tools to look for |
| --- | --- |
| Explore code | `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary` |
| Read and search files | `filesystem.list`, `filesystem.read`, `filesystem.search` |
| Check Git state | `git.status`, `git.diff`, `git.show` |
| Edit and verify | `filesystem.write`, `filesystem.replace`, `patch.preview`, `patch.apply` |
| Run a command | `shell.run` |
| Track work | `task.*`, `decision.*`, `plan.*`, `goal.*`, `agent.*` |
| Remember context | `memory.create`, `memory.search`, `memory.supersede` |
| Use external sources | `web.*`, `docs.fetch`, `network.http`, `mcp.*` |

For exact built-in names, action mappings, and effect boundaries, use
[Tools and action classes](../reference/tools-actions.md). Configured integrations
and MCP servers can add tools to the active list.
See [MCP servers](mcp-servers.md) to inspect those external sources in the TUI.

## Inspect from the CLI

```bash
colossus tools list
colossus config effective
```

`tools list` prints the active tool catalog and schemas for scripting or closer
inspection. `config effective` shows selected and hidden candidates with their
resolution details. If a tool you expected is absent, check the access profile and
its prerequisites there. [Plan Mode](planning.md) can further narrow the tools offered
for a planning turn.
