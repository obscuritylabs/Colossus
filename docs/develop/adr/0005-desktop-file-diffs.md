---
title: "ADR 0005: Desktop file search and diffs"
description: A bounded read-only file and diff viewer using native Git and the existing syntax highlighter.
audience: developer
type: concept
---

# ADR 0005: Desktop file search and diffs

- Status: implemented; platform acceptance required before merge
- Date: 2026-09-28
- Tracking: [Desktop file explorer and Git diff viewing #206](https://github.com/obscuritylabs/Colossus/issues/206)

## Context

The Git pane identifies changes but opening a working file alone cannot show what
was staged, removed, or changed in a historical commit. The file tree also requires
users to expand every parent before discovering deeply nested files.

## Component evaluation

| Approach | Accessibility and interaction | Performance and package cost | Security and maintenance |
| --- | --- | --- | --- |
| Monaco editor and diff editor | Mature editor keyboard and screen-reader support, with an editor-specific interaction model | Requires editor/model lifecycle integration and worker packaging; an exact Colossus bundle delta was not measured | Would require CSP/worker review, explicit model disposal on selection changes, theme integration, and another dependency lifecycle |
| Focused viewer with existing Shiki | Ordinary buttons, selectable text, explicit line numbers and change markers; keyboard hunk and file navigation | Reuses the existing lazy tokenizer and native Git dependency; bounded DOM rather than loading a full editor | Preserves the current text-only boundary; small local rendering surface, but Colossus owns alignment, navigation, accessibility tests, and display limits |

Monaco is the VS Code editor component, not the VS Code workbench or its extension
host. Its [upstream documentation](https://github.com/microsoft/monaco-editor) describes
models, disposal and worker integration; the
[accessibility guide](https://github.com/microsoft/monaco-editor/wiki/Monaco-Editor-Accessibility-Guide)
describes its supported screen-reader and keyboard features. Both are useful if Desktop
later needs editing, language services, or a full editor experience.

## Decision

Use a focused read-only viewer for this scope. Diff generation uses the packaged
`git2` reader from [ADR 0004](0004-desktop-git-inspection.md). The renderer uses the
existing lazy Shiki tokenizer, React text nodes, and shared theme tokens. No dependency,
worker, network source, HTML renderer, or editor extension host is added.

Changed-file details offer separate staged, unstaged, and new-file comparisons.
History opens a selected commit's affected file against its first parent (or an empty
tree for an initial commit). Inline and side-by-side presentations include line numbers,
textual addition/removal markers, next/previous change controls, current-file access,
and reveal-in-explorer navigation. Git comparisons expand the files panel; users can
restore the conversation alongside it, resize it, or hide the explorer. Drafts remain
mounted. Source and comparison tabs are distinct and close independently.

File search matches names and workspace-relative paths, including unopened directories.
It does not search content. Search and the tree apply the same protected-name and
link exclusions. No filesystem index is persisted.

## Bounds and authority

The diff command requires the trusted main controller, current selected local workspace,
non-Minimal access, repository binding, and selection epoch. A historical request must
name a commit already listed by the current native history session. Its path must appear
in that commit's scoped changed-file list; working comparisons must appear in current
status with the requested kind of change. Revision expressions and arbitrary blob IDs
are not accepted. Protected paths remain excluded even for deleted or historical files.

Each side is limited to 256 KiB and 6,000 lines. Historical object headers are checked
before loading blob content. Links and gitlinks are not dereferenced. Worktree reads use
the existing object-checked file reader. Native comparison runs on the existing serialized
Git blocking task, with its response deadline and retained slot after timeout. External
Git filters and text converters never run: unstaged comparisons explicitly describe raw
text semantics. HEAD, index and repository identity are rechecked; this remains a
best-effort snapshot against concurrent native writes, not an atomic repository transaction.

Returned diffs contain at most 2,000 rows and 128 hunks. Totals describe the bounded
comparison; truncated display is labeled. Empty, deleted, binary, conflicting, unsupported,
oversized and unavailable versions have explicit states rather than fabricated empty diffs.

Search runs on the blocking pool with one native slot, a cooperative three-second scan
budget, 50,000 visited entries, depth 48, and at most 200 results. A 20-second response
deadline retains the slot until the blocking task exits; directory I/O itself is not
forcibly interruptible. The renderer debounces and serializes queries, skipping
obsolete queued queries. Selection and workspace identity are validated before returning
results. Broad, incomplete or unreadable scans report a limit, never a complete absence.

Source previews display at most 6,000 lines; larger text remains explicitly truncated.
Both source and diff skip tokenization above 64,000 characters or for lines longer than
2,000 characters. Plain text stays available. At most eight documents retain content;
closing tabs releases it and switching workspaces disposes all documents. Late completions
cannot replace another workspace's view. Refresh is explicit for open source/diff content.

## Consequences and acceptance

This adds inspection, not editing, staging, committing, or agent authority. A future editor
decision can adopt Monaco without moving native filesystem/Git authority into the renderer.

Native tests use real repositories for staging separation, initial and renamed files,
deletions, protected historical paths, subdirectory scope, binary/large content, and diff
limits. Search tests scan 2,200 real files beyond the tree's listing cap. Browser tests
exercise a 5,000-path fixture, keyboard reveal, both layouts, hunk navigation, resizing,
draft preservation, theme/accessibility checks, errors and selection changes during loading.
Browser fixtures are presentation evidence; native tests establish filesystem behavior.
macOS native acceptance and assistive-technology checks remain platform acceptance work.

The Windows production build measured 3,882,091 bytes of renderer assets against the
existing 4,000,000-byte gate at initial implementation. This is the whole renderer, not
the incremental cost of this feature or a measured comparison against Monaco. Re-run
`npm run build` for the current figure; no budget was increased.
