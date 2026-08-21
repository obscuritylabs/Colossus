---
title: Rust engineering practices
description: Repository-specific guidance for readable, secure, maintainable Rust in Colossus.
audience: developer
type: concept
---

# Rust engineering practices

Colossus optimizes for explicit authority, reviewable behavior, and dependable operation
before cleverness. These practices complement the enforced workspace lints and the
[architecture](architecture.md); they do not replace either one.

## Design for the repository

- Put invariants and dependency-free types in `colossus-domain` or
  `colossus-contracts`, ports in `colossus-ports`, and effects behind adapters.
- Keep CLI, TUI, Desktop, gRPC, and SDK crates as interfaces or translations. Application
  policy, state transitions, and effect decisions belong behind their ports.
- Keep crate roots as maps. Split a module when it owns unrelated reasons to change, not
  simply because it crossed a line-count threshold.
- Prefer a small explicit type that makes invalid states hard to construct over a tuple,
  flag set, or long positional argument list.
- Preserve object-safe port traits where runtime composition uses `dyn Trait`; do not
  adopt language features mechanically when they break that boundary.

## Ownership and APIs

- Borrow when the callee only observes data; move when it assumes ownership. Clone only
  at a deliberate ownership boundary.
- Accept the least specific useful input (`&str`, `&Path`, slices, iterators) and return
  types that communicate ownership clearly.
- Use `Result` for recoverable failure. Reserve `panic!`, `unwrap`, and `expect` for
  tests or states already proved impossible by a local invariant.
- Give public items useful rustdoc that explains constraints, authority, failure, and
  persistence—not merely the type signature.
- Keep errors typed long enough for callers to make decisions. Add context at adapter
  boundaries without copying secrets or untrusted payloads into messages.

## Async, concurrency, and effects

- Do not hold a synchronous lock across `.await`.
- Bound channels, tasks, retries, output, and concurrency. Document who owns shutdown
  and cancellation.
- Route external and sensitive effects through the gateway described in
  [Security architecture](security-architecture.md). A convenience helper must not
  bypass policy, quarantine, audit, or post-effect release.
- Emit structured tracing fields with stable names. Redact credentials, model-private
  content, untrusted bodies, and sensitive paths before they reach logs.

## Performance and dependencies

- Measure before optimizing. Prefer an algorithmic or allocation-bound fix supported by
  a benchmark or profile over speculative micro-optimization.
- Do not add a crate, builder, macro, cache, unsafe block, or alternate runtime merely
  because a generic Rust checklist recommends it. `unsafe_code` is forbidden across the
  workspace.
- Keep ordinary builds independent of optional accelerators such as `sccache`.

## Review checklist

Before review, confirm that the change has one clear owner, no interface-owned business
logic, no accidental public API expansion, bounded resource behavior, actionable errors,
appropriate rustdoc, and tests at the narrowest meaningful boundary. Then run the tiers
in [Test strategy](testing.md).

External Rust guidance can be useful as a searchable reminder, but this repository's
architecture, security model, compiler version, lints, and executable checks are the
authority when advice conflicts.
