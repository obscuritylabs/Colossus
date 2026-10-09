---
title: Agent runs
description: Run a single Colossus task from the CLI and choose how to receive its result.
audience: user
type: how-to
icon: lucide/play
---

# Agent runs

Use `colossus run` when you have a prompt to complete from the shell. Each invocation
returns a final response and saves the conversation as a session, so you can return to
it later.

## Run a task

From an initialized repository, run:

```bash
colossus -w /absolute/path/to/repository run \
  "Summarize this repository and identify its main components"
```

In a terminal, the response is readable text. When stdout is redirected or piped,
Colossus uses JSON by default. Set `--output human` or `--output json` before `run`
when you need a particular format.

Give the agent a concrete outcome and any limits that matter. For a bounded change:

```bash
colossus -w /absolute/path/to/repository run --max-turns 12 \
  "Fix the failing parser test, then run the focused test"
```

`--max-turns` limits the model's turns. It does not grant tools or override the
configured [access and approval rules](../admin/access-and-approvals.md). If a run
needs interactive approval, use the [Terminal UI](terminal-ui.md). Noninteractive
runs deny outstanding approval requests by default.

## Watch progress or capture JSON

Add `--stream` to see released progress while the run is active:

```bash
colossus -w /absolute/path/to/repository run --stream \
  "Inspect the active tool surface"
```

Progress goes to stderr; the final result stays on stdout. You can save a clean JSON
result while watching progress in the terminal:

```bash
colossus -w /absolute/path/to/repository --output json \
  run --stream "Report repository status" > result.json
```

## Include files with the prompt

Use `--attach` for a supported workspace text file or image:

```bash
colossus -w /absolute/path/to/repository run \
  --attach design.md --attach src/lib.rs \
  "Review these files for inconsistent assumptions"
```

Attached files still follow the workspace's filesystem policy. Image input also
requires an image-capable model profile. See the [CLI reference](../reference/cli.md)
for accepted formats and limits.

## Continue a run later

Use `--resume` for the most recently updated session in this workspace, or
`--session SESSION_ID` to choose one exactly:

```bash
colossus -w /absolute/path/to/repository run --resume \
  "Continue with the next step"
```

See [Sessions](sessions.md) to list sessions, inspect messages, and resume the same
conversation in the terminal UI.

## What's next?

<div class="grid cards" markdown>

-   :lucide-terminal:{ .lg .middle } **Work interactively**

    ---

    Follow live output, answer questions, and review approvals in the terminal.

    [Open the Terminal UI guide :lucide-arrow-right:](terminal-ui.md)

-   :lucide-messages-square:{ .lg .middle } **Return to a session**

    ---

    Find a conversation and continue it from the CLI or terminal UI.

    [Explore sessions :lucide-arrow-right:](sessions.md)

-   :lucide-code:{ .lg .middle } **Work in an editor**

    ---

    Connect an ACP-compatible editor to the Colossus CLI.

    [Set up ACP :lucide-arrow-right:](../extend/acp.md)

</div>
