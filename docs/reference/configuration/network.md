---
title: Network trust configuration
description: Configure additional TLS certificate authorities and understand which Colossus clients inherit them.
audience: operator
type: reference
---

# Network trust configuration

`network` adds certificate authorities to Colossus-owned outbound TLS clients. It does
not authorize a destination, supply a credential, or disable hostname verification.

Keep these controls separate:

| Control | Question it answers | Configuration |
| --- | --- | --- |
| TLS trust | Which certificate authorities may authenticate the server? | `network.caBundlePath` or an adapter-specific CA field |
| Destination authorization | May this effect contact the origin? | [`sandbox.networkDestinations`](sandbox.md#network-destinations) |
| Authentication | How does Colossus authenticate to the service? | Provider, MCP, integration, audit, policy, or storage credential fields |
| Local API identity | Is this the enrolled Colossus daemon? | Separately provisioned certificate pin; unaffected by this bundle |

An endpoint normally needs both TLS trust and destination authorization. Under
acknowledged full access, the runtime supplies ambient HTTP(S) destination authority;
HTTPS trust and endpoint-specific credential validation remain separate. Ambient
authority also accepts canonical non-loopback plaintext HTTP, where a CA bundle has no
effect and there is no TLS confidentiality or server authentication. Adding a CA bundle
alone never permits network access under an isolating boundary.

Security-channel adapters can impose stricter transport rules regardless of ambient
authority. Remote OPA remains HTTPS with pinned CA trust and mTLS identity, and WORM
audit export remains HTTPS-only, create-only, and hash-bound.

## Choose a starting point

| Scenario | `caBundlePath` | Result |
| --- | --- | --- |
| Every endpoint uses a public CA | Omit `network` or set the field to `null` | Use built-in public roots only |
| One or more endpoints use an enterprise/private CA | Set one PEM bundle path | Add those roots to built-in public roots for shared clients |
| Remote OPA needs pinned trust | Use `policy.ca_pem_path`, or the shared bundle when it contains only the intended OPA trust roots | OPA uses an exclusive pinned trust policy |
| PostgreSQL has its own exclusive CA | Use `storage.postgres.tls.kind: custom_ca` | PostgreSQL ignores the shared runtime bundle |
| A sandboxed or stdio MCP child needs a private CA | Configure that program's own trust store | Child processes do not inherit the in-process bundle |

Use no additional bundle unless an endpoint actually requires one. Every added root
expands the set of certificates that Colossus-owned clients can trust.

## Field reference

### `network.caBundlePath`

The only `network` field is an optional path to a PEM certificate bundle:

```yaml
network:
  caBundlePath: .colossus/certs/company-ca-bundle.pem
```

| Property | Rule |
| --- | --- |
| Value | Workspace-relative path, absolute path, or `null` |
| Format | One or more PEM-encoded certificates intended as trust anchors |
| Maximum file size | 4 MiB |
| Maximum certificates | 256 |
| Load time | Once, during runtime startup |
| Public roots | Retained and augmented for ordinary shared clients |

Relative paths resolve from the canonical selected workspace. Keeping a reviewed bundle
under `.colossus/certs/` makes the configuration portable with the workspace while the
development sandbox continues to protect `.colossus` from shell access.

Use `null` or omit the entire block to rely only on built-in roots:

```yaml
network:
  caBundlePath: null
```

The bundle should contain certificate blocks only. Never include private keys, client
identities, bearer credentials, or endpoint URLs. Colossus does not read ambient
`SSL_CERT_FILE`, proxy, or similar environment settings as a substitute for this field.

## Startup validation

Colossus opens, bounds, parses, and validates the complete bundle before constructing
the runtime. Startup fails closed when the file is:

- Missing or unreadable.
- Empty or contains no PEM certificates.
- Malformed or contains certificate encodings the TLS stores cannot accept.
- Larger than 4 MiB.
- Larger than 256 certificates.

The configured path must be nonempty. The source path and PEM bytes are not included in
normal diagnostics. Safe native interfaces may expose only certificate count and
SHA-256 fingerprints.

The file is not watched. Replacing its contents does not change an already running
runtime; restart Colossus or Managed Local to load the new trust anchors.

## Clients that use the shared bundle

For ordinary Colossus-owned clients, the configured certificates augment the built-in
public roots:

| Client family | Examples |
| --- | --- |
| Model providers | Codex subscription, OpenAI Responses, and OpenAI-compatible provider profiles |
| Search | SearXNG and SerpAPI profiles |
| Integrations | Native and imported HTTP integrations |
| Brokered HTTP | `network.http` and WORM audit export |
| Remote MCP | Streamable HTTP requests and OAuth metadata/token calls |
| Agent Plugins | Exact-origin OCI registry, token-service, and permitted blob-redirect requests, each with its own optional CA root |
| Semantic memory | Chroma and OpenAI-compatible embedding endpoints |
| PostgreSQL | `webpki_roots` TLS policy |
| OPA | Shared pinned trust when remote OPA omits `ca_pem_path` |

Destination matching, DNS pinning, redirect rejection, bounded bodies, timeouts, permit
checks, quarantine, and audit remain active. Trusting a CA does not weaken those
controls.

## Adapter-specific precedence

Some adapters deliberately use a stricter trust policy instead of augmenting public
roots.

### Remote OPA

Remote OPA requires pinned CA trust and an mTLS identity:

- When `policy.ca_pem_path` is set, that adapter-specific bundle is exclusive. Public
  roots and `network.caBundlePath` are not used for OPA.
- When `policy.ca_pem_path` is omitted, a nonempty `network.caBundlePath` supplies the
  exclusive pinned roots for remote OPA.
- The client identity remains in `policy.identity_pem_path`; a CA bundle never supplies
  the client certificate or private key.

See [Policy and audit configuration](policy-audit.md) for the complete OPA shape.

### PostgreSQL

PostgreSQL trust follows `storage.postgres.tls.kind`:

| TLS kind | Shared bundle behavior |
| --- | --- |
| `webpki_roots` | Built-in WebPKI roots plus `network.caBundlePath` |
| `custom_ca` | Adapter-specific CA only; shared and public roots are excluded |
| `disabled` | No TLS; accepted only for loopback or Unix-socket targets |

See [Storage configuration](storage.md) for the PostgreSQL fields.

### Local Colossus public API

External application clients verify the local or fleet Colossus endpoint using its
separately enrolled certificate fingerprint. `network.caBundlePath` cannot replace or
widen that identity pin.

## What does not inherit the bundle

Programs started inside a process sandbox own their TLS implementation and trust store.
The shared bundle is not injected into:

- Native, Windows Job Object, or OCI workload processes.
- Stdio MCP server processes.
- Arbitrary subprocess environment variables or certificate files.

If one of those programs needs enterprise TLS, build the trust anchor into its controlled
runtime or pass a separately authorized certificate path using that program's documented
configuration. Granting the endpoint origin still remains necessary for networked
sandbox execution.

Remote Streamable HTTP MCP is different: Colossus owns that HTTP client, so it does use
the shared bundle.

## Practical examples

The following examples include exact destination grants for an isolating execution
boundary. Acknowledged full access needs no duplicate grants, and retaining these lists
does not constrain ambient HTTP(S) authority.

### Private model provider

The provider URL includes its API prefix. The sandbox receives only the canonical
origin, and the shared network block supplies the private CA:

```yaml
network:
  caBundlePath: .colossus/certs/company-ca-bundle.pem
providers:
  profiles:
    internal-models:
      kind: open_ai_compatible
      baseUrl: https://models.internal.example/v1
      credentialReference: env:INTERNAL_MODEL_TOKEN
sandbox:
  networkDestinations:
    - https://models.internal.example
```

The provider credential remains independent of certificate trust and destination
authorization.

### Public and private providers together

Additional roots do not remove public roots for ordinary provider clients:

```yaml
network:
  caBundlePath: /etc/colossus/company-ca-bundle.pem
providers:
  profiles:
    public-provider:
      kind: open_ai_responses
      baseUrl: https://api.openai.com/v1
      credentialReference: env:PUBLIC_PROVIDER_TOKEN
    internal-provider:
      kind: open_ai_compatible
      baseUrl: https://models.internal.example/v1
      credentialReference: env:INTERNAL_PROVIDER_TOKEN
sandbox:
  networkDestinations:
    - https://api.openai.com
    - https://models.internal.example
```

The public endpoint continues to validate against built-in roots; the internal endpoint
may validate against the added enterprise roots.

### Remote MCP behind an enterprise CA

```yaml
network:
  caBundlePath: .colossus/certs/company-ca-bundle.pem
mcp:
  oauthCredentialStore: auto
  servers:
    splunk:
      transport: streamable_http
      url: https://splunk.internal.example/services/mcp
      credentialHeaders:
        Authorization:
          scheme: Bearer
          reference: env:SPLUNK_MCP_TOKEN
      allowStateless: true
      allowedTools: [splunk_run_search]
sandbox:
  environment:
    - SPLUNK_MCP_TOKEN
  networkDestinations:
    - https://splunk.internal.example
```

The remote MCP adapter uses the shared CA bundle. A stdio MCP child would not.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| The endpoint is still denied under isolation | Add its exact origin to `sandbox.networkDestinations`; CA trust is not authorization |
| Certificate validation still fails | Confirm the bundle contains the issuing trust anchor and restart the runtime |
| A path works from one workspace only | Relative paths resolve from the selected workspace; use the intended workspace or an absolute managed path |
| Remote OPA fails after adding a general enterprise bundle | OPA treats shared roots as an exclusive pin; use the intended OPA roots or set `policy.ca_pem_path` |
| PostgreSQL ignores the shared roots | `custom_ca` is exclusive; choose `webpki_roots` to augment public roots with the shared bundle |
| A sandbox command still rejects enterprise TLS | Child processes own their TLS stacks and do not inherit this field |
| A changed bundle has no effect | Restart the runtime; the bundle is loaded once at startup |

## Validate the result

First confirm the parsed path without resolving credentials:

```bash
colossus --config .colossus/config.yaml config show
```

Then construct the runtime, which reads and validates the bundle:

```bash
colossus --config .colossus/config.yaml config effective
```

Finally, use the doctor command for the client you are configuring:

```bash
colossus --config .colossus/config.yaml provider doctor PROVIDER_PROFILE
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml state doctor
```

Run only the relevant doctor commands. A valid bundle cannot make an unapproved origin,
invalid credential, incorrect model ID, unhealthy OPA service, or unavailable database
succeed; those are separate checks.

Return to the [configuration overview](../configuration.md).
