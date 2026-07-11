# User Guide

This guide covers the active Rust CLI and REPL. All examples assume an installed
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

`run` prints a stable JSON result. `--stream` writes policy-released deltas and events to
stderr while preserving JSON on stdout. Every run creates or attaches to a durable
session and records the provider/effect lifecycle in the encrypted journal.

Global approval mode precedes the subcommand:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  run "Write the approved file"
```

`deny` is the noninteractive default. `ask`, `risk-auto`, and `full-access` satisfy only
approval obligations; none can override a policy deny or add sandbox capabilities.

## REPL

```bash
colossus --config .colossus/config.yaml repl
colossus --config .colossus/config.yaml repl --resume
colossus --config .colossus/config.yaml repl --session SESSION_ID
```

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

Presentation is local durable state, not model context:

```text
/theme
/theme preview ocean
/theme save ocean
/events compact
/stream on
/reasoning off
/transcript compact
/multiline toggle
/repl save
```

Redirected output remains control-sequence-free. REPL history and preferences are
encrypted journal records.

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

colossus --config .colossus/config.yaml goals run \
  "Complete the approved plan" --session SESSION_ID --max-iterations 5
colossus --config .colossus/config.yaml goals show GOAL_ID

colossus --config .colossus/config.yaml agents queue SESSION_ID \
  "Review the storage adapter"
colossus --config .colossus/config.yaml agents drain
```

Goal iterations and subagent turns reuse the ordinary provider, tool, policy, context,
and journal services. Interrupted effects become unknown; they are never silently
replayed.

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

CLI and REPL operations auto-discover a healthy worker. Authentication or protocol
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
