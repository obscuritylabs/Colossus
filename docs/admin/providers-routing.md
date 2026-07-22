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
