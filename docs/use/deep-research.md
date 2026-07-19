---
title: Deep research
description: Produce a durable, cited report from repository, web, and configured MCP evidence.
audience: user
type: how-to
---

# Deep research

## Goal

Turn a research question into a durable report whose sources, claims, progress, and
limitations remain available after the run ends.

## Prerequisites

- For repository evidence, a readable repository root.
- For web evidence, an operator-configured `research` search role.
- For MCP evidence, an explicitly configured and allowed research tool.

Operators own model and search setup in
[Providers and routing](../admin/providers-routing.md). MCP setup lives in
[MCP](../extend/mcp.md).

## Steps

### 1. Choose the depth and evidence lanes

Depth controls how broadly Colossus plans:

| Depth | Use it for |
| --- | --- |
| `quick` | A narrow question or fast first pass |
| `standard` | Most repository investigations |
| `deep` | A broader question that needs several evidence angles |

Choose one or more explicit lanes with `--source`:

- `repo` searches the active repository through read-only effects.
- `web` uses the configured `research` search role.
- `mcp` calls configured MCP research tools.

Explicit lanes make the run reproducible. Exact depth budgets, defaults, and bounds
remain in the [CLI reference](../reference/cli.md#important-defaults-and-bounds) and
[Configuration fields](../reference/configuration.md#context-memory-and-research-defaults).

A capable model route improves planning, claim extraction, and synthesis. When a
research model step is unavailable or returns invalid output, Colossus records the
fallback and continues deterministically.

### 2. Start with repository evidence

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  research run \
  "How does effect authorization work?" \
  --source repo --depth standard
```

The `development` access profile conservatively classifies `research.run` as
approval-required even when only the repository lane is selected. The global
`--approval-mode ask` option lets the noninteractive command request that approval.

Colossus then plans bounded queries, collects released repository evidence, extracts
source-backed claims, and synthesizes a cited report. A fresh session is created when
`--session` is omitted.

The terminal UI exposes a fixed research route. Use the CLI when you need explicit depth
or lane control; the exact TUI contract is in
[TUI commands and keys](../reference/tui.md#commands).

### 3. Add web or MCP evidence deliberately

After an operator configures the required route, add only the lanes the question needs:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  research run \
  "Compare the repository design with its published security claims" \
  --source repo,web --depth deep
```

Configuration selects every backend; neither the model nor the question chooses a
provider. Colossus bounds planned queries and collected results. Selected web and MCP
effects must pass access, policy, any approval obligations, and sandbox checks before
dispatch.

For a direct route check before a research run, use
[Web search](web-search.md).

### 4. Inspect the durable record

```bash
colossus --config .colossus/config.yaml research list
colossus --config .colossus/config.yaml research show RESEARCH_RUN_ID
colossus --config .colossus/config.yaml research sources RESEARCH_RUN_ID
colossus --config .colossus/config.yaml research claims RESEARCH_RUN_ID
```

`research show` includes the selected lanes, planned queries, progress, limitations,
report, and terminal status. Sources use stable citation labels such as `R1`; extracted
claims point back to those labels. The final report is also appended to the owning
session as an assistant message.

## Expected result

The run completes with a cited Markdown report and canonical source and claim records.
An unavailable, denied, failed, or budget-skipped collection attempt is recorded as a
limitation while Colossus continues with released evidence.

## Verification

Open `research sources` and `research claims`. Confirm that every material report claim
uses a released source label and that `research show` carries any incomplete lane into
the limitations. Restart Colossus and show the run again to confirm the record remains
available.

## Failure path

- **The run is denied before collection:** review `research.run` in
  [Access and approvals](../admin/access-and-approvals.md); approval cannot override a
  deny.
- **Repository collection releases no sources:** confirm Colossus started in the intended
  repository and that its read grant includes that root.
- **A selected web lane is disabled:** configure the exact `research` search role; search
  roles do not fall back to one another.
- **A selected MCP lane is disabled:** configure at least one MCP research template and
  its exact tool allowlist.
- **A collection attempt is skipped:** narrow the depth or lane set, or ask an operator
  to review the configured research bounds.
- **A process stops mid-run:** the run becomes `interrupted` at recovery and is not
  retried automatically. Inspect its recorded effects before starting a deliberate new
  run.

## Next step

Use [Web search](web-search.md) when you need normalized search results without a durable
research workflow. Preserve reusable, non-secret conclusions separately with
[Memories](memories.md).
