---
title: Connect a model provider
description: Choose the Colossus provider path that matches your subscription, API key, local server, or compatible endpoint.
audience: user
type: concept
---

# Connect a model provider

Every model-backed Colossus workflow resolves a logical role to a model profile, then
uses that model profile's provider connection. Choose the access method you already
have; you do not need to learn the complete configuration schema first.

## Choose your provider path

| Access method | Credential in Colossus | Provider kind | Best fit | Guide |
| --- | --- | --- | --- | --- |
| ChatGPT/Codex subscription | Codex-managed ChatGPT sign-in | `open_ai_codex` | Use an eligible subscription without an OpenAI API key | [Codex or ChatGPT subscription](codex-chatgpt.md) |
| OpenAI public API | Environment-backed OpenAI API key | `open_ai_responses` | Call the public Responses API with separate API billing | [OpenAI API](openai-api.md) |
| OpenRouter | Environment-backed OpenRouter API key | `open_ai_compatible` | Route an exact OpenRouter model through Chat Completions | [OpenRouter](openrouter.md) |
| Local server | None, or a credential injected by an embedding host | `open_ai_compatible` | Use a loopback server that implements the required OpenAI-compatible contracts | [Local models](local-models.md) |
| Another compatible endpoint | Environment- or host-backed credential | `open_ai_compatible` | Connect a gateway or hosted Chat Completions service | [Other OpenAI-compatible endpoints](openai-compatible.md) |

A ChatGPT subscription and OpenAI API access are separate products with separate
authentication and billing. If you have a ChatGPT/Codex plan, start with the
subscription guide. If you have an API key from the OpenAI platform, use the public API
guide.

## The shared connection model

Each guide configures the same four pieces:

1. A **provider profile** selects the transport and a late-bound credential reference.
2. A **model profile** names the exact model, limits, and supported capabilities.
3. The **primary role** selects that model profile for ordinary runs.
4. Under isolation, a **sandbox destination** grants only the endpoint's exact network
   origin. Acknowledged full access instead supplies ambient HTTP(S) authority, and an
   origin entry does not narrow it.

The API prefix, such as `/v1`, belongs in `baseUrl`; the sandbox grant contains only the
scheme, host, and effective port. Secret values never belong in YAML.

The task guides show copy/paste setup and diagnostics. Exact field semantics remain in
[Providers and models](../../reference/configuration/providers-models.md), and routing,
deployment, and policy guidance remains in
[Providers and routing](../../admin/providers-routing.md).
