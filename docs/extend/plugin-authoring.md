---
title: Create a plugin
description: Create a portable plugin with skills, resources, and optional MCP declarations, then package it for distribution.
audience: developer
type: how-to
---

# Create a plugin

Start with the [working local example](plugins.md#load-a-workspace-plugin). A plugin is a directory containing `plugin.json`; it can provide skills in `skills/NAME/SKILL.md`, resources, and a root `mcp.json`.

## Add components

Use the fixed portable layout:

```text
review-tools/
├── plugin.json
├── skills/
│   └── review/
│       ├── SKILL.md
│       ├── references/
│       │   └── checklist.md
│       └── scripts/
│           └── check.sh
└── mcp.json
```

Each skill needs `name` and `description` in YAML frontmatter. Its qualified ID
is `PLUGIN_NAME/SKILL_NAME`, for example `review-tools/review`. Colossus shows
metadata in the picker and loads the instruction body when the skill is selected
or explicitly previewed. It does not recursively search for other skills.

Put supporting documents in `references/` and executable helpers in `scripts/`.
Reference those files from `SKILL.md`. Text resources can be previewed; binary
resources remain listed by contained path. Scripts run through the ordinary
process tools and their approvals. The `allowed-tools` frontmatter is advisory.

To provide an MCP connection, create a root `mcp.json`:

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
  "mcpServers": {
    "docs": {
      "type": "streamable-http",
      "url": "https://mcp.example.com/api"
    }
  }
}
```

Its server ID is `review-tools/docs`. Credentials and OAuth belong to the client's
[connection settings](../reference/configuration/extensions.md), and every server
needs explicit enablement and an allowed tool list. Adding the plugin does not
start a server. Stdio and Streamable HTTP are supported; valid legacy SSE entries
are diagnosed independently as unsupported.

## Add an icon

Agent Plugins v1 uses client extensions for optional display assets. Add this
object to `plugin.json`, alongside its existing name and description:

```json
{
  "extensions": {
    "com.obscuritylabs.colossus": {
      "icon": "com.obscuritylabs.colossus/icon.png"
    }
  }
}
```

Put a square PNG at that contained path; 128 × 128 pixels is a useful size.
Source and normalized images must fit within 64 KiB and 512 × 512 pixels.
URLs, absolute paths, traversal, links, and SVG are rejected. An invalid icon
produces a diagnostic and falls back to a monogram without disabling valid skills.

Icons travel with the package and work offline. Colossus re-encodes pixels before
display and bounds each catalog to 2 MiB of retained icon data, 64 normalizations,
and 8 Mi decoded pixels. The bundled icon has reserved capacity. Once the budget
is exhausted, other plugins keep their metadata and use monograms.

## Package for distribution

```bash
colossus plugins validate .agents/plugins/review
colossus plugins package .agents/plugins/review --output ./review-tools.oci
```

Packaging creates a deterministic OCI layout. It does not sign the result. See [Distribute and trust plugins](plugin-distribution.md) for publishing, registry profiles, signature verification, and offline import. Exact schema, media types, and extraction constraints belong to [Agent Plugin formats](../reference/extension-formats.md).
