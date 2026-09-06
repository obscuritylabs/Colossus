---
title: Glossary
description: Canonical meanings for Colossus product, safety, state, workflow, and extension terms.
audience: user
type: reference
---

# Glossary

| Term | Meaning |
| --- | --- |
| Access profile | Metadata-driven baseline for model-visible tools and built-in action decisions |
| Action | Exact policy identity for an effect, such as a filesystem read or network request |
| Actor | User, model, worker, or system identity responsible for a request |
| Adapter | Implementation behind an application port, such as redb, PostgreSQL, a model provider, or sandbox |
| Approval | Proof satisfying an existing policy obligation; it does not widen policy or sandbox authority |
| Canonical state | Authoritative journal events from which application state is reconstructed |
| Capability | Trusted metadata connecting a tool or operation to its action, effect identity, prerequisites, and source |
| Agent Plugin | Portable v1 directory and whole-plugin OCI distribution unit containing Agent Skills, resources, and optional MCP configuration |
| Decision | Either a policy authorization result or a durable user key decision, according to context |
| Effect | External or sensitive operation that must cross the effect gateway |
| Effect gateway | Application boundary that validates, authorizes, permits, quarantines, and journals effects |
| Goal | Bounded autonomous loop over ordinary audited agent runs |
| Integration | Persisted connection exposing supported external operations as strict tools |
| Journal | Hash-chained canonical event store; optional keys encrypt payloads and sign checkpoints |
| MCP server | Explicitly configured stdio process with exact executable and tool allowlist |
| Memory | Durable, non-instructional background record with canonical lifecycle |
| Obligation | Resource and behavior constraint attached to an authorized effect |
| Outcome unknown | Effect may have escaped, but terminal success or failure is not established |
| Plugin snapshot | Immutable, leased set of globally active plugin digests narrowed by one workspace and retained for a top-level run |
| Permit | Opaque, authenticated, short-lived, one-use authority bound to one authorized request |
| Port | Application-owned interface implemented by replaceable adapters |
| Projection | Disposable, rebuildable read model derived from canonical events |
| Quarantine | Private adapter output held until any required post-effect policy allows release |
| Role | Named model use such as `primary`, `context_summarizer`, or `research_worker` |
| Safety Kernel | Non-bypassable local validation independent of built-in policy or OPA |
| Session | Durable conversation identity and canonical message stream |
| Skill | Declarative instructions and bounded resources; never executable on activation |
| Tool | Strict JSON-Schema operation visible to the model in the current run |
| Workflow | Hash-pinned, schema-validated durable YAML orchestration |
| Worker | Authenticated local application host that owns the writer lease and drains queued work |
