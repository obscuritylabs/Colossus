---
title: Deep research
description: Investigate a question with bounded evidence collection and inspect the resulting cited report.
audience: user
type: how-to
icon: lucide/book-open
---

# Deep research

Deep research turns a question into a durable report with a trail back to its
evidence. Colossus plans bounded queries, collects sources from the lanes you select,
extracts source-backed claims, and saves the report, progress, and limitations in a
research run. Use it when an answer needs more than one search result or must remain
inspectable after the session ends.

## Run a focused investigation

Start with repository evidence when the question is about the code or its docs:

```bash
colossus -w /absolute/path/to/repository --approval-mode ask \
  research run "How does effect authorization work in this repository?" \
  --source repo --depth standard
```

The result includes a **research run ID**, a **session ID**, and the report. Without
`--session SESSION_ID`, Colossus creates a new session for the run. Add that option
when the investigation belongs to an existing conversation. The one-shot command
uses `--approval-mode ask` so an approval obligation can be answered in the attached
terminal; policy can still deny an effect.

In the Terminal UI, `/research QUESTION` starts a run in the current session. That
route uses standard depth and requests repository, web, and MCP lanes. See
[Research in the terminal](research.md) for its mode and commands. Use the CLI when
you want to choose the depth or evidence lanes yourself.

## Choose the depth and evidence

| Depth | Good starting point |
| --- | --- |
| `quick` | A narrow question or a first pass. |
| `standard` | A focused investigation across a few queries. |
| `deep` | A broader question that needs more angles. |

Select only the evidence lanes the question needs:

| Lane | What Colossus collects | What must be available |
| --- | --- | --- |
| `repo` | Bounded, read-only evidence from the selected workspace. | Access to that repository. |
| `web` | Normalized results and snippets from planned searches; it does not fetch full pages. | A configured `research` search route. |
| `mcp` | Results from enabled MCP tools, with optional research projections. | A tool-capable research model or explicit projections, plus MCP access. |

For example, compare repository behavior with published information using two lanes:

```bash
colossus -w /absolute/path/to/repository --approval-mode ask \
  research run "How does the implementation compare with its published security claims?" \
  --source repo,web --depth deep
```

The CLI defaults to all three lanes if `--source` is omitted. Each planned query and
selected lane is a potential collection attempt. The default `maxWorkers` budget is
four attempts for the whole run, so a broad question across all three lanes may leave
some attempts **skipped**. Depth expands the query plan; it does not override that
budget. See [research limits and evidence lanes](../reference/configuration/context-memory-research.md#research-configuration)
for the exact bounds and setup, [Search](web-search.md) for the web route, and
[MCP research templates](../reference/configuration/mcp.md#research-templates) for
MCP setup.

## Use Desktop or VS Code

In Desktop, select a Managed Local workspace, choose **Research** in the composer,
and open **Sources** to choose depth and evidence. The **MCP connections** checkbox
selects the MCP evidence lane. Research starts with **This Workspace** evidence only.
Research inherits each server's **Allowed tools** selection and chooses relevant
calls using the live schemas. Optional **Global settings → MCP → Edit → Advanced
settings → Research tool projections** override that server's inherited selection.
Each projection names an allowed tool and its arguments, with `{query}` where the
research query belongs.
See [MCP research templates](../reference/configuration/mcp.md#research-templates)
for the exact shape. Desktop's External targets do not expose Research controls.

In VS Code, choose **Research** in the composer, then select **Research depth** and
**Evidence sources → MCP connections**. Configure the separately enrolled worker's
MCP servers and allowed tools, with optional research projections; the extension does
not edit runtime configuration.
The application enrollment must allow `filesystem.search`, `web.search`, or `mcp.call`
for the workspace, Web, or MCP lane respectively. See the
[application enrollment](../admin/storage-worker.md#first-application-enrollment)
for worker access grants.
The Research option is disabled when the connected worker does not advertise support.

Selecting **MCP connections** requests evidence from the normally enabled MCP tools,
unless a server has explicit research projections. Excluded tools remain unavailable.
Automatic selection needs a `research_worker` model with tool-call support; explicit
projections work without that model. Both clients require at least one selected source
and keep the report in the conversation.

## Follow the report back to its sources

List runs, then use the ID from the listing to inspect one report and its evidence:

```bash
colossus -w /absolute/path/to/repository research list
colossus -w /absolute/path/to/repository research show RESEARCH_RUN_ID
colossus -w /absolute/path/to/repository research sources RESEARCH_RUN_ID
colossus -w /absolute/path/to/repository research claims RESEARCH_RUN_ID
```

`research show` contains the question, depth, requested lanes, status, planned
queries, per-lane outcomes, progress, limitations, and final Markdown report.
`research sources` shows the released evidence, each with a stable label such as
`R1`, its origin, and the query that found it. `research claims` ties extracted
statements to those labels. A shortened report might read:

```markdown
## Findings

- The runtime checks authority before an effect reaches an adapter [R1].

## Sources

- [R1] Security architecture — docs/develop/security-architecture.md
```

The label `[R1]` lets you find the underlying source; it is traceability, not a
guarantee that the source is correct. Review the source itself before relying on
a consequential claim. The completed report is also appended to its session
conversation.

## Read incomplete results carefully

A run can finish with a useful report even when one lane was unavailable, denied,
failed, or skipped by a bound. Check **limitations** and the per-lane outcomes in
`research show` before treating the report as a complete answer. In particular, web
evidence contains search snippets; follow a source URL separately when its full
context matters.

| If you see | Check next |
| --- | --- |
| No repository sources | Confirm `-w` points to the intended repository and its read access is allowed. |
| Web or MCP lane disabled | Configure the exact research route or MCP template; a question cannot select a backend or grant access. |
| Denied lane | Review access, approval, and sandbox settings; approval cannot override a policy denial. |
| Skipped lane | Narrow the question or lane set, or review the configured worker and source bounds. |
| Interrupted status | Inspect the saved progress and any external effects before deliberately starting a new run. |

An interrupted run is preserved after process recovery and is **not automatically
retried or resumed**. Model-assisted planning, claim extraction, and synthesis can
also fall back to deterministic behavior when unavailable or invalid; `progress`
records those fallbacks. Each collection effect still passes the normal access,
approval, policy, and sandbox checks.

## What's next?

<div class="grid cards" markdown>

-   :lucide-search:{ .lg .middle } **Search**

    ---

    Run one direct web query when you need results and snippets without a research run.

    [Explore search :lucide-arrow-right:](web-search.md)

-   :lucide-terminal:{ .lg .middle } **Research in the terminal**

    ---

    Ask a question with `/research` and find its run from the Terminal UI.

    [Use Research Mode :lucide-arrow-right:](research.md)

</div>
