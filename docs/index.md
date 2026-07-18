---
title: Colossus
description: Run auditable AI agents, durable workflows, and trusted extensions from one local-first runtime.
audience: user
type: concept
---

<div class="hero" markdown>

# Colossus

Run auditable AI agents, durable workflows, and policy-controlled tools from one
local-first runtime. Colossus keeps effects, approvals, state, and evidence visible
instead of hiding them behind an opaque assistant.

[Install Colossus](get-started/install.md){ .md-button .md-button--primary }
[Run the five-minute quickstart](get-started/quickstart.md){ .md-button }

</div>

## Why Colossus

<div class="capability-grid" markdown>
<div class="capability-card" markdown>

### Work interactively

Use the terminal UI for repository tasks, streamed responses, approvals, durable
sessions, plans, goals, memories, and subagents. The transcript and current work survive
restart.

</div>
<div class="capability-card" markdown>

### Automate durable work

Author versioned YAML workflows with bounded steps, explicit inputs, schedules,
authenticated webhooks, repository-event triggers, recovery, and audit evidence.

</div>
<div class="capability-card" markdown>

### Control every effect

Tools are visible only when selected and configured. Policy, approval, one-use permits,
sandbox grants, result quarantine, and audit remain separate checks around every
effectful operation.

</div>
</div>

![The real Colossus terminal UI showing a deterministic offline session, user prompt, echo-provider response, and verified completion](assets/screenshots/tui-offline-session.png){ .tui-shot }

## The product mental model

<div class="diagram-scroll" markdown tabindex="0" role="region" aria-label="Product mental model diagram">

```mermaid
flowchart TD
    U["You<br/>CLI or terminal UI"] --> A["Agent run<br/>durable session"]
    W["Workflow trigger"] --> F["Workflow run<br/>pinned definition"]
    A --> M["Model provider"]
    M --> T["Requested effect<br/>optional tool"]
    F --> S["Typed workflow step"]
    S --> D["Deterministic branch<br/>condition or emit"]
    S --> T
    T --> G["Authorization<br/>policy + approval + permit"]
    G --> E["Sandboxed effect"]
    E --> Q["Quarantine and release"]
    Q --> O["Bounded result<br/>to originating run"]
    A --> J["Encrypted journal<br/>audit + recovery"]
    F --> J
```

</div>

Read the diagram from left to right: an interactive agent run and a triggered workflow
run are separate durable run types. Agent runs use a model; workflows may stay entirely
deterministic or execute an agent/tool step. Colossus authorizes and constrains each
requested effect before execution, releases allowed results back to the originating
run, and appends lifecycle evidence to the encrypted journal. Color is decorative—the
labels and arrows carry the meaning.

## Choose your path

<div class="path-grid" markdown>
<div class="path-card" markdown>

### Get started

Install Colossus and complete a deterministic offline run in five minutes.

[Start here](get-started/index.md)

</div>
<div class="path-card" markdown>

### Use Colossus

Run daily repository work with the terminal UI, durable sessions, goals, and research.

[Follow a user guide](use/index.md)

</div>
<div class="path-card" markdown>

### Automate and extend

Build workflows, skills, integrations, MCP connections, packs, and collections.

[Build an extension](extend/index.md)

</div>
<div class="path-card" markdown>

### Administer and secure

Configure providers, access, policy, sandboxing, storage, audit, and offline operation.

[Operate Colossus](admin/index.md)

</div>
<div class="path-card" markdown>

### Reference

Look up exact commands, keys, fields, schemas, formats, and limits.

[Open Reference](reference/index.md)

</div>
<div class="path-card" markdown>

### Develop

Set up the source tree and understand runtime, security, state, and presentation
boundaries.

[Contribute to Colossus](develop/index.md)

</div>
</div>

For migration guidance, see
[Upgrade and compatibility](get-started/upgrade-compatibility.md).
