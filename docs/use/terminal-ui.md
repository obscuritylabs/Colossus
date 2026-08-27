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
colossus -w /absolute/path/to/repository
```

The explicit form is `colossus -w /absolute/path/to/repository tui`. Configuration is
selected from an explicit path, the repository, then the Colossus home without merging.
`-w` chooses repository context, the relative-path anchor, and the state partition—not
the state directory itself. In acknowledged danger full access it is not a maximum
tool boundary. Resume the most recent session with:

```bash
colossus -w /absolute/path/to/repository tui --resume
```

A new empty session opens with a responsive launch rail showing the canonical workspace,
provider route, sandbox profile, approval mode, and readiness state. The rail is process-local:
it is not written to the durable transcript, and it recedes as soon as the first command or
prompt is submitted. Resumed sessions open directly on their retained transcript. At narrow or
short terminal sizes, the same startup context collapses into a compact briefing. Inline startup
moves the existing terminal view into native scrollback and begins from a clean visible viewport;
it does not purge earlier shell history.

Use `--session SESSION_ID` for an exact session. The default inline viewport writes
finalized output into native terminal scrollback immediately for ordinary selection,
copy, search, and wheel navigation. It grows while output is streaming, then returns to
the sticky composer and status when the output completes, without leaving cleared live
rows between the preceding transcript and final response. Global `--alt-screen` selects
the application-owned full-screen viewport; `--no-alt-screen` remains a compatibility
alias for the default.

Attach a supported static image before submitting a turn with `/attach PATH`. Use
`/attachments` to inspect the pending queue and `/detach INDEX` or `/detach all` to
remove entries. Inline mode renders deterministic half-block previews in native
scrollback; `--alt-screen` can use Kitty, iTerm2, or Sixel graphics when available.

For a development session with eligible low-risk shell and read-only network review:

```bash
colossus -w /absolute/path/to/repository \
  --approval-mode risk-auto tui
```

At top-level run start, the TUI snapshots bounded instructions from the home and
repository `AGENTS.md` files. All turns, Goal iterations, and delegated subagents from
that run keep the same snapshot. See
[Colossus home and workspace resolution](../reference/colossus-home.md#load-agentsmd).

Automatic low-risk grants appear inline as warning-toned **Automatic approval review**
cards. The notice is informational and never opens a modal or interrupts typing.

Inside the TUI, `/permissions` shows the active approval mode. Change the mode for
subsequent interactive agent and plan operations from that TUI with `/permissions deny`,
`/permissions ask`, `/permissions risk-auto`, or `/permissions full-access`. This changes
only how an existing approval obligation is satisfied; it does not override policy
denials, add tool authority, or change the sandbox boundary.

If the evaluator is unavailable or returns an invalid assessment, an **Automatic
approval review failed** card explains that Colossus is falling back to manual approval
before the bottom approval dock opens.

The canonical workspace is also the worker compatibility identity. A TUI client refuses
to attach to a worker serving another workspace.

When `sandbox.backend` is `external` or `danger_full_access` and its matching
configuration acknowledgement is `false`, startup opens a boundary warning. Accepting
it enables process effects only for the active session in this runtime process and
records audit evidence. Cancelling or submitting a blank response keeps process effects
blocked. This boundary acknowledgement is separate from approval mode; every normal
policy and approval obligation still applies.

The sparse schema-version-2 default already acknowledges `danger_full_access`, so its
warning is a persistent startup card and footer badge rather than a blocking prompt.
It means authorized process, structured filesystem, and HTTP tools can use ambient host
resources; choose an isolating execution boundary in configuration when that is not
acceptable. Startup findings use one primary risk line with a dim, concise recommendation;
the canonical diagnostic remains unchanged. The composer is separated from this guidance by a
quiet row, and the footer uses a full-width contrasting status surface with a distinct warning
segment so operational state does not blend into transcript content.

In Colossus Desktop, **Open Colossus TUI** launches the verified bundled CLI with fixed
native-generated arguments and requires the existing Managed Local worker. It never
falls back to a second local writer. This TUI retains normal Colossus policy and audit
behavior, and `/permissions` uses an authenticated client-scoped override without
changing the worker default used by Desktop or other clients. On macOS, the separately
confirmed **Open Shell** action launches only the validated system `/bin/zsh -l`; it is
a direct local-user terminal outside Colossus policy, approvals, journal, and audit and
receives no worker authentication. Neither terminal action accepts a renderer-selected
program or arguments. See
[Colossus Desktop](../get-started/desktop.md#7-opt-into-local-terminals).

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

A completed planning turn durably creates one Draft, selects it, and opens a review
dock. The dock shows the ordered Plan steps and offers **Keep refining**, **Approve**,
or **Discard**. Plan steps are execution guidance; they are not separate durable
`/tasks` records. Further prompts refine the selected Draft at its current revision.
The equivalent explicit commands remain available:

```text
/plan status
/plan show
/plan approve
/plan execute
```

Approving in the review dock, or running `/plan approve`, immediately opens the Direct
versus Goal Mode execution chooser. `/plan execute` reopens that chooser for an already
Approved plan. Direct consumes the Approved plan into one ordinary run. Goal Mode
defaults to five iterations; use
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

Effect approvals take focus in a compact bottom dock above the preserved composer. The
borderless Summary keeps requester, action, resource, policy reason, and risk review in
the initial view, with long values wrapped and scrollable. Use `S`, `R`, and `P` to
inspect Summary, Exact request, and Protections; PageUp/PageDown appears in the help row
only when the active section overflows. Exact request repeats the complete sanitized
approval scope. Up/Down or `A`/`D` selects a decision, and Enter confirms it. Nothing is
selected initially, so Enter, Esc, disconnect, or timeout fails closed. Filled neutral
controls distinguish available actions from the amber active control without implying
that an action has already been approved. `user.ask` continues to use a focused overlay
without discarding your draft.

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
- **A worker protocol mismatch appears:** protocol v10 is not compatible with an older
  resident worker, including one that predates the `<storage.path>.worker-auth` secret
  and is reported as listening without it. Restart the worker and client with the same
  Colossus version, inspect `/plans`, and do not assume an interrupted request was
  retried.
- **Terminal state looks damaged after a crash:** reset the terminal, then use durable
  session and audit commands to inspect application state.

## Next step

Learn how to [resume sessions and manage context](sessions-context.md). Exact slash
commands and keys live in [TUI commands and keys](../reference/tui.md).
