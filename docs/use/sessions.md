---
title: Sessions
description: Start, find, and resume durable conversations in the terminal UI or CLI.
audience: user
type: how-to
icon: lucide/messages-square
---

# Sessions

A session keeps a conversation and its runs together. You can leave the terminal and
return to the same work later. Browse sessions inside the terminal UI, or use CLI
commands when you need an exact ID or machine-readable history.

For a new conversation, the agent sets a short title based on the opening request
when `session.set_title` is available. It keeps that title as the conversation
continues, even if the work changes direction. Ask the agent to rename the session
if you want a different title. The TUI Sessions browser and Desktop use the saved
title; an unavailable or disabled title tool leaves the opening-request fallback.

## Work with several sessions in the terminal UI

Press F2 or enter `/sessions` to open the **Sessions** browser on **Open** for the
selected workspace. `/resume` opens **History** in the same browser. Tab and
Shift-Tab move between Open, History, and Archived; `/` searches the current list.

The left pane shows session titles, status, update times, and message counts.
Highlight a row with Up/Down or a mouse click to preview recent messages and work
on the right. PageUp/PageDown scrolls the preview. Enter opens the selected
unarchived conversation. Press `n` to create another session, `r` to refresh, or
Esc to return. On compact terminals, the browser keeps the list visible.

On wider terminals, workspace sessions stay pinned at the bottom of the right
sidebar. In the default full-screen view, click a row once to switch conversations.
If F6 terminal-native text selection is enabled, press F6 or Esc to return first.
The sidebar grows from 28 to 36 columns when there is room while preserving at
least 64 columns for the conversation. Alt-N starts a new session, and
Alt-Left/Alt-Right switches between sessions without opening the command center.

Sessions can work concurrently. Switching keeps each open session's draft,
future-turn queue, Plan Mode selection, and model settings. Output stays with its
conversation, and a decision indicator calls attention to a session needing input.
In the Sessions browser, `s` stops the highlighted running or waiting session while the others
continue.

The terminal retains up to eight open sessions and lists up to 100 saved sessions
in its catalogue. Working and decision indicators describe retained sessions in
this terminal. Other saved conversations show **ready**; that label does not report
whether they are running in another terminal or Desktop.

## Archive and restore a conversation

Open Sessions, highlight an idle conversation, and press `a` to archive it. Stop
running or waiting work first, and resolve unsent drafts or queued turns before
archiving an open session. Archiving the current session switches to another open
conversation or starts an empty one when needed.

Archived sessions leave Open and History but retain their messages, plans, tasks,
and other durable work. The archive state survives restarting Colossus. In
**Archived**, select a conversation to preview it and press `u` to restore it to
History; Enter can then open it. Restoration does not restart previous work.

Archive and restore apply to the selected runtime's saved state. Choose the same
workspace and configuration to find these conversations later.

## Resume in the terminal UI

If you are already in Colossus, enter `/resume`. From your configured workspace, you
can also reopen the most recently updated session directly:

```bash
colossus tui --resume
```

To switch to a known session without browsing, enter `/resume SESSION_ID`.
Navigation takes effect immediately even while the current session is running;
that session continues in the background. `/session show` displays the active
session.

## Start a clean conversation

Enter `/session new` in the TUI. Starting `colossus tui` without `--resume` or
`--session` also opens a fresh session. To give an empty session a title before
starting work, use the CLI:

```bash
colossus sessions new "Release review"
```

Copy its `id` from the output and attach with `colossus tui --session SESSION_ID`.

Closing the TUI requests cancellation of its attached runs. Durable conversations
remain available to resume later; exiting does not leave these runs automatically
working in the background. Drafts, queued turns, and process-local Plan and model
selections are retained only while that TUI process stays open.

## Find or continue a session from the CLI

Run these commands from the same initialized workspace and configuration you used
for the original conversation:

```bash
colossus sessions list
colossus sessions show SESSION_ID
colossus run --session SESSION_ID "Continue the review"
```

`sessions list` shows the 20 most recently updated sessions by default. `sessions show`
includes the title, timestamps, message count, and last run ID. Use
`colossus run --resume "Continue the review"` for the most recently updated session
when its exact ID does not matter. A plain `colossus run "..."` starts a fresh one.

To read the retained transcript, run:

```bash
colossus sessions messages SESSION_ID
```

Messages remain in the canonical journal even if Colossus later compacts the model's
working context. See [Context and snapshots](sessions-context.md) for long sessions
and [TUI commands and keys](../reference/tui.md) for the full interactive command list.

## If you can't find a session

- **Session not found:** copy the complete ID from `sessions list` and check that you
  selected the same workspace, configuration, and state as before.
- **The wrong conversation resumed:** `--resume` selects the most recently updated
  session. Use `--session SESSION_ID` for a specific one.
- **No session to resume:** start a fresh run or use `/session new` in the TUI.
