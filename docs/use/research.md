---
title: Research
description: Ask a source-backed question in the terminal and inspect its durable report.
audience: user
type: how-to
icon: lucide/search
---

# Research

Use Research Mode when you want Colossus to investigate a question and leave a cited
report you can inspect later. A research run plans bounded queries, gathers released
evidence, extracts claims, and records limitations alongside its report. The run and
its sources are durable; the terminal's current mode is not.

## Ask a question in the terminal

For one question, enter it directly as a command:

```text
/research How does Colossus recover an interrupted workflow?
```

This starts a research run in the current session without changing the terminal's
mode. To ask several research questions in a row, enter Research Mode first:

```text
/research on
```

Then submit an ordinary prompt, for example:

> Which parts of this repository enforce approval before a tool effect?

While Research Mode is active, ordinary prompts go to the research service rather
than the usual agent turn. Use `/research status` to check the current mode, and
`/research off` to return to Execute Mode. The bare `/research` command toggles
between Research and Execute modes. The mode resets when the TUI restarts.

Research Mode does not accept image attachments. If images are queued in the
composer, use `/detach all` before submitting a research question.

## Know what the terminal will search

The terminal route uses **standard** depth and selects repository, web, and configured
MCP evidence lanes. Colossus bounds collection in each lane and records unavailable,
denied, failed, or skipped attempts as limitations. Web and MCP collection require
their own configured routes and permissions; the question cannot choose a provider or
grant itself access.

Research produces a source-backed report, not just search results. Its stable source
labels, such as `R1`, tie material claims to the evidence Colossus released. The
final report is also appended to the session conversation.

## Find the report and its evidence

Inside the TUI, list the research runs belonging to the current session:

```text
/research list
```

Copy a run ID from the list. The CLI can show the full report, source labels, and
extracted claims:

```bash
colossus research show RESEARCH_RUN_ID
colossus research sources RESEARCH_RUN_ID
colossus research claims RESEARCH_RUN_ID
```

`/research status` reports the **terminal mode**; `research show` reports the
**durable run status**. When reviewing a report, read its limitations as well as its
citations. A missing lane may leave a useful partial answer, but it does not count
as evidence for a claim that depends on that lane.

If Colossus stops during research, the run is marked **interrupted** on recovery and
is not retried automatically. Inspect the recorded run and any external effects
before starting another question.

## Choose a narrower scope from the CLI

Use a shell command when you need to choose depth or evidence lanes explicitly. For
example, keep a repository investigation in the same session:

```bash
colossus --approval-mode ask research run \
  "How does effect authorization work?" \
  --session SESSION_ID --source repo --depth standard
```

The CLI accepts `quick`, `standard`, or `deep` depth and any selected combination of
`repo`, `web`, and `mcp` sources. A run without `--session` creates a fresh session.
See [Deep research](deep-research.md) for lane setup, bounds, and the complete
investigation workflow.

## What's next?

<div class="grid cards" markdown>

-   :lucide-book-open:{ .lg .middle } **Deep research**

    ---

    Set the depth and evidence lanes for a durable, cited report.

    [Explore deep research :lucide-arrow-right:](deep-research.md)

-   :lucide-search:{ .lg .middle } **Web search**

    ---

    Run one direct search when you need results and snippets without a research run.

    [Run a web search :lucide-arrow-right:](web-search.md)

</div>
