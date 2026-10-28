---
title: Connect OpenRouter
description: Route the primary model through OpenRouter's Chat Completions endpoint.
audience: user
type: how-to
---

# Connect OpenRouter

Use the `open_ai_compatible` adapter to connect to OpenRouter. You need an OpenRouter
API key, an exact catalog model ID and its limits, and a current configuration created
by `colossus config init` (schema version 3).

## Make the API key available

Set `OPENROUTER_API_KEY` in the environment of the Colossus process or through your
secret manager. Keep only `env:OPENROUTER_API_KEY` in YAML.

## Configure the OpenRouter route

Run `colossus -w . config effective` and edit the reported `resolution.configPath`.
Keep `storage` and your other settings. Merge these provider and model profiles into
the existing maps, set `models.roles.primary` to `openrouter`, and retain any other
model roles and sandbox destinations.

This example selects `openrouter/free`. Check that the route is available to your
account, or replace it with an exact model ID from the catalog. The token limits
and capability flags are examples; update them for the selected route.

<!-- provider-guide-config:start -->
```yaml
schemaVersion: 3
providers:
  profiles:
    openrouter-provider:
      kind: open_ai_compatible
      baseUrl: https://openrouter.ai/api/v1
      credentialReference: env:OPENROUTER_API_KEY
      chatCompletionsOutputTokenParameter: max_tokens
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
<!-- provider-guide-config:end -->

Keep `/api/v1` in `baseUrl`. Under isolation, grant only the origin
`https://openrouter.ai`. Acknowledged full access already supplies network authority;
an extra destination does not restrict it.

This profile uses bearer authentication. The current provider schema does not expose
arbitrary request headers.

The example sends the output limit as `max_tokens`. If the selected model requires
`max_completion_tokens`, change `chatCompletionsOutputTokenParameter` and rerun the
model diagnostic. `maxOutputTokens` remains the model's single output budget.
Colossus does not discover the right field by resubmitting a failed generation.

## Check the connection

```bash
colossus -w . models route primary
colossus -w . provider models openrouter-provider
colossus -w . provider doctor openrouter-provider
colossus -w . models doctor openrouter
```

The primary role should resolve to `openrouter`. After both diagnostics succeed,
send a small test request:

```bash
colossus -w . run "Reply with exactly: connected"
```

Check `colossus -w . config show`: the credential should remain an environment
reference. Use `config effective` to check the configured network authority.

## Troubleshoot the connection

- **Credential unavailable or HTTP 401:** make sure `OPENROUTER_API_KEY` is set for
  the process running Colossus, then rerun `provider doctor openrouter-provider`.
- **Model missing:** run `provider models openrouter-provider` and use an exact
  catalog identifier available to your account.
- **Catalog works but generation fails:** check the model limits, token parameter,
  tool-call support, and streaming support, then rerun `models doctor openrouter`.
- **Origin denied under isolation:** keep `/api/v1` in `baseUrl` and grant only
  `https://openrouter.ai` in the sandbox.

For compatibility requirements, see [Providers and models](../../reference/configuration/providers-models.md).
For deployment policy, see [Providers and routing](../../admin/providers-routing.md).
