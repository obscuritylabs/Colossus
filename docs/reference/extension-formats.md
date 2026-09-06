---
title: Agent Plugin formats
description: Portable Agent Plugins, Agent Skills, MCP, and Colossus OCI media types.
audience: developer
type: reference
---

# Agent Plugin formats

Agent Plugins v1 is Colossus's only portable extension package. The upstream
[Agent Plugins specification](https://agent-plugins.org/specification) and
[Agent Skills specification](https://agentskills.io/specification) are authoritative for
portable payloads. Colossus bundles the upstream v1 JSON Schemas and never fetches schemas
while loading a plugin.

## Discovery

| Component | Exact location | Failure boundary |
| --- | --- | --- |
| Manifest | `plugin.json` | Invalid manifest rejects the plugin |
| Agent Skill | `skills/NAME/SKILL.md` | Invalid skill is skipped and diagnosed |
| MCP servers | `mcp.json` | Invalid document disables plugin MCP; invalid entries are skipped independently |

Unknown root manifest fields are reported and ignored as v1 requires. Unknown client
extensions and unrelated files are preserved as opaque package content. Discovery does
not recurse for nested skills or accept lowercase `skill.md`.

Every selected skill uses the canonical ID `PLUGIN_NAME/SKILL_NAME`. Agent Skills accept
the standard `name`, `description`, `license`, `compatibility`, `metadata`, and experimental
`allowed-tools` frontmatter only. Additional files are arbitrary contained resources.

MCP server IDs are `PLUGIN_NAME/SERVER_NAME`. Stdio and `streamable-http` are supported;
valid `sse` entries are independently diagnosed as unsupported. Portable manifests do not
carry credentials or OAuth. Those are workspace-owned overlays.

## OCI profile

One complete plugin is one OCI artifact:

| Field | Required value |
| --- | --- |
| Manifest media type | `application/vnd.oci.image.manifest.v1+json` |
| `artifactType` | `application/vnd.colossus.agent-plugin.v1` |
| Config | `application/vnd.colossus.agent-plugin.config.v1+json` |
| Single layer | `application/vnd.colossus.agent-plugin.content.v1.tar+gzip` |
| Archive root | Exactly `PLUGIN_NAME/` |

The OCI manifest digest is the installation identity. Layout indexes may contain multiple
candidate manifests only when import supplies the exact digest. An OCI image index cannot
be used as a plugin manifest.

Archives are sorted and normalize uid, gid, mtime, and modes; gzip timestamps are zero.
Extraction accepts regular files and directories only, validates every descriptor digest
and size, and applies the limits documented in [Output and limits](output-environment-limits.md).

OCI 1.1 referrer manifests carry standard Sigstore/Cosign bundles and attestations. Air-gap
layout tar files include those referrers without introducing a Colossus signature format.

Release/offline executable bundles are a separate retained distribution surface; see
[Release bundle format](bundle-format.md). Native integrations, workflows, and standalone
configured MCP servers are not Agent Plugin payloads.
