---
title: Search configuration
description: Exact SearXNG and SerpAPI profiles plus agent and research route fields.
audience: operator
type: reference
---

# Search configuration

`search` defines provider-neutral search connections and explicitly routes agent and
research traffic. See [Research overview](../../use/research-search.md) and
[Web search](../../use/web-search.md) for usage.

```yaml
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      credentialReference: null
      authHeader: X-Searxng-Key
      userAgent: colossus/0.10
      timeoutMs: 30000
  roles:
    agent: local
    research: local
```

## Fields

| Field | Values / constraint |
| --- | --- |
| `search.profiles.NAME.kind` | `searxng` or `serp_api` |
| `.endpoint` | Full provider endpoint including its path |
| `.credentialReference` | Required `env:VARIABLE` for SerpAPI; optional for SearXNG |
| `.authHeader` | SearXNG credential header; defaults to `X-Searxng-Key` |
| `.userAgent` | Defaults to `colossus/0.10` |
| `.timeoutMs` | Positive bounded duration; defaults to `30000` |
| `search.roles.agent` | Existing profile name for ordinary agent search |
| `search.roles.research` | Existing profile name for research search |

Routes never silently fall back. Every profile origin must be present in
`sandbox.networkDestinations`; the destination contains only the canonical origin,
while the search path remains in `endpoint`.

## Hosted model plus local search example

```yaml
providers:
  profiles:
    openrouter:
      kind: open_ai_compatible
      baseUrl: https://openrouter.ai/api/v1
      credentialReference: env:OPENROUTER_API_KEY
      timeoutMs: 120000
models:
  profiles:
    openrouter-primary:
      providerProfile: openrouter
      model: openrouter/free
      contextWindowTokens: 131072
      maxOutputTokens: 16384
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: openrouter-primary
    risk_evaluator: openrouter-primary
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      credentialReference: null
      timeoutMs: 30000
  roles:
    agent: local
    research: local
sandbox:
  networkDestinations:
    - "*"
    - http://127.0.0.1:8888
```

The wildcard covers the public OpenRouter HTTPS origin. The local SearXNG loopback
origin remains an exact entry. The provider credential stays outside YAML.

Return to the [configuration overview](../configuration.md).
