# Durable Workflows

Versioned YAML workflows are first-class Rust application primitives. Repository
definitions live in `.colossus/workflows/*.yaml`; a configured user library may supply
additional definitions. Every run pins exact content hash and provenance.

## Validate, Register, And Run

```bash
colossus --config .colossus/config.yaml workflow validate \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow register \
  .colossus/workflows/release.yaml
colossus --config .colossus/config.yaml workflow list
colossus --config .colossus/config.yaml workflow show release 1.0.0
colossus --config .colossus/config.yaml workflow run release 1.0.0 \
  --inputs '{"branch":"main"}'
```

Use `--inputs @inputs.json` for a policy-bound file read. Use `--queued` to leave the
run for a worker rather than executing it in the initiating process.

## Definition Shape

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

The authoritative schema and examples live with the workflow crate tests. Validation is
strict: unknown fields, executable inline code, excessive budgets, invalid expressions,
direct/indirect call cycles, and call depth above 16 fail before registration.

Supported step families are `agent`, `tool`, `workflow`, `approval`, `condition`,
`parallel`, `foreach`, `wait_for_input`, and `emit`. Conditions use a bounded
non-executable grammar for JSON-pointer lookup, existence, equality, comparison, and
boolean operators. YAML cannot execute shell, Rust, JavaScript, Python, or Rego.

## Durable Operations

```bash
colossus --config .colossus/config.yaml workflow status RUN_ID
colossus --config .colossus/config.yaml workflow input RUN_ID \
  '{"approved":true}'
colossus --config .colossus/config.yaml workflow resume RUN_ID
colossus --config .colossus/config.yaml workflow cancel RUN_ID
```

Runs reconstruct from journal events with statuses `queued`, `running`, `waiting`,
`completed`, `failed`, `cancelled`, and `interrupted`. Each repeated/nested step receives
a scoped execution identity so one branch or iteration cannot consume another's permit,
input, result, or idempotency proof.

Effectful retry is permitted only when the step declares an idempotency strategy.
Compensation is explicit and separately authorized. A crash after an external effect
starts but before its terminal event becomes `outcome_unknown`; resume refuses unsafe
implicit replay.

## Worker Execution

```bash
colossus --config .colossus/config.yaml worker
colossus --config .colossus/config.yaml worker --status
colossus --config .colossus/config.yaml worker --once
```

The worker claims only queued runs, owns the canonical writer lease, and exposes the same
authenticated application API used by CLI/TUI. Waiting or interrupted runs are never
silently drained as new work.
