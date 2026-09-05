---
title: Extension and presentation architecture
description: How integrations, MCP, Agent Plugins, search, and terminal presentation remain inside core safety boundaries.
audience: developer
type: concept
---

# Extension and presentation architecture

Dynamic capability and terminal rendering are adapters around application contracts.
Neither is a plugin-shaped escape hatch.

## Extension composition

`IntegrationService` owns typed connector manifests and canonical connection lifecycle.
Connected operations become strict tools and enter normal access resolution. The
credential broker resolves environment references inside permit-bearing adapters.

`colossus-mcp` launches only configured stdio executables through the sandbox helper,
filters paginated discovery through exact allowlists, and validates each call against a
fresh schema. Each discovery page and call is a separate authorized effect.

`colossus-plugins` validates Agent Plugins v1 and Agent Skills, packages one whole plugin
as one deterministic OCI artifact, verifies registry descriptors and Sigstore/Cosign
evidence, and owns the owner-scoped global lifecycle journal. Each run leases one active
digest snapshot after workspace include/exclude narrowing.

Agent Skills remain declarative instructions and resources. The model receives bounded
metadata for all available skills and loads a body or resource only through qualified
`PLUGIN/SKILL` selection. Referenced scripts use ordinary process tools; selected plugin
roots only add read/execute grants and `allowed-tools` remains advisory.

Plugin MCP declarations are portable data until a matching workspace overlay explicitly
enables `PLUGIN/SERVER` with an exact tool allowlist. Stdio and Streamable HTTP effects use
the same MCP, policy, sandbox, credential, quarantine, and audit adapters as standalone MCP.

Provider-neutral search sits behind a `SearchProvider` port. Named profiles and explicit
agent/research routes keep provider selection outside model arguments. Search and fetch
results use the same permit, origin, quarantine, post-release, and audit boundaries as
other network effects.

## Presentation composition

`colossus-presentation` maps released typed contracts to bounded semantic documents.
Plain text, JSON, terminal CLI, and Ratatui render those documents. It contains no policy,
model, tool, workflow, or persistence decisions.

`colossus-tui` is a reducer and one terminal event loop. It owns editing, history
navigation, completion, layout, scrolling, overlays, queueing, and terminal restoration.
Background application tasks send typed host events through bounded channels; they never
write terminal state directly.

Worker IPC transports typed application contracts and interactive prompt frames, not
terminal markup. Embedded and worker-backed hosts therefore share behavior while
presentation remains interface-local.

## Review checklist

For a new extension:

- define trusted metadata, exact action and effect identity, prerequisites, and bounds;
- keep credentials as references;
- use a port when core services need the capability;
- route effects through the gateway and post-release boundary;
- add snapshot-lease, OCI round-trip, registry, and trust-revalidation tests;
- prove disabled, untrusted, unconfigured, and unselected states stay hidden.

For a new presentation:

- render only released contracts;
- sanitize controls and bound untrusted content;
- preserve a plain-text and structured output path;
- keep terminal ownership in one event loop;
- test narrow/wide layouts, Unicode, hostile controls, empty states, and worker parity.
