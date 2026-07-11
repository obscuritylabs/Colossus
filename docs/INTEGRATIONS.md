# Integrations

Integrations expose external operations as strict tools without exposing raw credentials
to the model. Connection lifecycle is canonical event-sourced state. Tools remain hidden
until connected, and every call crosses policy, exact-origin networking, quarantine,
hard-secret redaction, post-effect release, and audit.

## Credential References

Configuration and commands accept references such as `env:GITHUB_TOKEN`, never values.
The credential broker resolves a reference only after authorization and injects the
required header inside the adapter.

```bash
export GITHUB_TOKEN=...
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect github --credential-reference env:GITHUB_TOKEN
```

The provider or integration origin must also appear exactly in
`sandbox.networkDestinations`, and the policy must allow or require approval for the
corresponding operation. Pending-auth connections stay hidden from model tools.

## Lifecycle Commands

```bash
colossus --config .colossus/config.yaml integrations list
colossus --config .colossus/config.yaml integrations show github
colossus --config .colossus/config.yaml integrations call \
  github.repos '{"owner":"example"}'
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations disconnect github
```

`tools list` shows only operations currently exposed to the agent.

## GitHub

The native GitHub connector uses `https://api.github.com` and exposes bounded read
operations:

- `github.repos`
- `github.issues`
- `github.pull_requests`
- `github.checks`
- `github.releases`

Use a fine-grained least-privilege token limited to required repositories and metadata.
The connector never places the token in a tool schema, request content, transcript, or
audit payload.

## SearXNG

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect searxng --base-url http://127.0.0.1:8888 \
  --auth-type none
```

Operations are `searxng.search` and `searxng.health`. A protected endpoint can use an
API-key header:

```bash
export SEARXNG_API_KEY=...
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect searxng --base-url https://search.example.test \
  --auth-type api-key --credential-reference env:SEARXNG_API_KEY \
  --auth-header X-Searxng-Key
```

The native connector is model-callable. The separate `research.search` configuration
controls automatic web-source collection for research runs.

## OpenSearch

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect opensearch --base-url http://127.0.0.1:9200 \
  --auth-type none
```

Supported operations are:

- `opensearch.info`, `opensearch.health`, `opensearch.list_indices`
- `opensearch.get_mapping`, `opensearch.search`, `opensearch.get_document`
- `opensearch.index_document`, `opensearch.update_document`,
  `opensearch.delete_document`

Document writes are mutations and should require explicit approval. Colossus does not
replace OpenSearch index/role authorization; use least-privilege cluster credentials.

Bearer example:

```bash
export OPENSEARCH_TOKEN=...
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect opensearch --base-url https://search.example.test \
  --auth-type bearer --credential-reference env:OPENSEARCH_TOKEN
```

Basic auth uses `--username-reference` and `--password-reference`. Service-account and
API-key modes use reference-only credential fields as reported by `--help`.

## OpenAPI Imports

JSON OpenAPI 3 documents are read through the filesystem gateway and compiled into
strict operations:

```bash
export DEMO_API_TOKEN=...
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations import-openapi demo @openapi.json \
  --base-url https://api.example.test \
  --auth-type bearer --credential-reference env:DEMO_API_TOKEN
```

The `spec` argument accepts inline JSON or `@path`; file reads remain policy-bound.
Generated names use `openapi.NAME.OPERATION`. Path/query fields and the optional body are
validated against the compiled schema before any request. External `$ref`, embedded
origins, unsupported schemas, and unknown arguments fail closed.

## MCP

MCP is configured directly in YAML as exact stdio server identities and exact tool
allowlists:

```bash
colossus --config .colossus/config.yaml mcp servers
colossus --config .colossus/config.yaml mcp tools --server local-docs
colossus --config .colossus/config.yaml mcp call \
  local-docs search_docs '{"query":"policy"}'
```

Discovery and calls use official Rust SDK protocol models but still execute the server
through the sandbox helper. Pagination, schemas, output, environment, and credentials
are bounded and audited. Arbitrary unconfigured MCP processes or tools are never exposed.
