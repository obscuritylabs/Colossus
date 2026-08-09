---
title: TUI commands and keys
description: Keyboard navigation, interaction behavior, and slash-command families in the Colossus terminal UI.
audience: user
type: reference
---

# TUI commands and keys

Start with `colossus` or `colossus tui`. The default inline viewport commits every
finalized transcript entry to native terminal scrollback immediately, so ordinary mouse
selection, copy, search, and wheel scrolling keep working. The viewport expands while
output is streaming, then collapses back to the sticky composer and status as soon as
that output completes. Use `--alt-screen` for the application-owned full-screen
transcript viewport; `--no-alt-screen` remains a compatibility alias for the default.

With `--approval-mode risk-auto`, successful low-risk reviews appear as non-blocking
**Automatic approval review** transcript cards. They do not take focus from the composer
or require a response.

Evaluator outages and invalid assessments appear as non-blocking **Automatic approval
review failed** cards before the explicit approval dock opens above the composer. These
cards contain only a sanitized failure category, action, resource, and manual-fallback
explanation.

Security posture findings appear at startup as a non-durable **Security posture**
warning card and remain visible as a warning count in the footer. This includes an
explicit `danger_full_access` sandbox backend even when its boundary acknowledgement is
already configured.

Effect approvals use a compact bottom-docked, focus-taking surface that keeps the
current transcript visible and the composer draft preserved. **Summary** presents the
released actor, action, resource, policy reason, and risk metadata as borderless rows so
the decision context is visible without opening a nested table; long values wrap and
remain scrollable. **Exact request** shows the bounded prepared request, with any
65,536-character display truncation marked explicitly, and repeats the complete
sanitized approval scope before confirmation.
**Protections** explains request binding, one-use behavior, policy re-evaluation, and
the enforcement layers that remain active. Section and decision controls use filled,
theme-resolved surfaces so focus remains visible without implying approval. Inline mode
renders this transient dock on a temporary terminal screen, so dismissing it restores
native scrollback byte-for-byte.

## Keys

| Key | Context | Action |
| --- | --- | --- |
| `Enter` | Composer | Submit the current turn |
| `Enter` | Multiline composer | Insert a newline |
| `Ctrl-Enter` / `Alt-Enter` | Multiline composer | Submit the current turn |
| `Up` / `Down` | Composer | Navigate input history |
| `Ctrl-R` | Composer | Search submitted-input history |
| `Ctrl-C` | Idle TUI | Exit, including when a draft or non-running overlay is open |
| `Ctrl-C` | Active run | Request cooperative cancellation; press again to exit |
| `Ctrl-D` | Empty, idle composer | Exit |
| Terminal scroll shortcuts | Native scrollback | Inspect finalized output using the terminal's normal bindings |
| `PageUp` / `PageDown` | Alternate-screen transcript | Scroll retained output |
| Mouse wheel | Transcript | Use native scrollback by default; scroll a few retained lines in alternate-screen mode |
| `End` | Alternate-screen transcript | Return to live output |
| `Esc` | Menu or overlay | Dismiss or fail closed, depending on the prompt |
| `Up` / `Down` | Docked security decision | Select a decision without submitting it |
| `A` / `D` | Effect approval | Select **Allow once** or **Deny**; Enter still confirms |
| `A` / `D` | Sandbox boundary acknowledgement | Select acknowledge/enable or keep blocked; Enter still confirms |
| `S` / `R` / `P` | Docked security decision | Inspect Summary, Exact request, or Protections |
| `Tab` / `Shift-Tab` | Docked security decision | Move between detail sections |
| `PageUp` / `PageDown` | Docked security decision | Scroll the active detail section |
| `Enter` | Docked security decision | Confirm the explicitly selected decision; blank remains fail closed |
| `Down` | Suggestions | Select the next item |
| `Shift-Tab` / `Up` | Suggestions | Select the previous item |
| `Tab` / `Right` | Suggestions | Accept the visible suggestion |
| `Enter` | Explicitly selected suggestion | Accept the selection |
| `/` | Session browser | Focus search; `Esc` leaves search before dismissing the browser |
| `Up` / `Down` | Session browser | Move between matching sessions; the current session is marked and skipped |
| `PageUp` / `PageDown` | Session browser | Scroll the selected session's recent-conversation preview |
| `Enter` | Session browser | Resume the selected durable session |
| `/` | Theme browser | Focus search; `Esc` leaves search before cancelling the browser |
| `Up` / `Down` | Theme browser | Preview the previous or next matching theme without saving it |
| `Enter` | Theme browser | Save the previewed theme |
| `D` / `G` | Plan execution dock | Select Direct or Goal Mode; Enter still confirms |
| `Enter` | Plan execution dock | Confirm the explicitly selected strategy; no strategy is preselected |

Typing `/` at the beginning of a draft opens slash-command completion. Typing `@` at a
skill-token boundary opens skill completion. Suggestions are bounded and dismiss until
the draft changes after `Esc`.

In inline mode, the session and theme browsers open on a temporary alternate screen.
Closing either restores the inline viewport and native terminal history byte-for-byte,
so browser rows never become scrollback output. Theme navigation is a reversible live
preview: `Esc` restores the original theme and only `Enter` saves the selection.

## Commands

The in-product `/help` command is generated from the current completion catalog and is
the executable authority for available command families and required arguments in the
current runtime.

| Family | Commands |
| --- | --- |
| Help and exit | `/help`, `/exit` |
| Permissions | `/permissions [deny\|ask\|risk-auto\|full-access]` |
| TUI preferences | `/tui prefs`, `/tui save`, `/tui reset` |
| Themes | `/theme`, `/theme list`, `/theme preview`, `/theme validate`, `/theme scaffold`, `/theme reset` |
| Activity | `/stream on`, `/stream raw`, `/stream off`, `/events compact`, `/events verbose`, `/events off`, `/reasoning on`, `/reasoning off` |
| Composer and transcript | `/transcript comfortable`, `/transcript compact`, `/multiline on`, `/multiline off`, `/multiline toggle`, `/trace` |
| Sessions | `/sessions`, `/session show`, `/session new`, `/session resume`, `/resume` |
| Work | `/work`, `/tasks`, `/decisions`, `/plans`, `/goals`, `/goal`, `/goal resume GOAL_ID`, `/agents`, `/agents drain` |
| Plan workflow | `/plan`, `/plan on`, `/plan off`, `/plan status`, `/plan new`, `/plan list`, `/plan use PLAN_ID`, `/plan show [PLAN_ID]`, `/plan approve`, `/plan discard`, `/plan execute [direct\|goal [ITERATIONS]]` |
| Memory and research | `/memories`, `/memory search`, `/research`, `/research list` |
| Telemetry | `/telemetry`, `/telemetry metrics` |
| Skills | `/skills`, `/skill active`, `/skill use`, `/skill clear`, `/skill show`, `/skill resources`, `/skill read` |
| Packs and distribution | `/packs list`, `/packs show`, `/packs verify`, `/packs install`, `/packs enable`, `/packs disable`, `/packs uninstall`, `/packs call`, `/packs trust list`, `/packs trust add`, `/collections verify`, `/collections install`, `/registry pull`, `/registry push`, `/bundle verify` |
| Integrations and MCP | `/integrations`, `/integration show`, `/integration call`, `/integration disconnect`, `/mcp servers`, `/mcp tools`, `/mcp auth login SERVER`, `/mcp auth complete SERVER CALLBACK_URL`, `/mcp auth status SERVER`, `/mcp auth logout SERVER` |
| Context | `/context status`, `/context list`, `/context compact`, `/context restore` |
| Workflows | `/workflow list`, `/workflow status`; schedule `list`, `show`, `enable`, `disable`, `tick`; webhook `list`, `show`, `enable`, `disable`; subscription `list`, `show`, `enable`, `disable`, `tick` |
| Diagnostics | `/audit verify`, `/projection status`, `/models doctor [PROFILE]`, `/provider doctor [PROFILE]`, `/provider diagnostics on`, `/provider diagnostics off`, `/tools` |

Use `/resume` or `/session resume` without an ID for the searchable master-detail
browser; exact session IDs are accepted when deterministic selection matters. The
browser keeps the running-command row, composer draft, and status footer visible while
it is open.

## Plan workflow

The terminal starts in Execute mode. Plan mode and its selected plan are process-local:
they are not presentation preferences and are not restored after a restart. The mode
survives a session switch, but the selection is cleared so a plan from one session
cannot become authority in another. The footer and composer title show the current mode,
selected plan, status, and revision when space permits.

| Command | Behavior |
| --- | --- |
| `/plan` | Toggle between Execute and Plan modes |
| `/plan on` | Enter Plan mode |
| `/plan off` | Return to Execute mode without clearing the selection |
| `/plan status` | Show the process-local mode and selected-plan revision |
| `/plan new` | Enter Plan mode and clear the selection without discarding the old plan |
| `/plan list` | List plans in the current session |
| `/plans` | Canonical current-session listing alias |
| `/plan use PLAN_ID` | Select a same-session Draft or Approved plan and enter Plan mode |
| `/plan show [PLAN_ID]` | Show the named plan, or the selected plan when the ID is omitted; showing a named plan does not select it |
| `/plan approve` | Approve the selected Draft at its displayed revision, then open the Direct/Goal execution dock |
| `/plan discard` | Discard the selected Draft or Approved plan at its displayed revision |
| `/plan execute direct` | Atomically consume the selected Approved plan, then run it once |
| `/plan execute goal [ITERATIONS]` | Atomically consume the selected Approved plan into Goal Mode; the default is 5 and the accepted range is 1–50 |
| `/plan execute` | Open a contextual decision dock with plan revision, step/mutation counts, and Direct/Goal consequences; no strategy is preselected and Enter confirms it. Line mode uses choices 1, 2, and 3 |
| `/goal resume GOAL_ID` | Continue the remaining budget of an Active goal in the current session |

Submitting a prompt in Plan mode creates a new Draft when nothing is selected. With a
selected Draft, the prompt refines that exact revision. Each completed planning turn
opens a review dock with Keep refining, Approve, and Discard choices. The dock previews
the structured Plan steps and clarifies that durable Tasks are separate records.
Approving flows directly into the existing Direct/Goal execution dock. An Approved plan
cannot be refined; use `/plan execute`, `/plan new`, `/plan discard`, or `/plan off`.
Concurrent changes reject the stale revision. Reload the current record explicitly
with `/plan use PLAN_ID` before retrying.

Canceling the execution decision dock, or cancellation/failure before plan consumption,
keeps the mode and selection. The dock preserves the composer draft and requires an
explicit Direct or Goal selection before Enter can start execution. Once Direct
execution or Goal handoff commits consumption, the terminal returns to Execute mode and
clears the selection even if later work fails or is cancelled. The consumed plan and
completed, cancelled, or failed evidence remain inspectable. A cancelled or failed Goal
stays Active; `/goal resume GOAL_ID` uses only its remaining iteration budget.

`/events compact` shows only a short preview of raw `web.fetch`, `docs.fetch`, and
`network.http` response bodies. Use `/events verbose` when inspecting the full released
response is necessary. Verbose run-error cards also show a structured `HTTP status`
field when an upstream provider returned a non-success response. Ordinary run errors
remain body-free. `/models doctor [PROFILE]` issues a new representative tool-calling
probe and displays its exact credential-free request plus at most 16 KiB of the redacted
non-success response body. `/provider doctor [PROFILE]` does the same for provider
catalog diagnostics.

Doctor commands cannot reproduce a failure that occurs only on a later continuation.
Run `/provider diagnostics on`, retry the failing TUI turn, and inspect the error card's
response body, offered tool-name list, and exact provider-facing request. The setting
lasts only for the current TUI process and applies to every provider turn until
`/provider diagnostics off` or exit. The detailed evidence is not written to durable run
history, but the request can contain user, session, and tool-result data; review it
before sharing. Use `/events off` to hide successful tool results entirely.

`/research QUESTION` uses `standard` depth with the `repo`, `web`, and `mcp` lanes.
Use the CLI `research run` route when depth or lane selection must be explicit.

## Interaction contract

- Input stays available during a run; up to eight future turns may queue.
- Mode and lifecycle commands share that FIFO. Returned plan state is applied before the
  next item starts, and the queue does not drain while the execution-choice overlay is
  open.
- A failure or cooperative cancellation pauses the queue for explicit confirmation.
- Effect approval uses a focus-taking bottom dock; `user.ask` retains its one-use
  overlay. Both preserve the draft.
- Blank, cancelled, timed-out, disconnected, replayed, or malformed prompt answers fail
  closed.
- New output does not move an operator reading older content; the UI shows a new-item
  count.
- The transcript reflows on resize. Composer and footer remain pinned.
- Terminal state, cursor, bracketed paste, raw mode, and screen ownership are restored
  when the TUI exits.

Non-TTY stdin selects a bounded line runner. It does not own terminal control and is
appropriate for automation.
