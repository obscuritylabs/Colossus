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
if you want a different title. The TUI session browser and Desktop use the saved
title; an unavailable or disabled title tool leaves the opening-request fallback.

## List sessions in the terminal UI

Enter `/sessions` to see your 20 most recently updated sessions. The list shows each
session's ID, message count, and creation and update times. This example wraps the
metadata onto separate lines for readability:

```text
◆ Sessions
  • Item 1
    Id: 0195b640-8a41-7b35-948d-3d58b39e783a · Message count: 0
    Updated at: 2026-09-30T14:24:05Z
    Created at: 2026-09-30T14:24:05Z
  • Item 2
    Id: 0195b63e-bb32-76d1-a9e0-93b5f70f241c · Message count: 18
    Updated at: 2026-09-30T13:11:59Z
    Created at: 2026-09-30T13:10:38Z
```

Copy an `Id` to resume a specific conversation with `/resume SESSION_ID`. A message
count of `0` means the session has no messages yet. To browse by title and preview
messages before choosing, use `/resume` without an ID.

## Resume in the terminal UI

If you are already in Colossus, enter `/resume`. From your configured workspace, you
can also reopen the most recently updated session directly:

```bash
colossus tui --resume
```

The `/resume` browser fills the screen with searchable sessions on the left and a
view of the selected conversation on the right. It marks the session you are in as `CURRENT`.
This abbreviated example shows the labels and controls; your titles, messages, and
timestamps will differ:

```text
Resume session · 2 sessions       │ View
                                 │
/ Search sessions                │ Release review
                                 │ ID: 0195b640 · 8 messages
  Session           Updated Msgs │ Updated: 2 min ago
› Release review    2m ago      8 │ Recent conversation
  CURRENT Repo task 10m ago     4 │ USER       Review the release notes
                                 │ ASSISTANT  Here is the summary...

↑/↓ Select   / Search   PgUp/Dn View   Enter Resume   Esc Cancel
```

Press `/` to search, Up/Down to choose, PageUp/PageDown to scroll the preview, and
Enter to resume. To switch to a known session without browsing, enter
`/resume SESSION_ID`. `/session show` displays the active session.

## Start a clean conversation

Enter `/session new` in the TUI. Starting `colossus tui` without `--resume` or
`--session` also opens a fresh session. To give an empty session a title before
starting work, use the CLI:

```bash
colossus sessions new "Release review"
```

Copy its `id` from the output and attach with `colossus tui --session SESSION_ID`.

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
