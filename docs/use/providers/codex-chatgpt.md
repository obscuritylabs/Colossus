---
title: Connect a Codex or ChatGPT subscription
description: Sign in through Codex and use your subscription for the primary Colossus model.
audience: user
type: how-to
---

# Connect a Codex or ChatGPT subscription

Use this connection with a ChatGPT plan that includes Codex access. Colossus delegates
sign-in to the official Codex CLI and uses its file-backed credential. A ChatGPT
subscription does not provide OpenAI API credit; use [OpenAI API](openai-api.md) for
API-key authentication and separate API billing.

You need the official Codex CLI on `PATH` and a current configuration created by
`colossus config init` (schema version 3).

## Sign in through Codex

```bash
colossus codex login
colossus codex status
```

On a headless machine, use `colossus codex login --device-code`. If Codex is not on
`PATH`, put `--codex-bin /absolute/path/to/codex` before `login`, `status`, or `logout`.
If you set `CODEX_HOME`, use an absolute path.

A successful status check confirms that the official CLI completed and Colossus can
safely load its credential. The token never belongs in YAML. Colossus rejects an
existing unsafe auth file before starting an account operation.

To remove the sign-in, run `colossus codex logout`. It reports completion only after
the credential is unavailable; an unsafe remaining store fails verification.

## Configure the subscription route

Run `colossus -w . config effective` and edit the reported `resolution.configPath`.
Keep `storage` and your other settings. Merge the provider and model profiles below
into the existing maps, set `models.roles.primary` to `codex`, and retain any other
model roles and sandbox destinations.

Replace `YOUR_CODEX_MODEL_ID` with an exact model ID available to your account. The
token limits are examples; use the selected model's actual limits.

<!-- provider-guide-config:start -->
```yaml
schemaVersion: 3
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
<!-- provider-guide-config:end -->

The subscription adapter owns its backend URL, so do not add `baseUrl`. It sends the
credential only to its fixed service and refresh endpoints. Under isolation, retain
both sandbox origins so an expiring token can refresh. Acknowledged full access
already supplies authority for both endpoints.

Set `reasoningEffort` only to a level the selected model supports, or omit it to use
the backend default.

## Check the connection

```bash
colossus -w . models route primary
colossus -w . provider doctor codex-provider
colossus -w . provider models codex-provider
colossus -w . models doctor codex
```

The primary route should resolve to `codex`. After both diagnostics succeed, send
a small test request:

```bash
colossus -w . run "Reply with exactly: connected"
```

Check `colossus -w . config show`: it should contain
`credentialReference: codex:default`, without a token or ChatGPT account identifier.

## Attach native documents

For a file-capable Codex model, set `models.profiles.codex.capabilities.documentInputs`
to `on`. The preference defaults to Off and is separate from image support.
PDF, DOCX, XLSX and PPTX use the same signed-in subscription route; no API key is
needed. Colossus sends verified document bytes inline, and the provider handles
their contents without a local document parser or a separate Files upload.

Attach files with CLI `--attach` or Desktop's composer attachment control. Support
depends on the exact selected model and account. See
[native document inputs](../../reference/configuration/providers-models.md#native-document-inputs)
for the format matrix, limits and context behavior.

## Troubleshoot sign-in and model access

- **Missing or expired sign-in:** run `colossus codex status`, then sign in again.
  The Codex auth file must be a regular, owner-private file.
- **The backend requires a newer Codex version:** check `colossus --version` and
  upgrade Colossus, then rerun `provider doctor`. The adapter's reviewed wire
  contract changes with Colossus; updating the login CLI alone does not change it.
- **A model is missing from the catalog:** upgrade Colossus, then run
  `provider models codex-provider` or choose **Load models** in Desktop. Availability
  also depends on the account rollout, sign-in method, and workspace permissions.
  Use exact returned IDs; see [OpenAI's model availability guidance](https://learn.chatgpt.com/docs/models).
- **A catalog model is missing from Desktop's composer:** open
  **Settings → Global → Models → Add model**, select the Codex provider, choose the
  model with **Load models**, and save with **Add model**. Then select the saved
  model in **Workspace → Models** and choose **Apply Workspace changes**. Loading
  the catalog alone does not save a model or select it for the workspace.
- **The account flow fails:** check `codex --version`, update the official CLI, and
  retry login or the device-code flow.
- **A model or reasoning level is rejected:** use an exact catalog model ID and
  correct or omit `reasoningEffort`, then rerun `models doctor codex`.
- **Refresh is blocked under isolation:** retain both sandbox destinations shown
  above and rerun `provider doctor codex-provider`.

For field details, see [Providers and models](../../reference/configuration/providers-models.md).
For deployment policy, see [Providers and routing](../../admin/providers-routing.md).
Once connected, continue with [First repository task](../../get-started/first-repository-task.md).
