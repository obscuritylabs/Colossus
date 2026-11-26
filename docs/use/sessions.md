---
title: Sessions
description: Start, find, resume, and export durable conversations.
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
While an existing journal builds its session index after an upgrade, unnamed
History rows can show **Untitled session** and preview text may be unavailable
in search. The transcript preview is available when you highlight a row.
Highlight a row with Up/Down or a mouse click to preview recent messages and work
on the right. PageUp/PageDown scrolls the preview. Enter opens the selected
unarchived conversation. Press `n` to open a new draft, `r` to refresh, or
Esc to return. On compact terminals, the browser keeps the list visible.

On wider terminals, workspace sessions stay pinned at the bottom of the right
sidebar. In the default full-screen view, click a row once to switch conversations.
If F6 terminal-native text selection is enabled, press F6 or Esc to return first.
The sidebar grows from 28 to 36 columns when there is room while preserving at
least 64 columns for the conversation. Alt-N opens a new draft, and
Alt-Left/Alt-Right switches between sessions without opening the command center.

Sessions can work concurrently. Switching keeps each open session's draft,
future-turn queue, Plan Mode selection, and model settings. Output stays with its
conversation, and a decision indicator calls attention to a session needing input.
In the Sessions browser, `s` stops the highlighted running or waiting session while the others
continue.

### Follow another open session

In the full-screen TUI, press F2, highlight another session on **Open**, and press
`p` to pin its read-only transcript. The right sidebar shows the pinned session's
identity, status, and most recent output as it arrives. The active conversation
keeps its draft, reading position, and controls. To respond to a decision in the
pinned session, open that session with Enter in the Sessions browser.

To unpin, highlight the pinned session on **Open** and press `p` again. Opening
or archiving the pinned session also removes the pane. Compact terminals hide the
pane and restore it when space returns. The pin lasts for the current terminal
process and is available in both embedded and worker-backed full-screen TUI
sessions.

The terminal retains up to eight open sessions and lists up to 100 saved sessions
in its catalogue. Working and decision indicators describe retained sessions in
this terminal. Other saved conversations show **ready**; that label does not report
whether they are running in another terminal or Desktop.

## Archive and restore a conversation

Open Sessions, highlight a saved, idle conversation, and press `a` to archive it. Stop
running or waiting work first, and resolve unsent drafts or queued turns before
archiving an open session. Archiving the current session switches to another open
conversation or opens a new draft when needed. You can also archive old empty entries.

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

Enter `/session new` in the TUI or start `colossus tui` without `--resume` or
`--session` to open an unsaved draft. Starting a turn, running a shell command,
or starting a goal or research run saves the session. Browsing sessions, changing
terminal preferences, and composing without submitting keep the draft out of
**History**. Use **Close draft** in Command Center to free an open slot, or
exit the TUI to discard unsaved conversations.

To save and title an empty session before starting work, use the CLI:

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

To inspect saved message records, run:

```bash
colossus sessions messages SESSION_ID
```

Messages remain in the canonical journal even if Colossus later compacts the model's
working context. See [Context and snapshots](sessions-context.md) for long sessions
and [TUI commands and keys](../reference/tui.md) for the full interactive command list.

## Export a conversation

Choose a session with `colossus sessions list`, then export it to a new file:

```bash
colossus sessions export SESSION_ID --output conversation.md
colossus sessions export SESSION_ID --output conversation.txt --format text
colossus sessions export SESSION_ID --output conversation.json --format json
```

Markdown is the default. Plain text retains the message text, including any Markdown
or code it contains, with readable author and time separators. JSON includes versioned
snapshot metadata and ordered messages. Export preserves original visible text and
multipart order across the session's runs, including retained messages from before
context compaction. It reads saved history rather than the messages currently loaded
on screen. Export each branch as its own session; parent lineage metadata is unavailable.

The export contains human or application inputs and released visible assistant text.
It excludes peer-agent messages, system instructions, hidden reasoning, tool calls and
results, and private agent context. Text attachment content already inserted into
saved user or assistant messages is included. Images and opaque attachment
references appear as metadata placeholders with a display filename, media type,
and byte size. Export does not fetch attachment files or embed image bytes. Saved
text can still contain sensitive information, including file contents and paths.
Review the file before sharing it.

An export captures a fixed snapshot when reading begins. If work is running, later
messages and uncommitted output are excluded. The file reports active work when it
can be established, or unknown activity when it cannot. A completed export means the
captured snapshot is complete; it does not mean the conversation's work has finished.

In the TUI, export the current saved conversation with either command:

```text
/session export "conversation notes.md"
/export conversation.json --format json
```

Quote paths containing spaces. The TUI requires a file destination. The CLI can write
the completed transcript to standard output with `--output -`:

```bash
colossus sessions export SESSION_ID --output - --format text
```

File exports require an existing writable parent directory and a new filename.
Existing files are kept, including when another process creates the destination
during export. Colossus stages the output until the whole snapshot has been read;
failed reads do not publish a shortened file. CLI and TUI exports stop at 256 MiB
or 10,000 source pages rather than truncate the conversation. Standard output is
also staged before delivery; a broken pipe can still interrupt that final delivery.

[Desktop](../desktop/work.md#export-a-conversation) and
[Cloud](../cloud/work.md#export-the-retained-conversation) offer the same formats
through **Export conversation**, with their own save and clipboard limits. Full
export requires a runtime that supports it and current read access. The VS Code
extension has no conversation export command.

If export fails, keep the conversation open and check the reported cause:

- **Destination exists or is unwritable:** choose a new filename in a writable
  directory and retry.
- **Runtime unavailable or export unsupported:** reconnect to the conversation's
  runtime and update it if needed. Cloud's loaded-conversation actions can save
  an explicitly incomplete excerpt while its source is offline.
- **Access changed, cursor expired, or runtime restarted:** reopen the conversation
  and start a new export. It captures a new snapshot.
- **Total size or page limit:** the export fails without shortening the snapshot.
  Use Save instead of Copy for the clipboard limit, or the CLI for Cloud's smaller
  total download limit.
- **Individual message limit:** a retained message must fit the canonical reader's
  1.5 MiB page budget, including JSON encoding overhead. A message with extensive
  escaping can fail even when the whole conversation is below the total export
  limit. This fails explicitly without truncation. Switching to the CLI or another
  format does not bypass this reader limit.

## If you can't find a session

- **Session not found:** copy the complete ID from `sessions list` and check that you
  selected the same workspace, configuration, and state as before.
- **The wrong conversation resumed:** `--resume` selects the most recently updated
  session. Use `--session SESSION_ID` for a specific one.
- **No session to resume:** start a fresh run or use `/session new` in the TUI.
