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
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml
```

The explicit form is `colossus --config .colossus/config.yaml tui`. Resume the most
recent session with:

```bash
colossus --config .colossus/config.yaml tui --resume
```

Use `--session SESSION_ID` for an exact session. The default inline viewport writes
finalized output into native terminal scrollback immediately for ordinary selection,
copy, search, and wheel navigation. It grows while output is streaming, then returns to
the sticky composer and status when the output completes. Global `--alt-screen` selects
the application-owned full-screen viewport; `--no-alt-screen` remains a compatibility
alias for the default.

For a development session with eligible low-risk shell and read-only network review:

```bash
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml \
  --approval-mode risk-auto tui
```

Automatic low-risk grants appear inline as warning-toned **Automatic approval review**
cards. The notice is informational and never opens a modal or interrupts typing.

If the evaluator is unavailable or returns an invalid assessment, an **Automatic
approval review failed** card explains that Colossus is falling back to manual approval
before the approval overlay opens.

The canonical workspace is also the worker compatibility identity. A TUI client refuses
to attach to a worker serving another workspace.

In Colossus Desktop, **Open Colossus TUI** launches the verified bundled CLI with fixed
native-generated arguments and requires the existing Managed Local worker. It never
falls back to a second local writer. This TUI retains normal Colossus policy and
audit behavior. Desktop rejects arbitrary Shell PTYs at the native boundary; only the
authenticated bundled TUI contract is available. See
[Colossus Desktop](../get-started/desktop.md#7-opt-into-the-local-tui).

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

### 3. Plan before executing

Enter Plan mode without selecting an older draft:

```text
/plan new
Plan the requested change, including focused verification.
```

A completed planning turn durably creates one Draft and selects it. Further prompts
refine that selected Draft at its current revision. Inspect and approve it with:

```text
/plan status
/plan show
/plan approve
/plan execute
```

The last command opens Direct, Goal Mode, and Cancel choices. Direct consumes the
Approved plan into one ordinary run. Goal Mode defaults to five iterations; use
`/plan execute goal ITERATIONS` for an explicit value from 1 through 50. In non-TTY
line mode, the same unspecified-strategy choice is numbered on stdin. Use
`/goal resume GOAL_ID` to continue the remaining budget of an Active goal after a
cancelled or failed Goal run.

Use `/plan use PLAN_ID` only for a Draft or Approved plan in the current session.
`/plan new` clears the selection without discarding the old plan, while `/plan off`
returns to Execute mode and retains it. `/plan discard` is the explicit operator-only
abandonment transition.

Mode and selection are local to this terminal process. Mode survives session switches,
but selection is cleared; both restart in Execute mode with no selected plan. These
values are never stored as presentation preferences.

### 4. Compose and navigate

- Type `/` at the start of a draft for slash-command completion.
- Type `@` at a skill-token boundary for installed skill completion.
- Use Down/Up or Shift-Tab to move through suggestions, Tab or Right Arrow to accept the
  visible suggestion, and Enter to submit.
- In the default inline mode, use the terminal's normal wheel and scrollback shortcuts.
  With `--alt-screen`, use PageUp/PageDown and End to navigate the retained transcript.
- Use Ctrl-R to search encrypted prompt history.
- Toggle multiline composition with `/multiline toggle`.

The composer accepts up to eight future turns while a run is active. Failure or
cancellation pauses the queue for confirmation.

### 5. Handle approvals and questions

Approval and `user.ask` prompts take focus without discarding your draft. Select an exact
option or type an answer, then press Enter. Esc or a blank response fails closed.

Use `wait_for_input` in a workflow when a run must wait durably without an attached
terminal; `user.ask` is turn-scoped.

### 6. Leave cleanly

Press Ctrl-C to exit while idle, including when a draft or picker is open. During an
active run, the first Ctrl-C requests cooperative cancellation and a second Ctrl-C
exits. `/exit` and Ctrl-D on an empty idle composer remain available.

## Expected result

Finalized transcript output is immediately available in native scrollback above the
live composer and status viewport, live work is represented by semantic status,
approvals are one-use, and session messages remain available after the terminal exits.

## Verification

Restart with `tui --resume` and confirm that the transcript returns. Use `/session show`
and `/audit verify` to confirm the active session and journal.

## Failure path

- **Layout is cramped:** enlarge the terminal; at the minimum size, pickers scroll and
  optional footer fields disappear.
- **Native selection or scrollback is unavailable:** omit `--alt-screen`; the default
  inline mode restores terminal-native behavior.
- **A queued turn pauses:** inspect the preceding failure or cancellation, then confirm
  whether the queue should continue.
- **A modal disconnects or times out:** the answer fails closed; reopen the operation
  rather than assuming it continued.
- **A selected plan is stale:** another lifecycle change advanced its optimistic
  revision. Reload it with `/plan use PLAN_ID`, inspect it, and deliberately retry.
- **An Approved plan will not refine:** Approved plans are immutable. Execute, discard,
  start a new plan, or return to Execute mode.
- **A worker protocol mismatch appears:** protocol v6 is not compatible with an older
  resident worker. Restart the worker and client with the same Colossus version, inspect
  `/plans`, and do not assume an interrupted request was retried.
- **Terminal state looks damaged after a crash:** reset the terminal, then use durable
  session and audit commands to inspect application state.

## Next step

Learn how to [resume sessions and manage context](sessions-context.md). Exact slash
commands and keys live in [TUI commands and keys](../reference/tui.md).
