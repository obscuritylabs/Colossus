---
title: Context and snapshots
description: Read the model context budget and manage snapshots for a long session.
audience: user
type: how-to
icon: lucide/layers
---

# Context and snapshots

Colossus saves the full conversation in a durable session. For each model turn,
it prepares a bounded view of that history. A context snapshot summarizes older
messages when the working view gets large; it does not remove the original messages.

## Check the current context

In the Terminal UI, enter:

```text
/context status
```

A status display looks like this. Your model, session ID, and numbers will differ:

```text
◆ Context
  Session         0195b640-8a41-7b35-948d-3d58b39e783a
  Model profile   codex
  Messages        8
  Tokens          614 / 344000
  Context window  400000
  Output reserve  16000
  Safety reserve  40000
  Compacted       no
  Snapshot        —
```

| Field | What it means |
| --- | --- |
| **Session** | The conversation whose context is being measured. |
| **Model profile** | The profile selected for the `primary` model role. |
| **Messages** | Saved session messages; compaction does not delete them. |
| **Tokens** | Estimated current prepared context, followed by the effective input budget. This is not a usage or billing counter; the next prompt and tools can change the request size. |
| **Context window** | The configured total token window for the model profile. |
| **Output reserve** | Tokens held back for the model's response. |
| **Safety reserve** | Extra room kept for estimation differences and request overhead. |
| **Compacted / Snapshot** | Whether an active snapshot is shaping future context, and its ID when one is active. |

In this example, `400000 − 16000 − 40000 = 344000` tokens remain for input.
The estimated `614` tokens are well below that budget, so there is no reason to
compact this session now. The status is an estimate of the current context, not
a promise that every future prompt will fit.

From a shell, use the session ID in the same workspace and configuration:

```bash
colossus -w /absolute/path/to/repository context status SESSION_ID
```

## When Colossus compacts a session

With automatic compaction enabled, Colossus creates a snapshot when the working
context crosses the configured threshold. The default threshold is 70% of the
effective input budget. A snapshot keeps a bounded summary of older messages and
preserves recent messages for the next model turn. The full transcript remains in
the session; [Sessions](sessions.md) shows how to read it.

After compaction, `/context status` reports **Compacted: yes** and a **Snapshot**
ID. The token estimate may fall because the model receives the summary in place
of the older message range. If a single new turn or required context is too large,
compaction can still fail rather than silently discard it.

Operators can adjust the threshold and preservation settings in
[context configuration](../reference/configuration/context-memory-research.md#context-configuration).

## Inspect or restore snapshots

Use these commands in the Terminal UI:

| What you want to do | Command |
| --- | --- |
| List saved snapshots for this session | `/context list` |
| Create a snapshot now | `/context compact` |
| Use an older snapshot for future turns | `/context restore SNAPSHOT_ID` |

Manual compaction changes the model's working view even when the automatic
threshold has not been reached. Restore changes which immutable snapshot is
active; it does not delete later messages or rewrite the snapshot. Both actions
follow the configured authorization rules.

From a shell, use the matching commands with the session ID:

```bash
colossus -w /absolute/path/to/repository context list SESSION_ID
colossus -w /absolute/path/to/repository context compact SESSION_ID
colossus -w /absolute/path/to/repository context restore SESSION_ID SNAPSHOT_ID
```

Colossus Desktop also lists the same snapshots in a thread's **Snapshots** view.
For context that should be reusable across conversations, use
[Memories](memories.md) instead of relying on a session snapshot.

## What's next?

<div class="grid cards" markdown>

-   :lucide-messages-square:{ .lg .middle } **Sessions**

    ---

    Read the complete transcript or return to the conversation later.

    [Explore sessions :lucide-arrow-right:](sessions.md)

-   :lucide-brain:{ .lg .middle } **Memories**

    ---

    Save useful context that can be found beyond this one session.

    [Explore memories :lucide-arrow-right:](memories.md)

</div>
