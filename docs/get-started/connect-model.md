---
title: Connect a model
description: Route Colossus through a Codex subscription, OpenAI Responses API, or an OpenAI-compatible model without placing credentials in YAML.
audience: user
type: how-to
---

# Connect a model

## Goal

Replace the offline `echo` route with a provider connection and explicit model profile
while keeping the credential outside configuration. Under an isolating execution
boundary, grant only the provider's exact network origin.

For a provider-specific copy/paste path, choose from
[Connect a model provider](../use/providers/index.md). This onboarding page retains the
single end-to-end starting flow; the focused guides cover Codex/ChatGPT, the OpenAI API,
OpenRouter, local servers, and other compatible endpoints separately.

## Prerequisites

- A completed [five-minute quickstart](quickstart.md).
- A provider account and model identifier. API-backed providers also need an API
  credential; a Codex subscription uses a ChatGPT sign-in instead.
- Permission to expose the provider endpoint. Under isolation, authorize its exact
  HTTPS origin in the Colossus sandbox.
- For an endpoint issued by a private CA, a PEM CA certificate bundle.

## Steps

### Guided setup

For a new configuration, run `colossus provider setup`. In a terminal, choose a
provider from the shared preset list, load its model catalog, and select a model by
number or ID. Authenticate first using `colossus codex login` for Codex, or set the
environment variable shown by `colossus provider presets` for an API-key service.
Keys are never command-line arguments or YAML values.

```bash
colossus provider presets
colossus provider discover --preset openrouter
colossus provider setup --preset openrouter
```

Presets include Codex, OpenAI, OpenRouter, Groq, Together AI, DeepSeek, Mistral,
Ollama, and LM Studio. Choose `custom-chat` or `custom-responses` for another
compatible service and supply its API **base** URL, including any version prefix:

```bash
colossus provider discover --preset custom-responses \
  --base-url http://localhost:1234/v1 --no-credential
```

Model cards retain provider-reported names, descriptions, context/output limits,
capabilities, and reasoning options when available. A bare `/models` response may
contain only IDs; Colossus does not infer missing capabilities from a model name.
Setup uses 32,768 context and 4,096 output tokens when metadata is absent (with a
smaller output reservation for small contexts). Unknown capabilities remain off.
Pass `--context-window-tokens`, `--max-output-tokens`, `--tool-calls true`,
`--streaming true`, or `--image-inputs true` to declare supported features.

For manual or unattended setup, `--model` skips catalog discovery. This also works
when a server supports generation but has no model-list endpoint:

```bash
colossus --config new-provider.yaml provider setup --preset custom-chat \
  --base-url https://gateway.example.com/v1 --credential-env PROVIDER_API_KEY \
  --model YOUR_MODEL_ID --context-window-tokens 128000 \
  --max-output-tokens 16000 --tool-calls true --streaming true
```

Setup creates a user-level configuration, or a repository configuration with
`--local`. It refuses to overwrite an existing file; use `--config NEW_PATH` to
prepare a separate configuration. `provider discover` works before configuration
exists and sends only a catalog request through the normal policy and audit path.
Discovery evidence uses a separate `provider-discovery.redb` journal in the CLI
workspace partition. For an existing configuration, `provider models PROFILE`
loads the same normalized cards using that configured connection.

In Desktop, select a provider during setup, enter a custom base URL if needed, then
choose **Load models**. API keys are entered in the native credential prompt and
Codex uses its account sign-in. Select a model card to fill advertised metadata;
manual model entry and advanced overrides remain available. Switching connections
clears the prior catalog so results from another provider cannot be selected.

The following steps document manual configuration and credential setup in detail.

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
`CODEX_HOME` is unset. When set, `CODEX_HOME` must be absolute. After the official CLI
exits successfully, Colossus validates that `login` and `status` produced a
credential that passes runtime validation before reporting
`completed: true`; `logout` reports completion only after that credential is no longer
usable. Colossus rejects an existing auth file that fails runtime safety validation
before invoking the account command; a failing remaining file is an error, not a
successful logout. See OpenAI's
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
Colossus runs; close the shell when finished. Do not paste a secret into the selected
configuration.

### 2. Add a provider profile and route

Run `colossus config effective` and edit the reported `resolution.configPath`. After the
quickstart this is normally `$COLOSSUS_HOME/config.yaml`; a repository-local
`.colossus/config.yaml` is a complete higher-priority replacement, not an overlay.

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

Merge the provider and model fragments into the generated file. The shown sandbox
fragments are exact grants for an explicitly isolating boundary; their origin contains
only scheme, host, and effective port, while the API path remains in `baseUrl`.
Acknowledged full access needs no duplicate destination and adding one does not narrow
ambient HTTP(S) authority. See [Sandbox configuration](../reference/configuration/sandbox.md)
before treating an origin list as confinement.

### 3. Inspect routing and readiness

```bash
colossus -w . models route primary
colossus -w . provider doctor openai-provider
colossus -w . models doctor openai
```

The route command is network-free. `provider doctor` checks the provider connection and
catalog. `models doctor` sends one bounded generation probe for the configured model;
its response content is not printed. Substitute `codex-provider` and `codex`, or the
matching OpenRouter names, when following those examples.

### 4. Send one bounded model turn

```bash
colossus -w . run \
  "Reply with exactly: connected"
```

## Expected result

The route diagnostic names the configured profile, the provider doctor reports it ready,
and the model run returns `connected`.

## Verification

Inspect the active route and recent redacted audit envelopes:

```bash
colossus -w . provider profiles
colossus -w . models profiles
colossus -w . audit show --limit 10
```

The credential value must not appear in configuration, output, or audit evidence.

## Failure path

- **Credential unavailable:** for Codex, run `colossus codex status` and sign in again;
  for an API provider, confirm that the referenced variable is present in the Colossus
  process environment.
- **Origin denied under isolation:** add the exact provider origin, not its URL path.
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
