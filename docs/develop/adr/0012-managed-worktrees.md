---
title: "ADR 0012: Human-managed Git worktrees"
description: Share narrow managed-worktree ownership and use leases between Desktop and the CLI.
audience: developer
type: reference
---

# ADR 0012: Human-managed Git worktrees

Status: Accepted

Date: 2026-10-29

## Context

Desktop's Git inspector is read-only. Creating a checkout or deleting a directory is
a different authority and must not extend that inspector or become an agent tool.
Desktop and CLI also need to agree when a checkout is in use: a renderer flag cannot
protect a CLI worker from a different process removing its working directory.

## Decision

The `colossus-worktrees` crate owns managed worktree creation, inspection, archive,
restoration and removal. It depends on bounded local Git access, private filesystem
custody and cross-process native locks. It has no runtime, model, network or process
execution dependency. Desktop and CLI are human adapters to that service.

The service binds the selected repository root, its conventional Git directory and
shared metadata directory by identity. A linked worktree must have an exact backlink.
Every catalog entry binds the source repository identity, generated worktree ID,
checkout identity, generated Git worktree name and exact private checkout path.
A matching origin marker in the Git administration directory points back to the
private catalog. Neither an arbitrary renderer path nor an unrelated Git worktree
can authorize removal.

An explicit `COLOSSUS_HOME` stores managed state under `worktrees`. Otherwise Desktop
and CLI share the per-user local application-data `ColossusWorktrees` directory.
A private locator records an explicitly selected alternative directory and its
identity. Switching directories preserves all previous bytes. New directories must
be empty; unrelated nonempty directories are never adopted. Catalog updates use
confined staged replacement and a mutation lock.

The CLI acquires a shared-use lease before runtime/worker setup and retains it through
cleanup. Desktop additionally retains a selected-workspace lease and uses provisional
leases while switching or starting a runtime. Shared leases are existing-only file
locks in the owning private inventory. Archive and removal require the same lock in
exclusive mode. A failed acquisition is an error, never permission to continue
without a lease.

Creation durably records intent before changing refs or checkouts. Interrupted states
remain visible for inspection; automatic replay is not supported. Archive changes
admission while preserving checkout bytes. Removal requires an archived entry, exact
ownership, an unlocked Git worktree, no users, and a clean status including untracked
and ignored files. It rejects unsupported links, reparse-point parents, special files
and hardlinks
before bounded removal. Contained tracked relative file links require an exact
current HEAD tree map and regular tracked target; chains and directory links are
refused. Their leaf is rechecked and unlinked nonrecursively under a verified private
parent before ordinary removal. Windows regular-file materialization is supported.
The branch is retained and a receipt records completion.
There are no force, reset, fetch, broad prune or global Git-configuration operations.

The crate uses the packaged libgit2 dependency with network features disabled.
Git metadata and commit trees have explicit inspection limits. Configuration includes,
external attribute/ignore files and alternate stores are rejected. A private minimal
configuration allows only scalar file-comparison settings. No executable custom
filter, hook or credential helper is registered.

Desktop binds requests to the trusted main renderer's current local workspace and
selection epoch. It asks native confirmation for metadata admission and every
mutation. Directory overrides come only from a native picker. Blocking operations
keep a single operation slot even after a UI timeout, and uncertain mutations disable
further UI changes until explicit refresh. The SDK and runtime tool catalog expose
none of these operations.

## Consequences

Independent worktree branches are available without a provider. CLI `open` reuses
normal TUI startup. Existing Git inspection keeps its read-only boundary.

The first version deliberately supports fewer Git layouts than the Git executable.
It retains dirty or interrupted checkouts for human inspection. Private custody and
identity checks protect Colossus-owned paths; independent applications that ignore
Colossus leases are not counted as active users. Removing a checkout is never a way
to discard their uncommitted work.

Tests use owned disposable repositories and cover cross-process shared leases,
concurrent-use refusal, dirty/untracked/ignored retention, locked worktrees, ownership
tampering, interrupted intent, bounded inputs and branch retention.

The link unlink is pathname-based after parent/leaf revalidation, not a claim of
handle-anchored deletion or atomic exclusion of independent same-user writers.
A durable Removing record precedes the first unlink; partial failures retain it.
