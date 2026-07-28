# Application SDK examples

These examples exercise the public, authenticated application API rather than the
private CLI worker protocol. The same durable-run behavior is implemented for every
supported SDK:

| SDK | Example source | How it is checked |
| --- | --- | --- |
| Rust | `../../crates/colossus-sdk/examples/durable_run.rs` | compiled as a Cargo example |
| Python | `../../sdk/python/examples/durable_run.py` | Ruff and strict mypy |
| TypeScript | `../../sdk/typescript/examples/durable-run.ts` | strict TypeScript build |
| Go | `../../sdk/go/examples/durable-run/durable_run.go` | `go test` and `go vet` |

Each implementation creates an effectful run exactly once, uses the SDK's durable
read-only watch reconnection, records compact released tool activity, requires an
explicit callback for one-use interactions, and preserves known failure metadata.
The examples never automatically retry `CreateRun`, a response, or an
outcome-unknown effect.

## Authority model

SDKs create, observe, cancel, and answer interactions for runs. They do not bypass the
runtime by calling an integration or tool directly. A tool must exist in the connected
Colossus runtime, be included in the application's enrollment grant, and remain
permitted by runtime policy and sandbox configuration.

Bearer credentials do not belong in argv, environment variables, endpoint
descriptors, fixture files, logs, or renderer memory. Enrollment writes the bearer
directly to an application-selected OS credential-store entry. The application keeps
the independently provisioned instance ID and certificate fingerprint separate from
the mutable discovery directory.

## Enroll a development application

Stop the isolated worker before enrollment. Replace the paths and keyring names with
values owned by your test application:

```bash
./target/debug/colossus --config .colossus/config.yaml worker --shutdown
./target/debug/colossus --config .colossus/config.yaml worker \
  --public-api-dir "$PWD/.colossus/sdk-public-api" \
  --enroll-application app:sdk-example \
  --scope runs:execute \
  --scope runs:read \
  --scope runs:control \
  --scope prompts:respond \
  --scope approvals:respond \
  --role primary \
  --tool user.ask \
  --credential-keyring-service dev.obscuritylabs.colossus.sdk-example \
  --credential-keyring-account local
```

Enrollment prints only non-secret metadata. Record its stable `instance_id` and
`certificate_sha256` in application-owned trusted configuration. Then start the
worker:

```bash
./target/debug/colossus --config .colossus/config.yaml worker \
  --public-api-dir "$PWD/.colossus/sdk-public-api"
```

The generic keyring provider is suitable for this local acceptance test. A packaged
application with a stronger same-user threat model should supply a platform-bound
credential provider and verify its signed code identity.

## Run the Rust client

The Rust example is the complete native application composition. It loads the bearer
from the named OS credential-store entry and independently pins the enrolled server:

```bash
SDK_PROMPT="$(<examples/sdk/scenarios/01-model-smoke.txt)"
cargo run -p colossus-sdk --example durable_run -- \
  "$PWD/.colossus/sdk-public-api" \
  INSTANCE_ID_FROM_ENROLLMENT \
  CERTIFICATE_SHA256_FROM_ENROLLMENT \
  dev.obscuritylabs.colossus.sdk-example \
  local \
  "$SDK_PROMPT"
```

Add `--plan` before the prompt for Scenario 02. Add `--answer blue` for Scenario 04.
Approvals are denied by default; `--approve-effects` is intentionally explicit and
should be used only after displaying the released approval to an operator.

For a local model smoke test that must not use a credential store, run the isolated
ephemeral host instead:

```bash
SDK_PROMPT="$(<examples/sdk/scenarios/01-model-smoke.txt)"
cargo run -p colossus-cli --example sdk_ephemeral_local -- \
  "$PWD/.colossus/config.dev.yaml" \
  rust \
  "$SDK_PROMPT"
```

This development-only harness still uses the real Rust SDK, pinned TLS public API,
durable run API, and configured model provider. It creates a fresh worker authentication
root, TLS identity, instance ID, and application bearer in process memory for one run,
then shuts the worker down. It never enrolls an application, touches the OS credential
store, or serializes the bearer into argv, environment variables, files, discovery
metadata, logs, or renderer state. It intentionally supports only the noninteractive
model smoke scenario; use normal enrollment for persistent applications and interactive
runs.

The same ephemeral host can launch the Python, TypeScript, and Go SDK clients. Their
bearer travels once over an anonymous child-stdin pipe and is never placed in ambient
process state. Prepare the portable runners, then replace `rust` above with `python`,
`typescript`, or `go`:

```bash
# TypeScript
(cd sdk/typescript && npm exec -- tsc -p tsconfig.live.json)

# Go
(cd sdk/go && go build -o ../../target/sdk-go-live ./examples/live-run)
```

Python uses the isolated interpreter prepared by the SDK gate at
`sdk/python/.codegen/bin/python`. The live sources are
`sdk/python/examples/live_run.py`, `sdk/typescript/examples/live-run.ts`, and
`sdk/go/examples/live-run/main.go`. Each composes the language SDK's secure connector
with its existing `run_prompt`/`runPrompt`/`RunPrompt` application core.

## Scenarios

| File | Expected evidence | Required grant |
| --- | --- | --- |
| `01-model-smoke.txt` | exact `SDK_SUITE_OK` terminal output | provider only |
| `02-plan-mode.txt` | a plan, with no workspace/external mutation | plan run |
| `03-tool-activity.txt` | compact requested/started/completed rows for a read tool | exact workspace read tool |
| `04-user-interaction.txt` | a durable `user.ask` interaction and resumed result | `user.ask`, `prompts:respond` |
| `05-openapi-integration.txt` | `openapi.sdk-demo.getstatus` activity and `green` result | exact generated integration tool |
| `06-provider-failure.txt` | reason, recoverability, HTTP status, retry delay, certainty | intentionally unavailable provider |

Scenario 06 is a negative test. Start `provider-failure/server.py` and point a separate
OpenAI-compatible provider profile at `http://127.0.0.1:8100/v1`. The fixture
advertises one model but returns HTTP 503 and `Retry-After: 2` for generation. Do not
disturb the model profile used for the successful scenarios.

## Exercise a real OpenAPI integration

The fixture is credential-free and loopback-only. Start it in one terminal:

```bash
python3 examples/sdk/integration/server.py
```

Its OpenAPI document is fixed to `http://127.0.0.1:8099`. Add that exact origin to the
test sandbox's `networkDestinations`, then import the connection before enrolling the
SDK application:

```bash
./target/debug/colossus --config .colossus/config.yaml \
  --approval-mode full-access integrations import-openapi \
  sdk-demo examples/sdk/integration/openapi.json --auth-type none
```

Repeat enrollment with `--tool openapi.sdk-demo.getstatus`, restart the worker, and use
Scenario 05. A successful result proves the full chain:

```text
SDK client → authenticated public run → model tool call → policy/sandbox →
OpenAPI adapter → loopback fixture → released tool activity → terminal output
```

The direct `integrations call` CLI is useful for adapter diagnosis, but it is not a
substitute for this SDK run because it does not test model tool selection or public
application authority.

## Exercise provider failure metadata

Start the deterministic fixture:

```bash
python3 examples/sdk/provider-failure/server.py
```

Add a separate provider/model profile and role to your disposable configuration:

```yaml
providers:
  profiles:
    sdk-failure:
      kind: open_ai_compatible
      baseUrl: http://127.0.0.1:8100/v1
      credentialReference: null
      timeoutMs: 5000
models:
  profiles:
    sdk-failure:
      providerProfile: sdk-failure
      model: sdk-failure-model
      contextWindowTokens: 8192
      maxOutputTokens: 1024
      capabilities:
        toolCalls: false
        streaming: true
  roles:
    risk_evaluator: sdk-failure
```

Grant the exact loopback origin in `sandbox.networkDestinations`, enroll a separate
application for role `risk_evaluator`, and run Scenario 06 with that role. The terminal
failure should report `provider.temporarily_unavailable`, `recoverable=true`,
`http_status=503`, `retry_after_ms=2000`, and known outcome certainty. It must not
contain response headers, a response body, credentials, or private paths.

## What to inspect

- A lost watch connection may reconnect from its last exclusive cursor; no duplicate
  update should be presented.
- A sequence gap must fail closed.
- A waiting prompt or approval must include an opaque interaction ID and etag, and an
  approval answer must echo its one-use request hash.
- Tool activity contains the registered tool name and bounded status, never raw
  arguments, credentials, private paths, or quarantined output.
- `http_status` and `retry_after_ms` appear only when an upstream response supplied
  them. An unknown outcome is never automatically retried.
- Dropping a watcher does not cancel its durable run; reconnect with `GetRun` and
  `WatchRun(after_sequence=...)`.
