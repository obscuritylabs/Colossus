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

Workspace discovery examines `.agents/plugin.json` or immediate
`.agents/plugins/NAME/plugin.json` sources, plus directories explicitly registered
inside the workspace. Mixing the direct and collection layouts stops automatic
discovery and reports a conflict; previously accepted sources remain registered and
usable. The v1 specification defines directory loading; these discovery paths are a
Colossus convention.
Presence alone does not grant instruction or execution authority.

Automatic discovery is bounded to 128 sources, 128 collection entries, and 256 MiB of
cumulative validated source bytes, in deterministic path order. Failed captures do not
spend that shared budget; each attempt remains bounded and malformed manifests are
rejected before payload reads. Linked ancestry, linked files,
special files, and out-of-workspace paths are rejected. Accepted source grants bind the
workspace partition, relative directory path, platform directory identity, and manifest
name. Each run uses a deterministic OCI snapshot of captured bytes. Recovery uses the
exact cached digest and original workspace store; it never rereads mutable source bytes.

Workspace origin is unsigned even after acceptance. Installed origin records signature
evidence; bundled origin belongs to the executable. Accepted local sources can replace
an installed name for this workspace only; `colossus` remains reserved.

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

## Plugin paths and writable data

`${PLUGIN_ROOT}` and `${PLUGIN_DATA}` expand once, exactly, in MCP arguments,
environment values, and `cwd`. Reserved variables are assigned after manifest and
client overlays. The plugin root is immutable; writable data remains separate and
survives updates and disablement.

Installed packages use `$COLOSSUS_HOME/plugins/data/PLUGIN_NAME`. Workspace
sources use the corresponding data directory in their private workspace partition,
so they cannot inherit the installed package's data. Installed-package uninstall
preserves data unless `--purge-data` is explicit.

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
