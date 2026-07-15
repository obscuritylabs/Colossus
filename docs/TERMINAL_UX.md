# Terminal UX

UX-02 restores the useful human terminal experience from the frozen Python 0.5 runtime
without reintroducing Python code, state, or interface-owned application behavior. The
reference is commit `33de5b6a64f74428f86447e24630f97dfb8b0392`, retained by both
`python-v0.5.0` and `python-legacy`.

The reference implementation contains 62 Rich table constructions, eight Markdown
rendering paths, purpose-built semantic summaries for the complete legacy tool catalog,
and 182 focused tests across `test_cli.py`, `test_repl.py`, `test_trace.py`, and
`test_transcript.py`. Rust parity is behavioral rather than a source port.

UX-02 is implemented by the pure presentation documents in `colossus-presentation` and
the thin terminal selection/writing paths in `colossus-cli`. The authenticated worker
continues to transport typed data; embedded and worker REPLs apply the same renderer.

## Output Contract

- Interactive CLI and REPL output defaults to a human renderer.
- Redirected output defaults to stable JSON for command surfaces that expose structured
  contracts. `--output human` and `--output json` override automatic selection.
- REPL slash commands use human rendering by default; starting the REPL with
  `--output json` exposes bounded JSON for diagnostics.
- ANSI and terminal control sequences are emitted only after an interactive-terminal
  check. Untrusted released content is never interpreted as terminal control input.
- Rendering consumes only post-policy released contracts. It never changes policy,
  approval, execution, persistence, or audit decisions.

## Parity Matrix

| Area | Python reference | Rust acceptance |
| --- | --- | --- |
| Transcript | labeled user/agent blocks; comfortable and dense modes | equivalent semantic blocks and spacing in comfortable/compact modes |
| Markdown | final answers, plans, research, and child output | headings, lists, emphasis, links, quotes, code fences, and tables render safely at terminal width |
| Status cards | thinking, context, approval, input waits, risk, research, subagent, errors | distinct themed cards with bounded content and recoverable/terminal status; queued/running subagents render as pending rather than failed; foreground input never competes with a spinner |
| Files | line-numbered source excerpts and edit summaries | bounded head/tail previews, path/line metadata, and mutation summaries |
| Processes | command identity plus separated stdout/stderr | bounded styled streams, exit status, duration, and truncation state |
| Git and patches | status summaries and styled diffs | additions/deletions/hunks remain visually and textually distinct |
| Durable work | tasks, decisions, memories, plans, goals, agents | semantic list/detail cards rather than generic object keys |
| Discovery | tools, repositories, skills, web, MCP, integrations, packs | family-specific counts, identifiers, status, provenance, and bounded previews |
| Lists | tools, sessions, work, research, telemetry, extensions | width-aware tables with intentional columns and explicit empty states |
| Help and completion | stateful help table, inline type-ahead, slash completion, `@skill` completion | fish-style history/command/theme/skill hints rendered with theme-aware dim italic ghost text, Right Arrow acceptance, Tab menus, and grouped help with current settings |
| Choices | labeled option tables, resume picker, free-form fallback | guided numbered choices with exact-ID selection, validation retry, blank cancellation, and slash-command handoff |
| Themes | five built-ins, custom themes, prompt/toolbar/event previews | immutable palettes drive a numbered picker, full semantic preview, active-state table, dynamic completion, safe scaffold output, strict validation, and readable custom-theme search locations |
| Worker mode | same user-facing behavior as embedded mode | normalized plain output is byte-equivalent; terminal styling is semantically equivalent |

`user.ask` is a foreground, turn-scoped interaction: the current agent turn pauses until
the operator answers or leaves the line blank to cancel the question. Its activity line
becomes a stable `user.ask waiting for your answer` state before the input card is drawn. Durable
non-blocking interaction belongs to workflow `wait_for_input`, where the run can remain
waiting while the terminal handles other work.

## Presentation Boundary

`colossus-presentation` owns pure mappings from released typed contracts to a bounded
presentation document. Terminal, plain-text, and JSON backends render that document.
CLI and REPL select a backend and write the result; they do not own model, tool, policy,
repository, workflow, or persistence behavior. Worker IPC continues to transport typed
application contracts rather than terminal markup.

## Acceptance Evidence

- Semantic fixtures cover every built-in tool and normalized run event, with specialized
  source, process, edit/diff, reasoning, failure, work, and context cards.
- Markdown fixtures run at 60, 80, 120, and 160 columns, including Unicode and hostile ANSI,
  OSC, control, invisible, and oversized input.
- Table/card fixtures cover empty, single, many, truncated, long, and multiline values.
- Theme preview fixtures cover all five built-ins at 60, 80, 120, and 160 columns, both
  ANSI-enabled and control-sequence-free, plus the bundled custom-theme example.
- CLI acceptance covers TTY human, redirected automatic JSON, explicit human, and
  explicit JSON behavior.
- The worker smoke suite exercises every slash-command family through authenticated IPC
  and then repeats embedded fallback behavior.
- Existing release, policy, and terminal tests prove credentials, hidden reasoning,
  denied content, and unreleased stream data do not enter renderer output.
