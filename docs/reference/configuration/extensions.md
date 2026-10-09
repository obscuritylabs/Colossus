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
  workspaceDiscovery: true
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
      workspacePluginDigest: null
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
| `plugins.workspaceDiscovery` | Discover `.agents` plugins and accepted workspace directories; defaults to true, and never grants source acceptance |
| `plugins.include` | Optional exact allowlist applied to accepted local sources and the globally active set |
| `plugins.exclude` | Exact denylist applied after include |
| `plugins.trustProfiles` | Reusable `required`, `optional`, or `disabled` Sigstore policy |
| `plugins.registries` | Exact-origin OCI Distribution profiles |
| `plugins.mcpServers` | Explicit workspace enablement and authority overlay keyed by `PLUGIN/SERVER` |

The built-in `obscuritylabs` trust profile pins the `colossus-plugins` signing workflow.
Custom `trustProfiles` entries are added alongside it; the built-in signing identity cannot
be redefined. When `registries` is omitted, the built-in `obscuritylabs` GHCR registry is
available. An explicit `registries` map replaces that default, so `registries: {}` disables
OCI registry profiles for the workspace.

Global packages use `$COLOSSUS_HOME/plugins`. Accepted local sources, their immutable
snapshots, and writable data use a separate store under the home partition for the selected
workspace identity. These stores cannot be redirected by repository configuration.
Trust roots, CA bundles, Docker config files, and Docker helper executables
must use absolute paths. Registry credentials are references, never literal values.

Every enabled plugin MCP overlay requires an exact tool allowlist. Credential environment
and header overlays use references and cannot replace `PLUGIN_ROOT` or `PLUGIN_DATA`.
Portable `mcp.json` values remain package data and cannot expand workspace authority.

For a workspace source, set `workspacePluginDigest` to its exact `sha256:` manifest digest
shown by `plugins list`. Local connections require this binding before any tool, credential,
or process authority is compiled. A changed local snapshot needs an explicit new binding.
Installed and bundled connections use `null` (or omit the field). When switching sources,
configure credentials and tool permissions for the chosen source explicitly.

Workflow paths remain workspace-relative configuration. Workflows are not packaged or
activated as plugins.

See [Agent Plugins](../../extend/plugins.md), [Agent Plugin formats](../extension-formats.md),
and [Workflow schema](../workflow-schema.md).
