---
title: Connect another OpenAI-compatible endpoint
description: Connect a hosted or private Chat Completions service with explicit credentials, trust, and capabilities.
audience: user
type: how-to
---

# Connect another OpenAI-compatible endpoint

Use this guide for a hosted gateway or private Chat Completions service. Confirm its
exact HTTPS API prefix, model ID, catalog response, and generation response format.
An "OpenAI-compatible" label alone does not establish support for the contracts
Colossus validates.

Start with a current configuration created by `colossus config init` (schema version 3).
You also need the model's limits and capabilities, an environment-backed token if
authentication is required, and a PEM CA bundle if the service uses a private authority.

## Supply credentials and trust roots

Set `COLOSSUS_MODEL_TOKEN` in the environment of the Colossus process or through your
secret manager. Keep only `env:COLOSSUS_MODEL_TOKEN` in YAML. For an anonymous endpoint,
set `credentialReference: null` instead.

The example CA bundle contains public certificates, never private keys. Place it at
the configured workspace-owned path, or omit `network.caBundlePath` when the endpoint
already validates against public trust roots.

## Configure the compatible route

Run `colossus -w . config effective` and edit the reported `resolution.configPath`.
Keep `storage` and your other settings. Merge these provider and model profiles into
the existing maps, set `models.roles.primary` to `compatible`, and retain any other
model roles and sandbox destinations. Replace the example host, model, token limits,
capabilities, and CA path with your service's values.

<!-- provider-guide-config:start -->
```yaml
schemaVersion: 3
network:
  caBundlePath: .colossus/certs/provider-ca-bundle.pem
providers:
  profiles:
    compatible-provider:
      kind: open_ai_compatible
      baseUrl: https://models.example.com/v1
      credentialReference: env:COLOSSUS_MODEL_TOKEN
      chatCompletionsOutputTokenParameter: max_completion_tokens
models:
  profiles:
    compatible:
      providerProfile: compatible-provider
      model: example-model
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: compatible
sandbox:
  networkDestinations:
    - https://models.example.com
```
<!-- provider-guide-config:end -->

Include the API prefix in `baseUrl`, without `/models` or `/chat/completions`;
Colossus appends those operation paths. Under isolation, grant the exact origin
without the API path. Acknowledged full access already supplies network authority;
an extra destination does not restrict it.

`timeoutMs` controls transport inactivity, with defaults of five minutes for remote
hosts and fifteen minutes for `localhost` or IPv4/IPv6 loopback. Streaming generation
has a separate `generationTimeoutMs` wall-clock limit. See the
[timeout settings](../../reference/configuration/providers-models.md#timeoutms) before
changing either value.

Choose the output-token field your endpoint accepts:

| `chatCompletionsOutputTokenParameter` | Request behavior |
| --- | --- |
| `max_completion_tokens` | Sends the modern Chat Completions field used in the example. |
| `max_tokens` | Sends the legacy field; this is the default when the setting is absent. |
| `omit` | Sends neither field, for endpoints that reject both. |

Each mode uses the model's single `maxOutputTokens` budget. Colossus never sends both
fields or retries a generation with a different field. Set `toolCalls` and `streaming`
for the exact server and model. Provider profiles support bearer authentication and
do not expose arbitrary custom request headers.

PDF attachment support must be verified for the exact gateway and model. It requires
an explicitly enabled `capabilities.documentInputs` preference and native Chat
Completions file input; an OpenAI-compatible label or image support is insufficient.
DOCX, XLSX and PPTX are limited to the Responses adapter. See
[native document inputs](../../reference/configuration/providers-models.md#native-document-inputs).

## Check the connection

```bash
colossus -w . models route primary
colossus -w . provider models compatible-provider
colossus -w . provider doctor compatible-provider
colossus -w . models doctor compatible
```

The primary role should resolve to `compatible`. After both diagnostics succeed,
send a small test request:

```bash
colossus -w . run "Reply with exactly: connected"
```

Use `config effective` to check network authority, and `config show` to check the
credential reference and CA path. Neither should contain the token or a certificate
private key.

## Recover from connection failures

For HTTP 502–504 responses, the TUI and Desktop show a reconnecting status, retry count,
and countdown. Colossus retries up to five times after 1, 2, 4, 8, and 16 seconds; a
longer `Retry-After` takes precedence. All attempts share the original deadline.
Retries stop once the response stream starts. Interrupted streams and uncertain
transport failures are not replayed; inspect the original run before submitting again
when its outcome is unknown.

- **Credential unavailable:** check the environment of the process running Colossus,
  then rerun `provider doctor compatible-provider`.
- **HTTP 404:** check the API prefix in `baseUrl` and remove any explicit operation
  suffix such as `/models` or `/chat/completions`.
- **TLS validation failure:** correct `network.caBundlePath` and the PEM certificate
  chain, then rerun `provider doctor compatible-provider`.
- **Catalog works but generation fails:** check the model ID, limits, timeout,
  response format, output-token parameter, and capability flags. Correct the profile
  and rerun `models doctor compatible`.
- **Origin denied under isolation:** grant the exact scheme, host, and effective port
  in the sandbox, without the API path.

For field details, see [Providers and models](../../reference/configuration/providers-models.md).
For deployment policy, see [Providers and routing](../../admin/providers-routing.md).
