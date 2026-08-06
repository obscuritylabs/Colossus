---
title: MCP server configuration
description: Configure local stdio and remote Streamable HTTP MCP servers, credentials, OAuth, tool selection, and research templates.
audience: operator
type: reference
---

# MCP server configuration

`mcp` connects Colossus to explicitly configured Model Context Protocol servers. A
server may be a local process using stdio or an exact remote endpoint using Streamable
HTTP. Remote servers are stateful by default, with an explicit stateless compatibility
opt-in.

Colossus exposes only the MCP tool surface:

| MCP operation | Colossus action | Behavior |
| --- | --- | --- |
| `tools/list` | `mcp.tools` | Discovers and validates selected tool schemas |
| `tools/call` | `mcp.call` | Rediscovers the schema, validates arguments, and invokes one selected tool |

Resources, prompts, sampling, and elicitation are not exposed. MCP configuration does
not bypass access selection, policy authorization, approval obligations, sandbox
grants, quarantine, redaction, or audit.

For a step-by-step connection workflow, see [MCP](../../extend/mcp.md).

## Choose a starting point

| Scenario | Transport and authentication | Tool selection |
| --- | --- | --- |
| Local executable or Python console script | `stdio` with environment references | Explicit names |
| Remote Splunk endpoint with an unattended token | `streamable_http` with `credentialHeaders` | Explicit names for production |
| Remote service requiring interactive authorization | `streamable_http` with OAuth | Explicit names for production |
| Trusted development server whose tools change frequently | Either transport | `allowedTools: ["*"]` after reviewing the broader trust |
| MCP server distributed inside a signed pack | Pack-provided stdio only | Explicit names only |

Prefer explicit tool names for stable deployments. Use the wildcard only when the
configured server itself is trusted to publish future callable tools.

## Supported MCP surface

The native remote transport targets MCP `2025-11-25` and requires stateful Streamable
HTTP by default. It supports bounded JSON and SSE responses, server session headers,
POST requests, stateful GET event streams, and best-effort DELETE session cleanup.
`allowStateless: true` permits one reviewed server to omit `Mcp-Session-Id`; it does not
enable a different protocol version or legacy transport. For one-way JSON-RPC frames,
the bounded HTTP adapter accepts an empty successful response as equivalent to `202
Accepted`; requests still require a valid bounded JSON or SSE response.

The following modes are not enabled:

- The legacy HTTP+SSE transport with separate message and event endpoints.
- Streamable HTTP semantics specific to the `2026-07-28` release candidate.
- Automatic request retry or transparent expired-session reinitialization.

Each discovery page and tool call receives a fresh initialized transport. Colossus
closes a negotiated session best-effort after the result. Explicitly stateless servers
receive no GET event stream or DELETE cleanup because they publish no session identity.
If a remote call may have reached the server but completion cannot be confirmed, the
effect becomes `OutcomeUnknown` and is not automatically retried.

## Top-level `mcp` fields

```yaml
mcp:
  oauthCredentialStore: auto
  servers: {}
```

| Field | Values | Default |
| --- | --- | --- |
| `oauthCredentialStore` | `auto`, `platform`, or `encrypted_state` | `auto` |
| `servers` | Map of stable server names to strict declarations; at most 64 | Empty map |

Server names use 1–128 ASCII letters, digits, dots, underscores, or hyphens. The OAuth
store setting has no effect when no server configures OAuth.

## Local stdio servers

Stdio is the default transport, so `transport: stdio` may be included for clarity or
omitted for compatibility with existing configuration:

```yaml
mcp:
  oauthCredentialStore: auto
  servers:
    local-docs:
      transport: stdio
      command: /absolute/path/to/venv/bin/docs-mcp-server
      args: [--stdio]
      workingDirectory: /absolute/path/to/repository
      environment:
        API_TOKEN: env:HOST_DOCS_API_TOKEN
      allowedTools: [search_docs, read_document]
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

The matching sandbox configuration must grant the executable, working directory, and
child environment name:

```yaml
sandbox:
  executables:
    - /absolute/path/to/venv/bin/docs-mcp-server
  filesystem:
    - root: /absolute/path/to/repository
      mode: read
  environment:
    - API_TOKEN
```

This sandbox block is a fragment to merge into a complete sandbox declaration. In the
environment mapping, `API_TOKEN` is the name visible inside the child and
`HOST_DOCS_API_TOKEN` is the host variable Colossus reads. The sandbox grant names the
child variable, not the host variable. Using the same name for both is also valid.

### Stdio fields

| Field | Rule |
| --- | --- |
| `transport` | Omitted or `stdio` |
| `command` | Exact absolute executable also present in `sandbox.executables` |
| `args` | Literal arguments passed without a shell; at most 256 |
| `workingDirectory` | Existing workspace-relative or absolute directory inside a sandbox read/write grant |
| `environment` | At most 128 child variable names mapped to `env:HOST_VARIABLE` references |
| HTTP and OAuth fields | `url`, `headers`, `credentialHeaders`, `allowStateless`, and `oauth` must be absent |

When `workingDirectory` is omitted, Colossus uses the selected workspace, which still
needs a containing filesystem grant. Arguments do not use shell expansion, executable
lookup, command substitution, or an ambient environment.

Stdio MCP is a process effect. The MCP child receives only permit-authorized process,
filesystem, environment, network, time, output, process-count, and memory obligations.
The server owns its own TLS implementation and does not inherit
`network.caBundlePath`.

## Remote Streamable HTTP servers

Remote MCP uses one exact endpoint and is a network effect:

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
      allowStateless: true
      allowedTools:
        - splunk_run_search
        - splunk_get_indexes
      timeoutMs: 30000
      maxOutputBytes: 1048576
sandbox:
  environment:
    - SPLUNK_MCP_TOKEN
  networkDestinations:
    - https://splunk.example.com
```

Merge the sandbox fields into the deployment's complete sandbox block. The credential
reference is stored in YAML; its value is resolved only inside the permit-bearing MCP
adapter. Unlike stdio child mappings, a remote credential header requires the referenced
host variable itself in `sandbox.environment`.

### Remote transport fields

| Field | Rule |
| --- | --- |
| `transport` | Must be `streamable_http` |
| `url` | Exact HTTP(S) endpoint with a host and path |
| `headers` | Optional non-secret literal headers; at most 64 |
| `credentialHeaders` | Optional secret headers backed by environment references; at most 16 |
| `allowStateless` | Optional boolean; default `false`; remote-only compatibility opt-in for servers that omit `Mcp-Session-Id` |
| `oauth` | Optional OAuth 2.1 authorization-code configuration |
| Stdio fields | `command`, `args`, `workingDirectory`, and `environment` must be absent |

The URL must not contain a username, password, query, or fragment. HTTPS is required
except for exact `localhost` or IP-loopback development endpoints. Every request remains
bound to that exact configured endpoint; the transport cannot redirect itself to a
different path or origin.

The endpoint origin must match `sandbox.networkDestinations`. Public `*` may match a
public HTTPS origin, but loopback, private, link-local, and metadata destinations require
an exact origin. Remote MCP uses Colossus's pinned-DNS, redirect-free, ambient-proxy-free
HTTP client and inherits additional roots from
[Network trust configuration](network.md).

## Static headers and credentials

Use `headers` only for non-secret routing metadata:

```yaml
headers:
  X-Tenant-ID: operations
```

Use `credentialHeaders` for authorization or API-key values:

```yaml
credentialHeaders:
  Authorization:
    scheme: Bearer
    reference: env:SPLUNK_MCP_TOKEN
  X-Api-Key:
    reference: env:SPLUNK_SECONDARY_KEY
```

`scheme` is optional. When present, Colossus sends `SCHEME secret`; when omitted, it
sends the secret value alone. A scheme contains only bounded ASCII letters, digits,
hyphens, or underscores.

Header names are case-insensitively unique. Colossus rejects transport-managed headers,
cookies, proxy credentials, `Mcp-*` names, and literal authorization-like headers.
Header values must be nonempty, bounded, and free of control characters. A resolved
credential that is empty or contains CR/LF is rejected.

Literal headers may accompany static credentials or OAuth. `credentialHeaders` and
`oauth` are mutually exclusive because both own the authentication identity.

Static credential headers are the simplest choice for unattended operation when the
server supports a long-lived or externally rotated token.

## OAuth 2.1 with PKCE

OAuth is an alternative to `credentialHeaders`:

```yaml
mcp:
  oauthCredentialStore: auto
  servers:
    splunk:
      transport: streamable_http
      url: https://splunk.example.com/services/mcp
      oauth:
        clientId: colossus
        clientSecretReference: env:SPLUNK_MCP_CLIENT_SECRET
        callbackPort: 8787
        scopes: [openid, offline_access]
      allowedTools: [splunk_run_search, splunk_get_indexes]
      timeoutMs: 30000
      maxOutputBytes: 1048576
sandbox:
  environment:
    - SPLUNK_MCP_CLIENT_SECRET
  networkDestinations:
    - https://splunk.example.com
    - https://identity.example.com
```

The identity origin is illustrative. Authorize the actual protected-resource,
authorization, and token origins discovered for the deployment. OAuth discovery and
token exchange use the same exact-origin/public-wildcard network policy, TLS roots,
DNS pinning, redirect rejection, timeout, and response bounds as other Colossus-owned
clients.

### OAuth fields

| Field | Rule |
| --- | --- |
| `clientId` | Required bounded, control-free registered client identifier |
| `clientSecretReference` | Optional `env:VARIABLE`; the variable needs a sandbox environment grant |
| `callbackPort` | Required nonzero loopback port registered for `http://127.0.0.1:PORT/callback` |
| `scopes` | At most 32 unique, nonempty, whitespace-free tokens; may be empty |

Colossus performs protected-resource and authorization-server discovery, PKCE-S256,
CSRF state validation, issuer validation, token exchange, expiry handling, and refresh
rotation. The configured client secret remains behind its environment reference and is
not persisted with OAuth credentials.

### Login, status, and logout

Normal interactive login binds the callback only on `127.0.0.1`, prints the
authorization URL, and waits up to five minutes for the bounded callback request:

```bash
colossus --config .colossus/config.yaml mcp auth login splunk
```

For a headless host or container, copy the authorization URL to a browser, complete the
flow, and paste the final redirected URL into stdin:

```bash
colossus --config .colossus/config.yaml mcp auth login splunk --manual
```

Inspect local token presence or remove the local record:

```bash
colossus --config .colossus/config.yaml mcp auth status splunk
colossus --config .colossus/config.yaml mcp auth logout splunk
```

`status` reports whether a local token record exists; it does not contact the remote
server to prove the token is still accepted. `logout` does not revoke the remote grant.
Agents and tool calls never start browser login. Missing credentials return an explicit
authorization-required error for an operator to resolve.

Before an MCP operation, Colossus obtains or refreshes the access token. It does not
refresh and retry a tool call whose outcome may already be unknown.

### OAuth credential storage

| `oauthCredentialStore` | Behavior |
| --- | --- |
| `auto` | Uses platform storage with `storage.keys.kind: platform`; otherwise uses encrypted state |
| `platform` | Stores the server-bound OAuth record in the operating-system credential store |
| `encrypted_state` | Stores a domain-separated XChaCha20-Poly1305 record in a dedicated redb sidecar derived from `storage.path` |

Encrypted records are bound to repository, configured server name, endpoint, and active
storage key ID. They are re-encrypted after storage-key rotation when the historical key
remains available. Changing the workspace identity, server name, or endpoint therefore
requires a separate login instead of silently reusing another server's credentials.

Platform-store failure does not fall back to disk. Encrypted-state failure does not fall
back to plaintext or a platform entry.

## Selection and bounds

`allowedTools` is required for every server and accepts exactly one of these forms.

### Explicit tool names

```yaml
allowedTools:
  - splunk_run_search
  - splunk_get_indexes
```

Names must be unique and use 1–128 ASCII letters, digits, dots, underscores, or hyphens.
Tools published by the server but absent from this list are filtered out.

Explicit selection is recommended for production because a server update cannot make a
new tool callable until an operator reviews and adds its exact name.

### All discovered tools

```yaml
allowedTools: ["*"]
```

The wildcard must be the only entry. It cannot be mixed with names, and an empty list or
duplicate explicit names is rejected.

Wildcard mode dynamically trusts every current and future valid tool published by that
configured server. Colossus still validates tool-name uniqueness, schema validity,
description and annotation bounds, pages, argument objects, policy decisions,
approvals, results, and audit records. If any wildcard-discovered tool violates the
validation bounds, discovery fails closed.

This wildcard selects server-published tools only. It does not widen
`access.tools.include`, authorize the endpoint in `sandbox.networkDestinations`, grant a
credential, or approve `mcp.call`. Those remain separate trust boundaries; see
[Access configuration](access.md#wildcard-boundary).

Wildcard mode applies only to top-level server configuration. Signed-pack MCP
declarations remain stdio/process-only and require explicit tool names.

### Fresh schema binding

Every call performs live discovery again, finds the selected tool, validates its JSON
object schema, validates the supplied argument object, and binds the schema and its
SHA-256 hash into the effect request. A schema cached from an earlier `mcp tools` command
does not authorize a later call.

This protects both explicit and wildcard selection from silent schema drift. It also
means the MCP server must be available for discovery immediately before invocation.

### Risk-auto review

When policy requires approval, `--approval-mode risk-auto` may review a configured
top-level `mcp.call` made by a model or child agent outside workflow lineage. The
credential-free evaluator input contains the exact endpoint identity, transport,
configured server, tool, bounded description and annotations, fresh schema hash, and
validated arguments. Descriptions and annotations are server-provided advisory hints;
they are not authority or hard eligibility preconditions.

Both explicit tool selection and `allowedTools: ["*"]` use the same review rule because
the automatic proof binds one exact invocation. A change to the endpoint, server, tool,
schema hash, or arguments invalidates that authority. Stdio and Streamable HTTP calls
also share review eligibility, while retaining their separate process and network
obligations. Only a valid low-risk `allow` assessment avoids a dialog. Medium, high,
uncertain, unavailable, malformed, or unsupported reviews preserve explicit approval,
and an ineligible prompt explains why review was skipped.

### Server-specific resource limits

| Field | Rule | Inherited value when omitted |
| --- | --- | --- |
| `timeoutMs` | Positive and no greater than `sandbox.timeoutMs` | Sandbox timeout |
| `maxOutputBytes` | `1024..=sandbox.maxOutputBytes` | Sandbox output cap |

These values may narrow the deployment sandbox but cannot widen it. JSON bodies, SSE
streams, stdio output, parsed tool schemas, and released results remain bounded.

### Hard discovery bounds

| Bound | Maximum |
| --- | ---: |
| Configured servers | 64 |
| Discovered tools per server | 1,024 |
| Discovery pages per server | 32 |
| Research templates per server | 64 |
| One input schema | 256 KiB |
| Wildcard-discovered title | 8 KiB |
| Wildcard-discovered description | 32 KiB |

Duplicate tool names, invalid schemas, empty or cyclic cursors, and limit overruns fail
closed. `mcp servers` safely displays `allowed_tools: ["*"]` so operators can see that
dynamic trust is enabled.

## Research templates

`researchTools` explicitly maps selected MCP tools into the deep-research MCP lane:

```yaml
researchTools:
  - tool: splunk_run_search
    title: Splunk security events
    arguments:
      query: "{query}"
      earliest_time: "-24h"
```

| Field | Rule |
| --- | --- |
| `tool` | Exact explicitly allowed tool, or any valid tool under wildcard mode |
| `title` | Optional bounded source title |
| `arguments` | JSON object; `{query}` is replaced recursively in string values |

Research templates always remain explicit, even when `allowedTools: ["*"]`. Merely
allowing an MCP tool does not make it a research source. See
[Deep research](../../use/deep-research.md) for lane selection and evidence behavior.

## Configuration and invocation flow

Use the commands in this order:

1. Parse configuration without contacting the MCP server:

    ```bash
    colossus --config .colossus/config.yaml config show
    colossus --config .colossus/config.yaml mcp servers
    ```

2. For OAuth, bootstrap credentials as the operator:

    ```bash
    colossus --config .colossus/config.yaml mcp auth status splunk
    colossus --config .colossus/config.yaml mcp auth login splunk
    ```

3. Discover released live schemas:

    ```bash
    colossus --config .colossus/config.yaml mcp tools --server splunk
    ```

4. Invoke one exact tool with a JSON object:

    ```bash
    colossus --config .colossus/config.yaml --approval-mode ask \
      mcp call splunk splunk_get_indexes '{}'
    ```

Use `@path` in place of inline JSON to read arguments from a policy-readable file.
`mcp servers` is a safe configuration summary and does not launch or contact the
server. Discovery and invocation are separately authorized effects.

## Security behavior

| Boundary | Stdio | Streamable HTTP |
| --- | --- | --- |
| Effect type | Process | Network |
| Identity | Exact executable and working directory | Exact endpoint and authorized origin |
| Credentials | Cleared child environment populated from declared references | Permit-time static header or persisted OAuth token |
| TLS | Owned by the child process | Colossus-owned pinned client and shared CA bundle |
| Response | Bounded JSON-RPC on stdout | Bounded JSON or SSE |
| Session | Fresh process initialization | Fresh initialized transport; stateful by default, explicit stateless opt-in |

Configured credential values are hard-redacted from released schemas, results, errors,
policy input, diagnostics, and audit evidence. If discovery itself contains one of the
configured secret values, Colossus rejects the page rather than releasing it.

The remote client disables ambient proxies and automatic redirects, pins DNS resolution,
rejects an unexpected endpoint, and validates exact response media types. Every result
remains quarantined until post-effect policy permits release.

An MCP server is still an external authority. Tool descriptions, schemas, and results
are untrusted input. Explicit tool selection reduces the consequences of a compromised
or unexpectedly upgraded server; wildcard selection intentionally broadens that trust.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| A stdio command is rejected at startup | Use an absolute executable and add that exact path to `sandbox.executables` |
| The working directory is denied | Add a containing read/write filesystem grant; the selected workspace is used when the field is omitted |
| A stdio environment mapping is denied | Grant the child variable name and map it to a valid `env:HOST_VARIABLE` reference |
| A remote credential reference is denied | Grant the referenced host variable itself in `sandbox.environment` |
| A remote URL is rejected | Remove credentials, query, and fragment; use HTTPS outside exact loopback development |
| The endpoint is still denied | Add its exact origin to `sandbox.networkDestinations`; CA trust alone is not authorization |
| Enterprise TLS still fails | Configure `network.caBundlePath` and restart; stdio children need their own trust configuration |
| OAuth discovery is denied | Authorize every actual protected-resource, authorization, and token origin |
| `auth status` is true but calls return unauthorized | Status checks local token presence only; log in again or review remote revocation/scopes |
| OAuth login times out | Confirm the registered callback is the exact configured loopback URL, or use `--manual` |
| A tool is absent from discovery | Add the exact name, or deliberately select `allowedTools: ["*"]` |
| Wildcard configuration is rejected | `"*"` must be the sole entry and is not accepted in signed-pack declarations |
| A call fails argument validation | Rediscover the live schema and send a JSON object matching it |
| Discovery fails after a server update | Inspect invalid names, duplicate tools, schemas, descriptions, pagination, or limit overruns |
| A server-specific bound is rejected | It may only narrow the sandbox timeout/output cap; output must remain at least 1,024 bytes |
| A remote server omits `Mcp-Session-Id` | Review the server and set `allowStateless: true`; legacy HTTP+SSE still requires another adapter |
| A call reports `OutcomeUnknown` | Inspect the remote system before retrying; Colossus will not guess whether the tool executed |

Return to the [configuration overview](../configuration.md).
