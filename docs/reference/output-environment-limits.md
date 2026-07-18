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
| Interactive terminal | Human renderer |
| Redirected structured command | Stable JSON |
| TUI slash command | Human renderer |
| Non-TTY line runner | Bounded line/JSON contract |

`--output human` and `--output json` override automatic selection where the command
supports structured output. Interactive TUI JSON is rejected. ANSI and control
sequences are emitted only after an interactive-terminal check.

Renderers consume post-policy released contracts. They do not change execution,
authorization, persistence, or audit semantics. Untrusted released content is sanitized
and never interpreted as terminal control input.

## Environment contract

- Configuration stores credential references such as `env:VARIABLE`, not values.
- Sandbox configuration grants environment variable names, not values.
- Permit-bearing adapters resolve credentials only after authorization.
- Process helpers start from a cleared environment and pass only declared names.
- Pack and MCP environment references are bounded by both their declaration and the
  deployment sandbox.
- Raw credentials, authorization headers, private keys, key material, and hidden
  reasoning are hard-redacted from policy input, provider output, transcripts, and
  audit evidence.

## Default operational bounds

| Bound | Baseline |
| --- | ---: |
| Agent turns | `24` |
| Concurrent subagents | `10` |
| Sandbox timeout | `30000 ms` |
| Sandbox output | `1048576 bytes` |
| Sandbox processes | `16` |
| Sandbox memory | `268435456 bytes` |
| Sandbox concurrency | `1` |
| Context window estimate | `32768 tokens` |
| Auto-compaction threshold | `70%` |
| Compaction target | `45%` |
| Recent messages preserved | `8` |
| Memory retrieval | `6` |
| Research sources | `20` |
| Research workers | `4` |

These are baseline configuration values, not permission grants. Adapters may enforce
additional hard protocol and input bounds. Exact effective values come from `config
show`, `config effective`, and the relevant command help.

## Important hard bounds

- Provider tool arguments receive at most two correction turns.
- Workflow call depth is at most 16.
- Workflow schedule cadence is 60 seconds through 31 days.
- The TUI queues at most eight future turns.
- The TUI loads at most 1,000 submitted-history entries.
- OPA logical policy input is bounded at 1 MiB.
- Workflow conditions are non-executable and bounded by size, token, recursion, and
  boolean-composition limits.

## Completion and uncertainty

A successful transport status alone is not application completion. Provider streams
need a terminal item. Effects need a durable terminal event. If execution may have
escaped but completion evidence is missing, the outcome is unknown. Colossus preserves
already released content but never synthesizes completion or blindly retries a
non-idempotent external effect.
