---
title: TUI commands and keys
description: Look up terminal UI keys, slash commands, and plan controls.
audience: user
type: reference
icon: lucide/keyboard
---

# TUI commands and keys

Enter `/help` in Colossus for the command list supported by your running version.
Start with the [Terminal UI guide](../use/terminal-ui.md) if you are new to the
interface. The default view fills the alternate screen. Add the global
`--no-alt-screen` flag to use an inline view with native terminal scrollback.

## Keys

### Compose and navigate

| Key | Action |
| --- | --- |
| `Enter` | Send the prompt. |
| `Shift-Enter` | Insert a newline if your terminal reports the modifier. |
| `Ctrl-J` | Insert a newline, including when Shift-Enter is indistinguishable from Enter. |
| `Ctrl-R` | Search earlier submitted prompts. |
| `F2` | Open or close the Sessions browser on its Open tab. |
| `F3` or click a tool row | Inspect the most recent visible tool event, or the clicked event. Inside details, Left/Right selects an event; Up/Down, PageUp/PageDown, Home/End scroll; Esc closes. Mouse clicking requires captured input in the full-screen view. |
| `Alt-N` | Start a new session while existing sessions continue. |
| `Alt-Left` / `Alt-Right` | Switch between workspace sessions. |
| `Up` / `Down` | Move through wrapped or multiline draft rows. Up from the first row browses submitted prompts; Down past the newest restores your draft. |
| `Tab` / `Right` | Accept a visible completion. |
| `Up` / `Down` / `Shift-Tab` | Move through completion suggestions. |
| Terminal scroll shortcuts | Read finalized output when using `--no-alt-screen`. |
| `PageUp` / `PageDown` | Scroll the transcript in the default full-screen view. |
| `Alt-PageUp` / `Alt-PageDown` | Scroll the right status sidebar independently. |
| `End` | Return to live output in the full-screen view. |
| Mouse drag, then `Ctrl-C` / `Ctrl-Insert` | Select rendered text and copy it through the terminal clipboard. Esc clears the selection. |
| `F6` | Freeze the conversation view and enable terminal-native text selection. Drag to highlight text, then use your terminal's Copy shortcut. Press F6 or Esc to return. |
| `Ctrl-C` | Copy an active rendered-text selection. Otherwise exit when idle, or request cancellation during a run; press again to exit. |
| `Ctrl-D` | Exit from an empty, idle composer. |

If Shift-Enter submits instead of inserting a newline, use Ctrl-J or open
`/multiline` and select **Enter adds a newline**. In that mode, Enter inserts a line
and Ctrl-D submits a nonempty prompt.
Ctrl-Enter and Alt-Enter also submit when your terminal reports them distinctly.
`/multiline off` restores the default; `/multiline toggle` switches modes.

Dragging selects the exact rendered cells and pauses screen updates while the
agent continues working. Ctrl-C or Ctrl-Insert copies that selection using the
terminal clipboard protocol; Esc clears it and displays new output.

F6 instead releases mouse capture for terminal-native selection. In Desktop, copy
native highlighted text with Ctrl-Shift-C (Cmd-C on macOS); other terminal emulators
use their own Copy and Paste shortcuts. Pasting returns to the composer without
submitting. Without an active rendered-text selection, Ctrl-C retains its normal
interrupt behavior.

Typing `/` at the start of a draft offers slash-command completion. Typing `@` at a
skill token offers active plugin skills as `@PLUGIN/SKILL`. Esc closes suggestions.

### Workspace sessions

The right sidebar pins workspace sessions below the current session's agent and
status details. In the default full-screen view, click a session once to switch
to it. Press F6 or Esc first if terminal-native text selection is enabled. The
sidebar grows to 36 columns when space allows, keeping at least 64 columns for
the conversation; narrower terminals retain the smaller sidebar or hide it.

F2 and `/sessions` open the Sessions browser on **Open**. `/resume` opens its
**History** tab. The left pane lists sessions with status, update time, and message
count; the right pane previews recent messages and session work. A single click
selects a browser row for preview; Enter opens it. Compact terminals show the list.

| Key | Action in the Sessions browser |
| --- | --- |
| `Tab` / `Shift-Tab` | Cycle Open, History, and Archived. |
| `/` | Search the current list. |
| `Up` / `Down` | Highlight a session. |
| `Home` / `End` | Select the first or last session. |
| `PageUp` / `PageDown` | Scroll the selected session's preview. |
| `Enter` | Open the highlighted unarchived session. |
| `n` | Start a new session. |
| `r` | Refresh the saved-session catalogue. |
| `s` | Stop the highlighted running or waiting session; other sessions continue. |
| `a` | Archive an idle session with no unsent draft or queued turns. |
| `u` | Restore the highlighted session from Archived to History. |
| `Esc` / `F2` | Return to the conversation. While editing a search, Esc first leaves search editing. |

Working and decision indicators describe sessions retained by this terminal.
Saved sessions that are not open here show **ready**; this is not a live status
check of another terminal or Desktop. See [Sessions](../use/sessions.md) for
concurrency, archive/restore, retention, and exit behavior.

### Approvals and pickers

| Key | Context | Action |
| --- | --- | --- |
| `Up` / `Down` | Approval dock | Select a decision without confirming it. |
| `A` / `D` | Effect approval | Select Allow once or Deny. |
| `S` / `R` / `P` | Approval dock | Show Summary, Exact request, or Protections. |
| `Tab` / `Shift-Tab` | Approval dock | Move between detail sections. |
| `PageUp` / `PageDown` | Approval dock | Scroll the active section. |
| `Enter` | Approval dock | Confirm the selected decision; nothing is preselected. |
| `Esc` | Approval dock | Dismiss and deny the request. |
| `/` | Theme browser | Search. Esc leaves search before closing the browser. |
| `Up` / `Down` | Theme browser | Preview a theme. |
| `PageUp` / `PageDown` | History browser | Scroll the selected preview. |
| `Up` / `Down` | History browser | Select an earlier prompt; typing filters the list. |
| `Enter` | History browser | Place the selected prompt in the composer for editing. |
| `Esc` | History browser | Close and preserve the draft. |
| `Enter` | Theme browser | Save the selected theme. |
| `Esc` | Theme browser | Cancel and restore the original theme. |
| `D` / `G` | Plan execution dock | Select Direct or Goal Mode; Enter confirms. |

Theme and history browsers fill the screen with a list on the left and
an inspecting view on the right. Narrow terminals show the list without the view.
Closing a browser restores the conversation and draft.

The approval dock preserves the composer draft. Cancelling, timing out, or
disconnecting does not grant approval. For session and theme workflows, see
[Sessions](../use/sessions.md) and [Theme](../use/theme.md).

## Commands

Prefix a draft with `!` to run shell source directly, for example `!git status`.
Policy, approvals, sandboxing, cancellation, and audit remain active. Shell directory
and environment changes do not persist into the next invocation.

Slash commands run in the terminal UI; unknown commands are not sent to the model.
Use `/help` for arguments and commands available in your installed version.

| Family | Commands |
| --- | --- |
| Help and exit | `/help`, `/exit` |
| Sessions | `/sessions`, `/session show`, `/session new`, `/session resume [SESSION_ID]`, `/resume [SESSION_ID]` |
| Permissions | `/permissions` opens a chooser; `/permissions deny\|ask\|risk-auto\|full-access` sets a mode directly |
| Model selection | `/model` and `/effort` open searchable pickers; `/model PROFILE` and `/effort high` set a value directly. Use `default` to restore the configured setting. |
| Tools and work | `/tools`, `/work`, `/tasks`, `/decisions`, `/plans`, `/goals`, `/goal`, `/goal resume GOAL_ID`, `/agents`, `/agents drain` |
| Planning | `/plan`, `/plan on`, `/plan off`, `/plan status`, `/plan new`, `/plan list`, `/plan use PLAN_ID`, `/plan show [PLAN_ID]`, `/plan approve`, `/plan discard`, `/plan execute [direct\|goal [ITERATIONS]]` |
| Context | `/context status`, `/context list`, `/context compact`, `/context restore` |
| Memory and research | `/memories`, `/memory search`, `/research`, `/research list` |
| Skills and plugins | `/plugins`, `/plugins show`, `/plugin skills`, `/plugin active`, `/plugin use`, `/plugin clear`, `/plugin show`, `/plugin resources`, `/plugin read` |
| MCP and integrations | `/mcp servers`, `/mcp tools`, `/mcp auth login SERVER`, `/mcp auth complete SERVER CALLBACK_URL`, `/mcp auth status SERVER`, `/mcp auth logout SERVER`, `/integrations`, `/integration show`, `/integration call`, `/integration disconnect` |
| Images | `/attach PATH`, `/attachments`, `/detach INDEX`, `/detach all` |
| Theme | `/theme`, `/theme list`, `/theme preview`, `/theme validate`, `/theme scaffold`, `/theme reset` |
| Display | `/stream`, `/events`, `/reasoning`, `/transcript`, `/multiline` open setting choosers; direct value arguments remain supported. `/trace` toggles compact events. |
| Preferences | `/tui prefs`, `/tui save`, `/tui reset` |
| Telemetry and diagnostics | `/telemetry`, `/telemetry metrics`, `/audit verify`, `/projection status`, `/models doctor [PROFILE]`, `/provider doctor [PROFILE]`, `/provider diagnostics` |
| Workflows | `/workflow list`, `/workflow status`; schedule `list`, `show`, `enable`, `disable`, `tick`; webhook `list`, `show`, `enable`, `disable`; subscription `list`, `show`, `enable`, `disable`, `tick` |
| Bundles | `/bundle verify` |

`/decisions` lists active workspace decisions. `/tasks` and `/plans` show records in
the current session. `/sessions` opens Open in the Sessions browser; `/resume`
without an ID opens History in the same browser. `/session new` and `/resume SESSION_ID` navigate immediately,
including during a run; the previous session keeps working. `/research QUESTION`
uses standard depth with
repository, web, and MCP sources; use the CLI `research run` route to choose depth
or sources explicitly.

`/context status` shows the current session's model input budget and active
snapshot. See [Context and snapshots](../use/sessions-context.md) to interpret it.

## Plan workflow

The [Planning guide](../use/planning.md) shows the full Draft → approval → execution
journey. These commands control Plan Mode and the selected plan in the current TUI
process:

| Command | Behavior |
| --- | --- |
| `/plan new` | Enter Plan Mode with no plan selected; the next prompt creates a Draft. |
| `/plan use PLAN_ID` | Select a Draft or Approved plan from the current session. |
| `/plan show [PLAN_ID]` | Inspect the selected or named plan. |
| `/plan status` | Show the mode and selected plan revision. |
| `/plan approve` | Approve the selected Draft and open the execution choice. |
| `/plan execute` | Choose Direct or Goal Mode; neither is preselected. |
| `/plan execute direct` | Run the Approved plan once. |
| `/plan execute goal [ITERATIONS]` | Run a bounded goal loop; default 5, range 1–50. |
| `/plan discard` | Discard the selected Draft or Approved plan. |
| `/plan off` | Return to Execute mode without discarding the selection. |
| `/goal resume GOAL_ID` | Continue an Active goal with its remaining budget. |

`/plan` toggles Execute and Plan modes; `/plan on` enters Plan Mode. Mode and
selection are local to each open session in the TUI process. Switching sessions
retains them; restarting resets them. The plan record itself remains durable.
Approval of a plan does not preapprove tools used during execution.

## Attachments and diagnostics

`/attach PATH` queues a supported image from an absolute path or one relative to
the workspace; quote paths with spaces. `/attachments` lists pending images, and
`/detach INDEX` or `/detach all` removes them. Execute and Plan modes accept images;
Research mode does not. The model profile must enable
[`imageInputs`](configuration/providers-models.md#capabilities).

Use `/events compact` for shorter tool-result previews or `/events verbose` to
inspect more released detail. `/models doctor [PROFILE]` and
`/provider doctor [PROFILE]` run diagnostic probes. If a provider error only
appears later in a conversation, `/provider diagnostics on` exposes the next
provider-facing request in the error card until you turn it off or exit. That
request may contain conversation content, so review it before sharing.

While a run is active, each session's composer can queue up to eight future turns.
A failure or cancellation pauses the queue for your decision. Finalized output remains in
the transcript viewport; an active run does not move you away from older
content you are reading. With `--no-alt-screen`, finalized output goes to native
terminal scrollback.
