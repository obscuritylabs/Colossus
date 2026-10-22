---
title: "ADR 0009: Hosted execution and managed files"
description: Separate Hosted runtime placement and lifecycle from connected Fleet enrollment and Web Chat presentation.
audience: developer
type: concept
---

# ADR 0009: Hosted execution and managed files

- Status: accepted
- Date: 2026-10-20

## Context

Fleet represents independently connected runtimes. Built-in Chat originally reused
that transport by enrolling a personal virtual node and retaining its connector and
runtime indefinitely. This coupled stored conversations to resident execution and
made Hosted targets appear in Fleet. Hosted work needs bounded placement independent
from browser connections while reusing runtime authority, tools, persistence and SDKs.

## Decision

Introduce a server-owned Hosted execution boundary and an explicit target placement
kind. Hosted targets have no certificate or enrollment. Preserve historical target,
application and journal namespace identities; migrate only exact owner/instance
bindings, including the generated legacy workspace binding. Historical grouping stays
available to ownership/history joins; Fleet host, workspace and target projections
filter Hosted placements before pagination.

Reuse durable Cloud task/command admission, receipts and released-output cursors.
Execute Hosted operations directly through the ordinary caller-bound SDK. Share command
execution and release bounds with the connector in `colossus-cloud-executor`. A bounded
process-wide supervisor loads tenant runtimes when unfinished work exists, recovers
queued placement after restart and retires fully idle contexts. Shared SQL retains one
fenced execution owner per project/user. API and workers initially share a process.

Managed files are application-owned data in the existing protected artifact journal.
The host derives ownership from authentication; uploads and downloads never select
filesystem paths. General workspace, shell and code execution require a separately
provisioned environment and remain unavailable on this Hosted surface.

## Consequences

Chat and task submission share execution without owning its lifetime. Stored
conversations require no resident runtime. A browser watch can disconnect without
cancelling accepted work. Configuration discovery does not require a worker lease.

The initial pool moves only idle tenant contexts. Waiting runs retain residency;
crash recovery still distinguishes Interrupted from OutcomeUnknown and never blindly
repeats uncertain effects. This decision introduces bounded dispatch and lifecycle
management, not a second workflow engine or a claim of arbitrary checkpoint replay.
See [state and recovery](../state-recovery.md) for canonical execution guarantees and
[Hosted operation](../../admin/cloud-control-plane.md#scale-hosted-chat-across-replicas)
for current deployment and upgrade requirements.
