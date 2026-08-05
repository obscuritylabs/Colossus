---
title: Connect a model
description: Route Colossus through a Codex subscription, OpenAI Responses API, or an OpenAI-compatible model without placing credentials in YAML.
audience: user
type: how-to
---

# Connect a model

## Goal

Replace the offline `echo` route with a provider connection and explicit model profile
while keeping the credential outside configuration and granting only the provider's
exact network origin.

For a provider-specific copy/paste path, choose from
[Connect a model provider](../use/providers/index.md). This onboarding page retains the
single end-to-end starting flow; the focused guides cover Codex/ChatGPT, the OpenAI API,
OpenRouter, local servers, and other compatible endpoints separately.

## Prerequisites

- A completed [five-minute quickstart](quickstart.md).
- A provider account and model identifier. API-backed providers also need an API
  credential; a Codex subscription uses a ChatGPT sign-in instead.
- Permission to expose the provider's exact HTTPS origin from the Colossus sandbox.
- For an endpoint issued by a private CA, a PEM CA certificate bundle.

## Steps

### 1. Authenticate without placing a credential in YAML

For a Codex subscription, install the official Codex CLI and let it own the ChatGPT
OAuth flow. Colossus forces Codex's supported file-backed credential store so the
provider adapter can reuse and refresh that sign-in:

```bash
colossus codex login
colossus codex status
```

On a remote or headless machine, use `colossus codex login --device-code`. If Codex is
not on `PATH`, place `--codex-bin /absolute/path/to/codex` before the `login`, `status`,
or `logout` subcommand. These commands do not require a valid Colossus configuration.
Codex stores the sign-in under `$CODEX_HOME/auth.json`, or `~/.codex/auth.json` when
`CODEX_HOME` is unset. See OpenAI's
[Codex authentication documentation](https://learn.chatgpt.com/docs/app-server#authentication-endpoints)
for the underlying supported login modes and credential storage behavior.

For an API-key provider, use one process-scoped variable for the examples below. The prompt does not echo the
secret, and the command itself contains no credential value.

=== "macOS and Linux"

    ```bash
    printf "Provider API key: "
    IFS= read -rs COLOSSUS_PROVIDER_API_KEY
    printf "\n"
    export COLOSSUS_PROVIDER_API_KEY
    ```

=== "Windows PowerShell"

    ```powershell
    $secret = Read-Host "Provider API key" -AsSecureString
    $env:COLOSSUS_PROVIDER_API_KEY = [System.Net.NetworkCredential]::new("", $secret).Password
    ```

Use your platform's secure secret injection mechanism for persistent or unattended
operation. The process environment necessarily contains the resolved value while
Colossus runs; close the shell when finished. Do not paste a secret into
`.colossus/config.yaml`.

### 2. Add a provider profile and route

Edit `.colossus/config.yaml`.

If the provider uses a private CA, add the runtime-wide bundle once. Relative paths are
resolved from the selected workspace:

```yaml
network:
  caBundlePath: .colossus/certs/company-ca-bundle.pem
```

Publicly trusted endpoints can leave `caBundlePath` as `null` or omit the `network`
block.

=== "Codex/ChatGPT subscription"

    ```yaml
    providers:
      profiles:
        codex-provider:
          kind: open_ai_codex
          credentialReference: codex:default
          timeoutMs: 120000
    models:
      profiles:
        codex:
          providerProfile: codex-provider
          model: YOUR_CODEX_MODEL_ID
          contextWindowTokens: 128000
          maxOutputTokens: 16000
          reasoningEffort: high
          capabilities:
            toolCalls: true
            streaming: true
      roles:
        primary: codex

    sandbox:
      networkDestinations:
        - https://chatgpt.com
        - https://auth.openai.com
    ```

    `baseUrl` is intentionally omitted and cannot be overridden. The first origin is
    the subscription-backed Responses service; the second is used only when the
    Codex-managed access token enters its five-minute refresh window.

    `reasoningEffort` is optional. Valid values are `none`, `minimal`, `low`, `medium`,
    `high`, `xhigh`, `max`, and `ultra`; the selected Codex model may support only a
    subset. Omit it to use that model's backend default.

=== "OpenAI Responses"

    ```yaml
    providers:
      profiles:
        openai-provider:
          kind: open_ai_responses
          baseUrl: https://api.openai.com/v1
          credentialReference: env:COLOSSUS_PROVIDER_API_KEY
          timeoutMs: 120000
    models:
      profiles:
        openai:
          providerProfile: openai-provider
          model: YOUR_MODEL_ID
          contextWindowTokens: 128000
          maxOutputTokens: 16000
          capabilities:
            toolCalls: true
            streaming: true
      roles:
        primary: openai

    sandbox:
      networkDestinations:
        - https://api.openai.com
    ```

=== "OpenAI-compatible provider"

    ```yaml
    providers:
      profiles:
        openrouter-provider:
          kind: open_ai_compatible
          baseUrl: https://openrouter.ai/api/v1
          credentialReference: env:COLOSSUS_PROVIDER_API_KEY
          timeoutMs: 120000
    models:
      profiles:
        openrouter:
          providerProfile: openrouter-provider
          model: openrouter/free
          contextWindowTokens: 128000
          maxOutputTokens: 16000
          capabilities:
            toolCalls: true
            streaming: true
      roles:
        primary: openrouter

    sandbox:
      networkDestinations:
        - https://openrouter.ai
    ```

Merge the fragments into the generated file; keep its other required `sandbox` fields.
The origin grant contains only scheme, host, and effective port. The API path remains in
`baseUrl`.

### 3. Inspect routing and readiness

```bash
colossus --config .colossus/config.yaml models route primary
colossus --config .colossus/config.yaml provider doctor openai-provider
colossus --config .colossus/config.yaml models doctor openai
```

The route command is network-free. `provider doctor` checks the provider connection and
catalog. `models doctor` sends one bounded generation probe for the configured model;
its response content is not printed. Substitute `codex-provider` and `codex`, or the
matching OpenRouter names, when following those examples.

### 4. Send one bounded model turn

```bash
colossus --config .colossus/config.yaml run \
  "Reply with exactly: connected"
```

## Expected result

The route diagnostic names the configured profile, the provider doctor reports it ready,
and the model run returns `connected`.

## Verification

Inspect the active route and recent redacted audit envelopes:

```bash
colossus --config .colossus/config.yaml provider profiles
colossus --config .colossus/config.yaml models profiles
colossus --config .colossus/config.yaml audit show --limit 10
```

The credential value must not appear in configuration, output, or audit evidence.

## Failure path

- **Credential unavailable:** for Codex, run `colossus codex status` and sign in again;
  for an API provider, confirm that the referenced variable is present in the Colossus
  process environment.
- **Origin absent from the sandbox:** add the exact provider origin, not its URL path.
- **Provider or model not found:** verify `kind`, `baseUrl`, and `model` with the
  provider.
- **TLS or certificate failure:** set `network.caBundlePath` to the PEM bundle that
  issued the endpoint certificate, then rerun `provider doctor`.
- **Request denied:** inspect `config effective`; provider visibility, action policy,
  approval, and network grants are separate decisions.
- **Outcome unknown:** inspect provider-side usage before retrying. Colossus does not
  silently repeat a request that may have reached the service.

## Next step

Give the model a constrained workspace in
[First repository task](first-repository-task.md).
