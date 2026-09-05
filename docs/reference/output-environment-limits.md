---
title: Output, environment, and limits
description: Rendering modes, environment handling, bounds, redaction, and unknown-outcome behavior.
audience: developer
type: reference
---

# Output, environment, and limits

## Output selection

| Context | `auto` behavior |
| --- | --- |
| Interactive terminal | Human renderer; `run` prints only the assistant response |
| Redirected structured command | Stable JSON |
| TUI slash command | Human renderer |
| Non-TTY line runner | Bounded line/JSON contract |

`--output human` and `--output json` override automatic selection where the command
supports structured output. Interactive TUI JSON is rejected. ANSI and control
sequences are emitted only after an interactive-terminal check.

For `run`, human output deliberately omits run/session identifiers, provider routing,
event counts, and timing. Use `--output json` when those fields are required.

Renderers consume post-policy released contracts. They do not change execution,
authorization, persistence, or audit semantics. Untrusted released content is sanitized
and never interpreted as terminal control input.

## Environment contract

- Configuration stores credential references such as `env:VARIABLE`, not values.
- Sandbox configuration grants environment variable names, not values.
- Permit-bearing adapters resolve credentials only after authorization.
- Process helpers start from a cleared environment and pass only declared names.
- Plugin MCP and standalone MCP environment references are bounded by both their declaration and the
  deployment sandbox.
- Raw credentials, authorization headers, private keys, key material, and hidden
  reasoning are hard-redacted from policy input, provider output, transcripts, and
  audit evidence.

## Default operational bounds

| Bound | Baseline |
| --- | ---: |
| Agent turns | `100` |
| Concurrent subagents | `10` |
| Sandbox timeout | `30000 ms` |
| Sandbox output | `4194304 bytes` |
| Sandbox processes | `16` |
| Sandbox memory | `1073741824 bytes` |
| Sandbox concurrency | `1` |
| Context window estimate | `32768 tokens` |
| Auto-compaction threshold | `70%` |
| Compaction target | `45%` |
| Recent messages preserved | `8` |
| Memory retrieval | `6` |
| Research sources | `20` |
| Research query/lane jobs | `4` |

These are baseline configuration values, not permission grants. Adapters may enforce
additional hard protocol and input bounds. Exact effective values come from `config
show`, `config effective`, and the relevant command help. See
[Runtime limits configuration](configuration/limits.md) for field interactions,
validation ranges, and tuning examples.

Provider raw-stream bytes, model output tokens, and process-tree memory are separate
limits. The provider byte count includes the full SSE framing and metadata before
normalization. The memory value is an enforced effect-process-tree ceiling, not memory
preallocated by the Colossus process. Policy, adapter, and request-specific obligations
may narrow either configured ceiling and never widen it.

## Important hard bounds

- Provider tool arguments receive at most two correction turns.
- Workflow call depth is at most 16.
- Workflow schedule cadence is 60 seconds through 31 days.
- The TUI queues at most eight future turns.
- The TUI loads at most 1,000 submitted-history entries.
- Pre-effect logical policy input is bounded at 1 MiB. Post-effect policy input is
  bounded at 8 MiB so the base64 envelope for a permitted 4 MiB result remains
  inspectable without widening the pre-effect request boundary.
- Workflow conditions are non-executable and bounded by size, token, recursion, and
  boolean-composition limits.

## Completion and uncertainty

A successful transport status alone is not application completion. Provider streams
need a terminal item. Effects need a durable terminal event. If execution may have
escaped but completion evidence is missing, the outcome is unknown. Colossus preserves
already released content but never synthesizes completion or blindly retries a
non-idempotent external effect.
