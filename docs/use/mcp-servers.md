---
title: MCP servers
description: Inspect configured MCP servers and discover the tools they offer to Colossus.
audience: user
type: how-to
icon: lucide/network
---

# MCP servers

MCP connects Colossus to tools provided by external servers. Those servers are
configured separately from built-in tools and Agent Skills. Start in the terminal
with the server list:

```text
/mcp servers
```

For example, a workspace with four configured remote servers may show:

```text
◆ MCP servers
  • confluence
    Available: yes · Transport: streamable_http · Allow stateless: yes
  • gitlab
    Available: yes · Transport: streamable_http · Allow stateless: no
  • jira
    Available: yes · Transport: streamable_http · Allow stateless: yes
  • splunk
    Available: yes · Transport: streamable_http · Allow stateless: yes
```

This is a **configuration inventory**, not a list of default Colossus servers.
**Available** tells you whether Colossus can consider the configured server;
remote health is checked when tools are discovered or called. **Transport** shows
how Colossus connects. **Allow stateless** tells you whether that remote declaration
permits a server to omit `Mcp-Session-Id`; it is not a health indicator.

## Discover a server's tools

Ask for the live tool list from one server:

```text
/mcp tools jira
```

Colossus connects to `jira` and shows the tools selected by that server's
allowlist, with their schemas. `/mcp tools` discovers across configured servers.
If a server is listed but a tool is missing, the server may not offer it, or the
operator may not have selected it in `allowedTools`.

Once configured, Colossus can search MCP tool names and descriptions when your
request needs an external source. For example, ask it to find a Jira issue or
search Confluence. An MCP tool call still goes through normal schema validation,
policy, approval, and audit. Seeing a server or tool in a list does not guarantee
that a specific call will be authorized.

For an exact manual call, `/mcp call SERVER TOOL JSON` accepts the discovered tool
name and a JSON argument object. Inspect `/mcp tools SERVER` first to learn the
required arguments. The equivalent shell commands are `colossus mcp servers`,
`colossus mcp tools --server SERVER`, and `colossus mcp call SERVER TOOL JSON`.

## What's next?

<div class="grid cards" markdown>

-   :lucide-settings:{ .lg .middle } **Configure MCP**

    ---

    Add a local or remote server, choose its tools, and set up credentials or OAuth.

    [Configure a server :lucide-arrow-right:](../extend/mcp.md)

-   :lucide-wrench:{ .lg .middle } **Tools**

    ---

    See the active tool catalog and understand its effect and capability fields.

    [Explore tools :lucide-arrow-right:](tools.md)

</div>
