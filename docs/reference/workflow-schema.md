---
title: Workflow schema
description: Strict YAML definition, step families, trigger envelopes, and recovery semantics for durable workflows.
audience: developer
type: reference
---

# Workflow schema

Workflow definitions are strict YAML in configured repository or user roots. Every
registered run pins the exact definition hash and provenance.

## Definition

This minimal definition is parser-backed by the documentation contract:

<!-- rust-workflow-example:start -->
```yaml
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: release
  version: 1.0.0
  description: Validate and report a native release
inputs:
  type: object
  additionalProperties: false
  required: [branch]
  properties:
    branch:
      type: string
outputs:
  type: object
capabilities:
  - git.status
maxConcurrency: 2
stepBudget: 20
steps:
  - id: status
    type: tool
    tool: git.status
    arguments: {}
    idempotency: null
  - id: report
    type: emit
    value:
      ok: true
compensation: []
```
<!-- rust-workflow-example:end -->

| Field | Contract |
| --- | --- |
| `apiVersion` | `colossus.dev/v1alpha1` |
| `kind` | `Workflow` |
| `metadata.name` | Stable workflow identity |
| `metadata.version` | Stable definition version |
| `metadata.description` | Bounded human description |
| `inputs`, `outputs` | Strict JSON Schema fragments |
| `capabilities` | Declared effect identities needed by the call graph |
| `maxConcurrency` | Definition-level concurrency bound |
| `stepBudget` | Complete run step bound |
| `steps` | Ordered root step definitions |
| `compensation` | Optional ordered agent/tool steps after a known failure |

Unknown fields, executable inline code, excessive budgets, invalid expressions, direct
or indirect call cycles, and call depth above 16 are rejected before registration.
Definitions are at most 1 MiB, `maxConcurrency` is `1..=64`, and `stepBudget` is
`1..=10000`.

## Step families

| Type | Purpose | Important fields |
| --- | --- | --- |
| `agent` | Bounded model work | `id`, `prompt`, `idempotency` |
| `tool` | Call an active strict tool | `id`, `tool`, `arguments`, `idempotency` |
| `workflow` | Start a pinned child workflow | `id`, `workflow`, `version`, `inputs` |
| `approval` | Wait for explicit authorization | `id`, `prompt` |
| `condition` | Choose a branch with non-executable logic | `id`, `expression`, `then`, `otherwise` |
| `parallel` | Run bounded branches | `id`, `branches`, `max_concurrency` |
| `foreach` | Repeat over bounded input | `id`, `items`, `max_items`, `steps` |
| `wait_for_input` | Persist a durable external input wait | `id`, `prompt`, `schema` |
| `emit` | Produce deterministic data | `id`, `value` |

Conditions support bounded JSON-pointer lookup, existence, equality, comparison, and
boolean composition. They cannot execute shell, language runtime code, Rego, or template
expressions. A condition is at most 16 KiB, 4,096 tokens, 128 nested levels, and 128
boolean-composition nodes. `foreach.max_items` is at most 1,000.

`compensation` accepts only agent or tool steps with an explicit idempotency strategy.
Each compensation effect is authorized independently through the normal effect gateway.

Each nested or repeated step receives a scoped execution identity such as
`each[1]/approval`. Completion, waiting input, permits, idempotency, retries, and child
links bind to that scoped identity.

## Run lifecycle

Run states are `queued`, `running`, `waiting`, `completed`, `failed`, `cancelled`, and
`interrupted`.

```bash
colossus workflow validate .colossus/workflows/release.yaml
colossus workflow register .colossus/workflows/release.yaml
colossus workflow run release 1.0.0 --inputs '{"branch":"main"}'
colossus workflow status RUN_ID
colossus workflow input RUN_ID '{"approved":true}'
colossus workflow resume RUN_ID
colossus workflow cancel RUN_ID
```

Effectful retry is valid only when the step declares an idempotency strategy.
Compensation is explicit and separately authorized. A started effect without terminal
evidence becomes an unknown outcome and is never implicitly replayed.

## Schedule contract

A schedule binds:

- an operator-selected identifier;
- exact registered workflow hash;
- validated input snapshot;
- fixed cadence from 60 seconds through 31 days;
- UTC start timestamp;
- `fire-once` or `skip` misfire policy;
- enable state and next occurrence.

Cron and local-time/DST semantics are outside this contract. Schedule transition and
deterministically identified queued run commit in one journal batch.

## Webhook contract

A webhook stores a credential reference, never its HMAC secret. The sender signs:

```text
TIMESTAMP + "\n" + DELIVERY_ID + "\n" + EXACT_RAW_JSON_BODY
```

The signature is lowercase hexadecimal, optionally prefixed `sha256=`. The top-level
workflow input is:

```json
{
  "body": {"branch": "main"},
  "delivery_id": "delivery-0001",
  "headers": {"x-event": "release"},
  "timestamp": "2026-08-01T02:00:00Z"
}
```

Timestamp freshness, delivery replay, HMAC, body/header bounds, definition trust, and
input schema are verified before policy dispatch. The optional HTTP listener binds only
to loopback and belongs behind a trusted reverse proxy.

## Subscription contract

A subscription selects one exact domain event type and optional stream prefix. Delivery
starts after the current journal head unless the operator intentionally supplies an
earlier sequence.

The input envelope includes `subscription_id`, a stable `idempotency_key`, and the
immutable source event with sequence, stream, actor, context, time, and payload. The
subscription checkpoint, delivery receipt, and deterministic queued run commit together.
Replaying the same event reuses that receipt and run.
