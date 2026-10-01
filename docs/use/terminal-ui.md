---
title: Terminal UI
description: Work interactively with Colossus in a terminal, including sessions, approvals, and live output.
audience: user
type: how-to
icon: lucide/terminal
---

# Terminal UI

The Terminal UI is the place to work with Colossus in a conversation. You can send
prompts, watch tools run, answer questions, and approve individual actions without
leaving the terminal.

## Start in a workspace

```bash
colossus -w /absolute/path/to/repository
```

`colossus tui` opens the same interface. A fresh session shows your workspace,
model route, approval mode, and readiness state above the composer. Type a task and
press Enter to start. For example:

> Explain how this repository handles configuration. Show me the relevant files.

The transcript appears above the composer as work progresses. By default, completed
output is available in your terminal's normal scrollback, where you can select,
copy, and search it. Use `--alt-screen` before `tui` if you prefer a full-screen
application viewport:

```bash
colossus -w /absolute/path/to/repository --alt-screen tui
```

To return to the latest conversation, start with `tui --resume`. Use
`tui --session SESSION_ID` when you know the exact session. Inside the TUI,
`/resume` opens a searchable session browser.

## Find your way around

Enter a slash command in the composer. These are useful starting points:

| Command | What it opens |
| --- | --- |
| `/help` | The command list for your running version |
| `/session show` | The current session |
| `/resume` | Searchable past sessions |
| `/context status` | Current context budget and snapshot |
| `/tools` | Tools available to the agent |
| `/work` | Current work records |
| `/permissions` | The active approval mode |
| `/theme` | Theme previews and selection |

Type `/` to complete commands. Type `@` at a skill token to choose an active plugin
skill. Use Up/Down to browse suggestions and Tab to accept one. For every command
and key, see [TUI commands and keys](../reference/tui.md).
See [Context and snapshots](sessions-context.md) to read the budget and manage
long-session history.

## Write and review a turn

Enter sends the current prompt. Shift-Enter adds a new line when your terminal
reports that key combination. If Shift-Enter sends instead, enter `/multiline on`:
Enter will then add lines and Ctrl-D will send the prompt. Use `/multiline off` to
restore the default.

You can continue typing while a run is active. Colossus queues up to eight future
turns; after a failure or cancellation, it pauses the queue for your decision.
Use Ctrl-R to search earlier prompts. To include a supported image, use `/attach PATH`,
inspect the queue with `/attachments`, and remove one with `/detach INDEX`.

When an action needs approval, a decision dock opens above the preserved draft.
Read the summary, inspect the exact request and protections if needed, then select
a decision and press Enter. There is no preselected approval: dismissing the dock,
disconnecting, or waiting until it times out denies the request. `/permissions`
shows the current mode; the [access and approvals guide](../admin/access-and-approvals.md)
explains the modes and their limits.

For a multi-step change, `/plan new` starts a plan you can review before execution.
The [Planning guide](planning.md) covers approving it and choosing Direct or Goal
Mode. For source-backed questions, see [Research](research.md).

## Leave and come back

Press Ctrl-C while idle, or enter `/exit`, to leave the TUI. During an active run,
the first Ctrl-C requests cancellation; a second exits. Your session remains
available. Reopen the latest one with:

```bash
colossus -w /absolute/path/to/repository tui --resume
```

If you cannot find earlier work, check that you are using the same workspace and
configuration. [Sessions](sessions.md) shows how to list and resume a specific ID.

## What's next?

<div class="grid cards" markdown>

-   :lucide-messages-square:{ .lg .middle } **Sessions**

    ---

    Browse conversations and continue one from the CLI or terminal UI.

    [Resume a session :lucide-arrow-right:](sessions.md)

-   :lucide-list-checks:{ .lg .middle } **Planning**

    ---

    Review a durable plan, then run it once or through a bounded goal loop.

    [Plan a task :lucide-arrow-right:](planning.md)

-   :lucide-keyboard:{ .lg .middle } **Commands and keys**

    ---

    Look up shortcuts, slash commands, and their arguments.

    [Open the TUI reference :lucide-arrow-right:](../reference/tui.md)

</div>
