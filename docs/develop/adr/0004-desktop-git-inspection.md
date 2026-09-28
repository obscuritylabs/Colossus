---
title: "ADR 0004: Desktop Git inspection"
description: Read-only repository inspection bound to the selected Desktop workspace.
audience: developer
type: concept
---

# ADR 0004: Desktop Git inspection

- Status: implemented; platform acceptance required before merge
- Date: 2026-09-27
- Tracking: [Desktop Git status and history #204](https://github.com/obscuritylabs/Colossus/issues/204)

## Context

Users need to see the current checkout's branch, local changes, and recent commits
without leaving their conversation. The existing file viewer deliberately excludes
Git internals. Linked worktrees and selected subdirectories can also need repository
metadata outside the selected workspace.

## Decision

Use the packaged `git2` reader with vendored libgit2, without SSH or HTTPS features.
No system Git installation is needed for inspection. Commands are a native Desktop
human-inspection surface, not agent tools. There are no write, network, executable,
arbitrary revision, blob, or raw-configuration commands.

The main bundled WebView alone can call status, history, and commit-detail commands.
Each requires the exact selected local workspace with non-Minimal file access. Native
code validates the persisted workspace identity and selection epoch before reading
and before returning results. Repository bindings include directory object identities;
replacing a repository or switching workspaces invalidates prior handles and history.

Discovery inspects bounded `.git`, `commondir`, and backlink markers. Linked worktrees
must have both a conventional common directory relationship and a matching backlink.
Reading shared metadata outside the workspace requires native confirmation for that
binding and selection. Subdirectory status and affected-file lists stay scoped to the
selected subtree; commit messages describe the whole repository, as the native dialog
and panel explain. General separate-Git-directory and submodule-root layouts are not
inferred from arbitrary pointer files.

Repository metadata links, alternate object stores, oversized metadata, and repository
configuration includes are rejected. System/user Git configuration is used only to
derive scalar filemode, case, and line-ending comparison settings. Before status or
history, the repository receives a private configuration and a fixed native workdir.
External attribute/ignore paths, custom filters, hooks, transports, and helpers are
never invoked. Repository `.gitignore` rules remain active. A detected custom filter
or global ignore configuration produces a visible status limitation. The reader does
not recursively inspect submodules; staged gitlink changes remain visible.

Status compares HEAD and index metadata before and after reading and rejects detected
changes. A filesystem status scan is a best-effort observation, not an atomic snapshot
against other native processes. Periodic refresh reconciles subsequent edits.

## Bounds and lifetime

One native inspection task runs at a time. The renderer serializes its requests and
refreshes on focus, selection, run state changes, and a visible-window interval. History
and commit details load on demand. Data from a previous selection is never rendered.

Metadata and non-ignored worktree preflight scans stop at 100,000 entries or five
seconds. Index files are limited to 16 MiB, repository config to 256 KiB, and individual
metadata/object-store files to 512 MiB. Worktree preflight rejects tracked files larger
than 64 MiB. Results contain at most 1,000 file rows, 40
commits per page, and 400 listed commits. Commit messages and filenames are bounded
plain text. Truncation is explicit and file counts refer to the displayed list.

Native reads run on the blocking pool with a 20-second response deadline. Libgit2 calls
are not forcibly interruptible; after a timeout, the task retains its single slot until
it returns, preventing requests from accumulating blocked native work. These bounds
are not an operating-system CPU or memory sandbox for libgit2. Human confirmation is
outside the query deadline and has its own single prompt slot.

History cursors and repository IDs are native-issued. Commit details accept only IDs
already listed for the current repository and HEAD; revision expressions and arbitrary
object lookup are not exposed. Initial commits compare to an empty tree; merges compare
to the first parent. Git messages, paths, and errors never become instructions.

## Consequences and acceptance

Desktop gains a compact branch indicator and a resizable Changes/History pane, while
retaining the existing composer and file-preview boundary. Submodule working-tree
inspection, external filters, alternate object stores, full diffs, and writes remain
separate work. Unsupported repository formats fail clearly instead of appearing clean.

Native fixtures cover real repositories, linked worktrees, subdirectories, detached and
unborn HEAD, mixed staging, ignored paths, renames, conflicts, history, replacement, and
unsafe metadata. Parity fixtures compare to installed Git in tests only. Windows and
macOS acceptance must exercise the native reader; browser fixtures prove presentation
and interaction, not native filesystem authorization.

Future Git writes and agent automation require separate review. Agent effects must
remain behind runtime-owned ports with policy, approval, audit, cancellation, and
uncertain-outcome handling; Desktop read commands cannot grant that authority.
