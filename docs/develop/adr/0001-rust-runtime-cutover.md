---
title: "ADR 0001: Rust runtime cutover"
description: Why the Rust runtime, configuration, and state formats are canonical and Python runtime compatibility is not retained.
audience: developer
type: concept
---

# ADR 0001: Rust runtime cutover

- Status: accepted
- Date: 2026-08-20

## Context

Colossus moved from a Python 0.5 implementation to a Rust workspace with stronger type,
isolation, persistence, and distribution boundaries. Continuing to parse Python runtime
configuration, SQLite state, terminal theme schemas, and transitional aliases increased
the attack surface and made the active contract ambiguous.

## Decision

The repository-root Rust implementation is canonical. It uses the documented Rust YAML
configuration and redb or PostgreSQL state contracts and never silently imports Python
configuration or state. Python 0.5 is frozen on `python-v0.5.0` and `python-legacy` for
historical access. The maintained `sdk/python` package is a client for the current public
API and is not the legacy runtime.

Python-era compatibility is removed from the active runtime once a supported Rust
replacement exists. During the 2026 cleanup this included the nested `research.search`
adapter, unversioned Python theme imports, and unused REPL history aliases. Deployed Rust
state and protocol compatibility remains a separate, explicit decision.

## Consequences

- Upgrades from Python require deliberate reconfiguration rather than implicit import.
- Unknown legacy fields fail closed under strict deserialization.
- Search uses named top-level profiles and role routes; themes require schema version 1.
- Historical reconstruction prose lives in Git history instead of an unmaintained
  parallel specification.
- Pre-1.0 releases may continue to make breaking changes, with current migration notes
  in [Upgrade and compatibility](../../get-started/upgrade-compatibility.md).
