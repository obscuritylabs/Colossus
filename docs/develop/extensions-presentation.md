---
title: Extension and presentation architecture
description: How integrations, MCP, packs, skills, search, and terminal presentation remain inside core safety boundaries.
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

`colossus-packs` verifies signed file inventories, publisher trust, declared
permissions, executable identities, skills, integrations, MCP declarations, and
dependency closure. Only enabled, reverified, trusted pack capabilities become
candidates.

Skills remain declarative instructions and resources. Activation can compose text into
provider instructions but cannot execute a script. Executable behavior belongs in an
explicit pack tool, MCP server, or integration.

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
- add restart/replay and trust-revalidation tests;
- prove disabled, untrusted, unconfigured, and unselected states stay hidden.

For a new presentation:

- render only released contracts;
- sanitize controls and bound untrusted content;
- preserve a plain-text and structured output path;
- keep terminal ownership in one event loop;
- test narrow/wide layouts, Unicode, hostile controls, empty states, and worker parity.
