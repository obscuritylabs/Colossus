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

## SearXNG

SearXNG is the first local-first native connector. It exposes model-callable search
tools backed by a configured local or private SearXNG instance.

Start the bundled local development instance:

```bash
docker compose -f docker-compose.searxng.yml up -d
curl 'http://localhost:8888/search?q=colossus&format=json'
```

Connect it:

```bash
uv run colossus integrations connect searxng --base-url http://localhost:8888
uv run colossus tools list
```

Current tools:

- `searxng.search`
- `searxng.health`

`searxng.search` accepts `query` and optional `max_results`. The connector normalizes
SearXNG JSON results into title, URL, content, and metadata fields.

For a protected SearXNG instance, keep the key in the environment and pass only a
credential ref:

```bash
export SEARXNG_API_KEY=...
uv run colossus integrations connect searxng \
  --base-url https://search.example.test \
  --credential-ref env:SEARXNG_API_KEY \
  --auth-header X-Searxng-Key \
  --auth-scheme raw
```

`--auth-scheme raw` sends the secret value as the header value. The default
`--auth-scheme bearer` sends `Bearer VALUE`.

This integration is separate from the Deep Research `web.search` provider setting. Use
the integration when you want model-callable `searxng.*` tools; use the research config
when you want `/research` source collection to use SearXNG.

## OpenSearch

OpenSearch is a native document-focused connector for local, private, or proxied
OpenSearch-compatible clusters. It is hidden until connected and all tools remain
network approval-gated. Document writes are marked mutating and high risk.

Start the bundled local development cluster:

```bash
docker compose -f docker-compose.opensearch.yml up -d
curl 'http://localhost:9200/_cluster/health'
```

The compose file binds OpenSearch to `127.0.0.1`, disables the OpenSearch security
plugin, and is intended for local development and integration testing only. Override the
image tag or host ports with `OPENSEARCH_VERSION`, `OPENSEARCH_PORT`, and
`OPENSEARCH_PERF_PORT`.

Connect the local unauthenticated development cluster:

```bash
uv run colossus integrations connect opensearch \
  --base-url http://localhost:9200 \
  --auth-type none
```

Connect through a bearer-token or proxy-auth endpoint:

```bash
export OPENSEARCH_TOKEN=...
uv run colossus integrations connect opensearch \
  --base-url https://search.example.test \
  --auth-type bearer \
  --credential-ref env:OPENSEARCH_TOKEN
```

Connect with basic auth:

```bash
export OPENSEARCH_USER=...
export OPENSEARCH_PASSWORD=...
uv run colossus integrations connect opensearch \
  --base-url https://search.example.test \
  --auth-type basic \
  --username-ref env:OPENSEARCH_USER \
  --password-ref env:OPENSEARCH_PASSWORD
```

Current tools:

- `opensearch.info`
- `opensearch.health`
- `opensearch.list_indices`
- `opensearch.get_mapping`
- `opensearch.search`
- `opensearch.get_document`
- `opensearch.index_document`
- `opensearch.update_document`
- `opensearch.delete_document`

V1 is document-focused: no bulk API, index administration, security APIs, role APIs, or
scripted updates. Colossus does not enforce an index allowlist; use OpenSearch roles and
least-privilege credentials for cluster and index permissions.

Amazon OpenSearch Service can be used through a local or hosted proxy that performs
AWS SigV4 signing, then connect Colossus with `--auth-type bearer`, `basic`, or `none`
as appropriate for that proxy. Native SigV4 signing is intentionally deferred.

Run the opt-in live integration smoke test against the local cluster:

```bash
COLOSSUS_OPENSEARCH_LIVE=1 \
COLOSSUS_OPENSEARCH_URL=http://127.0.0.1:9200 \
uv run pytest tests/test_opensearch_live.py
```

Tear down local data when you are done:

```bash
docker compose -f docker-compose.opensearch.yml down -v
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
