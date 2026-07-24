---
title: Connect a model
description: Route Colossus to an OpenAI Responses or OpenAI-compatible model without placing credentials in YAML.
audience: user
type: how-to
---

# Connect a model

## Goal

Replace the offline `echo` route with a provider connection and explicit model profile
while keeping the credential outside configuration and granting only the provider's
exact network origin.

## Prerequisites

- A completed [five-minute quickstart](quickstart.md).
- A provider account, model identifier, and API credential.
- Permission to expose the provider's exact HTTPS origin from the Colossus sandbox.

## Steps

### 1. Inject the credential without placing it in command history

Use one process-scoped variable for the examples below. The prompt does not echo the
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
its response content is not printed. Use the matching OpenRouter profile names when
following that example.

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

- **Credential unavailable:** confirm that the referenced variable is present in the
  environment of the Colossus process.
- **Origin absent from the sandbox:** add the exact provider origin, not its URL path.
- **Provider or model not found:** verify `kind`, `baseUrl`, and `model` with the
  provider.
- **Request denied:** inspect `config effective`; provider visibility, action policy,
  approval, and network grants are separate decisions.
- **Outcome unknown:** inspect provider-side usage before retrying. Colossus does not
  silently repeat a request that may have reached the service.

## Next step

Give the model a constrained workspace in
[First repository task](first-repository-task.md).
