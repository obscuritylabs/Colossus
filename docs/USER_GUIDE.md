# User Guide

This guide covers the active Rust CLI and terminal UI. All examples assume an installed
`colossus` binary and a strict `.colossus/config.yaml` initialized for the current
repository.

## One-Shot Agent Runs

```bash
colossus --config .colossus/config.yaml run "Summarize this repository"
colossus --config .colossus/config.yaml run --stream \
  "Inspect the active tool surface"
colossus --config .colossus/config.yaml run --max-turns 12 \
  "Implement and verify the requested change"
colossus --config .colossus/config.yaml run --role research_worker \
  "Collect repository evidence"
```

On a terminal, `run` renders a human response card with Markdown. When stdout is piped or
redirected, it emits the stable JSON result used by automation. `--stream` writes
policy-released deltas and events to stderr while preserving the selected result format
on stdout. Every run creates or attaches to a durable session and records the
provider/effect lifecycle in the encrypted journal.

Override output selection globally when needed:

```bash
colossus --output human tools list
colossus --output json sessions list | jq .
```

Global approval mode precedes the subcommand:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  run "Write the approved file"
```

`deny` is the noninteractive default. `ask`, `risk-auto`, and `full-access` satisfy only
approval obligations; none can override a policy deny or add sandbox capabilities.
`risk-auto` reviews eligible `shell.run` requests and auto-proves only a strict
low-risk/allow result; all other assessments require a prompt.

## Terminal UI

```bash
colossus --config .colossus/config.yaml
colossus --config .colossus/config.yaml tui --resume
colossus --config .colossus/config.yaml tui --session SESSION_ID
colossus --config .colossus/config.yaml --no-alt-screen tui
```

The removed `colossus repl` alias is no longer accepted. The default alternate screen
protects native scrollback; `--no-alt-screen` uses an inline viewport, and Zellij selects
inline mode automatically.

Useful commands include:

```text
/help
/session show
/resume
/work
/tools
/context status
/tasks
/decisions
/plans
/goals
/agents
/memories
/research QUESTION
/workflow list
/audit verify
/exit
```

`/resume` opens a focused session overlay. Enter a listed number or exact ID, or press
Esc/submit a blank answer to cancel. The durable transcript remains scrollable above a
pinned composer and stable width-aware footer; narrower terminals hide optional footer
fields instead of moving or overwriting input.

The TUI displays fish-style inline type-ahead from prior prompts, slash commands, theme
names, and installed `@skills`. Suggestions use each theme's low-emphasis color plus dim
italic styling so typed text and the cursor position remain clear. Press Right Arrow to
accept the full suggestion, or press Tab to advance matching commands, themes, and skills.
The `/session resume` command opens the same picker as `/resume`; unknown slash commands
stay inside the terminal command parser and are never sent to the model.

When the agent uses `user.ask` or policy requires approval, a modal takes focus without
discarding the current draft. Type an answer and press Enter, choose an exact option, or
press Esc/submit a blank answer to fail closed. The run resumes after the one-use answer
returns through the embedded or authenticated-worker bridge. Use workflow
`wait_for_input` when a run must wait durably without an attached interactive client.

`/help` is grouped by task, and Tab completes slash commands and installed `@skill`
names. Put one or more known skill names at the beginning of a message to apply them to
that turn, for example `@repo-review Review the current changes`.

The composer remains usable during a run and accepts up to eight queued turns. Successful
completion starts the next turn; failure or cancellation pauses the queue for operator
confirmation. PageUp/PageDown scroll the transcript, End returns to live output, Ctrl-R
searches encrypted history, Ctrl-C clears a draft/cancels a modal/requests cooperative run
cancellation, and Ctrl-D exits only while idle with an empty draft. Interactive
`--output json` is rejected; redirect stdin to use the compatible line runner and JSON.

Presentation is local durable state, not model context:

```text
/theme
/theme list
/theme hacker
/theme preview high_contrast
/theme validate
/theme scaffold night_sky
/theme reset
/events compact
/stream on
/reasoning off
/transcript compact
/multiline toggle
/tui save
```

Normal `stream=on` responses are buffered and rendered as Markdown; `stream=raw` emits
provider text immediately, and `stream=off` suppresses intermediate stream output.
Redirected output remains control-sequence-free. Terminal history and preferences are
encrypted journal records.

`/theme` opens a numbered picker. Enter a number or theme name to apply and save it,
enter `p NUMBER` to inspect the complete prompt/Markdown/tool/approval/error/diff sample,
or leave the line blank to cancel. `/theme NAME` is the direct form and also saves
immediately; a separate save command is unnecessary.

Custom themes are strict JSON or TOML files. `/theme scaffold NAME` prints a validated
TOML starter and its suggested config-adjacent path without writing a file from the
terminal interface. The bundled [Ocean example](../examples/themes/ocean.toml) can also
be copied into `.colossus/themes/`. Restart Colossus after adding a file, then run
`/theme validate` before selecting it. Theme loading rejects unknown fields, invalid
colors, oversized libraries, symlinks, duplicate names, and built-in name collisions.

## Sessions And Context

```bash
colossus --config .colossus/config.yaml sessions list
colossus --config .colossus/config.yaml sessions show SESSION_ID
colossus --config .colossus/config.yaml sessions messages SESSION_ID
colossus --config .colossus/config.yaml sessions new "Release review"
colossus --config .colossus/config.yaml run --resume "Continue"
```

Context snapshots never delete canonical messages:

```bash
colossus --config .colossus/config.yaml context status SESSION_ID
colossus --config .colossus/config.yaml context compact SESSION_ID
colossus --config .colossus/config.yaml context list SESSION_ID
colossus --config .colossus/config.yaml context restore SESSION_ID SNAPSHOT_ID
```

Automatic compaction preserves a configured recent tail, injects active decisions and
relevant memories, and uses the `context_summarizer` role when available. Invalid or
unavailable summaries fall back deterministically while raw history remains intact.

## Durable Work

Tasks, decisions, plans, goals, and subagents are canonical event-sourced records:

```bash
colossus --config .colossus/config.yaml tasks create SESSION_ID \
  "Run release gates" --description "Capture the outputs"
colossus --config .colossus/config.yaml tasks list --session SESSION_ID

colossus --config .colossus/config.yaml decisions create SESSION_ID \
  "Storage authority" "The encrypted journal is authoritative" --priority high
colossus --config .colossus/config.yaml decisions list --session SESSION_ID

colossus --config .colossus/config.yaml plans create SESSION_ID \
  "Cut a release" --step "Run gates" --step "Build archives"
colossus --config .colossus/config.yaml --approval-mode ask plans approve PLAN_ID
colossus --config .colossus/config.yaml run --execute-plan PLAN_ID

colossus --config .colossus/config.yaml run --plan --session SESSION_ID \
  "Plan the Rust cutover without changing the workspace"

colossus --config .colossus/config.yaml run --execute-plan PLAN_ID \
  --goal --goal-max-iterations 5
colossus --config .colossus/config.yaml goals show GOAL_ID

colossus --config .colossus/config.yaml agents queue SESSION_ID \
  "Review the storage adapter"
colossus --config .colossus/config.yaml agents drain
```

Goal iterations and subagent turns reuse the ordinary provider, tool, policy, context,
and journal services. Interrupted effects become unknown; they are never silently
replayed.

When a model calls `agent.delegate`, Colossus wakes the bounded scheduler immediately.
The child runs through the configured `subagent_default` role, and the parent can read
its completed output with `agent.result` before answering. Manually queued jobs continue
to wait for `agents drain` or a worker drain. A queued/running result is displayed as
pending rather than failed.

## Memories

```bash
colossus --config .colossus/config.yaml memories create \
  "This repository requires warnings-as-errors Clippy" --scope repository \
  --scope-id REPOSITORY_ID --kind constraint
colossus --config .colossus/config.yaml memories search "Clippy" \
  --repository REPOSITORY_ID
colossus --config .colossus/config.yaml memories index status
colossus --config .colossus/config.yaml memories archive MEMORY_ID
```

The journal owns lifecycle state. Tantivy and optional Chroma return disposable
candidates; Colossus reloads canonical records and reapplies status, expiry, scope, and
policy before release. Store no secret values in memory text.

## Skills And Packs

```bash
colossus --config .colossus/config.yaml skills list
colossus --config .colossus/config.yaml skills show coding
colossus --config .colossus/config.yaml skills compose \
  "Implement this" --skill coding
colossus --config .colossus/config.yaml run --skill coding \
  "Implement the approved change"
```

Authorable installed skills use guarded operations:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  skills scaffold release-checklist "Review a native release" \
  --resource-dir references
colossus --config .colossus/config.yaml skills inspect release-checklist
colossus --config .colossus/config.yaml skills validate release-checklist
```

Skills are data-only instructions and resources. Executables belong in verified packs:

```bash
colossus --config .colossus/config.yaml packs verify ./pack
colossus --config .colossus/config.yaml --approval-mode ask packs install ./pack
colossus --config .colossus/config.yaml packs list
```

Signed release bundles can be verified and installed without network access:

```bash
colossus --config .colossus/config.yaml bundle verify ./bundle
colossus --config .colossus/config.yaml --approval-mode ask bundle install \
  ./bundle --prefix "$HOME/.local"
```

## Research

```bash
colossus --config .colossus/config.yaml research run \
  "How does effect authorization work?" --source repo --depth standard
colossus --config .colossus/config.yaml research list
colossus --config .colossus/config.yaml research show RESEARCH_RUN_ID
colossus --config .colossus/config.yaml research sources RESEARCH_RUN_ID
colossus --config .colossus/config.yaml research claims RESEARCH_RUN_ID
```

Repository evidence works offline. Web and MCP lanes run only when explicitly configured,
policy-allowed, and post-effect released; unavailable lanes become durable limitations.

## Workflows

```bash
colossus --config .colossus/config.yaml workflow validate \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow register \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow run release 1.0.0 \
  --inputs '{"branch":"main"}'
colossus --config .colossus/config.yaml workflow status WORKFLOW_RUN_ID
```

Definitions are exact-content hash pinned. A changed file is a new trust identity.
Effectful retries require an explicit idempotency strategy; recovery records abandoned
attempts as interrupted or unknown instead of rerunning them.

## Integrations And MCP

```bash
colossus --config .colossus/config.yaml integrations list
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect github --credential-reference env:GITHUB_TOKEN
colossus --config .colossus/config.yaml integrations show github
colossus --config .colossus/config.yaml integrations call \
  github.repository.get '{"owner":"example","repo":"project"}'

colossus --config .colossus/config.yaml mcp servers
colossus --config .colossus/config.yaml mcp tools --server local-docs
colossus --config .colossus/config.yaml mcp call \
  local-docs search_docs '{"query":"authorization"}'
```

Connections remain hidden from model tools until canonical connect events exist.
Credentials are references, resolved after authorization, and removed from quarantined
results before release.

## Worker

The optional worker owns the redb writer lease and serves authenticated local IPC:

```bash
colossus --config .colossus/config.yaml worker
colossus --config .colossus/config.yaml worker --status
colossus --config .colossus/config.yaml worker --shutdown
colossus --config .colossus/config.yaml worker --once
```

CLI and TUI operations auto-discover a healthy worker. Authentication or protocol
failure is surfaced and never downgraded to embedded execution; only an unavailable
endpoint permits embedded fallback.

## Audit And Diagnostics

```bash
colossus --config .colossus/config.yaml audit verify
colossus --config .colossus/config.yaml audit show --limit 20
colossus --config .colossus/config.yaml audit anchor-status
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
colossus --config .colossus/config.yaml projection status
colossus --config .colossus/config.yaml telemetry runs
```

Audit commands expose bounded redacted envelope evidence, never decrypted payload bodies.
Startup verification failure puts the runtime into read-only recovery mode and blocks new
effects.
