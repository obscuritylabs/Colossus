---
title: ACP editor setup
description: Launch Colossus as a local ACP v1 agent from an editor.
audience: developer
type: how-to
icon: lucide/code
---

# ACP editor setup

Colossus can serve the stable Agent Client Protocol (ACP) v1 as a local agent.
[Install the CLI](../get-started/install.md), then give your editor the installed
`colossus` executable and these arguments, replacing the
path with the repository the editor will open:

```json
{
  "command": "colossus",
  "args": ["--workspace", "/absolute/path/to/repository", "acp"]
}
```

Use `--config /absolute/path/to/config.yaml` before `acp` when the workspace does
not use the normal Colossus configuration search. Configure the `primary` model role
and the normal access, policy, sandbox, and audit settings before launching the
editor. The editor must send the same canonical workspace path as `cwd` in
`session/new`. Colossus rejects additional workspace directories and editor supplied
MCP servers. Configure MCP servers in Colossus's own reviewed configuration instead.

The ACP connection uses JSON-RPC on stdin/stdout. Colossus writes diagnostics to
stderr. It negotiates v1 and advertises only its implemented capabilities. New ACP
sessions receive durable Colossus session IDs; prompts use the existing bounded run
engine and stream released assistant text and tool status updates. `session/cancel`
requests a cooperative stop. Editor permission choices support **allow once** and
**reject**; approval is bound to the exact Colossus policy request and cannot turn a
policy denial into an allowed effect. Do not pass `--approval-mode` to `acp`.
Each connection can create up to 64 sessions and run one prompt at a time for the
selected workspace.

This first ACP interface accepts text and resource links in prompts. Resource links
are passed as references in the prompt; Colossus does not fetch them on the editor's
authority. Images, audio, embedded resources, extra workspace roots, and editor
supplied MCP servers are rejected. ACP `session/load` is not advertised: after an
editor restart, create a new ACP session. Existing Colossus sessions remain durable
and can be inspected with the ordinary CLI. The interface uses the workspace's
single writer lease, so stop another worker or Colossus process holding that lease
before launching this agent for the same workspace.

ACP draft v2 and Colossus Desktop acting as an ACP client are separate work.
