# Advanced workflow examples

These definitions exercise Colossus's strict workflow engine without hiding what the
current runtime supports. They are intended for a development workspace and use only
workspace-relative references.

## What each example covers

| File | Expected result | Features |
| --- | --- | --- |
| `01-control-flow-lab.yaml` | completes | conditions, bounded parallel branches, `foreach` |
| `02-operator-gated-deployment.yaml` | waits twice, then completes | structured input, explicit approval, nested parallel work |
| `03-component-review-loop.yaml` | waits once per component, then completes | scoped `foreach` input and per-item conditions |
| `04-component-health-check.yaml` | completes | reusable child workflow |
| `05-release-orchestrator.yaml` | completes after the child is registered | pinned parallel child workflows |
| `06-recovery-compensation-lab.yaml` | intentionally fails | idempotent retry and separately authorized compensation |
| `07-local-llm-parallel-review.yaml` | validates; see the note below | three parallel `agent` steps |

The examples are versioned fixtures. Change the workflow version before registering a
modified definition that you want to keep alongside the original.

## Validate everything

From the repository root:

```bash
for workflow in examples/workflows/*.yaml; do
  ./target/debug/colossus workflow validate "$workflow"
done
```

Validation checks the strict YAML contract but does not grant policy, sandbox, model,
or tool authority.

## Run the deterministic control-flow lab

```bash
./target/debug/colossus workflow register \
  examples/workflows/01-control-flow-lab.yaml
./target/debug/colossus workflow run control-flow-lab 1.0.0 \
  --inputs '{"environment":"staging","components":["api","worker","desktop"]}'
```

Try `environment` values `development`, `staging`, and `production` to exercise both
condition branches. The `foreach` output retains each component as the scoped `item`.

## Run the two-stage operator gate

```bash
./target/debug/colossus workflow register \
  examples/workflows/02-operator-gated-deployment.yaml
./target/debug/colossus workflow run operator-gated-deployment 1.0.0 \
  --inputs '{"environment":"staging","release":"2026.7.0"}'
```

Copy the returned `run_id`, then satisfy the structured release decision:

```bash
./target/debug/colossus workflow input RUN_ID \
  '{"approved":true,"change_ticket":"CHG-2026-0700"}'
```

The same run now waits for the explicit approval step:

```bash
./target/debug/colossus workflow input RUN_ID true
./target/debug/colossus workflow status RUN_ID
```

Use `{"approved":false,"change_ticket":"CHG-2026-0700"}` for the first response to
exercise the blocked branch without requesting the second approval.

## Run a durable per-component review

```bash
./target/debug/colossus workflow register \
  examples/workflows/03-component-review-loop.yaml
./target/debug/colossus workflow run component-review-loop 1.0.0 \
  --inputs '{"components":["api","worker","desktop"]}'
```

The run waits once for each component. Reuse the returned `run_id` and submit one
response at a time:

```bash
./target/debug/colossus workflow input RUN_ID \
  '{"status":"pass","notes":"focused checks passed"}'
./target/debug/colossus workflow input RUN_ID \
  '{"status":"needs_work","notes":"worker retry path needs review"}'
./target/debug/colossus workflow input RUN_ID \
  '{"status":"pass","notes":"desktop smoke test passed"}'
```

Status output identifies the scoped wait as `review-components[INDEX]/review`.

## Run pinned child workflows

Register the child before the parent so the complete call graph can be resolved:

```bash
./target/debug/colossus workflow register \
  examples/workflows/04-component-health-check.yaml
./target/debug/colossus workflow register \
  examples/workflows/05-release-orchestrator.yaml
./target/debug/colossus workflow run release-orchestrator 1.0.0 \
  --inputs '{"release":"2026.7.0"}'
```

The parent output contains three linked child run IDs and their pinned workflow hashes.

## Exercise retry and compensation

This example is supposed to finish in `failed` state:

```bash
./target/debug/colossus workflow register \
  examples/workflows/06-recovery-compensation-lab.yaml
./target/debug/colossus workflow run recovery-compensation-lab 1.0.0
```

The unavailable primary action has an idempotency strategy, so Colossus records one
bounded retry. It then dispatches the deterministic `echo` compensation independently.
Use `workflow status RUN_ID` and verbose events to inspect the evidence.

## Local-LLM parallel review

`07-local-llm-parallel-review.yaml` is the intended local-model example and is covered
by parser tests. It deliberately uses non-idempotent model steps, because a lost model
response has an unknown outcome and must not be silently replayed.

The composed workflow executor routes `agent.run` through the normal model runtime,
policy gateway, approval handling, provider error classification, and workflow
lineage. Register and run the example with:

```bash
./target/debug/colossus workflow register \
  examples/workflows/07-local-llm-parallel-review.yaml
./target/debug/colossus --approval-mode full-access \
  workflow run local-llm-parallel-review 1.0.0
```

The explicit approval mode is required for this non-interactive example because model
steps are approval-gated effects. Use it only in a disposable development workspace
after reviewing the workflow. Without it, the expected failure is `operator declined`.
With it, each `agent.run` step uses the configured primary model and sees only tools
whose exact names appear in the workflow's `capabilities` ceiling; every invocation
still crosses the normal policy and sandbox boundary.
