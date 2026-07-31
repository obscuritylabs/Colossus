---
title: MCP
description: Configure stdio or stateful Streamable HTTP MCP servers through Colossus's policy boundary.
audience: operator
type: how-to
---

# MCP

## Goal

Configure a local stdio or remote Streamable HTTP MCP server, select explicit tools or
opt into dynamic wildcard discovery, and invoke tools without bypassing Colossus policy.
The native remote transport targets MCP `2025-11-25`; legacy HTTP+SSE and the stateless
`2026-07-28` release-candidate behavior are not enabled.

## Prerequisites

- For stdio, an MCP server executable at a canonical absolute path.
- For Streamable HTTP, an exact credential-free endpoint using HTTPS, except for exact
  loopback development endpoints.
- Any secret behind an `env:VARIABLE` reference.
- Matching process or network, filesystem, and environment grants.

## Steps

### 1. Add an exact server declaration

Merge this fragment into `.colossus/config.yaml`:

```yaml
mcp:
  oauthCredentialStore: auto
  servers:
    local-docs:
      command: /absolute/path/to/mcp-server
      args: [--stdio]
      workingDirectory: /absolute/path/to/repository
      environment:
        API_TOKEN: env:MCP_API_TOKEN
      allowedTools: [search_docs]
      researchTools:
        - tool: search_docs
          title: Internal documentation
          arguments:
            query: "{query}"
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

Add the command to `sandbox.executables`, its working directory to the appropriate
filesystem roots, and `API_TOKEN` to the allowed environment names. Configuration stores
the reference, not the secret value.

For a remote Splunk endpoint with a static bearer token:

```yaml
mcp:
  oauthCredentialStore: auto
  servers:
    splunk:
      transport: streamable_http
      url: https://splunk.example.com/services/mcp
      credentialHeaders:
        Authorization:
          scheme: Bearer
          reference: env:SPLUNK_MCP_TOKEN
      allowedTools: ["*"]
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

Add the exact endpoint origin to `sandbox.networkDestinations` and
`SPLUNK_MCP_TOKEN` to `sandbox.environment`. `allowedTools: ["*"]` is deliberately
broad: every currently or subsequently published valid tool becomes eligible for normal
schema validation, policy, approval, quarantine, and audit. An empty list, duplicate
names, or a wildcard mixed with explicit names is rejected. Signed-pack MCP declarations
remain explicit-only.

OAuth is an alternative to `credentialHeaders`:

```yaml
      oauth:
        clientId: colossus
        clientSecretReference: env:SPLUNK_MCP_CLIENT_SECRET
        callbackPort: 8787
        scopes: [openid, offline_access]
```

Use `colossus mcp auth login splunk`; add `--manual` to paste the final redirect URL in a
headless environment. `status` inspects local token presence and `logout` removes local
tokens without remote revocation. Agents never initiate browser login.

### 2. Inspect configuration without launching

```bash
colossus --config .colossus/config.yaml mcp servers
```

This lists configured names and exact allowlists.

### 3. Discover live allowed schemas

```bash
colossus --config .colossus/config.yaml mcp tools \
  --server local-docs
```

Discovery launches the exact executable or creates a fresh stateful HTTP session and
returns only selected, validated tool schemas.

### 4. Invoke the exact tool

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  mcp call \
  local-docs search_docs '{"query":"authorization"}'
```

Use `@path` instead of inline JSON when arguments come from a policy-readable file. MCP
invocation is approval-required under the development access profile; the global
`--approval-mode ask` option lets the noninteractive CLI request that approval before it
launches the configured process.

## Expected result

Colossus starts only the configured process or contacts the permit-authorized exact HTTP
origin, validates the selected schema and input, bounds JSON and SSE output, removes hard
secrets, closes the session best-effort, and releases an audited result.

## Verification

Confirm that `tools list` contains only the intended connected MCP operations and that
recent audit evidence identifies the exact server and tool without credential values.
The maintainer-only live Splunk smoke-test command is documented in
[Source setup and test tiers](../develop/setup-testing.md).

## Failure path

- **Executable denied:** add the exact canonical executable and required working
  directory grants; no shell lookup is used.
- **Tool is absent:** include the exact discovered name in `allowedTools`.
- **Environment denied:** allow the variable name and keep its value behind an
  `env:VARIABLE` reference.
- **Authorization required:** run `colossus mcp auth login SERVER`; tool calls never
  trigger interactive login.
- **Network denied:** grant the exact canonical origin. Public `*` grants remain
  public-address-only and cannot authorize loopback or private destinations.
- **Output exceeds bounds or is malformed:** fix the server; Colossus does not release
  unbounded or invalid output.

## Next step

Package a distributable executable capability with [Packs](packs.md). Exact MCP
configuration fields live in [Configuration fields](../reference/configuration.md).
