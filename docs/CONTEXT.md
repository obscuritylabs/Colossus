# Context Compaction

Canonical session messages remain immutable encrypted events. Compaction creates a
derived snapshot for future provider requests; it never rewrites or deletes history.

Before every provider turn Colossus estimates the complete request, including
instructions and tool schemas. At the configured threshold it appends a snapshot and an
activation event to the same optimistic session stream. Provider input becomes the
active snapshot plus the preserved recent tail.

## Configuration

```yaml
context:
  autoCompaction: true
  contextWindowTokens: 32768
  compactAtPercent: 70
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: true
```

`contextWindowTokens` is the deterministic fallback budget. `targetPercent` must be
below `compactAtPercent`; percentages are integer values. The preserved recent messages
are never summarized automatically.

When `modelAssisted` is enabled, the `context_summarizer` provider role receives a normal
policy-bound model request. Invalid, failed, echo-only, or unavailable summaries fall
back to deterministic extraction. Raw history remains the authority either way.

## Commands

```bash
colossus --config .colossus/config.yaml sessions list
colossus --config .colossus/config.yaml sessions messages SESSION_ID
colossus --config .colossus/config.yaml context status SESSION_ID
colossus --config .colossus/config.yaml context compact SESSION_ID
colossus --config .colossus/config.yaml context list SESSION_ID
colossus --config .colossus/config.yaml context restore SESSION_ID SNAPSHOT_ID
```

In the REPL:

```text
/context status
/context compact
/context list
/context restore SNAPSHOT_ID
```

Restore changes only the active derived snapshot and therefore requires its own policy
decision. It does not delete later messages or mutate the selected snapshot.

## Composition Order

The prepared model context contains, in order:

1. Run instructions and active skill material.
2. Active key decisions as binding commitments.
3. Relevant active memories as non-instructional background.
4. The active compacted snapshot, when one exists.
5. The preserved canonical recent-message tail.

Every turn records a `context.prepared.v1` event with bounded budgeting metadata. A
projection can be rebuilt, but the session stream and encrypted snapshots remain
canonical.

Research final reports are appended as normal assistant messages. Raw research sources
and claims remain in their canonical research streams and are not pasted into every
later prompt.
