---
title: Colossus
description: Run auditable AI agents, durable workflows, and trusted extensions from one local-first runtime.
audience: user
type: concept
hide:
  - navigation
---

<div class="home-page" markdown>

<div class="home-hero" markdown>

<div class="home-hero__copy" markdown>

# Colossus

<p class="home-lede">Run auditable AI agents, durable workflows, and
policy-controlled tools from one local-first runtime. Effects, approvals, state, and
evidence stay visible.</p>

<div class="home-actions" markdown>

[Run the five-minute quickstart](get-started/quickstart.md){ .md-button .md-button--primary }
[Install Colossus](get-started/install.md){ .md-button }

</div>

</div>

</div>

<figure class="home-proof" tabindex="0">
  <img
    class="home-proof__image"
    src="assets/screenshots/tui-offline-session.png"
    alt="Colossus terminal showing a completed offline prompt, composer, approval mode, and successful status"
  >
</figure>

<section class="home-outcomes" aria-labelledby="why-colossus" markdown>

## Why Colossus { .home-visually-hidden }

<div class="home-outcome-grid" markdown>
<div class="home-outcome" markdown>

:lucide-message-square:{ .home-outcome__icon }

### Work interactively

Use the terminal UI for conversational control with full visibility into every step.

[Use the terminal UI](use/terminal-ui.md)

</div>
<div class="home-outcome" markdown>

:lucide-network:{ .home-outcome__icon }

### Automate durable work

Author versioned YAML workflows with retries, checkpoints, and approvals.

[Build your first workflow](extend/workflows/first-workflow.md)

</div>
<div class="home-outcome" markdown>

:lucide-shield-check:{ .home-outcome__icon }

### Control every effect

Inspect every effect and choose acknowledged full host access or an explicit
least-privilege boundary for tools, data, network, and execution.

[Understand access and approvals](admin/access-and-approvals.md)

</div>
</div>

</section>

<section class="home-audiences" aria-labelledby="find-your-path" markdown>

## Find your path

<p class="home-section-intro">Documentation organized around your role and the outcome
you need.</p>

<div class="home-audience-grid" markdown>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[User](get-started/index.md)</p>

Install Colossus and complete your first task.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Operator](admin/index.md)</p>

Run, monitor, and secure workloads.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Extension author](extend/index.md)</p>

Build workflows, skills, and integrations.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Developer](develop/index.md)</p>

Work with the source tree and architecture.

</div>
</div>

<p class="home-reference-link" markdown>Looking for exact commands, fields, schemas, formats,
or limits? [Open the Reference](reference/index.md).</p>

</section>

<section class="home-mental-model" aria-labelledby="the-product-mental-model" markdown>

## The product mental model

<div class="diagram-scroll" markdown tabindex="0" role="region" aria-label="Product mental model diagram">

```mermaid
flowchart LR
    U["You<br/>CLI or terminal UI"] --> A["Agent run<br/>durable session"]
    W["Workflow trigger"] --> F["Workflow run<br/>pinned definition"]
    A --> M["Model provider"]
    M --> T["Requested effect<br/>optional tool"]
    F --> S["Typed workflow step"]
    S --> D["Deterministic branch<br/>condition or emit"]
    S --> T
    T --> G["Authorization<br/>policy + approval + permit"]
    G --> E["Authorized effect<br/>selected execution boundary"]
    E --> Q["Quarantine and release"]
    Q --> O["Bounded result<br/>to originating run"]
    A --> J["Hash-chained journal<br/>optional encryption"]
    F --> J
```

</div>

Interactive agent runs and triggered workflow runs are separate durable run types.
Colossus authorizes and constrains each requested effect before execution, releases an
allowed result only to its originating run, and records lifecycle evidence in the
hash-chained journal. Configured platform or environment keys add authenticated payload
encryption. The labels and arrows carry the meaning; color is decorative.

</section>

<p class="home-compatibility" markdown>Moving from an earlier release? See
[Upgrade and compatibility](get-started/upgrade-compatibility.md).</p>

</div>
