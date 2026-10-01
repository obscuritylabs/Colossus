---
title: Memories
description: Save useful context, find it later, and replace or retire stale guidance.
audience: user
type: how-to
icon: lucide/brain
---

# Memories

Memories keep useful, non-secret context available for later work: a preference, a
repository convention, or a fact you do not want to repeat in every prompt. Colossus
offers relevant active memories to the model as **background context**. They do not
override instructions or policy. For a binding workspace commitment, use a
[key decision](decisions.md) instead.

## Save something worth remembering

In the terminal UI, ask Colossus to save a specific fact and say how widely it
should apply:

> Save this as a repository-scoped memory: release notes need a compatibility section.

If `memory.create` is available and authorized, Colossus can save it with the current
repository identity. Check the resulting record rather than assuming a conversational
acknowledgement saved it:

```text
/memories
```

The list shows active memories. Each record has an ID, text, kind, scope, and status.
Keep memory text short and factual. Do not store credentials, tokens, or private keys
in it.

### Choose a scope

| Scope | Use it for |
| --- | --- |
| Session | Context relevant to this conversation only. |
| Repository | A convention to reuse while working in this repository. |
| Global | A broad preference across sessions in the selected Colossus state. |

For a precise session-scoped record, use the CLI. Get the session ID with
`/session show` or `colossus sessions list`:

```bash
colossus memories create \
  "Prefer release notes with a compatibility section." \
  --scope session --scope-id SESSION_ID --kind preference
```

The CLI defaults to `global` when `--scope` is omitted, so set the scope deliberately.
Session and repository scopes require an exact `--scope-id`.

## Find a memory later

Search by a phrase in the terminal:

```text
/memory search release notes
```

This search uses the current session and global scopes. For a repository-scoped
record, use its repository ID with the CLI; `/memories` or `memories show` displays
the ID on an existing record:

```bash
colossus memories search "release notes" --repository REPOSITORY_ID
colossus memories show MEMORY_ID
```

Colossus also retrieves relevant in-scope memories for later model turns. Search
indexes find candidates; the canonical journal determines whether a record is still
active, unexpired, and in scope before it is released.

## Correct or retire stale guidance

Replace an active memory when its meaning changes:

```bash
colossus memories supersede MEMORY_ID \
  "Release notes need compatibility notes only for breaking changes." \
  --rationale "Narrowed the convention"
```

The replacement is a new active record linked to the old one. To stop using a memory
without replacing it, archive it:

```bash
colossus memories archive MEMORY_ID
```

Superseded and archived records remain in the journal for inspection, but no longer
enter new model context. Use `colossus memories show MEMORY_ID` to inspect the exact
record and its lineage.

If search misses a record, check its scope, status, and expiry first. Then run
`colossus memories index status`. The index is disposable; its `sync` and `rebuild`
commands can restore search from the canonical journal. See
[Memory configuration](../reference/configuration/context-memory-research.md#memory-configuration)
for index behavior and bounds.

## What's next?

<div class="grid cards" markdown>

-   :lucide-scale:{ .lg .middle } **Decisions**

    ---

    Record a binding choice that should guide future turns.

    [Manage durable work :lucide-arrow-right:](decisions.md)

-   :lucide-layers:{ .lg .middle } **Context and snapshots**

    ---

    Inspect what a long conversation keeps in its working context.

    [Explore context :lucide-arrow-right:](sessions-context.md)

</div>
