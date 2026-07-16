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

## Persisted Schedules

Schedules bind a fixed UTC cadence to an exact registered workflow hash and validated
input snapshot. They are canonical journal state, not terminal or worker configuration.

```bash
colossus --config .colossus/config.yaml workflow schedule create nightly \
  release 1.0.0 --cadence-seconds 86400 --inputs '{"branch":"main"}' \
  --misfire fire-once --starts-at 2026-08-01T02:00:00Z
colossus --config .colossus/config.yaml workflow schedule list
colossus --config .colossus/config.yaml workflow schedule show nightly
colossus --config .colossus/config.yaml workflow schedule disable nightly
colossus --config .colossus/config.yaml workflow schedule enable nightly
colossus --config .colossus/config.yaml workflow schedule tick \
  --at 2026-08-02T02:00:00Z
```

Cadence is bounded from 60 seconds through 31 days. `--starts-at` accepts only UTC
RFC3339 with the `Z` offset; when omitted, the first occurrence is one cadence after
creation. Fixed cadence is intentional—cron expressions and local-time/DST semantics are
not part of this contract.

When multiple occurrences are overdue, `fire-once` queues exactly one deterministic run
for the latest occurrence and records how many earlier occurrences were missed. `skip`
records and advances beyond the complete backlog without queuing a catch-up run. A
single due occurrence always queues once. The schedule transition and queued run are one
atomic journal batch, so restart or process loss cannot advance the schedule without its
run or create the same occurrence twice.

Changing, removing, or invalidating the pinned workflow definition disables a due
schedule with a bounded reason. Explicit enable rechecks the definition hash, complete
call graph, and saved inputs. The long-running worker evaluates schedules during its
one-second coordinated drain; `worker --once` does the same once. The explicit
`workflow schedule tick` command only evaluates and queues due runs, leaving execution
to the ordinary worker or workflow drain path.

## Authenticated Webhooks

Webhooks bind an identifier and late-resolved HMAC secret reference to an exact
registered workflow hash. The referenced environment variable must contain at least 32
bytes; its value is never stored in configuration or the journal.

```bash
export COLOSSUS_RELEASE_WEBHOOK_SECRET='replace-with-at-least-32-random-bytes'
colossus --config .colossus/config.yaml workflow webhook create release-hook \
  release 1.0.0 --secret-reference env:COLOSSUS_RELEASE_WEBHOOK_SECRET \
  --replay-window-seconds 300 --max-body-bytes 65536
colossus --config .colossus/config.yaml workflow webhook list
colossus --config .colossus/config.yaml workflow webhook show release-hook
colossus --config .colossus/config.yaml workflow webhook disable release-hook
colossus --config .colossus/config.yaml workflow webhook enable release-hook
```

The sender signs these exact bytes with HMAC-SHA256:

```text
TIMESTAMP + "\n" + DELIVERY_ID + "\n" + EXACT_RAW_JSON_BODY
```

The signature is lowercase hexadecimal, optionally prefixed by `sha256=`. Timestamp is
UTC RFC3339 with `Z`; a delivery identifier can be accepted only once. To exercise the
same ingress path without HTTP:

```bash
colossus --config .colossus/config.yaml workflow webhook ingest release-hook \
  --delivery-id delivery-0001 --timestamp 2026-08-01T02:00:00Z \
  --signature 'sha256=LOWERCASE_HEX_HMAC' --header x-event=release \
  --body '{"branch":"main"}'
```

Webhook runs receive an envelope rather than the raw body as their top-level input:

```json
{
  "body": {"branch": "main"},
  "delivery_id": "delivery-0001",
  "headers": {"x-event": "release"},
  "timestamp": "2026-08-01T02:00:00Z"
}
```

The workflow input schema must declare or otherwise allow that envelope. Authentication,
replay, body/header bounds, definition trust, and schema validation occur before the
ordinary `workflow.webhook.ingest` policy request. Acceptance and its deterministic
queued run commit atomically; execution still follows the normal worker queue and every
effect inside the workflow remains independently authorized.

For a trusted local reverse proxy, run the bounded loopback listener:

```bash
colossus --config .colossus/config.yaml workflow webhook serve \
  --bind 127.0.0.1:8787
```

Send POST requests to `/v1/workflow-webhooks/WEBHOOK_ID` with `Content-Length`,
`X-Colossus-Delivery-Id`, `X-Colossus-Timestamp`, and `X-Colossus-Signature` headers.
The listener deliberately has no public bind, TLS, chunked transfer, or rate limiter;
deploy those at the reverse proxy. It automatically routes through the authenticated
worker when one is active and otherwise uses the embedded runtime.

## Worker Execution

```bash
colossus --config .colossus/config.yaml worker
colossus --config .colossus/config.yaml worker --status
colossus --config .colossus/config.yaml worker --once
```

The worker evaluates due schedules, claims only queued runs, owns the canonical writer
lease, and exposes the same authenticated application API used by CLI/TUI. Waiting or
interrupted runs are never silently drained as new work.
