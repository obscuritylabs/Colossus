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

A fresh launch opens a centered entry screen: the Obscurity Labs crest spins,
then settles above the assembling Colossus wordmark. The workspace appears after
about 1.5 seconds. Typing or clicking skips the entry screen; typed input is kept
in the composer. Background workspace initialization continues during the reveal.
Startup warnings and decisions take priority and open the normal interface immediately.
Resumed sessions, inline mode, the Mono theme, and `NO_COLOR` skip the animation.
Set `COLOSSUS_TUI_ANIMATIONS=off` before launching to disable it. Terminals smaller
than 40 columns or 24 rows also use the static layout.

The transcript appears above the composer as work progresses. The TUI fills the
alternate screen by default; use the mouse wheel or PageUp/PageDown to read older
output and End to return to live output. Exiting restores your prior terminal view.

The top bar shows the workspace, session, and installed version when there is
enough vertical space. The composer has a neutral background that separates your
draft from the conversation. A single status row below it highlights the mode and
security warnings, alongside readiness, approval handling, the model route, and
context budget. Smaller widths shorten or omit runtime details to keep status
and warnings readable. Compact terminals omit the top bar.

On wider terminals, a right sidebar shows subagents and workspace status,
including open tasks, goals, plans, retained memory, and configured MCP servers.
It scrolls independently from the conversation and hides when the terminal is narrow.
MCP indicators distinguish configured availability from successful use in this session.
Workspace sessions stay at the bottom of the sidebar; click a row once to switch.
The sidebar grows to 36 columns when there is room and preserves at least 64
columns for the conversation. F6 terminal-native text selection releases mouse
capture; press F6 or Esc to return before clicking a session.

Use `--no-alt-screen` for an inline interface that leaves completed output in your
terminal's normal scrollback:

```bash
colossus -w /absolute/path/to/repository --no-alt-screen tui
```

To return to the latest conversation, start with `tui --resume`. Use
`tui --session SESSION_ID` when you know the exact session. Inside the TUI,
F2 or `/sessions` opens the Sessions browser on Open; `/resume` opens History in
the same browser. Its right pane previews recent messages and work. Switch
sessions there while other conversations continue running, or use Archived to
restore a conversation you previously archived.

## Find your way around

Enter a slash command in the composer. These are useful starting points:

| Command | What it opens |
| --- | --- |
| `/help` | The command list for your running version |
| `/session show` | The current session |
| `/resume` | Searchable past sessions |
| `/model` | Choose a configured model for the next turn |
| `/effort` | Choose the next turn's reasoning effort |
| `/context status` | Current context budget and snapshot |
| `/tools` | Tools available to the agent |
| `/work` | Current work records |
| `/permissions` | Choose approval handling |
| `/theme` | Theme previews and selection |

Type `/` to complete commands. Type `@` at a skill token to choose an active plugin
skill. Use Up/Down to browse suggestions and Tab to accept one. For every command
and key, see [TUI commands and keys](../reference/tui.md).
See [Context and snapshots](sessions-context.md) to read the budget and manage
long-session history.

## Read tool activity

The default `/events compact` view shows one human-readable row per completed
call, with its target, available counts, and outcome. The same shared summaries
appear in Desktop and VS Code. Large file contents and tables stay out of the
transcript until requested.

Press F3 to toggle between compact summaries and expanded results in the same
conversation. Expanded results show status, timing, readable fields, tables, and
short code snippets. The composer and sidebar stay visible, and your draft and
reading position are preserved. `/events compact` and `/events verbose` select
these same modes. Use `/prefs save` to save the current preference.

Click a tool row in the full-screen view for a longer, scrollable detail preview.
Use Left/Right to move between events and PageUp/PageDown to scroll. Esc closes
the inspector; F3 closes it and toggles the conversation mode. Dragging still
selects text.

Details are bounded (up to 64 KiB and 160 logical lines; tables show up to 20 rows
and 8 columns). Inline code snippets show up to 20 opening and 8 closing lines.
A truncation notice identifies omitted content; the retained result is unchanged.
With `--no-alt-screen`, F3 changes live and future output; previously committed
native scrollback keeps its original formatting. `/events off` suppresses
successful live tool rows while keeping failures visible. Restored results say **recorded** because saved model messages
do not include an authoritative exit status.

## Read token usage

The footer shows provider-reported tokens for the current run and keeps the latest
run's counts when it ends. The **Run usage** summary shows input, output, and total
tokens, plus cached input and reasoning output when reported. Cached input is
already included in input; reasoning is already included in output. Colossus keeps
the provider's total rather than recalculating it. Missing counts stay unknown
instead of becoming zero.

Counts cover one run, not a session total or a price estimate. Goal counts refer
to the latest iteration. Reported counts remain available after failure or
cancellation when the run can be identified; reopening a session restores the
latest recorded counts when available. The separate `ctx` value estimates the
prepared context against the input budget. Human CLI run output also includes a
summary; [JSON output](../reference/cli.md#json-output-contracts) keeps its documented
format.

## Write and review a turn

Use `/model` and `/effort` to change the next turn without leaving the current
conversation. Up/Down selects a choice, Enter applies it, and Esc keeps the current
selection. You can also enter `/model PROFILE` or `/effort high` directly.
`/model default` restores the configured role, and `/effort default` restores the
selected model's saved reasoning setting. Changing models resets the reasoning
override. Choices stay with this session for the current TUI process. Research
keeps its independently configured model roles; `/reasoning` controls the display
of safe reasoning summaries.

Enter sends the current prompt. Shift-Enter adds a new line when your terminal
reports that key combination. Ctrl-J also adds a new line, including in terminals
that send Shift-Enter as plain Enter. Use `/multiline` to choose Enter behavior:
Enter can add lines while Ctrl-D sends the prompt. Direct shortcuts such as
`/multiline on` and `/multiline off` remain available.

Start a draft with `!` to run a shell command directly, such as `!git status`.
It uses the existing approval, sandbox, cancellation, and audit path without asking
the model. Each command starts in the workspace; directory and environment changes
last only for that invocation.

Drag across rendered text to select it, then press Ctrl-C or Ctrl-Insert to copy.
The selected view stays still while work continues. Esc clears the selection and
shows new output. See [TUI commands and keys](../reference/tui.md) for terminal-native
selection and independent sidebar scrolling.

`/permissions` opens an approval-mode chooser with the current mode highlighted.
Up/Down moves the selection, Enter applies it, and Esc returns without changing
anything. This changes approval handling for subsequent operations in this TUI
process; policy, tool authority, and sandbox boundaries still apply. `/stream`,
`/events`, `/reasoning`, `/transcript`, and `/provider diagnostics` use the same
chooser. Slash completion shows each setting once; type the setting followed by a
space to see direct argument shortcuts. `/theme list` and other theme tools remain
available alongside the theme picker.

You can continue typing while a run is active. Colossus queues up to eight future
turns; after a failure or cancellation, it pauses the queue for your decision.

After a known provider failure, an inline **Retry** and **Dismiss** notice appears
above the composer. Tab selects an action and Enter activates it; ordinary slash
completion keeps priority. Retry sends **Continue** in the same session and keeps
your draft. You can also type your own continuation with additional guidance.
Completed tools keep their recorded results. A retry does not repeat their effects.
Uncertain tool outcomes require reconciliation and do not offer this action.

Use Ctrl-R to open full-screen prompt history. Type to filter, use Up/Down to
choose, and press Enter to place a prompt in your draft. Esc keeps the draft you
had before opening history. To include a supported image, use `/attach PATH`,
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

### Restore in Herdr

When an interactive TUI starts inside a [Herdr](https://herdr.dev/docs/add-herdr-support/) pane,
Colossus reports its current session and whether it is idle, working, or waiting for a
decision. Herdr 0.9.2 or later can reopen that session after a server restart with the
same workspace, configuration, approval mode, and screen mode. Switching sessions
updates the restore command, and exiting Colossus releases the pane. Check the
integration with `herdr pane get "$HERDR_PANE_ID"` or `herdr agent list`.

If the TUI is connected to an active worker, `--approval-mode` sets the mode for that
TUI client without changing the worker default for other clients.

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
