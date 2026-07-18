---
title: Terminal UI
description: Work in Colossus's responsive terminal interface with durable sessions, approvals, completion, and safe cancellation.
audience: user
type: how-to
---

# Terminal UI

## Goal

Start an interactive Colossus session, navigate the transcript and composer, respond to
one-use prompts safely, and leave without losing durable work.

## Prerequisites

- An initialized Colossus configuration.
- An interactive terminal at least 40 columns by 12 rows.
- A provider route and any required tool grants.

## Steps

### 1. Start the interface

```bash
colossus --config .colossus/config.yaml
```

The explicit form is `colossus --config .colossus/config.yaml tui`. Resume the most
recent session with:

```bash
colossus --config .colossus/config.yaml tui --resume
```

Use `--session SESSION_ID` for an exact session. The alternate screen is the default;
global `--no-alt-screen` selects an inline viewport, and Zellij selects inline mode
automatically.

### 2. Inspect the session before acting

Enter these commands in the composer:

```text
/help
/session show
/tools
/work
/context status
```

Unknown slash commands remain in the terminal parser and are not sent to the model.

### 3. Compose and navigate

- Type `/` at the start of a draft for slash-command completion.
- Type `@` at a skill-token boundary for installed skill completion.
- Use Tab or Down/Up to move through suggestions, Right Arrow to accept an inline
  suggestion, and Enter to submit.
- Use PageUp/PageDown to read older transcript content and End to return to live output.
- Use Ctrl-R to search encrypted prompt history.
- Toggle multiline composition with `/multiline toggle`.

The composer accepts up to eight future turns while a run is active. Failure or
cancellation pauses the queue for confirmation.

### 4. Handle approvals and questions

Approval and `user.ask` prompts take focus without discarding your draft. Select an exact
option or type an answer, then press Enter. Esc or a blank response fails closed.

Use `wait_for_input` in a workflow when a run must wait durably without an attached
terminal; `user.ask` is turn-scoped.

### 5. Leave cleanly

Enter `/exit`, or press Ctrl-D only while idle with an empty draft. Ctrl-C clears a
draft, cancels a modal, or requests cooperative run cancellation according to the
current state.

## Expected result

The transcript remains scrollable above a pinned composer, live work is represented by
semantic status, approvals are one-use, and session messages remain available after the
terminal exits.

## Verification

Restart with `tui --resume` and confirm that the transcript returns. Use `/session show`
and `/audit verify` to confirm the active session and journal.

## Failure path

- **Layout is cramped:** enlarge the terminal; at the minimum size, pickers scroll and
  optional footer fields disappear.
- **Native scrollback is unavailable:** restart with `--no-alt-screen`.
- **A queued turn pauses:** inspect the preceding failure or cancellation, then confirm
  whether the queue should continue.
- **A modal disconnects or times out:** the answer fails closed; reopen the operation
  rather than assuming it continued.
- **Terminal state looks damaged after a crash:** reset the terminal, then use durable
  session and audit commands to inspect application state.

## Next step

Learn how to [resume sessions and manage context](sessions-context.md). Exact slash
commands and keys live in [TUI commands and keys](../reference/tui.md).
