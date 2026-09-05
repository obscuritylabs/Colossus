---
title: Plugins and workflow configuration
description: Workspace narrowing, trust, OCI registries, plugin MCP overlays, and workflow roots.
audience: operator
type: reference
---

# Plugins and workflow configuration

```yaml
plugins:
  enabled: true
  include: []
  exclude: []
  trustProfiles:
    default:
      mode: required
      publicKeys: []
      identities: []
      trustRootPath: null
  registries:
    production:
      origin: https://registry.example.com
      auth:
        kind: bearer
        credentialReference: env:REGISTRY_TOKEN
      trustProfile: default
      tokenOrigins:
        - https://auth.example.com
      blobRedirectOrigins: []
      caBundlePath: null
      tokenCaBundlePaths: {}
      blobRedirectCaBundlePaths: {}
      allowNonPublic: false
  mcpServers:
    example-plugin/server:
      enabled: true
      allowedTools: [lookup]
      environment: {}
      credentialHeaders: {}
      oauth: null
      researchTools: []
      allowStateless: false
      timeoutMs: null
      maxOutputBytes: null

workflows:
  repository: .colossus/workflows
  user: workflows
```

| Field | Meaning |
| --- | --- |
| `plugins.enabled` | Disable all plugin exposure for this workspace when false |
| `plugins.include` | Optional exact allowlist applied to the globally active set |
| `plugins.exclude` | Exact denylist applied after include |
| `plugins.trustProfiles` | Reusable `required`, `optional`, or `disabled` Sigstore policy |
| `plugins.registries` | Exact-origin OCI Distribution profiles |
| `plugins.mcpServers` | Explicit workspace enablement and authority overlay keyed by `PLUGIN/SERVER` |

The owner-scoped plugin store is always `$COLOSSUS_HOME/plugins`; it is not configurable by
a workspace. Trust roots, CA bundles, Docker config files, and Docker helper executables
must use absolute paths. Registry credentials are references, never literal values.

Every enabled plugin MCP overlay requires an exact tool allowlist. Credential environment
and header overlays use references and cannot replace `PLUGIN_ROOT` or `PLUGIN_DATA`.
Portable `mcp.json` values remain package data and cannot expand workspace authority.

Workflow paths remain workspace-relative configuration. Workflows are not packaged or
activated as plugins.

See [Agent Plugins](../../extend/plugins.md), [Agent Plugin formats](../extension-formats.md),
and [Workflow schema](../workflow-schema.md).
