---
title: Sessions and context
description: Resume durable conversations and compact model context without deleting canonical messages.
audience: user
type: how-to
---

# Sessions and context

## Goal

Find or create a durable session, continue it explicitly, and control the derived model
context while preserving the complete encrypted transcript.

## Prerequisites

- An initialized configuration with at least one completed run.
- The same canonical state and keys used to create the session.

## Steps

### 1. Find a session

```bash
colossus --config .colossus/config.yaml sessions list
colossus --config .colossus/config.yaml sessions show SESSION_ID
colossus --config .colossus/config.yaml sessions messages SESSION_ID
```

Create an empty named session when work needs a clean boundary:

```bash
colossus --config .colossus/config.yaml sessions new "Release review"
```

### 2. Continue the exact session

```bash
colossus --config .colossus/config.yaml run --session SESSION_ID \
  "Continue the review"
```

`run --resume` uses the most recently updated session. In the terminal UI, `/resume`
opens a full-width session browser with the current session marked, searchable recent
sessions on the left, and the selected session's recent conversation on the right.
The preview shows the last eight user and assistant messages; tool-heavy sessions are
paged backward past tool records so the preview stays populated. Use `/` to search, Up/Down to select, PageUp/PageDown to scroll the preview, and Enter
to resume. In the default inline TUI, the browser uses a temporary full-screen viewport
and restores the original terminal history when it closes. `/resume SESSION_ID` still
chooses an exact record directly.

### 3. Inspect the context budget

```bash
colossus --config .colossus/config.yaml context status SESSION_ID
```

Colossus estimates the complete provider request, including instructions and tool
schemas. Automatic compaction can create a snapshot at the configured threshold.

### 4. Compact or restore deliberately

```bash
colossus --config .colossus/config.yaml context compact SESSION_ID
colossus --config .colossus/config.yaml context list SESSION_ID
colossus --config .colossus/config.yaml context restore \
  SESSION_ID SNAPSHOT_ID
```

Restore changes the active derived snapshot for future turns. It does not delete later
messages or mutate the snapshot.

In Desktop, open a thread and select **Snapshots** to list the same immutable records.
Select a snapshot to inspect its bounded summary, source message range, pinned facts,
open tasks, touched files, notable tool results, and compaction strategy. **Resources**
also links snapshots beside every other released, listable session record.

## Expected result

New runs append to the selected session. Context status identifies the active snapshot
and budget, while all canonical messages remain available through `sessions messages`.

## Verification

Compare `sessions messages SESSION_ID` before and after compaction. The message history
should remain append-only even though `context status` reports a new active snapshot.

## Failure path

- **Session is not found:** copy the complete ID from `sessions list`; state identities
  do not cross independent configurations.
- **Summary generation fails:** Colossus uses deterministic fallback extraction and
  preserves raw history.
- **Restore is denied:** it is an independently authorized context transition; inspect
  the exact action in `config effective`.
- **Context still overflows:** reduce tool surface or context settings with an operator;
  do not delete canonical state to solve a request budget.

## Next step

Capture durable commitments with
[Tasks, decisions, and plans](tasks-decisions-plans.md).
