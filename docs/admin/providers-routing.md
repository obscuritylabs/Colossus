---
title: Providers and routing
description: Connect model profiles and assign them to Colossus runtime roles.
audience: operator
type: how-to
---

# Providers and routing

## Goal

Connect a provider endpoint, define its model limits and capabilities, and keep role
routing explicit while credentials remain late-bound.

Users choosing an access method can start with the focused
[Connect a model provider](../use/providers/index.md) guides. This page owns the
operator workflow for routing, deployment policy, and diagnostics.

## Prerequisites

- A valid configuration.
- An endpoint and model identifier.
- The credential environment variable, when the endpoint requires one.
- For a Codex subscription, the official Codex CLI and a completed
  `colossus codex login` flow instead of an API key.
- An exact canonical origin grant in `sandbox.networkDestinations`.

## Steps

1. Add a named provider connection and a separate model profile. For an
   OpenAI-compatible endpoint:

    ```yaml
    providers:
      profiles:
        primary-provider:
          kind: open_ai_compatible
          baseUrl: https://models.example.com/v1
          credentialReference: env:COLOSSUS_MODEL_TOKEN
          timeoutMs: 120000
    models:
      profiles:
        primary-model:
          providerProfile: primary-provider
          model: example-model
          contextWindowTokens: 131072
          maxOutputTokens: 16384
          capabilities:
            toolCalls: true
            streaming: true
      roles:
        primary: primary-model
    sandbox:
      networkDestinations:
        - https://models.example.com
    ```

    Use `open_ai_responses` for a Responses-compatible endpoint. Use `echo` for a
    credential-free, network-free smoke route.

    To consume a Codex plan through a ChatGPT sign-in, use the fixed subscription
    adapter instead:

    ```yaml
    providers:
      profiles:
        codex-provider:
          kind: open_ai_codex
          credentialReference: codex:default
          timeoutMs: 120000
    sandbox:
      networkDestinations:
        - https://chatgpt.com
        - https://auth.openai.com
    ```

    Run `colossus codex login` before provider diagnostics. Do not configure a
    `baseUrl`: Colossus fixes the service origin so subscription credentials cannot be
    sent to a different host. The auth origin is separately required for token refresh.

    When the endpoint uses a private CA, configure the shared PEM bundle once:

    ```yaml
    network:
      caBundlePath: .colossus/certs/company-ca-bundle.pem
    ```

    The roots are added to the public trust roots used by every Colossus-owned
    outbound client. Relative paths resolve from the selected workspace.

    `providers.profiles.NAME.timeoutMs` is the transport ceiling for that connection's
    catalog and generation requests. With the built-in policy it remains effective even
    when `sandbox.timeoutMs` is lower; the sandbox timeout continues to bound ordinary
    sandboxed effects. An external OPA decision may impose a stricter timeout obligation.

2. Export the named API credential in the process environment, or run
   `colossus codex login` for a subscription profile. The YAML stores only a reference.

3. Review the configured profiles and role resolution:

    ```bash
    colossus --config .colossus/config.yaml provider profiles
    colossus --config .colossus/config.yaml models profiles
    colossus --config .colossus/config.yaml models routes
    colossus --config .colossus/config.yaml models route primary
    ```

4. Diagnose the selected profile:

    ```bash
    colossus --config .colossus/config.yaml provider doctor primary-provider
    colossus --config .colossus/config.yaml provider models primary-provider
    colossus --config .colossus/config.yaml models doctor primary-model
    ```

`provider doctor` checks the connection and catalog boundary. `models doctor` makes one
bounded generation probe using the selected model profile and its configured limits and
capabilities. Tool-enabled profiles carry a representative tool schema so a public catalog
endpoint cannot mask a generation or tool-schema incompatibility. Text-only profiles omit
tools. Probe response content is not printed. This separation distinguishes connection
failures from an invalid model ID, generation response contract, or capability mismatch.

Both network adapters require each canonical function-tool schema to declare
`type: object` at its root before transport. They clone the schema and omit root-level
`oneOf`, `anyOf`, `allOf`, `enum`, and `const` keywords from the provider-facing copy to
satisfy OpenAI function-tool request rules. Responses requests set `strict` to `false`;
Chat Completions requests omit `strict` and additionally remove `maxLength` annotations
recursively because grammar-compiling compatible servers can reject otherwise valid
large string bounds before generation. Colossus retains the canonical schema and
enforces every original bound and cross-field rule before a tool can execute; this
projection changes provider guidance, not runtime authority or validation.

Colossus also projects canonical dotted tool names to portable provider function names:
for example, `filesystem.write` is sent as `filesystem_write`. Continuation history uses
the same alias, and a returned alias is restored to `filesystem.write` before policy,
audit, or dispatch sees it. Configure access and policy with canonical dotted names.
Unrepresentable names and alias collisions fail locally before a request is sent.

For `risk_evaluator`, Colossus expects the same strict three-field JSON assessment from
every provider. Local compatible models that wrap that single object in one whole-output
`json` code fence are accepted as a narrow transport compatibility case. Surrounding
prose, multiple fences, unknown fields, malformed JSON, and unsupported values still fail
closed and fall back to the configured approval behavior.

Some local servers return HTTP 503 while a model is loading. Colossus reports that status
as `provider.temporarily_unavailable` with `Recoverable: yes` and does not retry the turn
implicitly. Wait until the endpoint reports ready, run `models doctor` again, and then
resubmit the turn. `/events verbose` includes the structured numeric `HTTP status` in the
run-error card while keeping provider response headers and bodies private. Other client
errors, including HTTP 400 schema rejection, remain terminal so configuration and
compatibility failures are not mislabeled as startup delay.

Specialized roles include `risk_evaluator`, `context_summarizer`,
`subagent_default`, `research_planner`, `research_worker`, and
`research_synthesizer`. An unmapped specialized role resolves through `primary`.

## Configure search routing

Model roles and search roles are independent. Configure the provider-neutral `agent`
and `research` search roles explicitly; neither falls back to the other:

```yaml
search:
  profiles:
    local-search:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      credentialReference: null
      authHeader: X-Searxng-Key
      userAgent: colossus/0.10
      timeoutMs: 30000
  roles:
    agent: local-search
    research: local-search
sandbox:
  networkDestinations:
    - http://127.0.0.1:8888
```

The endpoint includes the search path; the sandbox entry is only its canonical origin.
Use separate named profiles when agent and research traffic need different backends,
credentials, or limits. Diagnose without resolving credentials, then make one
explicitly approved query:

```bash
colossus --config .colossus/config.yaml search profiles
colossus --config .colossus/config.yaml --approval-mode ask \
  search query "routing smoke" --role research --limit 3
```

## Expected result

The route command identifies both `primary-model` and `primary-provider`; the two doctor
commands confirm their respective boundaries, search roles resolve to their selected
profiles, and no diagnostic output includes a credential value.

## Verification

```bash
colossus --config .colossus/config.yaml run \
  "Reply with exactly: connected"
```

## Failure path

For models, check in order: role mapping, credential variable presence, access decision,
exact origin grant, TLS trust, model identifier, and provider response shape. For
search, also confirm that the logical `agent` or `research` role names an existing
profile. Origins match by scheme, host, and effective port; URL paths belong in
`baseUrl` or a search `endpoint`, not the sandbox origin. Loopback HTTP is allowed for a
deliberately local endpoint. Remote endpoints require HTTPS.

## Next step

Review the provider action and tool visibility in
[Access and approvals](access-and-approvals.md). Exact profile fields are in
[Provider and model configuration](../reference/configuration/providers-models.md).
