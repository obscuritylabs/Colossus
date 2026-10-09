---
title: Inspect agent messages
description: Inspect agent inboxes and distinguish durable acceptance, prepared input, and undelivered messages.
audience: user
type: how-to
---

# Inspect agent messages

Open a conversation backed by a runtime with message inspection enabled. In Desktop,
choose **Inboxes** in the session views, then choose the execution. In VS Code, open
the run's state inspector and choose **Agent inboxes**. In the web application, expand
**Agent inboxes** on a task with a connected runtime.

Choose a **Recipient attempt** to see messages addressed to that root or child. A
requeued child has a new attempt and inbox. Closed attempts remain inspectable so
you can see what happened to their pending messages.

The receipt describes durable delivery evidence:

| Receipt | What it establishes |
| --- | --- |
| Accepted | The runtime stored the message and is waiting for a permitted input boundary. |
| Included in prepared turn | The runtime recorded the message in the session input and the exact prepared provider request. Message details include its run, turn, and request hash. This does not establish that the provider completed the request or followed the instruction. |
| Undelivered | The recipient closed before including the message. The reason identifies completion, cancellation, failure, interruption, budget exhaustion, or a superseded attempt. |

Use **Refresh** to read current receipts. **Load more messages** reads the next page;
search and receipt filters apply to messages already loaded. **Message details** shows
the sender, recipient, message identity, and optional reply reference. Peer text is
displayed as plain text.

The inbox is backed by the runtime's existing durable journal. Restarting an app does
not erase accepted messages or receipts. After a runtime interruption, messages for
started attempts are settled as undelivered; queued attempts retain their pending
messages until their execution starts or closes.

If the connection says inbox inspection is unavailable, it needs a compatible runtime
and an application enrollment with message read permission. Existing enrollments need
an explicit grant update. A shared conversation does not grant access to its owner's
inboxes. See [application permissions](../develop/application-sdk.md#agent-communication)
for the exact capabilities and scopes.

For delegation and child results, see [Goals and subagents](goals-subagents.md).
