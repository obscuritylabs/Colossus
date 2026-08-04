---
title: Develop Colossus
description: Contributor setup, architecture, state, security, extension, presentation, and documentation guidance.
audience: developer
type: concept
---

# Develop Colossus

This lane is for contributors changing Colossus itself. Product installation and normal
operation stay in [Get started](../get-started/index.md) and
[Administer and secure](../admin/index.md).

- [Contributing](contributing.md) explains repository conventions and change ownership.
- [Source setup and test tiers](setup-testing.md) provides a reproducible development
  loop.
- [Tiered CI/CD](ci-cd.md) explains the cost-bounded PR, pre-merge, and release gates.
- [Core release operations](releasing.md) covers registry bootstrap, stable CLI/SDK
  publication, and partial-release recovery.
- [Architecture overview](architecture.md) defines dependency direction.
- [Rust crate structure](crate-structure.md) keeps crate roots readable and behavior in
  responsibility-focused modules.
- [Runtime and ports](runtime-ports.md) maps application responsibilities.
- [Public API and application SDKs](application-sdk.md) defines the gRPC, SDK, and
  Tauri integration boundary.
- [State and recovery](state-recovery.md) explains canonical state and replay.
- [Security architecture](security-architecture.md) defines the non-bypassable effect
  path.
- [Extension and presentation architecture](extensions-presentation.md) covers dynamic
  capability and UI boundaries.
- [Documentation authoring](documentation.md) defines the public information contract.

Read the architecture and security pages before changing boundaries. Keep interfaces
thin, the domain dependency-free, and behavior changes covered by tests.
