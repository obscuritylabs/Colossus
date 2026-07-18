# Terminal UX

UX-02 restored the useful human terminal experience from the frozen Python 0.5 runtime;
UX-03 carries that presentation into a responsive Ratatui terminal UI without
reintroducing Python code, state, or interface-owned application behavior. The
reference is commit `33de5b6a64f74428f86447e24630f97dfb8b0392`, retained by both
`python-v0.5.0` and `python-legacy`.

The transcript is a borderless conversation surface. Speaker labels, semantic tones,
status words, and small glyphs provide visual grouping without relying on color alone.
Semantic cards remain part of the backend-neutral document contract, but the TUI flattens
them into colored headings and indented content. Key/value details and resource
collections are borderless; collection rows emphasize identity and semantic status, then
wrap readable metadata below. Genuine authored data tables may keep one grid, while
approvals or input requests retain the strongest framed overlay. A transcript item must
never render recursively nested card chrome.

The reference implementation contains 62 Rich table constructions, eight Markdown
rendering paths, purpose-built semantic summaries for the complete legacy tool catalog,
and 182 focused tests across `test_cli.py`, `test_repl.py`, `test_trace.py`, and
`test_transcript.py`. Rust parity is behavioral rather than a source port.

Pure retained presentation documents live in `colossus-presentation`; editing, layout,
overlays, and the single terminal event loop live in `colossus-tui`; `colossus-cli`
provides embedded and worker-backed host adapters. The authenticated worker transports
typed data and protocol-v4 prompts rather than terminal markup.

## Output Contract

- Interactive CLI and TUI output defaults to a human renderer.
- Redirected output defaults to stable JSON for command surfaces that expose structured
  contracts. `--output human` and `--output json` override automatic selection.
- TUI slash commands use human rendering. Interactive `--output json` is rejected;
  non-TTY line mode continues supporting bounded JSON for scripts and acceptance tests.
- ANSI and terminal control sequences are emitted only after an interactive-terminal
  check. Untrusted released content is never interpreted as terminal control input.
- Rendering consumes only post-policy released contracts. It never changes policy,
  approval, execution, persistence, or audit decisions.

## Parity Matrix

| Area | Python reference | Rust acceptance |
| --- | --- | --- |
| Transcript | labeled user/agent blocks; comfortable and dense modes | equivalent semantic blocks and spacing in comfortable/compact modes |
| Markdown | final answers, plans, research, and child output | headings, lists, emphasis, links, quotes, code fences, and tables render safely at terminal width |
| Status semantics | thinking, context, approval, input waits, risk, research, subagent, errors | distinct themed transcript sections or focused overlays with bounded content and recoverable/terminal status; queued/running subagents render as pending rather than failed; foreground input never competes with a spinner |
| Files | line-numbered source excerpts and edit summaries | bounded head/tail previews, path/line metadata, and mutation summaries |
| Processes | command identity plus separated stdout/stderr | bounded styled streams, exit status, duration, and truncation state |
| Git and patches | status summaries and styled diffs | additions/deletions/hunks remain visually and textually distinct |
| Durable work | tasks, decisions, memories, plans, goals, agents | semantic list/detail sections rather than generic object keys |
| Discovery | tools, repositories, skills, web, MCP, integrations, packs | family-specific counts, identifiers, status, provenance, and bounded previews |
| Lists | tools, sessions, work, research, telemetry, extensions | adaptive borderless collection rows with highlighted identity, readable wrapped metadata, semantic status, and explicit empty states |
| Help and completion | stateful help table, inline type-ahead, slash completion, `@skill` completion | fish-style history hints plus an adaptive visible menu while typing `/` commands or `@skill` names, theme-aware ghost text, keyboard selection and acceptance, and grouped help with current settings |
| Choices | labeled option tables, resume picker, free-form fallback | one responsive session picker with recent-message previews, highlighted keyboard selection, exact-ID commands, bounded scrolling, fail-closed cancellation, and worker/embedded parity |
| Themes | five built-ins, custom themes, prompt/toolbar/event previews | immutable palettes drive a numbered picker, full semantic preview, active-state table, dynamic completion, safe scaffold output, strict validation, and readable custom-theme search locations |
| Worker mode | same user-facing behavior as embedded mode | typed documents, prompts, cancellation, and terminal styling are semantically equivalent |

## TUI Interaction Contract

- `colossus` and `colossus tui` start the TUI. The former `colossus repl` alias is removed.
- The flexible transcript reflows retained Markdown, tables, cards, source, diffs, and
  process output on resize. The composer and one width-aware footer remain pinned below it.
- PageUp/PageDown scroll while End returns to live output. New items do not move an
  operator who is reading older content and instead increment a visible counter.
- Input remains active during a run. Up to eight future turns are queued; a failure or
  cooperative cancellation pauses that queue for explicit confirmation.
- Typing `/` at the start of the composer or `@` at a skill-token boundary opens a
  bounded suggestion menu. Tab/Down advances, Shift-Tab/Up moves backward, Right accepts
  the preview, and Enter accepts a suggestion after explicit keyboard selection. Escape
  dismisses the menu until the draft changes.
- Approval and `user.ask` use focus-taking one-use overlays that preserve the current
  draft. Blank, cancelled, timed-out, disconnected, replayed, or malformed answers fail
  closed.
- `/resume` and `/session resume` use one responsive, scrollable picker surface with
  recent-user previews. Up/Down changes the highlighted session and Enter accepts it;
  exact session IDs remain available as command arguments.
- Alternate-screen mode is the default. `--no-alt-screen` selects Ratatui inline mode,
  and Zellij selects inline mode automatically. Raw mode, bracketed paste, cursor state,
  and screen ownership are restored by an RAII guard.
- Non-TTY input uses the bounded line runner. It has no terminal-control ownership
  and remains suitable for existing scripts.

`user.ask` is a foreground, turn-scoped interaction: the current agent turn pauses until
the operator answers or cancels the overlay. Durable non-blocking interaction belongs to
workflow `wait_for_input`, where the run can remain waiting without an attached TUI.

## Presentation Boundary

`colossus-presentation` owns pure mappings from released typed contracts to a bounded
presentation document. Terminal, plain-text, and JSON backends render that document.
CLI and TUI select a backend and render the result; they do not own model, tool, policy,
repository, workflow, or persistence behavior. Exactly one TUI event loop writes terminal
state. Background application tasks publish typed events through bounded channels. Worker
IPC continues to transport typed application contracts rather than terminal markup.

## Acceptance Evidence

- Semantic fixtures cover every built-in tool and normalized run event, with specialized
  source, process, edit/diff, reasoning, failure, work, and context cards.
- Markdown fixtures run at 60, 80, 120, and 160 columns, including Unicode and hostile ANSI,
  OSC, control, invisible, and oversized input.
- Table/card fixtures cover empty, single, many, truncated, long, and multiline values;
  resource collections additionally prove borderless scan rows and normal-contrast detail
  text.
- Theme preview fixtures cover all five built-ins at 60, 80, 120, and 160 columns, both
  ANSI-enabled and control-sequence-free, plus the bundled custom-theme example.
- Ratatui `TestBackend` fixtures cover 40×12, 60×20, 80×24, 120×40, and 160×50 across
  built-in and custom themes, including minimum-size state preservation and adaptive
  slash/skill suggestion menus.
- A `portable-pty`/`vt100` regression types character-by-character, resizes, and proves
  uniquely identified durable transcript rows are not erased. A second PTY case exercises
  inline mode and restoration of bracketed paste and cursor visibility.
- CLI acceptance covers TTY human, redirected automatic JSON, explicit human, and
  explicit JSON behavior.
- Worker protocol-v4 tests cover authenticated prompt responses, cancellation, replay,
  wrong connection/request/prompt identity, oversized responses, and explicit restart
  guidance for mismatched versions.
- Existing release, policy, and terminal tests prove credentials, hidden reasoning,
  denied content, and unreleased stream data do not enter renderer output.
