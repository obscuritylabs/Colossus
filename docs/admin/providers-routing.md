---
title: Providers and routing
description: Connect model profiles and assign them to Colossus runtime roles.
audience: operator
type: how-to
---

# Providers and routing

## Goal

Connect one model endpoint while keeping credentials late-bound and routing explicit.

## Prerequisites

- A valid configuration.
- An endpoint and model identifier.
- The credential environment variable, when the endpoint requires one.
- An exact canonical origin grant in `sandbox.networkDestinations`.

## Steps

1. Add a named provider profile. For an OpenAI-compatible endpoint:

    ```yaml
    providers:
      profiles:
        primary-model:
          kind: open_ai_compatible
          model: example-model
          baseUrl: https://models.example.com/v1
          credentialReference: env:COLOSSUS_MODEL_TOKEN
          timeoutMs: 120000
      roles:
        primary: primary-model
    sandbox:
      networkDestinations:
        - https://models.example.com
    ```

    Use `open_ai_responses` for a Responses-compatible endpoint. Use `echo` for a
    credential-free, network-free smoke route.

    `providers.profiles.NAME.timeoutMs` is the transport ceiling for that profile's
    catalog and generation requests. With the built-in policy it remains effective even
    when `sandbox.timeoutMs` is lower; the sandbox timeout continues to bound ordinary
    sandboxed effects. An external OPA decision may impose a stricter timeout obligation.

2. Export the named credential in the process environment. The YAML stores the
   reference, not its value.

3. Review the configured profiles and role resolution:

    ```bash
    colossus --config .colossus/config.yaml provider profiles
    colossus --config .colossus/config.yaml models routes
    colossus --config .colossus/config.yaml models route primary
    ```

4. Diagnose the selected profile:

    ```bash
    colossus --config .colossus/config.yaml provider doctor primary-model
    colossus --config .colossus/config.yaml provider models primary-model
    ```

For network providers, `provider doctor` checks both the model catalog and one bounded
generation probe carrying a representative tool schema. This verifies that a public
catalog endpoint has not masked an invalid credential, model identifier, generation
response contract, or tool-schema incompatibility. The probe response is not printed.
`provider models` remains the catalog-only diagnostic.

The Chat Completions adapter omits `maxLength` annotations from the provider-facing tool
schema because grammar-compiling compatible servers can reject otherwise valid large
string bounds before generation. Colossus retains the canonical schema and enforces every
original bound before a tool can execute; this projection changes provider guidance, not
runtime authority or validation.

For `risk_evaluator`, Colossus expects the same strict three-field JSON assessment from
every provider. Local compatible models that wrap that single object in one whole-output
`json` code fence are accepted as a narrow transport compatibility case. Surrounding
prose, multiple fences, unknown fields, malformed JSON, and unsupported values still fail
closed and fall back to the configured approval behavior.

Some local servers return HTTP 503 while a model is loading. Colossus reports that status
as `provider.temporarily_unavailable` with `Recoverable: yes` and does not retry the turn
implicitly. Wait until the endpoint reports ready, run `provider doctor` again, and then
resubmit the turn. Other client errors, including HTTP 400 schema rejection, remain
terminal so configuration and compatibility failures are not mislabeled as startup delay.

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

The route command identifies `primary-model`, the doctor command confirms the endpoint
contract, search roles resolve to their selected profiles, and no diagnostic output
includes a credential value.

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
[Configuration fields](../reference/configuration.md).
