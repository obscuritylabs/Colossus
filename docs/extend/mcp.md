---
title: MCP
description: Configure and invoke one exact allowlisted MCP stdio server through Colossus's sandbox and policy boundary.
audience: operator
type: how-to
---

# MCP

## Goal

Configure one local MCP stdio server by exact executable identity, allowlist a tool, and
invoke it without exposing arbitrary discovered processes or operations.

## Prerequisites

- An MCP server executable already installed at a canonical absolute path.
- Its working directory and exact tool name.
- Any secret as an environment credential reference.
- Matching sandbox executable, filesystem, and environment grants.

## Steps

### 1. Add the exact server declaration

Merge this fragment into `.colossus/config.yaml`:

```yaml
mcp:
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

Discovery launches the exact executable through the sandbox and returns only allowed
tool schemas.

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

Colossus starts only the configured process, validates the allowlisted schema and input,
bounds output, removes hard secrets, and releases an audited result.

## Verification

Confirm that `tools list` contains only the intended connected MCP operations and that
recent audit evidence identifies the exact server and tool without credential values.

## Failure path

- **Executable denied:** add the exact canonical executable and required working
  directory grants; no shell lookup is used.
- **Tool is absent:** include the exact discovered name in `allowedTools`.
- **Environment denied:** allow the variable name and keep its value behind an
  `env:VARIABLE` reference.
- **Output exceeds bounds or is malformed:** fix the server; Colossus does not release
  unbounded or invalid output.
- **A network server is needed:** place network behavior behind the local exact process
  and grant only its declared destinations, or use an integration.

## Next step

Package a distributable executable capability with [Packs](packs.md). Exact MCP
configuration fields live in [Configuration fields](../reference/configuration.md).
