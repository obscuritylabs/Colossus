---
title: OpenTelemetry implementation
description: Architecture and compatibility contract for Colossus live GenAI observability.
audience: developer
type: concept
---

# OpenTelemetry implementation

Colossus has two deliberately separate observability planes:

- `colossus-telemetry` derives bounded historical analytics from the authoritative
  journal regardless of its configured payload-protection mode.
- `colossus-observability` emits best-effort live OpenTelemetry signals and decorates
  successful journal appends. It never replays the journal after restart.

The observability crate owns semantic names, recording helpers, W3C Trace Context, OTLP
provider construction, metric views, and shutdown. Runtime composition owns the journal
decorator. The worker host owns the global subscriber and exporter lifecycle. Agent,
workflow, provider, MCP, and gRPC adapters depend only on recording and propagation
helpers; an embedded `Runtime` remains subscriber-neutral.

## Semantic-convention compatibility

The GenAI conventions are Development status and have no authoritative generated Rust
binding used here. `conventions.rs` carries the small reviewed surface and pins upstream
revision
[`46d43c8949afb53765a202e89f4534eeb75ca3fa`](https://github.com/open-telemetry/semantic-conventions-genai/tree/46d43c8949afb53765a202e89f4534eeb75ca3fa).
Update the revision, constants, recommended units and buckets, compliance tests, and
this document together. Do not use the deprecated legacy GenAI constants.

The span tree uses:

- `invoke_agent {role}` (`INTERNAL`) for each agent run.
- `plan {role}` (`INTERNAL`) only for Plan Mode, parenting its model and tool work.
- `chat {model}` (`CLIENT`) for every provider turn.
- `execute_tool {tool}` (`INTERNAL`) for every client-side tool call.
- `invoke_workflow {name}` (`INTERNAL`) for workflow execution segments.
- `colossus.api.v1alpha1.AgentRunService/CreateRun` (`SERVER`) for authenticated public
  creation calls.

Model spans include provider/request/response model, response ID when returned, token
counts, streaming first-chunk timing, and a bounded error class. Inter-chunk timing is
reported by the standard client metric rather than copied into span attributes. Tool
spans include tool name, type, call ID, and terminal status. Metrics use the upstream
names, units, recommended explicit buckets, allowed-attribute views, and a cardinality
limit. Correlation IDs, response IDs, tool-call IDs, session IDs, workflow-run IDs, and
end-user IDs are not metric dimensions.

## Identity and propagation

The authenticated application is `colossus.application.id`. Optional caller-asserted
`end_user_id` is emitted as `enduser.id`; it is correlation PII, not authorization. The
value is validated, included in the idempotency fingerprint, and persisted only inside
the encrypted run execution request. Trace context is excluded from that fingerprint.

Authenticated gRPC CreateRun accepts only `traceparent` and `tracestate`. The normalized
context is stored with the encrypted execution request so queued and recovered runs can
remain in the accepted trace. Arbitrary baggage is neither accepted nor propagated.
Provider HTTP and Streamable HTTP MCP adapters inject the current W3C context. Subprocess
arguments, subprocess environments, and tool payloads never receive propagation data.
Spawned in-process subagent futures inherit the active tracing span.

An in-process workflow may keep its logical span across a wait by retaining its local
span handle. A process restart cannot make a span object durable; recovered execution
starts a new recovery segment linked to the stored normalized context. Durable workflow
identity remains available as span/log correlation even where a backend presents those
segments separately.

## Runtime startup

The worker installs its subscriber before runtime construction, so synchronous startup
work is visible in the same trace export without making embedded `Runtime` consumers
install a global subscriber. `colossus.runtime.open` measures total composition time and
parents bounded internal phase spans for workspace ownership, storage open and startup
verification, projection catch-up, uncertain-effect recovery, research recovery, and
workflow recovery. Storage spans record only the adapter and verification mode; paths,
repository identities, key references, and credentials are deliberately excluded.

These spans are emitted whenever a caller has installed a compatible `tracing`
subscriber. They are not emitted by direct CLI commands that intentionally run without
the worker-owned OTLP host.

## Failure and disclosure boundary

OTLP and stdout use bounded SDK/nonblocking queues. Export errors are fail-open and may
drop live data; they cannot affect canonical append or run results. Journal records are
created only after successful append. Metadata mode never serializes the plaintext
payload. Full mode is an explicit dual acknowledgement described in
[Live observability](../reference/configuration/observability.md).

Provider authorization headers, resolved credentials, environment credential values,
and hidden reasoning are never recorded. Span attributes stay metadata-only even in
full journal mode.
