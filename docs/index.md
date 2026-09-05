---
title: Colossus
description: Operate auditable, sandboxed AI agents in online and offline mission environments.
audience: user
type: concept
hide:
  - navigation
---

<div class="home-page" markdown>

<section class="home-hero" markdown>

<div class="home-hero__copy" markdown>

<p class="home-eyebrow">COLOSSUS · ALPHA</p>

# Agent operations for mission environments

<p class="home-lede">Colossus is an alpha-stage runtime for organizations that need
AI agents to perform real work under explicit authority. Policy, approvals, sandbox
boundaries, durable state, and audit evidence stay part of the operation whether the
system is connected, local, or offline.</p>

<div class="home-actions" markdown>

[Start offline](get-started/quickstart.md){ .md-button .md-button--primary }
[Read the security model](develop/security-architecture.md){ .md-button }

</div>

<p class="home-alpha" markdown>Alpha software means change is expected. Review
[upgrade and compatibility guidance](get-started/upgrade-compatibility.md) before
updating a deployment you depend on.</p>

</div>

</section>

<figure class="home-proof" tabindex="0">
  <img
    class="home-proof__image"
    src="assets/screenshots/tui-offline-session.png"
    alt="Current Colossus terminal launch rail showing workspace, model route, sandbox profile, approval mode, security posture, composer, and readiness"
  >
  <figcaption class="home-proof__caption">The current terminal launch rail names the
  workspace, model route, sandbox profile, approval mode, and security posture before
  work begins.</figcaption>
</figure>

<section class="home-outcomes" aria-labelledby="operational-assurance" markdown>

## Operational assurance, not hidden autonomy { #operational-assurance }

<p class="home-section-intro">Colossus is being built for enterprise, public-sector,
regulated, and disconnected environments where an agent action must be reviewable,
bounded, and recoverable.</p>

<div class="home-outcome-grid" markdown>
<div class="home-outcome" markdown>

:lucide-file-check-2:{ .home-outcome__icon }

### Account for every effect

Requested actions, policy decisions, approvals, execution, release, and uncertain
outcomes produce durable evidence in a hash-chained journal.

[Audit and recovery](admin/audit-telemetry-recovery.md)

</div>
<div class="home-outcome" markdown>

:lucide-shield-check:{ .home-outcome__icon }

### Enforce the boundary

Strict schemas, one-use permits, sandbox profiles, resource ceilings, and quarantined
output keep authority explicit from request through release.

[Sandbox and isolation](admin/sandbox.md)

</div>
<div class="home-outcome" markdown>

:lucide-unplug:{ .home-outcome__icon }

### Operate online or offline

Use hosted or local models, run a credential-free offline proof, and prepare controlled
or air-gapped deployments without changing the core authorization path.

[Offline operation](admin/offline-airgap.md)

</div>
</div>

</section>

<section class="home-mental-model" aria-labelledby="how-colossus-governs-work" markdown>

## How Colossus governs work

<div class="diagram-scroll diagram-scroll--wide" markdown tabindex="0" role="region" aria-label="Colossus governed work flow diagram">

```mermaid
flowchart LR
    I["CLI · TUI · Desktop · SDK"] --> R["Durable runs<br/>agent · workflow · research"]
    R --> C["Capability catalog<br/>strict tool schemas"]
    C --> K["Safety Kernel<br/>policy · approval · one-use permit"]
    K --> B["Execution boundary<br/>native · OCI · Windows · external"]
    B --> A["Effect adapters<br/>models · files · processes · network · integrations"]
    A --> Q["Quarantine and release<br/>bounded result"]
    Q --> R
    R --> J["Hash-chained journal<br/>audit · recovery · projections"]
    K --> J
    B --> J
```

</div>

Interfaces start durable work but do not own model, policy, tool, or state behavior.
The runtime validates a requested capability, authorizes one exact effect, binds it to
an execution boundary, and releases bounded output only after the required checks.
Lifecycle evidence remains available for audit and recovery.

[Explore the architecture](develop/architecture.md) or
[inspect the effect lifecycle](develop/security-architecture.md).

</section>

<section class="home-audiences" aria-labelledby="choose-your-path" markdown>

## Choose your path

<p class="home-section-intro">Start with the operational outcome you need.</p>

<div class="home-audience-grid" markdown>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Evaluate Colossus](get-started/index.md)</p>

Install it and prove the runtime offline before adding credentials.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Secure a deployment](admin/index.md)</p>

Configure identity, access, isolation, storage, policy, and audit.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Connect your systems](extend/index.md)</p>

Add workflows, enterprise integrations, standalone MCP servers, and OCI-distributed Agent Plugins.

</div>
<div class="home-audience-card" markdown>

<p class="home-audience-card__title" markdown>[Build with Colossus](develop/index.md)</p>

Use the application SDKs or work on the Rust runtime and Desktop.

</div>
</div>

<p class="home-reference-link" markdown>Looking for exact commands, fields, schemas,
formats, or limits? [Open the Reference](reference/index.md).</p>

</section>

</div>
