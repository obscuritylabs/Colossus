---
title: Connect the Anthropic API
description: Configure native Anthropic Messages with model discovery, tools, streaming, images, and private thinking replay.
audience: user
type: how-to
---

# Connect the Anthropic API

Choose **Anthropic** in Desktop's provider setup, enter a workspace-scoped Anthropic
API key through the native credential dialog, discover models, and select the exact
model you want to use. Colossus uses the native Messages API.

For CLI setup, supply `ANTHROPIC_API_KEY` through the process environment or a secret
manager, then run:

```sh
colossus provider setup --preset anthropic
```

Follow setup's model selection prompts. Keep the secret out of configuration files
and shell history. See [Choose a provider](index.md) for non-interactive setup and
[Providers and models](../../reference/configuration/providers-models.md) for complete
configuration fields and explicit limits.

For an existing connection, use `kind: anthropic_messages`,
`baseUrl: https://api.anthropic.com/v1`, and
`credentialReference: env:ANTHROPIC_API_KEY`. Include the `/v1` prefix and omit
`/messages` and `/models`; Colossus adds those operation paths. Under an isolating
boundary, authorize the exact `https://api.anthropic.com` origin through
`sandbox.networkDestinations`.

## Supported behavior

The native adapter supports text and verified image input, JSON and streamed
responses, local and MCP tools including parallel tool calls, and token usage.
Model discovery follows bounded catalog pagination. Missing model metadata remains
unknown. Native input limits are used as conservative context budgets with an
additional output and safety reservation.

Tool schemas omit top-level `oneOf`, `anyOf`, and `allOf` from the definitions sent
to Anthropic, which rejects those keywords at the root. Nested schemas remain
intact. Colossus still validates every tool call against the full original schema
before execution.

Leave reasoning effort unset to use the model's default. Explicit `low`, `medium`,
`high`, `xhigh`, or `max` requests use native adaptive thinking and effort controls;
the selected model must support the combination. `none` requests disabled thinking
where supported. `minimal` and `ultra` are rejected locally. Legacy manual thinking
budgets are not configured by this adapter.

Thinking blocks and signatures are replayed privately during tool exchanges and do
not enter visible conversation history or ordinary diagnostics. With protected
storage, replay state is encrypted; keyless development storage retains it only in
bounded process memory. Missing or oversized required state stops the tool exchange
explicitly. Local compaction cannot replace required replay state in an active tool
exchange.

After a provider failure, continue in the same session with **Continue** or new
guidance. Colossus preserves the unfinished exchange's original context and sends
its recorded tool results back to the provider. Completed tools do not execute again.
Protected storage retains this checkpoint across runtime restarts; keyless storage
can continue only while its process still holds the private state. A changed provider,
model, reasoning effort, tool catalog, or restored context snapshot can prevent exact
continuation; the checkpoint is retained when those checks fail. Older affected
protected sessions recover only when retained private state still matches their
original context. Missing signatures cannot be recreated from visible history.

## Connection failures and limits

Use a key scoped to one Anthropic workspace. Multi-workspace selectors, workload
identity federation, Claude subscription authentication, Bedrock, Vertex, and Foundry
are not configured through this connection type.

Authentication and unsupported model/effort errors remain explicit failures. An
interrupted stream is not automatically replayed. Existing bounded HTTP 502–504
recovery applies before response content starts; rate limits and overload remain
reported failures. Inspect the existing provider doctor commands when troubleshooting.

Prompt-cache controls, native compaction, token-count requests, PDF/document inputs,
batch jobs, and Anthropic-hosted tools are not enabled by this adapter. Local tool
execution continues through Colossus policy, approvals, sandboxing, and audit.

See Anthropic's [Messages reference](https://platform.claude.com/docs/en/api/messages/create)
and [thinking preservation requirements](https://platform.claude.com/docs/en/build-with-claude/thinking#preserving-thinking-blocks)
for provider-side protocol details.
