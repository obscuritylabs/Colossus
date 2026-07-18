---
title: Your first workflow
description: Create, validate, register, and run a deterministic Colossus workflow with no external effects.
audience: developer
type: tutorial
---

# Your first workflow

## Goal

Create a versioned YAML workflow that emits a deterministic result, then validate,
register, run, and inspect it through Colossus.

## Prerequisites

- An initialized Colossus configuration.
- A configured repository workflow root, normally `.colossus/workflows`.
- Permission to create a file in that root.

This tutorial uses no provider, process, filesystem, or network effect.

## Steps

### 1. Create the definition

Save this as `.colossus/workflows/hello.yaml`:

```yaml
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: hello
  version: 1.0.0
  description: Emit a deterministic greeting
inputs:
  type: object
  additionalProperties: false
  properties:
    name:
      type: string
outputs:
  type: object
capabilities: []
maxConcurrency: 1
stepBudget: 2
steps:
  - id: greeting
    type: emit
    value:
      message: Hello from Colossus
```

Workflow YAML is declarative. It cannot contain executable inline shell, Rust,
JavaScript, Python, or Rego.

### 2. Validate before registration

```bash
colossus --config .colossus/config.yaml workflow validate \
  .colossus/workflows/hello.yaml
```

Validation rejects unknown fields, invalid schemas and expressions, unsafe cycles, and
excessive bounds.

### 3. Register the exact content

```bash
colossus --config .colossus/config.yaml workflow register \
  .colossus/workflows/hello.yaml
colossus --config .colossus/config.yaml workflow show hello 1.0.0
```

Registration pins the exact content hash and provenance. Editing the file later creates
a different trust identity.

### 4. Run and inspect

```bash
colossus --config .colossus/config.yaml workflow run hello 1.0.0 \
  --inputs '{"name":"operator"}'
colossus --config .colossus/config.yaml workflow status WORKFLOW_RUN_ID
```

## Expected result

The run completes with an emitted object containing `message: Hello from Colossus`.
Status identifies the registered workflow hash and completed step.

## Verification

```bash
colossus --config .colossus/config.yaml workflow list
colossus --config .colossus/config.yaml audit verify
```

Confirm that the registered definition appears and the journal verifies.

## Failure path

- **Definition is rejected:** fix the first validation error and rerun validation; do
  not register around it.
- **Registered hash differs:** inspect the exact file bytes and register the intended
  content.
- **Input is rejected:** make the JSON object conform to `inputs`; unknown properties
  are rejected in this example.
- **Run remains queued:** execute without `--queued` or start the worker.

## Next step

Use [Workflow authoring](authoring.md) for step composition and
[Triggers and recovery](triggers-recovery.md) for unattended execution.
