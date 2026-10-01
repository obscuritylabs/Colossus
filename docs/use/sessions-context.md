---
title: Context and snapshots
description: Inspect, compact, and restore model context while preserving session messages.
audience: user
type: how-to
---

# Context and snapshots

A long session can have more history than fits in a model request. Colossus builds a
bounded working view from the durable conversation. Compaction creates an immutable
snapshot of earlier history; the original messages stay available.

Use the same workspace and configuration as the [session](sessions.md) you want to
inspect.

## Check the context budget

```bash
colossus context status SESSION_ID
```

Colossus estimates the complete provider request, including instructions and tool
schemas. Automatic compaction can create a snapshot at the configured threshold.

## Create or restore a snapshot

```bash
colossus context compact SESSION_ID
colossus context list SESSION_ID
colossus context restore SESSION_ID SNAPSHOT_ID
```

Restore changes the active derived snapshot for future turns. It does not delete later
messages or mutate the snapshot.

In Desktop, open a thread and select **Snapshots** to list the same immutable records.
Select a snapshot to inspect its bounded summary, source message range, pinned facts,
open tasks, touched files, notable tool results, and compaction strategy. **Resources**
also links snapshots beside every other released, listable session record.

To capture commitments across runs, see
[Tasks, decisions, and plans](tasks-decisions-plans.md).

## Check the retained messages

Compare `colossus sessions messages SESSION_ID` before and after compaction. The
message history should remain append-only even though `context status` reports a new
active snapshot.

## If something goes wrong

- **Summary generation fails:** Colossus uses deterministic fallback extraction and
  preserves raw history.
- **Restore is denied:** it is an independently authorized context transition; inspect
  the exact action in `config effective`.
- **Context still overflows:** reduce tool surface or context settings with an operator;
  do not delete canonical state to solve a request budget.
