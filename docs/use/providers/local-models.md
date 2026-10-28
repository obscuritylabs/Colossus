---
title: Connect a local model server
description: Route Colossus to a loopback Chat Completions server and declare the model's capabilities.
audience: user
type: how-to
---

# Connect a local model server

Start your model server and load the model before configuring Colossus. It must expose
compatible `GET /v1/models` and `POST /v1/chat/completions` endpoints. Ollama, LM Studio,
and vLLM can expose these APIs, but support depends on the server version, model, and
enabled features.

Use a current configuration created by `colossus config init` (schema version 3).
Have the model's exact catalog ID, context and output limits, and tool-call and
streaming capabilities available.

## Configure the loopback route

The example assumes an unauthenticated server at `http://127.0.0.1:11434/v1`.
Confirm the server's actual bind address and port first.

Run `colossus -w . config effective` and edit the reported `resolution.configPath`.
Keep `storage` and your other settings. Merge these provider and model profiles into
the existing maps, set `models.roles.primary` to `local`, and retain any other model
roles and sandbox destinations. Replace `local-model` and the example limits with
the exact loaded model's values.

<!-- provider-guide-config:start -->
```yaml
schemaVersion: 3
providers:
  profiles:
    local-provider:
      kind: open_ai_compatible
      baseUrl: http://127.0.0.1:11434/v1
      credentialReference: null
      chatCompletionsOutputTokenParameter: max_tokens
models:
  profiles:
    local:
      providerProfile: local-provider
      model: local-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: local
sandbox:
  networkDestinations:
    - http://127.0.0.1:11434
```
<!-- provider-guide-config:end -->

Enable `toolCalls` only when both the server and model handle function tools and
tool history correctly. Enable `streaming` only when the server implements streaming
Chat Completions. Set unsupported capabilities to `false`.

You manage this server's lifecycle. `credentialReference: null` means no
authentication. A Desktop or other embedding host may manage a server and inject a
`host:IDENTIFIER` credential; the standard CLI does not resolve `host:` credentials.

The example sends `max_tokens`. Use `max_completion_tokens` when the server requires
it, or `omit` when it rejects both limit fields. `maxOutputTokens` remains Colossus's
output reservation in every mode.

## Check the local model

```bash
colossus -w . models route primary
colossus -w . provider models local-provider
colossus -w . provider doctor local-provider
colossus -w . models doctor local
```

The primary role should resolve to `local`, and the catalog should contain the loaded
model's exact ID. After both diagnostics succeed, send a small test request:

```bash
colossus -w . run "Reply with exactly: connected"
```

To check failure handling, stop the server and run `provider doctor local-provider`.
It should fail against this endpoint. Restart the server, reload the model, and rerun
both diagnostics before submitting more work.

## Troubleshoot the server

- **Connection refused:** start the server and check its bind address and port, then
  rerun `provider doctor local-provider`.
- **HTTP 503 while loading:** Colossus retries up to five times with increasing
  delays within the request deadline. If it still fails, wait for the server to report
  readiness and rerun `models doctor local` before resubmitting the test request.
- **HTTP 400 or malformed response:** check the exact model ID, catalog and Chat
  Completions compatibility. Correct the output-token parameter or disable unsupported
  tool-call and streaming capabilities, then rerun `models doctor local`.
- **Loopback denied under isolation:** grant the exact scheme, address, and port
  from `baseUrl` in the sandbox, without `/v1`.

For the compatible-server contract, see [Providers and models](../../reference/configuration/providers-models.md).
For deployment policy, see [Providers and routing](../../admin/providers-routing.md).
For remote services, see [Other OpenAI-compatible endpoints](openai-compatible.md).
