---
title: Connect the OpenAI API
description: Use the OpenAI Responses API with an environment-backed API key.
audience: user
type: how-to
---

# Connect the OpenAI API

This connection uses OpenAI API billing and the Responses API. To use an eligible
ChatGPT/Codex subscription instead, follow [Codex or ChatGPT subscription](codex-chatgpt.md).

You need an API key, an accessible model's exact ID and limits, and a current
configuration created by `colossus config init` (schema version 3).

## Make the API key available

Set `OPENAI_API_KEY` in the environment of the Colossus process, using your platform's
environment controls or secret manager. Keep only `env:OPENAI_API_KEY` in YAML.

## Configure the Responses route

Run `colossus -w . config effective` and edit the reported `resolution.configPath`.
Keep `storage` and your other settings. Merge these provider and model profiles into
the existing maps, set `models.roles.primary` to `openai`, and retain any other model
roles and sandbox destinations.

Replace `YOUR_OPENAI_MODEL_ID` and the example token limits with values for the
selected model. Declare only the capabilities that model supports.

<!-- provider-guide-config:start -->
```yaml
schemaVersion: 3
providers:
  profiles:
    openai-provider:
      kind: open_ai_responses
      baseUrl: https://api.openai.com/v1
      credentialReference: env:OPENAI_API_KEY
models:
  profiles:
    openai:
      providerProfile: openai-provider
      model: YOUR_OPENAI_MODEL_ID
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
<!-- provider-guide-config:end -->

Keep `/v1` in `baseUrl`: Colossus appends `/responses` and `/models` to that prefix.
Under isolation, the sandbox destination is the exact `https://api.openai.com` origin,
without `/v1`. Acknowledged full access already supplies network authority; an extra
destination does not restrict it.

The catalog does not fill in model limits or capabilities. Set them from the selected
model's documentation.

## Check the connection

```bash
colossus -w . models route primary
colossus -w . provider doctor openai-provider
colossus -w . provider models openai-provider
colossus -w . models doctor openai
```

The primary role should resolve to `openai`. After both diagnostics succeed, send
a small test request:

```bash
colossus -w . run "Reply with exactly: connected"
```

Check `colossus -w . config show` for the credential reference. It must not contain
the API key. Use `config effective` to check the configured network authority.

## Troubleshoot the connection

- **Credential unavailable or HTTP 401:** make sure `OPENAI_API_KEY` is set for the
  process running Colossus, then rerun `provider doctor openai-provider`.
- **Model absent or access denied:** run `provider models openai-provider`, use an
  exact accessible model ID, and rerun `models doctor openai`.
- **Origin denied under isolation:** grant `https://api.openai.com` in
  `sandbox.networkDestinations`, without `/v1`.
- **Generation rejected:** check the model's limits and disable `toolCalls` or
  `streaming` if unsupported, then rerun `models doctor openai`.

For field details, see [Providers and models](../../reference/configuration/providers-models.md).
For deployment policy, see [Providers and routing](../../admin/providers-routing.md).
Once connected, continue with [First repository task](../../get-started/first-repository-task.md).
