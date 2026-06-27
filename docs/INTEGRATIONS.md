# Integrations

Integrations let Colossus connect to external systems without exposing raw credentials
to the model. Connected tools are normal `ToolSpec`s: they pass through policy,
approval, audit, HTTP config, and output limits.

## Design Rules

- Integrations are hidden until explicitly configured.
- Connection records store credential refs, never raw secrets.
- Model-visible tool schemas contain operation arguments only.
- Tool handlers resolve credentials locally and inject auth headers in adapters.
- Network-capable integration tools require approval unless the selected approval mode
  auto-approves them.
- Audit records include connector names, tool names, credential refs, scopes, and
  argument keys, not secret values.

## Credential Refs

The first credential broker supports environment refs:

```text
env:VARIABLE_NAME
```

Example:

```bash
export GITHUB_TOKEN=...
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
```

If an integration requires auth and no ref is supplied, Colossus stores a pending-auth
connection. Reconnect with a credential ref when the secret is available.

## GitHub

GitHub is the first native connector. It exposes read-oriented tools for repositories,
issues, pull requests, checks, and releases.

```bash
export GITHUB_TOKEN=...
uv run colossus integrations connect github --credential-ref env:GITHUB_TOKEN
uv run colossus tools list
```

Current tools:

- `github.repos`
- `github.issues`
- `github.pull_requests`
- `github.checks`
- `github.releases`

Use a least-privilege token. For read-only coding workflows, prefer fine-grained
permissions that only cover the repositories and metadata you need.

Disconnect:

```bash
uv run colossus integrations disconnect github
```

## OpenAPI Imports

JSON OpenAPI documents can be imported into the brokered runtime:

```bash
export DEMO_API_TOKEN=...
uv run colossus integrations import-openapi demo ./openapi.json \
  --base-url https://api.example.test \
  --credential-ref env:DEMO_API_TOKEN \
  --auth-type bearer
```

Generated tool names use:

```text
openapi.NAME.OPERATION
```

Path and query parameters become tool arguments. Request bodies are passed through a
`body` object. Auth never becomes a model-visible argument.

Supported auth labels:

- `none`
- `api-key`
- `bearer`
- `oauth2-authorization-code`
- `service-account`

The local v1 broker resolves environment refs only. OS keychain or encrypted local store
support should be added behind the same credential broker port.

## MCP Position

MCP remains the preferred live-tool protocol for external data and tool servers, but it
should stay configured, allowlisted, approval-gated, and audited. Colossus should not
expose arbitrary MCP calls by default.

Today, MCP research and model-callable extension points are configured through the
existing MCP gateway settings. Future MCP integration work should reuse the integration
registry and credential broker rather than passing secrets in model-visible arguments.

## AI Proxy Position

The integration broker should come before an AI proxy. A future proxy can handle model
provider routing, provider API-key brokerage, usage tracking, rate limits, and team
deployment. App credentials should remain separate from model-provider credentials.
