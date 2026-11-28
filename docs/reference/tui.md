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
| `Ctrl-Z` / `F7` | Undo an edit in the focused draft. |
| `Ctrl-Y` / `F8` | Redo an edit in the focused draft. |
| `Ctrl-R` | Search earlier submitted prompts. |
| `F2` | Open or close the Sessions browser on its Open tab. |
| `F3` | Toggle compact tool summaries and expanded inline results in the conversation. Keeps the draft and reading position. |
| Click a tool row | Inspect that event in detail. Left/Right selects an event; Up/Down, PageUp/PageDown, Home/End scroll; Esc closes. Requires captured mouse input in the full-screen view. |
| `Alt-N` | Start a new session while existing sessions continue. |
| `Alt-Left` / `Alt-Right` | Switch between workspace sessions. |
| `Up` / `Down` | Move through wrapped or multiline draft rows. Up from the first row browses submitted prompts; Down past the newest restores your draft. |
| `Tab` / `Right` | Accept a visible completion. |
| `Up` / `Down` / `Shift-Tab` | Move through completion suggestions. |
| Terminal scroll shortcuts | Read finalized output when using `--no-alt-screen`. |
| `PageUp` / `PageDown` | Scroll the transcript in the default full-screen view. |
| `Alt-PageUp` / `Alt-PageDown` | Scroll the right status sidebar independently. |
| `End` | Return to live output in the full-screen view. |
| Mouse drag, then `Ctrl-C` / `Ctrl-Insert` | Copy the selected rendered text. Esc clears the selection. |
| `F6` | Freeze the conversation view and enable terminal-native text selection. Drag to highlight text, then use your terminal's Copy shortcut. Press F6 or Esc to return. |
| `Ctrl-C` | Copy an active rendered-text selection or discard a draft. With an empty draft, request cancellation during a run or exit when idle. |
| `Ctrl-D` | Exit from an empty, idle composer. |

If Shift-Enter submits instead of inserting a newline, use Ctrl-J or open
`/multiline` and select **Enter adds a newline**. In that mode, Enter inserts a line
and Ctrl-D submits a nonempty prompt.
Ctrl-Enter and Alt-Enter also submit when your terminal reports them distinctly.
`/multiline off` restores the default; `/multiline toggle` switches modes.

Use F7 and F8 if your terminal intercepts Ctrl-Z or Ctrl-Y. Typing and deletion
record one edit per keypress. A paste or an accepted completion is one edit.
Undo and redo restore the draft text, cursor, and any original large-paste
marker with its bound text. A marker typed by hand is literal text. Making a
new edit after undo discards the redo path.

Each open workspace session keeps up to 32 edits and 8 MiB of retained draft
data across undo and redo. Switching sessions preserves each draft's own edit
history. Sending or discarding the draft, closing its session, or exiting the
TUI clears its edit history. Replacing it through Ctrl-R or dictation clears
earlier edit history. Use `/attach` and `/detach` to manage attachments.

During a run, undo and redo apply only to the unsent draft. An open approval
dock keeps key precedence. Ctrl-R and Up/Down still recall submitted prompts.
While browsing with Up/Down, press Down past the newest entry to return to the
live draft and its edit history.

Dragging selects rendered text and pauses screen updates while the agent
continues working. Press Ctrl-C or Ctrl-Insert to copy. A standalone TUI in a
local Windows session writes directly to the Windows clipboard and shows
**Selection copied** on success. Other sessions use the terminal clipboard
protocol (OSC 52). **Copy requested** means Colossus sent a request to the
terminal. Clipboard updates depend on terminal support. Esc clears the
selection and displays new output.

If you see **Copy failed** or your terminal does not handle the request, press
F6 to release mouse capture, drag to select the text again, and use your
terminal's Copy shortcut. In the Desktop terminal, use this F6 workflow and
copy the highlighted text with Ctrl-Shift-C (Cmd-C on macOS). Pasting returns
to the composer without submitting. Ctrl-C retains its normal interrupt
behavior when no rendered text is selected.

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
| `Tab` / `Shift-Tab` | Cycle Open, History, Archived, and Agents. |
| `/` | Search the current list. |
| `Up` / `Down` | Highlight a session. |
| `Home` / `End` | Select the first or last session. |
| `PageUp` / `PageDown` | Scroll the selected session's preview. |
| `Enter` | Open the highlighted unarchived session. |
| `p` | Pin an inactive session from Open or unpin the highlighted pinned session. Available in the full-screen TUI. |
| `n` | Start a new session. |
| `r` | Refresh the saved-session catalogue. |
| `s` | Stop the highlighted running or waiting session; other sessions continue. |
| `a` | Archive an idle session with no unsent draft or queued turns. |
| `u` | Restore the highlighted session from Archived to History. |
| `Esc` / `F2` | Return to the conversation. While editing a search, Esc first leaves search editing. |

From **Open**, Shift-Tab goes directly to **Agents**. The Agents tab uses
the following keys.

| Key | Action in Agents |
| --- | --- |
| `Up` / `Down` | Move between parent sessions and visible child jobs. |
| `Home` / `End` | Select the first or last visible row. |
| `PageUp` / `PageDown` | Move five rows. |
| `Space` | Expand or collapse the selected session's jobs. |
| `Right` / `Left` | Expand a session, return from a job to its parent, or collapse the parent. |
| `Enter` | Open the selected session or the selected job's parent. |
| `Esc` / `F2` | Return to the conversation. |

The pinned transcript follows recent output in the sidebar at widths of 100
columns or more when there is enough vertical space. It occupies the usual right
sidebar while visible. PageUp, PageDown, and End
still control the active transcript. The pane disappears in compact layouts and
returns when there is room. Open the pinned session to read older output or act
on a decision.

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
| Conversation export | `/session export PATH [--format FORMAT]`, `/export PATH [--format FORMAT]` export the current saved session to a new file. Quote paths containing spaces. |
| Permissions | `/permissions` opens a chooser; `/permissions deny\|ask\|risk-auto\|full-access` sets a mode directly |
| Model selection | `/models` opens configured profiles and provider catalogs. `/model` and `/effort` open focused pickers. `/model PROFILE` and `/effort high` set a value directly. Use `default` to restore the configured setting. |
| Tools and work | `/tools`, `/work`, `/tasks`, `/decisions`, `/plans`, `/goals`, `/goal`, `/goal resume GOAL_ID`, `/agents`, `/agents drain` |
| Planning | `/plan`, `/plan on`, `/plan off`, `/plan status`, `/plan new`, `/plan list`, `/plan use PLAN_ID`, `/plan show [PLAN_ID]`, `/plan approve`, `/plan discard`, `/plan execute [direct\|goal [ITERATIONS]]` |
| Context | `/context status`, `/context list`, `/context compact`, `/context restore` |
| Memory and research | `/memories`, `/memory search`, `/research`, `/research list` |
| Skills and plugins | `/plugins`, `/plugins show`, `/plugin skills`, `/plugin active`, `/plugin use`, `/plugin clear`, `/plugin show`, `/plugin resources`, `/plugin read` |
| MCP and integrations | `/mcp servers`, `/mcp tools`, `/mcp auth login SERVER`, `/mcp auth complete SERVER CALLBACK_URL`, `/mcp auth status SERVER`, `/mcp auth logout SERVER`, `/integrations`, `/integration show`, `/integration call github.pull_request JSON`, `/integration disconnect` |
| Images | `/attach PATH`, `/attachments`, `/detach INDEX`, `/detach all` |
| Theme | `/theme`, `/theme list`, `/theme preview`, `/theme validate`, `/theme scaffold`, `/theme reset` |
| Display | `/stream`, `/events`, `/reasoning`, `/transcript`, `/multiline` open setting choosers; direct value arguments remain supported. `/trace` toggles compact events. |
| Next prompt | `/suggestions` opens a chooser; `/suggestions off\|local\|model` saves the mode. Local is the default and makes no provider request. Model makes one bounded auxiliary request per completed source turn and reports its tokens separately. Tab inserts a suggestion into the empty draft; Esc dismisses it. |
| Preferences | `/tui prefs`, `/tui save`, `/tui reset` |
| Support report | [`/support-report PATH [--replace]`](#save-a-support-report) |
| Telemetry and diagnostics | `/telemetry`, `/telemetry metrics`, `/audit verify`, `/projection status`, `/models doctor [PROFILE]`, `/provider doctor [PROFILE]`, `/provider diagnostics` |
| Workflows | `/workflow list`, `/workflow status`; schedule `list`, `show`, `enable`, `disable`, `tick`; webhook `list`, `show`, `enable`, `disable`; subscription `list`, `show`, `enable`, `disable`, `tick` |
| Bundles | `/bundle verify` |

`/decisions` lists active workspace decisions. `/tasks` and `/plans` show records in
the current session. `/sessions` opens Open in the Sessions browser; `/resume`
without an ID opens History in the same browser. `/session new` opens an unsaved
draft. `/resume SESSION_ID` opens a saved session. Both navigate immediately
while any work in the previous session continues. See
[Start a clean conversation](../use/sessions.md#start-a-clean-conversation).
`/research QUESTION`
uses standard depth with
repository, web, and MCP sources; use the CLI `research run` route to choose depth
or sources explicitly.

Pass inline JSON to `/integration call github.pull_request JSON` to read a pull
request through the connected GitHub integration and active permission mode.
See [Inspect a pull request and its head checks](../extend/integrations.md#inspect-a-pull-request-and-its-head-checks).

Conversation export reads the retained canonical snapshot across runs, including
messages outside the viewport. `FORMAT` accepts `markdown` (default), `text`, or
`json`. It requires a file destination; standard output is
available only through the CLI. See [export formats, scope, and recovery](../use/sessions.md#export-a-conversation).

`/context status` shows the current session's model input budget and active
snapshot. See [Context and snapshots](../use/sessions-context.md) to interpret it.

### Browse provider models

Use Up/Down to browse, Enter to expand a provider or choose a model, and Esc to
return. Search includes configured profiles and models in expanded provider
catalogs. Provider discovery starts on expansion and follows the runtime's
provider policy. The last successful response is cached in the running runtime.
Ctrl-R requests a fresh catalog for the selected provider.

Discovered models use the provider's advertised token limits when available.
The editor starts with a 32,768-token context window or a 4,096-token output
budget when the corresponding limit is unknown and reduces the initial output
budget to fit the context window. The selected limits apply to this TUI
conversation for the current process.

### Save a support report

Enter `/support-report PATH [--replace]` in the interactive TUI. Quote paths that
contain spaces. Relative destinations resolve from the TUI workspace directory.
Add `--replace` only when you want to overwrite an existing file. The command
uses the TUI host's last observed worker status, when available. Press Esc from
the composer to request cancellation of a pending save. The
[CLI support report](cli.md#local-support-report) covers startup failures and
redirected input. It also describes the file format.

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
