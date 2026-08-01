---
title: Research and search
description: Choose between a durable deep-research run and a direct provider-neutral web search.
audience: user
type: concept
---

# Research and search

<span id="goal"></span>

Colossus offers two related paths for finding evidence. Use **deep research** when the
outcome is a durable, cited report. Use **web search** when you need one bounded set of
normalized search results.

<span id="prerequisites"></span>
<span id="steps"></span>

## Choose the right path

| Need | Deep research | Web search |
| --- | --- | --- |
| Primary outcome | Cited report with durable evidence | Ranked result metadata and snippets |
| Scope | One or more planned queries | One explicit query |
| Evidence | Repository, web, configured MCP tools, or a selected combination | Configured search provider only |
| Durable records | Run, progress, sources, claims, limitations, and report | No research run or research records |
| Best starting point | [Run deep research](deep-research.md) | [Run a web search](web-search.md) |

Both paths keep provider choice in operator-owned configuration. Models and user prompts
never select a backend. Access, policy, approval, exact-origin sandboxing, quarantine,
and post-effect release still apply before network results reach the caller.

## How they fit together

<span id="1-run-repository-only-research"></span>
<span id="2-inspect-evidence-and-claims"></span>
<span id="4-run-research-with-selected-lanes"></span>

A deep-research run plans bounded queries, collects evidence, extracts claims, and
synthesizes a report. When that run includes the `web` lane, it calls the same
provider-neutral search subsystem described in [Web search](web-search.md), using the
independent `research` logical route.

<span id="3-diagnose-a-web-route-directly"></span>
<span id="search-routing"></span>

A direct search stops after returning normalized result metadata. It does not create a
session, preserve sources, extract claims, or produce a report. Fetching a selected
result page is also a separate authorized effect.

<span id="expected-result"></span>
<span id="verification"></span>
<span id="failure-path"></span>
<span id="next-step"></span>

## Where configuration lives

Operators configure model and search routes in
[Providers and routing](../admin/providers-routing.md#configure-search-routing).
Exhaustive fields belong to
[Search configuration](../reference/configuration/search.md), while exact command
options and output contracts belong to the [CLI reference](../reference/cli.md).
