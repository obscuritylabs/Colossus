---
title: Workflow authoring
description: Author bounded Colossus workflows with explicit schemas, capabilities, and deterministic step composition.
audience: developer
type: how-to
---

# Workflow authoring

## Goal

Author a workflow that declares its input contract, capability ceiling, execution
bounds, and ordered work before it is registered.

## Prerequisites

- A completed [first workflow](first-workflow.md).
- Familiarity with JSON Schema object validation.
- The exact tool names and action classes required by effectful steps.

## Steps

### 1. Start with the complete document shape

The following fenced example is parser-checked by the repository documentation contract.

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
```
<!-- rust-workflow-example:end -->

The schemas describe the top-level input and output values. `capabilities` is the
definition ceiling; it does not grant policy or sandbox authority.

### 2. Choose a step family

Use the family that expresses the transition directly:

- `agent` for one bounded model task;
- `tool` for one strict tool invocation;
- `workflow` for a registered child definition;
- `approval` for an explicit authorization transition;
- `condition` for bounded branching;
- `parallel` or `foreach` for scoped repeated work;
- `wait_for_input` for a durable external response; and
- `emit` for a deterministic value.

Conditions use a non-executable grammar for JSON-pointer lookup, existence, comparison,
equality, and boolean operators.

### 3. Bound concurrency and work

Set `maxConcurrency` to the smallest useful branch concurrency and `stepBudget` to an
upper bound that includes repeated and nested steps. Nested workflow calls have a
maximum depth and cycle validation before registration.

### 4. Design effectful retries

Declare an idempotency strategy only when the target operation provides a real stable
identity. Compensation is a separate authorized effect. Never treat a timeout as proof
that an external operation did not happen.

### 5. Validate the finished graph

```bash
colossus --config .colossus/config.yaml workflow validate \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow register \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow show release 1.0.0
```

## Expected result

Validation accepts one bounded, acyclic graph whose declared capabilities cover its
steps. Registration records the exact content hash and provenance.

## Verification

Change a harmless byte in a copy, validate it, and compare its hash identity with the
registered definition. Restore the intended file before running. This demonstrates that
registration trusts exact content rather than a mutable path.

## Failure path

- **Unknown field or step type:** use the exact
  [workflow schema](../../reference/workflow-schema.md).
- **Capability is missing:** add the exact declared capability, then separately confirm
  access, policy, and sandbox configuration.
- **Cycle or depth failure:** flatten the call graph or split ownership between
  independently triggered workflows.
- **Retry is refused:** provide a real idempotency strategy or require operator
  reconciliation.

## Next step

Add schedules, webhooks, or repository events with
[Triggers and recovery](triggers-recovery.md).
