---
title: TUI commands and keys
description: Keyboard navigation, interaction behavior, and slash-command families in the Colossus terminal UI.
audience: user
type: reference
---

# TUI commands and keys

Start with `colossus` or `colossus tui`. Alternate-screen mode is the default;
`--no-alt-screen` preserves terminal scrollback. Zellij selects inline mode
automatically.

## Keys

| Key | Context | Action |
| --- | --- | --- |
| `Enter` | Composer | Submit the current turn |
| `Enter` | Multiline composer | Insert a newline |
| `Ctrl-Enter` / `Alt-Enter` | Multiline composer | Submit the current turn |
| `Up` / `Down` | Composer | Navigate input history |
| `Ctrl-R` | Composer | Search submitted-input history |
| `Ctrl-C` | Any focused work | Clear draft, dismiss a modal, or cancel the active run |
| `Ctrl-D` | Empty, idle composer | Exit |
| `PageUp` / `PageDown` | Transcript | Scroll retained output |
| `End` | Transcript | Return to live output |
| `Esc` | Menu or overlay | Dismiss or fail closed, depending on the prompt |
| `Tab` / `Down` | Suggestions | Select the next item |
| `Shift-Tab` / `Up` | Suggestions | Select the previous item |
| `Right` | Suggestions | Accept the preview |
| `Enter` | Explicitly selected suggestion | Accept the selection |
| `Up` / `Down`, `Enter` | Session picker | Move and resume the selected session |

Typing `/` at the beginning of a draft opens slash-command completion. Typing `@` at a
skill-token boundary opens skill completion. Suggestions are bounded and dismiss until
the draft changes after `Esc`.

## Commands

The in-product `/help` command is the executable authority for commands and required
arguments in the current runtime.

| Family | Commands |
| --- | --- |
| Help and exit | `/help`, `/exit` |
| TUI preferences | `/tui prefs`, `/tui save`, `/tui reset` |
| Themes | `/theme`, `/theme list`, `/theme preview`, `/theme validate`, `/theme scaffold`, `/theme reset` |
| Activity | `/stream on`, `/stream raw`, `/stream off`, `/events compact`, `/events verbose`, `/events off`, `/reasoning on`, `/reasoning off` |
| Composer and transcript | `/transcript comfortable`, `/transcript compact`, `/multiline on`, `/multiline off`, `/multiline toggle`, `/trace` |
| Sessions | `/sessions`, `/session show`, `/session new`, `/session resume`, `/resume` |
| Work | `/work`, `/tasks`, `/decisions`, `/plans`, `/goals`, `/goal`, `/agents`, `/agents drain` |
| Memory and research | `/memories`, `/memory search`, `/research`, `/research list` |
| Telemetry | `/telemetry`, `/telemetry metrics` |
| Skills | `/skills`, `/skill active`, `/skill use`, `/skill clear`, `/skill show`, `/skill resources`, `/skill read` |
| Packs and distribution | `/packs list`, `/packs show`, `/packs verify`, `/packs install`, `/packs enable`, `/packs disable`, `/packs uninstall`, `/packs call`, `/packs trust list`, `/packs trust add`, `/collections verify`, `/collections install`, `/registry pull`, `/registry push`, `/bundle verify` |
| Integrations and MCP | `/integrations`, `/integration show`, `/integration call`, `/integration disconnect`, `/mcp servers`, `/mcp tools`, `/mcp call` |
| Context | `/context status`, `/context list`, `/context compact`, `/context restore` |
| Workflows | `/workflow list`, `/workflow status`; schedule `list`, `show`, `enable`, `disable`, `tick`; webhook `list`, `show`, `enable`, `disable`; subscription `list`, `show`, `enable`, `disable`, `tick` |
| Diagnostics | `/audit verify`, `/projection status`, `/tools` |

Use `/resume` or `/session resume` without an ID for the picker; exact session IDs are
accepted when deterministic selection matters.

`/research QUESTION` uses `standard` depth with the `repo`, `web`, and `mcp` lanes.
Use the CLI `research run` route when depth or lane selection must be explicit.

## Interaction contract

- Input stays available during a run; up to eight future turns may queue.
- A failure or cooperative cancellation pauses the queue for explicit confirmation.
- Approval and `user.ask` use focus-taking, one-use overlays and preserve the draft.
- Blank, cancelled, timed-out, disconnected, replayed, or malformed prompt answers fail
  closed.
- New output does not move an operator reading older content; the UI shows a new-item
  count.
- The transcript reflows on resize. Composer and footer remain pinned.
- Terminal state, cursor, bracketed paste, raw mode, and screen ownership are restored
  when the TUI exits.

Non-TTY stdin selects a bounded line runner. It does not own terminal control and is
appropriate for automation.
