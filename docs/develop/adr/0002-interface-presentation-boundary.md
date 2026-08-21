---
title: "ADR 0002: Interface and presentation boundary"
description: Why CLI, TUI, Desktop, worker, and SDK surfaces do not own application behavior.
audience: developer
type: concept
---

# ADR 0002: Interface and presentation boundary

- Status: accepted
- Date: 2026-08-20

## Context

The Python-to-Rust transition initially used terminal parity notes to track visible
behavior. As the Ratatui TUI and Desktop application evolved, parity with an archived UI
stopped being a useful architectural target. Duplicating model, policy, tool, or state
logic in each interface would also make authorization and audit behavior inconsistent.

## Decision

CLI, TUI, Desktop, gRPC, worker, and SDK crates are interfaces and translations only.
Application services own durable behavior behind ports. `colossus-presentation` maps
released typed contracts into bounded semantic documents; renderers choose terminal,
plain-text, JSON, or application-specific views without changing policy or state.

Acceptance is based on current semantic and security contracts, not pixel or command
parity with Python 0.5. Interface-specific tests remain appropriate for layout, input,
transport, restoration, and serialization behavior.

## Consequences

- New interfaces reuse the same application and effect boundaries.
- Presentation code cannot expose quarantined output or make authorization decisions.
- TUI and Desktop can change substantially without preserving archived Python chrome.
- Cross-interface parity tests assert typed behavior; visual and interaction tests stay
  in the interface that owns them.

See [Runtime and ports](../runtime-ports.md) and
[Extensions and presentation](../extensions-presentation.md) for the current design.
